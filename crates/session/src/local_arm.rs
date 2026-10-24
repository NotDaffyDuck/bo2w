//! Putting the local player into the match: arming the camera and controls
//! when the first live state arrives, asking to join from class select, and
//! where the camera sits until then.

use bevy::prelude::*;
use frame::{AppScreen, ClassEquipResolved, HasWorld, LaunchReport, RuntimeRole};
use net::{
    ActionRequestIds, AuthorityInputGate, AuthorityLoadHold, ClientActionInbox,
    ClientPredictionState, ClientSet, LocalPresentClient, LookState, PresentedSnapshot,
    SignonState, look_angles_from_degrees,
};
use render_frontend::adapters::anim::fpv_present::LocalSpawnArmed;
use render_frontend::prepare::scene::camera::{
    FlyCamera, SimCamera, WorldCameraPose, transform_from_iw_view,
};
use sim::{ClientAction, ClientLifecycle};

/// Whether the first live state may arm the player yet: the world has to be
/// installed, and a process that plays (anything but a replay) has to have
/// been let through to class select. With no role recorded, only the world
/// is asked for.
fn may_arm(world_installed: bool, role: Option<RuntimeRole>, may_select_class: bool) -> bool {
    let needs_class_select = role.is_some_and(|role| role != RuntimeRole::Replay);
    world_installed && (may_select_class || !needs_class_select)
}

