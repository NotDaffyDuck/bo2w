//! Putting the local player into the match: arming the camera and controls
//! when the first live state arrives, asking to join from class select, and
//! where the camera sits until then.

use bevy::prelude::*;
use net::{
    ActionRequestCounter, HostInputValve, HostLoadingPause, PlayerActionIntake,
    ClientGuessState, ClientStep, HostViewClient, ViewAimAccumulator, ShownWorldFrame,
    JoinProgressRecord, aim_angles_from_degrees,
};
use render_frontend::adapters::motion::gun_present::OwnSpawnPrimed;
use sim::{PlayerIntent, PlayerSessionStage};
use frame::{ProcessDuty, LevelPresent, ShellScreen, StartupReport, KitEquipDone};
use render_frontend::prepare::scene::camera::{GameplayCamera, FreeRoamCamera, transform_from_game_view, WorldEyePlacement};

/// Whether the first live state may arm the player yet: the world has to be
/// installed, and a process that plays (anything but a replay) has to have
/// been let through to class select. With no role recorded, only the world
/// is asked for.
fn may_arm(level_ready: bool, role: Option<ProcessDuty>, class_select_open: bool) -> bool {
    if !level_ready {
        return false;
    }
    let plays = matches!(role, Some(found) if found != ProcessDuty::Replay);
    !plays || class_select_open
}

/// The angles the player is looking along: the view minus what the server
/// has already turned them by.
fn look_degrees(view: [f32; 3], turned_by: [f32; 3]) -> [f32; 3] {
    std::array::from_fn(|axis| view[axis] - turned_by[axis])
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn prime_self_from_displayed(
    snapshot: Res<ShownWorldFrame>,
    me: Res<HostViewClient>,
    world_flag: Option<Res<LevelPresent>>,
    signon: Res<JoinProgressRecord>,
    mut spawn_armed: ResMut<OwnSpawnPrimed>,
    mut current_screen: ResMut<ShellScreen>,
    mut camera_state: ResMut<GameplayCamera>,
    mut look: ResMut<ViewAimAccumulator>,
    mut valve: ResMut<HostInputValve>,
    runtime_role: Option<Res<ProcessDuty>>,
) {
    if spawn_armed.0 {
        return;
    }
    let role = runtime_role.as_deref().copied();
    let level_ready = world_flag.is_some_and(|flag| flag.0);
    if !may_arm(level_ready, role, signon.can_choose_loadout()) {
        return;
    }
    let Some(player) = snapshot.living_participant(me.0) else {
        return;
    };

    let degrees = look_degrees(player.look_angles, player.view_offset_angles);
    look.angles = aim_angles_from_degrees(degrees);
    camera_state.enabled = true;
    camera_state.hold_free_camera = false;
    // A replay only watches: what the viewer presses never reaches the match.
    if role != Some(ProcessDuty::Replay) {
        valve.local_cmds_enabled = true;
    }
    *current_screen = ShellScreen::InGame;
    spawn_armed.0 = true;

    let [x, y, z] = player.origin;
    diag::info!(
        ViewModelPerspective,
        "spawn: presented Alive at [{x:.1}, {y:.1}, {z:.1}] yaw={:.0} weapon={}",
        player.look_angles[1],
        player.weapon
    );
}

/// What the join system should do this frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum JoinStep {
    /// Not on class select: forget any request in flight.
    Reset,
    /// On class select but not allowed to ask yet.
    Hold,
    /// The server already lists the client past connecting.
    Landed,
    /// Ask, unless the last request is still waiting to be sent.
    Ask,
}

fn join_step(
    screen: ShellScreen,
    role: Option<ProcessDuty>,
    class_select_open: Option<bool>,
    load_held: bool,
    past_connecting: bool,
) -> JoinStep {
    if screen != ShellScreen::KitPicker {
        return JoinStep::Reset;
    }
    let barred = role == Some(ProcessDuty::Replay) || class_select_open == Some(false);
    if barred || load_held {
        return JoinStep::Hold;
    }
    // The authority lists a client as connecting until it has joined, so any
    // later state means the request landed.
    if past_connecting {
        return JoinStep::Landed;
    }
    JoinStep::Ask
}

#[allow(clippy::too_many_arguments)]
fn enter_self_at_loadout_pick(
    current_screen: Res<ShellScreen>,
    snapshot: Res<ShownWorldFrame>,
    me: Res<HostViewClient>,
    mut inbox: ResMut<PlayerActionIntake>,
    mut ids: ResMut<ActionRequestCounter>,
    runtime_role: Option<Res<ProcessDuty>>,
    load_hold: Option<Res<HostLoadingPause>>,
    signon: Option<Res<JoinProgressRecord>>,
    mut awaiting: Local<Option<sim::IntentSerial>>,
) {
    let past_connecting = snapshot
        .snapshot()
        .and_then(|view| view.meta.for_client(me.0))
        .is_some_and(|meta| meta.lifecycle != PlayerSessionStage::Connecting);
    let step = join_step(
        *current_screen,
        runtime_role.as_deref().copied(),
        signon.map(|state| state.can_choose_loadout()),
        load_hold.is_some_and(|held| held.0),
        past_connecting,
    );
    if matches!(step, JoinStep::Reset | JoinStep::Landed) {
        *awaiting = None;
        return;
    }
    if step == JoinStep::Hold {
        return;
    }
    if let Some(earlier) = *awaiting {
        let still_queued = inbox
            .awaiting_from(me.0)
            .any(|queued| sim::intent_serial(&queued) == earlier);
        if still_queued {
            return;
        }
        *awaiting = None;
    }
    let ask_id = ids.allocate();
    if let Err(error) = inbox.push(me.0, PlayerIntent::JoinMatch { request_id: ask_id }) {
        diag::warn!(ViewModelPerspective, "class select: JoinMatch not queued - {error}");
        return;
    }
    *awaiting = Some(ask_id);
    diag::info!(
        ViewModelPerspective,
        "class select: queued JoinMatch request_id={ask_id} \
         (meta-only; spectator pm_type is a named gap)"
    );
}

