//! Small yes/no rules about worlds, epochs and who may pick a class.

use frame::GameSessionStamp;

use crate::net_links::match_handoff::same_generation;

/// What a side reports once its world for a match is up: the epoch and a digest each of the map,
/// the weapon registry and the class registry.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct HostMapLoaded {
    pub round_era: u32,
    pub map: u64,
    pub weapon_digest: u64,
    pub class_digest: u64,
}

/// A peer's loaded world matches the host's: same live epoch on both and identical digests.
pub fn level_ready_agrees_with_host(
    running_generation: u32,
    host: Option<HostMapLoaded>,
    loaded: HostMapLoaded,
) -> bool {
    host.is_some_and(|ours| {
        same_generation(loaded.round_era, running_generation)
            && same_generation(ours.round_era, running_generation)
            && ours == loaded
    })
}

/// A teardown of `torn_epoch` hits the live match. Epoch 0 means no match on both sides.
pub fn teardown_hits_epoch(ended_epoch: u32, running_generation: u32) -> bool {
    ended_epoch == running_generation
}

/// The same rule for whole keys: the empty key only ends the empty key.
pub fn teardown_hits_key(torn: GameSessionStamp, live: GameSessionStamp) -> bool {
    torn == live || (torn.is_none() && live.is_none())
}

/// Picking a class needs every one of these to hold.
pub fn loadout_pick_permitted(
    open: bool,
    world_ok: bool,
    display_prepared: bool,
    seated: bool,
    broken: bool,
) -> bool {
    [open, world_ok, display_prepared, seated].into_iter().all(|ok| ok)
        && !broken
}

/// A world-ready fact is taken as confirmed when a world is installed for the live match, the
/// authority is not holding it back and the fact carries an epoch.
pub fn acknowledge_tagged_scene_loaded(
    has_world: bool,
    held_back: bool,
    world_is_live: bool,
    fact: HostMapLoaded,
) -> Option<HostMapLoaded> {
    let usable = has_world && !held_back && world_is_live && fact.round_era != 0;
    usable.then_some(fact)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ready(epoch: u32) -> HostMapLoaded {
        HostMapLoaded {
            round_era: epoch,
            map: 1,
            weapon_digest: 2,
            class_digest: 3,
        }
    }

    #[test]
    fn a_loaded_world_matches_only_the_same_digests_in_the_live_epoch() {
        assert!(level_ready_agrees_with_host(5, Some(ready(5)), ready(5)));
        assert!(!level_ready_agrees_with_host(5, None, ready(5)));
        assert!(!level_ready_agrees_with_host(6, Some(ready(5)), ready(5)));
        assert!(!level_ready_agrees_with_host(5, Some(ready(5)), ready(4)));
        let mut other = ready(5);
        other.class_digest = 9;
        assert!(!level_ready_agrees_with_host(5, Some(ready(5)), other));
    }

    #[test]
    fn epoch_zero_never_counts_as_a_live_world() {
        assert!(!level_ready_agrees_with_host(0, Some(ready(0)), ready(0)));
    }

    #[test]
    fn the_epoch_boundary_is_plain_equality() {
        assert!(teardown_hits_epoch(0, 0));
        assert!(!teardown_hits_epoch(0, 3));
        assert!(!teardown_hits_epoch(3, 0));
        assert!(teardown_hits_epoch(3, 3));
        assert!(!teardown_hits_epoch(3, 4));
    }

    #[test]
    fn the_key_boundary_matches_whole_keys_and_the_empty_pair() {
        let a = GameSessionStamp::new([1; 16], 2);
        let b = GameSessionStamp::new([2; 16], 2);
        assert!(teardown_hits_key(a, a));
        assert!(!teardown_hits_key(a, b));
        assert!(teardown_hits_key(GameSessionStamp::NONE, GameSessionStamp::NONE));
        assert!(!teardown_hits_key(GameSessionStamp::NONE, a));
        assert!(!teardown_hits_key(a, GameSessionStamp::NONE));
    }

    #[test]
    fn class_selection_needs_all_four_and_no_failure() {
        assert!(loadout_pick_permitted(true, true, true, true, false));
        assert!(!loadout_pick_permitted(false, true, true, true, false));
        assert!(!loadout_pick_permitted(true, false, true, true, false));
        assert!(!loadout_pick_permitted(true, true, false, true, false));
        assert!(!loadout_pick_permitted(true, true, true, false, false));
        assert!(!loadout_pick_permitted(true, true, true, true, true));
    }

    #[test]
    fn a_world_ready_fact_is_confirmed_only_when_usable() {
        let fact = ready(7);
        assert_eq!(acknowledge_tagged_scene_loaded(true, false, true, fact), Some(fact));
        assert_eq!(acknowledge_tagged_scene_loaded(false, false, true, fact), None);
        assert_eq!(acknowledge_tagged_scene_loaded(true, true, true, fact), None);
        assert_eq!(acknowledge_tagged_scene_loaded(true, false, false, fact), None);
        assert_eq!(acknowledge_tagged_scene_loaded(true, false, true, ready(0)), None);
    }
}
