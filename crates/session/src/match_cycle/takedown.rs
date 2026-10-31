//! Taking a match down: forgetting the world, going back to the menu screen,
//! and handing what the match owned to a worker thread to be dropped off the
//! frame.

use bevy::prelude::*;
use frame::{
    ShellScreen, KitPickerPassover, LevelPresent, StartupPersona, OwnLevelLoadTicket, GameSessionArmed,
    GameSessionStamp, GameSessionClosed, Retiring, ShutdownCause, LevelGeneration,
};
use net::{HostLoadingPause, GameSessionSummary};
use bevy::tasks::{AsyncComputeTaskPool, TaskPool};

/// Set to take the match down on the next pass.
#[derive(Resource, Default, Debug)]
pub struct ShutdownOrder(pub Option<ShutdownCause>);

/// The load the world on screen came from.
#[derive(Resource, Clone, Copy, Debug, Default)]
pub struct RunningWorldLabel {
    pub decrypt_secret: OwnLevelLoadTicket,
}

/// What a teardown leaves behind on purpose, for the log and the tests that
/// look for leaks.
#[derive(Resource, Default, Debug)]
pub struct ShutdownLeftovers(pub Vec<&'static str>);

const DECLARED_GAPS: [&str; 3] = [
    "image and mesh handles dropped with the world scene wait for Bevy's own \
     clean-up; they are not drawable leftover world",
    "the world scene resource stays so the hold still freezes level time; the \
     renderer empties its geometry and plans",
    "the authority world stays and is shut down in place, not removed",
];

/// The match a teardown report names: the one on screen, else the lobby's,
/// else an anonymous one at the signon epoch.
fn torn_match_key(live: Option<GameSessionStamp>, lobby: Option<GameSessionStamp>, signon_epoch: u32) -> GameSessionStamp {
    live.or(lobby.filter(|key| !key.is_none()))
        .unwrap_or_else(|| GameSessionStamp::new([0; 16], signon_epoch))
}

#[allow(clippy::too_many_arguments)]
pub(super) fn run_teardown(
    mut commands: Commands,
    mut pending: ResMut<ShutdownOrder>,
    mut current_screen: ResMut<ShellScreen>,
    mut world_flag: ResMut<LevelPresent>,
    mut generation_slot: ResMut<LevelGeneration>,
    mut class_handoff: ResMut<KitPickerPassover>,
    mut launch: Option<ResMut<StartupPersona>>,
    mut hold: Option<ResMut<HostLoadingPause>>,
    on_screen: Option<Res<RunningWorldLabel>>,
    signon: Option<Res<net::JoinProgressRecord>>,
    bridge: Option<Res<net::DirectoryLink>>,
    mut left_behind: ResMut<ShutdownLeftovers>,
    mut retiring: ResMut<Retiring>,
    mut torn: MessageWriter<GameSessionClosed>,
) {
    let Some(reason) = pending.0.take() else {
        return;
    };
    if let Some(link) = bridge.as_ref() {
        link.assign_installed_loadout(None);
    }
    let generation_before = *generation_slot;
    let lobby_key = bridge
        .as_ref()
        .map(|link| link.state().identity().game_session_key());
    let game_session_key = torn_match_key(
        on_screen.map(|live| live.decrypt_secret.game_session_key),
        lobby_key,
        signon.map_or(0, |state| state.epoch.0),
    );

    commands.remove_resource::<crate::GameContentLedger>();
    commands.remove_resource::<GameSessionSummary>();
    commands.insert_resource(RunningWorldLabel::default());

    *current_screen = ShellScreen::MainMenu;
    *world_flag = LevelPresent(false);
    *generation_slot = LevelGeneration(None);
    *class_handoff = KitPickerPassover::default();
    // The old authority stays frozen until the next load releases it.
    if let Some(frozen) = hold.as_mut() {
        frozen.0 = true;
    }
    if let Some(launch) = launch.as_mut() {
        launch.zone.clear();
    }
    left_behind.0 = DECLARED_GAPS.to_vec();

    let gap_count = left_behind.0.len();
    diag::info!(
        Sim,
        "session: match torn down ({reason:?}); {gap_count} declared gaps"
    );
    perf::match_torn(reason.label());
    let why = format!(" reason={}", reason.label());
    diag::lifecycle_boundary("local_session_revoked", &why);
    retiring.mark_teardown();
    let game_generation = game_session_key.game_generation;
    torn.write(GameSessionClosed {
        reason,
        level_generation: generation_before,
        game_session_key,
        game_generation,
    });
}

/// Records the zone that just installed as the one this process runs.
pub(super) fn stamp_loaded_zone(
    mut arrivals: MessageReader<GameSessionArmed>,
    mut launch: Option<ResMut<StartupPersona>>,
) {
    let Some(latest) = arrivals.read().last() else {
        return;
    };
    let Some(launch) = launch.as_mut() else {
        return;
    };
    launch.zone = latest.zone.clone();
}

/// Drops what the torn-down match handed over on a worker thread, so a big
/// world never stalls the frame it is dropped on.
pub(super) fn retire_handed_over(mut retiring: ResMut<Retiring>) {
    let (batch, is_teardown) = retiring.take_batch();
    if batch.is_empty() {
        return;
    }
    let values = batch.len();
    let guard = retiring.ship(is_teardown);
    AsyncComputeTaskPool::get_or_init(TaskPool::default)
        .spawn(async move {
            drop(batch);
            let settled = guard.settle() == Some(0);
            if settled {
                let note = format!(" values={values}");
                diag::lifecycle_boundary("retirement_settled", &note);
            }
        })
        .detach();
    let (jobs, total, waiting) = retiring.counts();
    diag::info!(
        Sim,
        "session: retired {values} value(s) off the frame \
         (batches={jobs} values={total} waiting={waiting})"
    );
    if is_teardown {
        let note = format!(" values={values} in_flight={}", retiring.in_flight());
        diag::lifecycle_boundary("old_runtime_teardown_complete", &note);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(fill: u8, epoch: u32) -> GameSessionStamp {
        GameSessionStamp::new([fill; 16], epoch)
    }

    #[test]
    fn the_world_on_screen_names_the_match_first() {
        let named = torn_match_key(Some(key(1, 3)), Some(key(2, 4)), 9);
        assert_eq!(named, key(1, 3));
    }

    #[test]
    fn a_world_with_no_key_yet_is_named_with_the_none_key_not_the_lobbys() {
        let named = torn_match_key(Some(GameSessionStamp::NONE), Some(key(2, 4)), 9);
        assert_eq!(named, GameSessionStamp::NONE);
    }

    #[test]
    fn with_no_world_the_lobbys_match_is_named() {
        assert_eq!(torn_match_key(None, Some(key(2, 4)), 9), key(2, 4));
    }

    #[test]
    fn a_lobby_with_no_match_falls_back_to_the_signon_epoch() {
        let from_empty_lobby = torn_match_key(None, Some(GameSessionStamp::NONE), 9);
        assert_eq!(from_empty_lobby, GameSessionStamp::new([0; 16], 9));
        let from_nothing = torn_match_key(None, None, 0);
        assert_eq!(from_nothing, GameSessionStamp::new([0; 16], 0));
    }

    #[test]
    fn a_teardown_declares_every_gap_it_leaves() {
        assert!(DECLARED_GAPS.iter().all(|gap| !gap.is_empty()));
    }

    #[test]
    fn the_request_starts_empty() {
        assert!(ShutdownOrder::default().0.is_none());
        assert_eq!(
            RunningWorldLabel::default().decrypt_secret,
            OwnLevelLoadTicket::default()
        );
    }
}
