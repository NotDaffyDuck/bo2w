//! Identities that tie a loaded world, a match and a load request together.

use bevy::prelude::Resource;

/// Which install of the world is current (the id of the request that
/// installed it). `None` while no world is installed.
#[derive(Resource, Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct WorldGeneration(pub Option<u64>);

impl WorldGeneration {
    pub fn from_install(request_id: u64) -> Self {
        Self(Some(request_id))
    }
}

/// Which walk of the world's products is current. Id 0 means no walk.
#[derive(Resource, Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct WorldProducts(pub Option<u64>);

impl WorldProducts {
    pub fn from_walk(products_id: u64) -> Self {
        Self((products_id != 0).then_some(products_id))
    }

    /// Both name the same walk. Two "no walk" values are not the same walk.
    pub fn same_as(self, other: Self) -> bool {
        self.0.is_some() && self == other
    }
}

/// One match: the session it belongs to and its epoch within that session.
/// Epoch 0 is reserved for "no match".
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct MatchKey {
    pub session_id: [u8; 16],
    pub match_epoch: u32,
}

impl MatchKey {
    pub const NONE: Self = Self::new([0; 16], 0);

    pub const fn new(session_id: [u8; 16], match_epoch: u32) -> Self {
        Self {
            session_id,
            match_epoch,
        }
    }

    pub fn is_none(self) -> bool {
        self.match_epoch == 0
    }
}

/// One local load request, within one match, within one engine run.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct LocalLoadKey {
    pub incarnation: u64,
    pub match_key: MatchKey,
    pub local_load_request_id: u64,
}

impl LocalLoadKey {
    pub fn from_request(request_id: u64, match_key: MatchKey, incarnation: u64) -> Self {
        Self {
            incarnation,
            match_key,
            local_load_request_id: request_id,
        }
    }

    /// The load was made for `match_key`. No load belongs to "no match".
    pub fn belongs_to(self, match_key: MatchKey) -> bool {
        !match_key.is_none() && self.match_key == match_key
    }

    /// `other` completes this request: same incarnation, match and request.
    pub fn matches_completion(self, other: Self) -> bool {
        self == other
    }
}

/// One player's admission to a match, over one connection.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct AdmissionKey {
    pub match_key: MatchKey,
    pub member_id: [u8; 16],
    pub connection_id: u64,
    pub bootstrap_id: u32,
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;

    const SESSION_A: [u8; 16] = [1; 16];
    const SESSION_B: [u8; 16] = [2; 16];

    #[test]
    fn an_install_names_its_request() {
        assert_eq!(WorldGeneration::from_install(9), WorldGeneration(Some(9)));
        assert_eq!(WorldGeneration::default(), WorldGeneration(None));
    }

    #[test]
    fn walk_id_zero_means_no_walk() {
        assert_eq!(WorldProducts::from_walk(0), WorldProducts(None));
        assert_eq!(WorldProducts::from_walk(5), WorldProducts(Some(5)));
    }

    #[test]
    fn a_walk_matches_itself_but_never_the_absence_of_one() {
        let walk = WorldProducts::from_walk(5);
        assert!(walk.same_as(WorldProducts::from_walk(5)));
        assert!(!walk.same_as(WorldProducts::from_walk(6)));
        assert!(!walk.same_as(WorldProducts::default()));
        assert!(!WorldProducts::default().same_as(WorldProducts::default()));
    }

    #[test]
    fn epoch_zero_is_no_match() {
        assert!(MatchKey::NONE.is_none());
        assert!(MatchKey::default().is_none());
        assert!(MatchKey::new(SESSION_A, 0).is_none());
        assert!(!MatchKey::new(SESSION_A, 1).is_none());
        assert_eq!(MatchKey::NONE, MatchKey::default());
    }

    #[test]
    fn match_keys_differ_by_session_and_by_epoch() {
        let keys: HashSet<MatchKey> = [
            MatchKey::new(SESSION_A, 1),
            MatchKey::new(SESSION_A, 2),
            MatchKey::new(SESSION_B, 1),
            MatchKey::new(SESSION_A, 1),
        ]
        .into();
        assert_eq!(keys.len(), 3);
    }

    #[test]
    fn a_load_request_carries_all_three_ids() {
        let key = LocalLoadKey::from_request(7, MatchKey::new(SESSION_A, 3), 11);
        assert_eq!(key.local_load_request_id, 7);
        assert_eq!(key.match_key, MatchKey::new(SESSION_A, 3));
        assert_eq!(key.incarnation, 11);
    }

    #[test]
    fn a_load_belongs_only_to_the_match_it_was_made_for() {
        let mine = MatchKey::new(SESSION_A, 3);
        let key = LocalLoadKey::from_request(7, mine, 1);
        assert!(key.belongs_to(mine));
        assert!(!key.belongs_to(MatchKey::new(SESSION_A, 4)));
        assert!(!key.belongs_to(MatchKey::new(SESSION_B, 3)));
    }

    #[test]
    fn no_load_belongs_to_no_match() {
        let orphan = LocalLoadKey::from_request(7, MatchKey::NONE, 1);
        assert!(!orphan.belongs_to(MatchKey::NONE));
    }

    #[test]
    fn a_completion_must_match_every_field() {
        let mine = MatchKey::new(SESSION_A, 3);
        let request = LocalLoadKey::from_request(7, mine, 1);
        assert!(request.matches_completion(LocalLoadKey::from_request(7, mine, 1)));
        assert!(!request.matches_completion(LocalLoadKey::from_request(8, mine, 1)));
        assert!(!request.matches_completion(LocalLoadKey::from_request(7, mine, 2)));
        assert!(!request.matches_completion(LocalLoadKey::from_request(
            7,
            MatchKey::new(SESSION_A, 4),
            1
        )));
    }

    #[test]
    fn admissions_are_told_apart_by_connection() {
        let base = AdmissionKey {
            match_key: MatchKey::new(SESSION_A, 1),
            member_id: [5; 16],
            connection_id: 1,
            bootstrap_id: 1,
        };
        let reconnect = AdmissionKey {
            connection_id: 2,
            ..base
        };
        let set: HashSet<AdmissionKey> = [base, reconnect, base].into();
        assert_eq!(set.len(), 2);
    }
}
