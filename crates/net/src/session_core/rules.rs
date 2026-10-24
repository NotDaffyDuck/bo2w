//! Small yes/no rules about worlds, epochs and who may pick a class.

use frame::MatchKey;

use crate::transport::bootstrap::epoch_applies;

/// What a side reports once its world for a match is up: the epoch and a digest each of the map,
/// the weapon registry and the class registry.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct HostWorldReady {
    pub epoch: u32,
    pub map: u64,
    pub weapons: u64,
    pub classes: u64,
}

/// A peer's loaded world matches the host's: same live epoch on both and identical digests.
pub fn map_loaded_matches_host(
    live_epoch: u32,
    host: Option<HostWorldReady>,
    loaded: HostWorldReady,
) -> bool {
    host.is_some_and(|host| {
        epoch_applies(loaded.epoch, live_epoch)
            && epoch_applies(host.epoch, live_epoch)
            && host == loaded
    })
}

/// A teardown of `torn_epoch` hits the live match. Epoch 0 means no match on both sides.
pub fn match_boundary_applies(torn_epoch: u32, live_epoch: u32) -> bool {
    torn_epoch == live_epoch
}

/// The same rule for whole keys: the empty key only ends the empty key.
pub fn match_key_boundary_applies(torn: MatchKey, live: MatchKey) -> bool {
    torn == live || (torn.is_none() && live.is_none())
}

/// Picking a class needs every one of these to hold.
pub fn class_select_allowed(
    session_open: bool,
    installed_belongs_to_match: bool,
    presentation_ready: bool,
    admitted: bool,
    failed: bool,
) -> bool {
    [session_open, installed_belongs_to_match, presentation_ready, admitted].into_iter().all(|ok| ok)
        && !failed
}

/// A world-ready fact is taken as confirmed when a world is installed for the live match, the
/// authority is not holding it back and the fact carries an epoch.
pub fn confirm_keyed_world_ready(
    installed: bool,
    authority_hold: bool,
    installed_belongs_to_live_match: bool,
    fact: HostWorldReady,
) -> Option<HostWorldReady> {
    let usable = installed && !authority_hold && installed_belongs_to_live_match && fact.epoch != 0;
    usable.then_some(fact)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ready(epoch: u32) -> HostWorldReady {
        HostWorldReady {
            epoch,
            map: 1,
            weapons: 2,
            classes: 3,
        }
    }

    #[test]
    fn a_loaded_world_matches_only_the_same_digests_in_the_live_epoch() {
        assert!(map_loaded_matches_host(5, Some(ready(5)), ready(5)));
        assert!(!map_loaded_matches_host(5, None, ready(5)));
        assert!(!map_loaded_matches_host(6, Some(ready(5)), ready(5)));
        assert!(!map_loaded_matches_host(5, Some(ready(5)), ready(4)));
        let mut other = ready(5);
        other.classes = 9;
        assert!(!map_loaded_matches_host(5, Some(ready(5)), other));
    }

    #[test]
    fn epoch_zero_never_counts_as_a_live_world() {
        assert!(!map_loaded_matches_host(0, Some(ready(0)), ready(0)));
    }

    #[test]
    fn the_epoch_boundary_is_plain_equality() {
        assert!(match_boundary_applies(0, 0));
        assert!(!match_boundary_applies(0, 3));
        assert!(!match_boundary_applies(3, 0));
        assert!(match_boundary_applies(3, 3));
        assert!(!match_boundary_applies(3, 4));
    }

    #[test]
    fn the_key_boundary_matches_whole_keys_and_the_empty_pair() {
        let a = MatchKey::new([1; 16], 2);
        let b = MatchKey::new([2; 16], 2);
        assert!(match_key_boundary_applies(a, a));
        assert!(!match_key_boundary_applies(a, b));
        assert!(match_key_boundary_applies(MatchKey::NONE, MatchKey::NONE));
        assert!(!match_key_boundary_applies(MatchKey::NONE, a));
        assert!(!match_key_boundary_applies(a, MatchKey::NONE));
    }

    #[test]
    fn class_selection_needs_all_four_and_no_failure() {
        assert!(class_select_allowed(true, true, true, true, false));
        assert!(!class_select_allowed(false, true, true, true, false));
        assert!(!class_select_allowed(true, false, true, true, false));
        assert!(!class_select_allowed(true, true, false, true, false));
        assert!(!class_select_allowed(true, true, true, false, false));
        assert!(!class_select_allowed(true, true, true, true, true));
    }

    #[test]
    fn a_world_ready_fact_is_confirmed_only_when_usable() {
        let fact = ready(7);
        assert_eq!(confirm_keyed_world_ready(true, false, true, fact), Some(fact));
        assert_eq!(confirm_keyed_world_ready(false, false, true, fact), None);
        assert_eq!(confirm_keyed_world_ready(true, true, true, fact), None);
        assert_eq!(confirm_keyed_world_ready(true, false, false, fact), None);
        assert_eq!(confirm_keyed_world_ready(true, false, true, ready(0)), None);
    }
}