/// The angles the player is looking along: the view minus what the server
/// has already turned them by.
fn look_degrees(view: [f32; 3], delta: [f32; 3]) -> [f32; 3] {
    std::array::from_fn(|axis| view[axis] - delta[axis])
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn arm_local_from_presented(
    presented: Res<PresentedSnapshot>,
    local: Res<LocalPresentClient>,
    has_world: Option<Res<HasWorld>>,
    signon: Res<SignonState>,
    mut armed: ResMut<LocalSpawnArmed>,
    mut screen: ResMut<AppScreen>,
    mut sim_cam: ResMut<SimCamera>,
    mut look: ResMut<LookState>,
    mut input_gate: ResMut<AuthorityInputGate>,
    role: Option<Res<RuntimeRole>>,
) {
    if armed.0 {
        return;
    }
    let role = role.as_deref().copied();
    let world_installed = has_world.is_some_and(|world| world.0);
    if !may_arm(world_installed, role, signon.may_select_class()) {
        return;
    }
    let Some(ps) = presented.alive_player(local.0) else {
        return;
    };

    look.angles = look_angles_from_degrees(look_degrees(ps.viewangles, ps.delta_angles));
    sim_cam.enabled = true;
    sim_cam.freeze_fly = false;
    // A replay watches; nothing the viewer presses goes to the match.
    if role != Some(RuntimeRole::Replay) {
        input_gate.local_cmds_enabled = true;
    }
    *screen = AppScreen::InGame;
    armed.0 = true;

    diag::info!(
        Fpv,
        "spawn: presented Alive at [{:.1}, {:.1}, {:.1}] yaw={:.0} weapon={}",
        ps.origin[0],
        ps.origin[1],
        ps.origin[2],
        ps.viewangles[1],
        ps.weapon
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
    screen: AppScreen,
    role: Option<RuntimeRole>,
    may_select_class: Option<bool>,
    load_held: bool,
    past_connecting: bool,
) -> JoinStep {
    if screen != AppScreen::ClassSelect {
        JoinStep::Reset
    } else if role == Some(RuntimeRole::Replay) || may_select_class == Some(false) || load_held {
        JoinStep::Hold
    } else if past_connecting {
        // The authority lists a client as connecting until it has joined, so
        // any later state means the request landed.
        JoinStep::Landed
    } else {
        JoinStep::Ask
    }
}

#[allow(clippy::too_many_arguments)]
fn join_local_on_class_select(
    screen: Res<AppScreen>,
    presented: Res<PresentedSnapshot>,
    local: Res<LocalPresentClient>,
    mut actions: ResMut<ClientActionInbox>,
    mut request_ids: ResMut<ActionRequestIds>,
    role: Option<Res<RuntimeRole>>,
    hold: Option<Res<AuthorityLoadHold>>,
    signon: Option<Res<SignonState>>,
    mut in_flight: Local<Option<sim::ActionRequestId>>,
) {
    let past_connecting = presented
        .snapshot()
        .and_then(|snapshot| snapshot.meta.for_client(local.0))
        .is_some_and(|meta| meta.lifecycle != ClientLifecycle::Connecting);
    let step = join_step(
        *screen,
        role.as_deref().copied(),
        signon.map(|signon| signon.may_select_class()),
        hold.is_some_and(|hold| hold.0),
        past_connecting,
    );
    match step {
        JoinStep::Reset | JoinStep::Landed => *in_flight = None,
        JoinStep::Hold => {}
        JoinStep::Ask => {
            if let Some(request_id) = *in_flight {
                let still_queued = actions
                    .pending_for(local.0)
                    .any(|action| sim::action_request_id(&action) == request_id);
                if still_queued {
                    return;
                }
                *in_flight = None;
            }
            let request_id = request_ids.allocate();
            match actions.push(local.0, ClientAction::JoinMatch { request_id }) {
                Ok(()) => {
                    *in_flight = Some(request_id);
                    diag::info!(
                        Fpv,
                        "class select: queued JoinMatch request_id={request_id} \
                         (meta-only; spectator pm_type is a named gap)"
                    );
                }
                Err(error) => diag::warn!(Fpv, "class select: JoinMatch not queued - {error}"),
            }
        }
    }
}

/// Behind class select the camera sits where the match script put the local
/// client: connecting runs the script's spectator spawn, which places the
/// player at a pose of its choosing. That pose is in the presented
/// spectator state, and the camera follows it until the player spawns alive.
fn spectator_camera_from_presented(
    screen: Res<AppScreen>,
    presented: Res<PresentedSnapshot>,
    local: Res<LocalPresentClient>,
    sim_cam: Res<SimCamera>,
    mut cameras: Query<&mut Transform, With<FlyCamera>>,
    mut logged: Local<bool>,
) {
    if *screen != AppScreen::ClassSelect || sim_cam.enabled {
        return;
    }
    if presented.alive_player(local.0).is_some() {
        return;
    }
    let Some(ps) = presented.player(local.0) else {
        return;
    };
    // A spectator has no eye height: the view is the spawn origin itself.
    let eye = transform_from_iw_view(WorldCameraPose {
        origin: ps.origin,
        angles: ps.viewangles,
    });
    for mut transform in &mut cameras {
        transform.translation = eye.translation;
        transform.rotation = eye.rotation;
    }
    if !*logged && ps.origin != [0.0; 3] {
        *logged = true;
        diag::info!(
            Fpv,
            "class select: camera follows the script spectator at {:?} angles {:?}",
            ps.origin,
            ps.viewangles
        );
    }
}

fn sync_prediction_metrics_to_probe(
    prediction: Res<ClientPredictionState>,
    report: Option<ResMut<LaunchReport>>,
) {
    if !prediction.is_changed() {
        return;
    }
    if let Some(mut report) = report {
        report.prediction_metrics = Some(prediction.0.metrics().report_line());
    }
}

pub(crate) fn install(app: &mut App) {
    app.add_systems(
        Update,
        (
            arm_local_from_presented.after(ClassEquipResolved),
            join_local_on_class_select,
            spectator_camera_from_presented,
        )
            .in_set(ClientSet::Present),
    )
    .add_systems(
        Update,
        sync_prediction_metrics_to_probe.in_set(ClientSet::Diag),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nobody_is_armed_without_a_world() {
        assert!(!may_arm(false, Some(RuntimeRole::Replay), true));
        assert!(!may_arm(false, None, true));
    }

    #[test]
    fn a_player_waits_for_class_select_and_a_replay_does_not() {
        for role in [
            RuntimeRole::Client,
            RuntimeRole::Listen,
            RuntimeRole::Dedicated,
        ] {
            assert!(!may_arm(true, Some(role), false));
            assert!(may_arm(true, Some(role), true));
        }
        assert!(may_arm(true, Some(RuntimeRole::Replay), false));
    }

    #[test]
    fn with_no_role_recorded_only_the_world_is_needed() {
        assert!(may_arm(true, None, false));
    }

    #[test]
    fn the_look_starts_at_the_view_less_the_servers_turn() {
        assert_eq!(
            look_degrees([10.0, 90.0, 0.0], [4.0, 30.0, 0.0]),
            [6.0, 60.0, 0.0]
        );
    }

    #[test]
    fn leaving_class_select_forgets_the_request() {
        for screen in [AppScreen::MainMenu, AppScreen::Loading, AppScreen::InGame] {
            assert_eq!(
                join_step(screen, None, Some(true), false, false),
                JoinStep::Reset
            );
        }
    }

    #[test]
    fn nothing_is_asked_while_held_back() {
        let screen = AppScreen::ClassSelect;
        assert_eq!(
            join_step(screen, Some(RuntimeRole::Replay), Some(true), false, false),
            JoinStep::Hold
        );
        assert_eq!(
            join_step(screen, Some(RuntimeRole::Client), Some(false), false, false),
            JoinStep::Hold
        );
        assert_eq!(
            join_step(screen, Some(RuntimeRole::Client), Some(true), true, false),
            JoinStep::Hold
        );
    }

    #[test]
    fn a_client_the_server_has_moved_on_has_landed() {
        assert_eq!(
            join_step(AppScreen::ClassSelect, None, None, false, true),
            JoinStep::Landed
        );
    }

    #[test]
    fn a_connecting_client_asks_to_join() {
        assert_eq!(
            join_step(
                AppScreen::ClassSelect,
                Some(RuntimeRole::Listen),
                Some(true),
                false,
                false
            ),
            JoinStep::Ask
        );
    }

    #[test]
    fn no_signon_state_does_not_block_the_ask() {
        assert_eq!(
            join_step(AppScreen::ClassSelect, None, None, false, false),
            JoinStep::Ask
        );
    }
}
