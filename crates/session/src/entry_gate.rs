//! When the local client may leave the loading screen, and what the loading
//! screen does about a load that failed.

use assets::{LoadBackdrop, MapLoadProcess};
use bevy::prelude::*;
use frame::{ShellScreen, ClientStep, LevelPresent, Headless, ProcessDuty};
use net::{HostLoadingPause, PlayerEntryTicket, JoinSequenceStep, JoinProgressRecord};
use render_frontend::prepare::scene::world::LevelStage;

use crate::RunningWorldLabel;

/// The world is drawn and the sound is ready (or there is no screen or
/// speaker to wait for).
fn display_prepared(no_screen: bool, scene_spawned: bool, sound_ready: bool) -> bool {
    no_screen || scene_spawned && sound_ready
}

/// Whether this process's player may pick a class. A remote client waits for
/// the server to say so; a process that is the server only waits for itself;
/// a replay has nobody to ask.
fn admitted_for(
    role: ProcessDuty,
    remote_says_yes: bool,
    self_says_yes: bool,
    can_be_shown: bool,
) -> bool {
    match role {
        ProcessDuty::Replay => can_be_shown,
        ProcessDuty::Client => remote_says_yes,
        ProcessDuty::Listen | ProcessDuty::Dedicated => self_says_yes,
    }
}

#[allow(clippy::too_many_arguments)]
fn update_admission(
    mut signon: ResMut<JoinProgressRecord>,
    mut gate: ResMut<PlayerEntryTicket>,
    role: Res<ProcessDuty>,
    load_hold: Option<Res<HostLoadingPause>>,
    world_flag: Option<Res<LevelPresent>>,
    world_scene: Option<Res<LevelStage>>,
    sound: Option<Res<audio::SoundSystemLive>>,
    mut on_screen: Option<ResMut<RunningWorldLabel>>,
    headless: Option<Res<Headless>>,
) {
    // The first install of a load carries no match key yet; take the one the
    // admission recorded for it.
    if let (Some(current), Some(recorded)) = (on_screen.as_mut(), gate.core.installed())
        && current.decrypt_secret.own_level_ticket_no == recorded.own_level_ticket_no
        && current.decrypt_secret.game_session_key.is_none()
    {
        current.decrypt_secret = recorded;
    }
    let audio_ready = sound.is_none_or(|flag| flag.0);
    let scene_spawned = world_scene.is_some_and(|drawn| drawn.spawned);
    let ready = display_prepared(headless.is_some(), scene_spawned, audio_ready);
    if let Some(current) = on_screen.as_ref().filter(|_| ready) {
        gate.core.take_shown_level(current.decrypt_secret);
    }
    let world_installed = world_flag.is_some_and(|flag| flag.0);
    let held = load_hold.is_some_and(|frozen| frozen.0);
    gate.core.apply_local_authority_ready(world_installed && !held);
    let cleared = admitted_for(
        *role,
        gate.core.loadout_pick_permitted(),
        gate.core.self_loadout_pick_permitted(),
        ready,
    );
    if signon.admitted == cleared {
        return;
    }
    signon.admitted = cleared;
    let phase = &signon.phase;
    diag::info!(
        Sim,
        "admission admitted={cleared} role={:?} phase={phase:?} \
         presentation={ready} audio={audio_ready}",
        *role
    );
}

/// What the load screen should do this frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LoadStep {
    /// The load failed: show why and go back to the menu.
    Abandon,
    /// Not yet; `waiting_on_admission` says the world is up and only the
    /// session's go-ahead is missing.
    Wait { waiting_on_admission: bool },
    /// Done: end the load and move on to class select.
    Finish,
}

fn load_step(broke: bool, go_ahead: bool, world_installed: bool) -> LoadStep {
    if broke {
        return LoadStep::Abandon;
    }
    if go_ahead && world_installed {
        return LoadStep::Finish;
    }
    LoadStep::Wait {
        waiting_on_admission: world_installed,
    }
}

/// The screen to show after a step; `None` leaves it alone. `backdrop` is a
/// front-end load nobody started a match for (bo2zm's ZOMBIES button opens
/// on the map behind the menu): it ends on the menu, not in the match.
fn screen_after(step: LoadStep, shown: ShellScreen, backdrop: bool) -> Option<ShellScreen> {
    use ShellScreen::{KitPicker, InGame, Loading, MainMenu};
    match (step, shown) {
        (LoadStep::Abandon, KitPicker | InGame) => Some(MainMenu),
        (LoadStep::Finish, Loading) if backdrop => Some(MainMenu),
        (LoadStep::Finish, MainMenu) if backdrop => None,
        (LoadStep::Finish, Loading | MainMenu) => Some(KitPicker),
        _ => None,
    }
}

