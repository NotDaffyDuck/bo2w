//! When the local client may leave the loading screen, and what the loading
//! screen does about a load that failed.

use assets::{LoadingScreen, MapLoadProcess};
use bevy::prelude::*;
use frame::{AppScreen, ClientSet, HasWorld, Headless, RuntimeRole};
use net::{AuthorityLoadHold, ClientAdmission, SignonPhase, SignonState};
use render_frontend::prepare::scene::world::WorldScene;

use crate::LiveWorldIdentity;

/// The world is drawn and the sound is ready (or there is no screen or
/// speaker to wait for).
fn presentation_ready(headless: bool, scene_spawned: bool, audio_ready: bool) -> bool {
    headless || (scene_spawned && audio_ready)
}

/// Whether this process's player may pick a class. A remote client waits for
/// the server to say so; a process that is the server only waits for itself;
/// a replay has nobody to ask.
fn admitted_for(
    role: RuntimeRole,
    server_allows: bool,
    local_allows: bool,
    presentation_ready: bool,
) -> bool {
    match role {
        RuntimeRole::Client => server_allows,
        RuntimeRole::Listen | RuntimeRole::Dedicated => local_allows,
        RuntimeRole::Replay => presentation_ready,
    }
}

#[allow(clippy::too_many_arguments)]
fn update_admission(
    mut signon: ResMut<SignonState>,
    mut admission: ResMut<ClientAdmission>,
    role: Res<RuntimeRole>,
    hold: Option<Res<AuthorityLoadHold>>,
    has_world: Option<Res<HasWorld>>,
    scene: Option<Res<WorldScene>>,
    audio: Option<Res<audio::AudioReady>>,
    mut live: Option<ResMut<LiveWorldIdentity>>,
    headless: Option<Res<Headless>>,
) {
    // The first install of a load carries no match key yet; take the one the
    // admission recorded for it.
    if let (Some(live), Some(installed)) = (live.as_mut(), admission.core.installed())
        && live.load_key.local_load_request_id == installed.local_load_request_id
        && live.load_key.match_key.is_none()
    {
        live.load_key = installed;
    }
    let audio_ready = audio.is_none_or(|ready| ready.0);
    let scene_spawned = scene.is_some_and(|scene| scene.spawned);
    let ready = presentation_ready(headless.is_some(), scene_spawned, audio_ready);
    if ready && let Some(live) = live.as_ref() {
        admission.core.apply_presentation(live.load_key);
    }
    let world_installed = has_world.is_some_and(|world| world.0);
    let holding = hold.is_some_and(|hold| hold.0);
    admission
        .core
        .apply_local_authority_ready(!holding && world_installed);
    let admitted = admitted_for(
        *role,
        admission.core.class_select_allowed(),
        admission.core.local_class_select_allowed(),
        ready,
    );
    if signon.admitted != admitted {
        signon.admitted = admitted;
        diag::info!(
            Sim,
            "admission admitted={admitted} role={:?} phase={:?} presentation={ready} audio={audio_ready}",
            *role,
            signon.phase
        );
    }
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

fn load_step(failed: bool, admitted: bool, world_installed: bool) -> LoadStep {
    if failed {
        LoadStep::Abandon
    } else if !admitted || !world_installed {
        LoadStep::Wait {
            waiting_on_admission: world_installed,
        }
    } else {
        LoadStep::Finish
    }
}

/// The screen to show after a step; `None` leaves it alone.
fn screen_after(step: LoadStep, screen: AppScreen) -> Option<AppScreen> {
    match (step, screen) {
        (LoadStep::Abandon, AppScreen::ClassSelect | AppScreen::InGame) => {
            Some(AppScreen::MainMenu)
        }
        (LoadStep::Finish, AppScreen::Loading | AppScreen::MainMenu) => {
            Some(AppScreen::ClassSelect)
        }
        _ => None,
    }
}

fn drive_class_select_screen(
    mut screen: ResMut<AppScreen>,
    mut loading: Option<ResMut<LoadingScreen>>,
    mut load: Option<ResMut<MapLoadProcess>>,
    signon: Res<SignonState>,
    has_world: Option<Res<HasWorld>>,
) {
    let world_installed = has_world.is_some_and(|world| world.0);
    let step = load_step(
        signon.phase.is_failed(),
        signon.may_select_class(),
        world_installed,
    );
    match step {
        LoadStep::Abandon => {
            if let (Some(loading), SignonPhase::Failed(reason)) =
                (loading.as_deref_mut(), &signon.phase)
                && loading.failure().is_none()
            {
                loading.fail(reason.to_string());
            }
            if let Some(load) = load.as_deref_mut() {
                load.fail();
            }
        }
        LoadStep::Wait {
            waiting_on_admission,
        } => {
            // Preparing locally can end long before the session lets this
            // client go on; the load table names that wait as its own row.
            if waiting_on_admission && let Some(load) = load.as_deref_mut() {
                load.await_admission();
            }
        }
        LoadStep::Finish => {
            if let Some(load) = load.as_deref_mut() {
                load.finish();
            }
            // The overlay comes down whatever the screen is: a replay is
            // already in game by now, and an overlay left up would hide the
            // whole demo.
            if let Some(loading) = loading.as_deref_mut() {
                loading.finish();
            }
        }
    }
    if let Some(next) = screen_after(step, *screen) {
        *screen = next;
    }
}

pub(crate) fn install(app: &mut App) {
    app.add_systems(
        Update,
        update_admission
            .in_set(ClientSet::Present)
            .before(crate::local_arm::arm_local_from_presented),
    )
    .add_systems(Update, drive_class_select_screen.in_set(ClientSet::Ui));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn headless_runs_need_no_scene_or_sound() {
        assert!(presentation_ready(true, false, false));
    }

    #[test]
    fn a_windowed_run_needs_the_scene_and_the_sound() {
        assert!(presentation_ready(false, true, true));
        assert!(!presentation_ready(false, false, true));
        assert!(!presentation_ready(false, true, false));
    }

    #[test]
    fn a_remote_client_waits_for_the_server() {
        assert!(!admitted_for(RuntimeRole::Client, false, true, true));
        assert!(admitted_for(RuntimeRole::Client, true, false, false));
    }

    #[test]
    fn a_server_process_waits_only_for_itself() {
        for role in [RuntimeRole::Listen, RuntimeRole::Dedicated] {
            assert!(admitted_for(role, false, true, false));
            assert!(!admitted_for(role, true, false, true));
        }
    }

    #[test]
    fn a_replay_is_admitted_once_it_can_be_shown() {
        assert!(admitted_for(RuntimeRole::Replay, false, false, true));
        assert!(!admitted_for(RuntimeRole::Replay, true, true, false));
    }

    #[test]
    fn a_failed_load_is_abandoned_whatever_else_is_true() {
        assert_eq!(load_step(true, true, true), LoadStep::Abandon);
    }

    #[test]
    fn without_the_world_the_load_waits_but_is_not_waiting_on_admission() {
        assert_eq!(
            load_step(false, true, false),
            LoadStep::Wait {
                waiting_on_admission: false
            }
        );
    }

    #[test]
    fn with_the_world_but_no_go_ahead_the_load_waits_on_admission() {
        assert_eq!(
            load_step(false, false, true),
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
        assert_eq!(
            screen_after(LoadStep::Finish, AppScreen::Loading),
            Some(AppScreen::ClassSelect)
        );
        assert_eq!(
            screen_after(LoadStep::Finish, AppScreen::MainMenu),
            Some(AppScreen::ClassSelect)
        );
        assert_eq!(screen_after(LoadStep::Finish, AppScreen::InGame), None);
        assert_eq!(screen_after(LoadStep::Finish, AppScreen::ClassSelect), None);
    }

    #[test]
    fn abandoning_sends_a_player_in_the_match_back_to_the_menu() {
        assert_eq!(
            screen_after(LoadStep::Abandon, AppScreen::InGame),
            Some(AppScreen::MainMenu)
        );
        assert_eq!(
            screen_after(LoadStep::Abandon, AppScreen::ClassSelect),
            Some(AppScreen::MainMenu)
        );
        assert_eq!(screen_after(LoadStep::Abandon, AppScreen::Loading), None);
    }

    #[test]
    fn waiting_never_changes_the_screen() {
        let wait = LoadStep::Wait {
            waiting_on_admission: true,
        };
        assert_eq!(screen_after(wait, AppScreen::Loading), None);
    }
}
