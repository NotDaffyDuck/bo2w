//! Taking a match down: forgetting the world, going back to the menu screen,
//! and handing what the match owned to a worker thread to be dropped off the
//! frame.

use bevy::prelude::*;
use bevy::tasks::{AsyncComputeTaskPool, TaskPool};
use frame::{
    AppScreen, ClassSelectHandoff, HasWorld, LaunchIdentity, LocalLoadKey, MatchInstalled,
    MatchKey, MatchTornDown, Retiring, TeardownReason, WorldGeneration,
};
use net::{AuthorityLoadHold, MatchDescriptor};

/// Set to take the match down on the next pass.
#[derive(Resource, Default, Debug)]
pub struct TeardownRequest(pub Option<TeardownReason>);

/// The load the world on screen came from.
#[derive(Resource, Clone, Copy, Debug, Default)]
pub struct LiveWorldIdentity {
    pub load_key: LocalLoadKey,
}

/// What a teardown leaves behind on purpose, for the log and the tests that
/// look for leaks.
#[derive(Resource, Default, Debug)]
pub struct TeardownGaps(pub Vec<&'static str>);

const DECLARED_GAPS: [&str; 3] = [
    "image and mesh handles dropped with the world scene wait for Bevy's own \
     clean-up; they are not drawable leftover world",
    "the world scene resource stays so the hold still freezes level time; the \
     renderer empties its geometry and plans",
    "the authority world stays and is shut down in place, not removed",
];

/// The match a teardown report names: the one on screen, else the lobby's,
/// else an anonymous one at the signon epoch.
fn torn_match_key(live: Option<MatchKey>, lobby: Option<MatchKey>, signon_epoch: u32) -> MatchKey {
    live.or(lobby.filter(|key| !key.is_none()))
        .unwrap_or_else(|| MatchKey::new([0; 16], signon_epoch))
}

#[allow(clippy::too_many_arguments)]
pub(super) fn run_teardown(
    mut commands: Commands,
    mut request: ResMut<TeardownRequest>,
    mut screen: ResMut<AppScreen>,
    mut has_world: ResMut<HasWorld>,
    mut world_generation: ResMut<WorldGeneration>,
    mut handoff: ResMut<ClassSelectHandoff>,
    mut identity: Option<ResMut<LaunchIdentity>>,
    mut load_hold: Option<ResMut<AuthorityLoadHold>>,
    live_world: Option<Res<LiveWorldIdentity>>,
    signon: Option<Res<net::SignonState>>,
    bridge: Option<Res<net::MasterBridge>>,
    mut gaps: ResMut<TeardownGaps>,
    mut retiring: ResMut<Retiring>,
    mut torn: MessageWriter<MatchTornDown>,
) {
    let Some(reason) = request.0.take() else {
        return;
    };
    if let Some(bridge) = bridge.as_ref() {
        bridge.set_installed_load(None);
    }
    let torn_generation = *world_generation;
    let match_key = torn_match_key(
        live_world.map(|live| live.load_key.match_key),
        bridge
            .as_ref()
            .map(|bridge| bridge.state().identity().match_key()),
        signon.map_or(0, |signon| signon.epoch.0),
    );

    commands.remove_resource::<crate::SessionContentManifest>();
    commands.remove_resource::<MatchDescriptor>();
    commands.insert_resource(LiveWorldIdentity::default());

    *screen = AppScreen::MainMenu;
    *has_world = HasWorld(false);
    *world_generation = WorldGeneration(None);
    *handoff = ClassSelectHandoff::default();
    // Nothing runs on the old authority until the next load lets it.
    if let Some(hold) = load_hold.as_mut() {
        hold.0 = true;
    }
    if let Some(identity) = identity.as_mut() {
        identity.zone.clear();
    }
    gaps.0 = DECLARED_GAPS.to_vec();

    diag::info!(
        Sim,
        "session: match torn down ({reason:?}); {} declared gaps",
        gaps.0.len()
    );
    perf::match_torn(reason.label());
    diag::lifecycle_boundary(
        "local_session_revoked",
        &format!(" reason={}", reason.label()),
    );
    retiring.mark_teardown();
    torn.write(MatchTornDown {
        reason,
        world_generation: torn_generation,
        match_key,
        match_epoch: match_key.match_epoch,
    });
}

/// Records the zone that just installed as the one this process runs.
pub(super) fn stamp_loaded_zone(
    mut installed: MessageReader<MatchInstalled>,
    mut identity: Option<ResMut<LaunchIdentity>>,
) {
    let Some(fact) = installed.read().last() else {
        return;
    };
    if let Some(identity) = identity.as_mut() {
        identity.zone = fact.zone.clone();
    }
}

/// Drops what the torn-down match handed over on a worker thread, so a big
/// world never stalls the frame it is dropped on.
pub(super) fn retire_handed_over(mut retiring: ResMut<Retiring>) {
    let (batch, teardown) = retiring.take_batch();
    if batch.is_empty() {
        return;
    }
    let values = batch.len();
    let guard = retiring.ship(teardown);
    AsyncComputeTaskPool::get_or_init(TaskPool::default)
        .spawn(async move {
            drop(batch);
            if guard.settle() == Some(0) {
                diag::lifecycle_boundary("retirement_settled", &format!(" values={values}"));
            }
        })
        .detach();
    let (jobs, total, waiting) = retiring.counts();
    diag::info!(
        Sim,
        "session: retired {values} value(s) off the frame (batches={jobs} values={total} waiting={waiting})"
    );
    if teardown {
        diag::lifecycle_boundary(
            "old_runtime_teardown_complete",
            &format!(" values={values} in_flight={}", retiring.in_flight()),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(byte: u8, epoch: u32) -> MatchKey {
        MatchKey::new([byte; 16], epoch)
    }

    #[test]
    fn the_world_on_screen_names_the_match_first() {
        assert_eq!(
            torn_match_key(Some(key(1, 3)), Some(key(2, 4)), 9),
            key(1, 3)
        );
    }

    #[test]
    fn a_world_with_no_key_yet_is_named_with_the_none_key_not_the_lobbys() {
        assert_eq!(
            torn_match_key(Some(MatchKey::NONE), Some(key(2, 4)), 9),
            MatchKey::NONE
        );
    }

    #[test]
    fn with_no_world_the_lobbys_match_is_named() {
        assert_eq!(torn_match_key(None, Some(key(2, 4)), 9), key(2, 4));
    }

    #[test]
    fn a_lobby_with_no_match_falls_back_to_the_signon_epoch() {
        assert_eq!(
            torn_match_key(None, Some(MatchKey::NONE), 9),
            MatchKey::new([0; 16], 9)
        );
        assert_eq!(torn_match_key(None, None, 0), MatchKey::new([0; 16], 0));
    }

    #[test]
    fn a_teardown_declares_every_gap_it_leaves() {
        assert!(DECLARED_GAPS.iter().all(|gap| !gap.is_empty()));
    }

    #[test]
    fn the_request_starts_empty() {
        assert!(TeardownRequest::default().0.is_none());
        assert_eq!(
            LiveWorldIdentity::default().load_key,
            LocalLoadKey::default()
        );
    }
}