/// Behind class select the camera sits where the match script put the local
/// client: connecting runs the script's spectator spawn, which places the
/// player at a pose of its choosing. That pose is in the presented
/// spectator state, and the camera follows it until the player spawns alive.
fn spectator_camera_from_presented(
    current_screen: Res<ShellScreen>,
    snapshot: Res<ShownWorldFrame>,
    me: Res<HostViewClient>,
    camera_state: Res<GameplayCamera>,
    mut cameras: Query<&mut Transform, With<FreeRoamCamera>>,
    mut already_logged: Local<bool>,
) {
    if *current_screen != ShellScreen::KitPicker || camera_state.enabled {
        return;
    }
    if snapshot.living_participant(me.0).is_some() {
        return;
    }
    let Some(spectator) = snapshot.player(me.0) else {
        return;
    };
    // A spectator has no spot height: the view is the spawn origin itself.
    let spot = transform_from_game_view(WorldEyePlacement {
        origin: spectator.origin,
        turn: spectator.look_angles,
    });
    for mut view in &mut cameras {
        view.translation = spot.translation;
        view.rotation = spot.rotation;
    }
    if *already_logged || spectator.origin == [0.0; 3] {
        return;
    }
    *already_logged = true;
    diag::info!(
        ViewModelPerspective,
        "class select: camera follows the script spectator at {:?} angles {:?}",
        spectator.origin,
        spectator.look_angles
    );
}

fn sync_prediction_metrics_to_probe(
    predicted: Res<ClientGuessState>,
    launch_report: Option<ResMut<StartupReport>>,
) {
    if !predicted.is_changed() {
        return;
    }
    let Some(mut launch_report) = launch_report else {
        return;
    };
    launch_report.forecast_figures = Some(predicted.0.metrics().summary_text());
}

pub(crate) fn install(host: &mut App) {
    const PRESENT: ClientStep = ClientStep::Present;
    const DIAG: ClientStep = ClientStep::Diag;
    let present_group = (
        prime_self_from_displayed.after(KitEquipDone),
        enter_self_at_loadout_pick,
        spectator_camera_from_presented,
    );
    host.add_systems(bevy::app::Update, present_group.in_set(PRESENT));
    host.add_systems(
        bevy::app::Update,
        sync_prediction_metrics_to_probe.in_set(DIAG),
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use frame::{ProcessDuty, ShellScreen};

    const ALLOWED: Option<bool> = Some(true);
    const BARRED: Option<bool> = Some(false);

    #[test]
    fn nobody_is_armed_without_a_world() {
        assert!(!may_arm(false, Some(ProcessDuty::Replay), true));
        assert!(!may_arm(false, None, true));
    }

    #[test]
    fn a_player_waits_for_class_select_and_a_replay_does_not() {
        for role in [
            ProcessDuty::Client,
            ProcessDuty::Listen,
            ProcessDuty::Dedicated,
        ] {
            assert!(!may_arm(true, Some(role), false));
            assert!(may_arm(true, Some(role), true));
        }
        assert!(may_arm(true, Some(ProcessDuty::Replay), false));
    }

    #[test]
    fn with_no_role_recorded_only_the_world_is_needed() {
        assert!(may_arm(true, None, false));
    }

    #[test]
    fn the_look_starts_at_the_view_less_the_servers_turn() {
        let looking = look_degrees([10.0, 90.0, 0.0], [4.0, 30.0, 0.0]);
        assert_eq!(looking, [6.0, 60.0, 0.0]);
    }

    #[test]
    fn leaving_class_select_forgets_the_request() {
        for other in [ShellScreen::MainMenu, ShellScreen::Loading, ShellScreen::InGame] {
            let step = join_step(other, None, ALLOWED, false, false);
            assert_eq!(step, JoinStep::Reset);
        }
    }

    #[test]
    fn nothing_is_asked_while_held_back() {
        let on = ShellScreen::KitPicker;
        let replaying = join_step(on, Some(ProcessDuty::Replay), ALLOWED, false, false);
        assert_eq!(replaying, JoinStep::Hold);
        let not_allowed = join_step(on, Some(ProcessDuty::Client), BARRED, false, false);
        assert_eq!(not_allowed, JoinStep::Hold);
        let load_held = join_step(on, Some(ProcessDuty::Client), ALLOWED, true, false);
        assert_eq!(load_held, JoinStep::Hold);
    }

    #[test]
    fn a_client_the_server_has_moved_on_has_landed() {
        let step = join_step(ShellScreen::KitPicker, None, None, false, true);
        assert_eq!(step, JoinStep::Landed);
    }

    #[test]
    fn a_connecting_client_asks_to_join() {
        let role = Some(ProcessDuty::Listen);
        let step = join_step(ShellScreen::KitPicker, role, ALLOWED, false, false);
        assert_eq!(step, JoinStep::Ask);
    }

    #[test]
    fn no_signon_state_does_not_block_the_ask() {
        let step = join_step(ShellScreen::KitPicker, None, None, false, false);
        assert_eq!(step, JoinStep::Ask);
    }
}