fn step_loadout_screen(
    mut current_screen: ResMut<ShellScreen>,
    mut overlay: Option<ResMut<LoadBackdrop>>,
    mut process: Option<ResMut<MapLoadProcess>>,
    signon: Res<JoinProgressRecord>,
    world_flag: Option<Res<LevelPresent>>,
) {
    let world_installed = world_flag.is_some_and(|flag| flag.0);
    let broken = signon.phase.has_failed();
    let step = load_step(broken, signon.can_choose_loadout(), world_installed);
    match step {
        LoadStep::Abandon => {
            if let (Some(overlay), JoinSequenceStep::Failed(cause)) =
                (overlay.as_deref_mut(), &signon.phase)
                && overlay.failure().is_none()
            {
                overlay.fail(cause.to_string());
            }
            if let Some(process) = process.as_deref_mut() {
                process.fail();
            }
        }
        LoadStep::Wait {
            waiting_on_admission,
        } => {
            // Preparing locally can end long before the session lets this
            // client go on; the load table names that wait as its own row.
            if let Some(process) = process.as_deref_mut().filter(|_| waiting_on_admission) {
                process.await_admission();
            }
        }
        LoadStep::Finish => {
            // bo2zm: a loading screen that asked to stay up a little longer.
            if overlay.as_deref().is_some_and(LoadBackdrop::held) {
                return;
            }
            if let Some(shown) = overlay.as_deref() {
                diag::info!(Ui, "loading: load done at {} ms", shown.elapsed().as_millis());
            }
            if let Some(process) = process.as_deref_mut() {
                process.finish();
            }
            // The overlay comes down whatever the screen is: a replay is
            // already in game by now, and an overlay left up would hide the
            // whole demo.
            if let Some(overlay) = overlay.as_deref_mut() {
                overlay.finish();
            }
        }
    }
    let backdrop = frame::bo2_frontend_waits_on_menu();
    if let Some(next) = screen_after(step, *current_screen, backdrop) {
        *current_screen = next;
    }
}

pub(crate) fn install(host: &mut App) {
    const PRESENT: ClientStep = ClientStep::Present;
    let admission_system = update_admission
        .in_set(PRESENT)
        .before(crate::self_arm::prime_self_from_displayed);
    host.add_systems(bevy::app::Update, admission_system);
    host.add_systems(bevy::app::Update, step_loadout_screen.in_set(ClientStep::Ui));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn headless_runs_need_no_scene_or_sound() {
        assert!(display_prepared(true, false, false));
    }

    #[test]
    fn a_windowed_run_needs_the_scene_and_the_sound() {
        assert!(display_prepared(false, true, true));
        assert!(!display_prepared(false, false, true));
        assert!(!display_prepared(false, true, false));
    }

    #[test]
    fn a_remote_client_waits_for_the_server() {
        assert!(!admitted_for(ProcessDuty::Client, false, true, true));
        assert!(admitted_for(ProcessDuty::Client, true, false, false));
    }

    #[test]
    fn a_server_process_waits_only_for_itself() {
        for role in [ProcessDuty::Listen, ProcessDuty::Dedicated] {
            assert!(admitted_for(role, false, true, false));
            assert!(!admitted_for(role, true, false, true));
        }
    }

    #[test]
    fn a_replay_is_admitted_once_it_can_be_shown() {
        assert!(admitted_for(ProcessDuty::Replay, false, false, true));
        assert!(!admitted_for(ProcessDuty::Replay, true, true, false));
    }

    #[test]
    fn a_failed_load_is_abandoned_whatever_else_is_true() {
        assert_eq!(load_step(true, true, true), LoadStep::Abandon);
    }

    #[test]
    fn without_the_world_the_load_waits_but_is_not_waiting_on_admission() {
        let step = load_step(false, true, false);
        assert_eq!(
            step,
            LoadStep::Wait {
                waiting_on_admission: false
            }
        );
    }

    #[test]
    fn with_the_world_but_no_go_ahead_the_load_waits_on_admission() {
        let step = load_step(false, false, true);
        assert_eq!(
            step,
            LoadStep::Wait {
                waiting_on_admission: true
            }
        );
    }

    #[test]
    fn world_and_go_ahead_finish_the_load() {
        assert_eq!(load_step(false, true, true), LoadStep::Finish);
    }

    #[test]
    fn finishing_moves_loading_and_menu_to_class_select_only() {
        let done = LoadStep::Finish;
        let class_select = Some(ShellScreen::KitPicker);
        assert_eq!(screen_after(done, ShellScreen::Loading, false), class_select);
        assert_eq!(screen_after(done, ShellScreen::MainMenu, false), class_select);
        assert_eq!(screen_after(done, frame::ShellScreen::InGame, false), None);
        assert_eq!(screen_after(done, ShellScreen::KitPicker, false), None);
    }

    #[test]
    fn a_front_end_backdrop_load_stays_on_the_menu() {
        let done = LoadStep::Finish;
        let menu = Some(ShellScreen::MainMenu);
        assert_eq!(screen_after(done, ShellScreen::Loading, true), menu);
        assert_eq!(screen_after(done, ShellScreen::MainMenu, true), None);
        assert_eq!(screen_after(done, ShellScreen::KitPicker, true), None);
    }

    #[test]
    fn abandoning_sends_a_player_in_the_match_back_to_the_menu() {
        let give_up = LoadStep::Abandon;
        let menu = Some(ShellScreen::MainMenu);
        assert_eq!(screen_after(give_up, frame::ShellScreen::InGame, false), menu);
        assert_eq!(screen_after(give_up, ShellScreen::KitPicker, false), menu);
        assert_eq!(screen_after(give_up, ShellScreen::Loading, false), None);
    }

    #[test]
    fn waiting_never_changes_the_screen() {
        let wait = LoadStep::Wait {
            waiting_on_admission: true,
        };
        assert_eq!(screen_after(wait, ShellScreen::Loading, false), None);
    }
}
