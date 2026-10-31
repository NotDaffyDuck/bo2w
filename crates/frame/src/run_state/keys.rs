//! Identities that tie a loaded world, a match and a load request together.

use bevy::prelude::Resource;

/// Which install of the world is current (the id of the request that
/// installed it). `None` while no world is installed.
#[derive(Resource, Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct LevelGeneration(pub Option<u64>);

impl LevelGeneration {
    pub fn out_of_setup_dir(install_request: u64) -> Self {
        Self(Some(install_request))
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
    pub fn same_as(self, walk: Self) -> bool {
        self.0.is_some() && self == walk
    }
}

/// One match: the session it belongs to and its epoch within that session.
/// Epoch 0 is reserved for "no match".
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct GameSessionStamp {
    pub stamp_bytes: [u8; 16],
    pub game_generation: u32,
}

impl GameSessionStamp {
    pub const NONE: Self = Self::new([0; 16], 0);

    pub const fn new(id: [u8; 16], game_generation: u32) -> Self {
        Self {
            stamp_bytes: id,
            game_generation,
        }
    }

    pub fn is_none(self) -> bool {
        self.game_generation == 0
    }
}

/// One local load request, within one match, within one engine run.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct OwnLevelLoadTicket {
    pub incarnation: u64,
    pub game_session_key: GameSessionStamp,
    pub own_level_ticket_no: u64,
}

impl OwnLevelLoadTicket {
    pub fn from_request(ticket_no: u64, game_session_key: GameSessionStamp, run: u64) -> Self {
        Self {
            incarnation: run,
            game_session_key,
            own_level_ticket_no: ticket_no,
        }
    }

    /// The load was made for `game_session_key`. No load belongs to "no match".
    pub fn issued_for(self, game_session_key: GameSessionStamp) -> bool {
        !game_session_key.is_none() && self.game_session_key == game_session_key
    }

    /// `finished` completes this request: same incarnation, match and request.
    pub fn fits_finish_signal(self, finished: Self) -> bool {
        finished == self
    }
}

/// One player's admission to a match, over one connection.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct EntryPassToken {
    pub game_session_key: GameSessionStamp,
    pub seat_bytes: [u8; 16],
    pub link_number: u64,
    pub opening_number: u32,
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;

    const SESSION_A: [u8; 16] = [1; 16];
    const SESSION_B: [u8; 16] = [2; 16];

    #[test]
    fn an_install_names_its_request() {
        assert_eq!(LevelGeneration::out_of_setup_dir(9), LevelGeneration(Some(9)));
        assert_eq!(LevelGeneration::default(), LevelGeneration(None));
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
        assert!(GameSessionStamp::NONE.is_none());
        assert!(GameSessionStamp::default().is_none());
        assert!(GameSessionStamp::new(SESSION_A, 0).is_none());
        assert!(!GameSessionStamp::new(SESSION_A, 1).is_none());
        assert_eq!(GameSessionStamp::NONE, GameSessionStamp::default());
    }

    #[test]
    fn match_keys_differ_by_session_and_by_epoch() {
        let keys: HashSet<GameSessionStamp> = [
            GameSessionStamp::new(SESSION_A, 1),
            GameSessionStamp::new(SESSION_A, 2),
            GameSessionStamp::new(SESSION_B, 1),
            GameSessionStamp::new(SESSION_A, 1),
        ]
        .into();
        assert_eq!(keys.len(), 3);
    }

    #[test]
    fn a_load_request_carries_all_three_ids() {
        let key = OwnLevelLoadTicket::from_request(7, GameSessionStamp::new(SESSION_A, 3), 11);
        assert_eq!(key.own_level_ticket_no, 7);
        assert_eq!(key.game_session_key, GameSessionStamp::new(SESSION_A, 3));
        assert_eq!(key.incarnation, 11);
    }

    #[test]
    fn a_load_belongs_only_to_the_match_it_was_made_for() {
        let mine = GameSessionStamp::new(SESSION_A, 3);
        let key = OwnLevelLoadTicket::from_request(7, mine, 1);
        assert!(key.issued_for(mine));
        assert!(!key.issued_for(GameSessionStamp::new(SESSION_A, 4)));
        assert!(!key.issued_for(GameSessionStamp::new(SESSION_B, 3)));
    }

    #[test]
    fn no_load_belongs_to_no_match() {
        let orphan = OwnLevelLoadTicket::from_request(7, GameSessionStamp::NONE, 1);
        assert!(!orphan.issued_for(GameSessionStamp::NONE));
    }

    #[test]
    fn a_completion_must_match_every_field() {
        let mine = GameSessionStamp::new(SESSION_A, 3);
        let request = OwnLevelLoadTicket::from_request(7, mine, 1);
        assert!(request.fits_finish_signal(OwnLevelLoadTicket::from_request(7, mine, 1)));
        assert!(!request.fits_finish_signal(OwnLevelLoadTicket::from_request(8, mine, 1)));
        assert!(!request.fits_finish_signal(OwnLevelLoadTicket::from_request(7, mine, 2)));
        assert!(!request.fits_finish_signal(OwnLevelLoadTicket::from_request(
            7,
            GameSessionStamp::new(SESSION_A, 4),
            1
        )));
    }

    #[test]
    fn admissions_are_told_apart_by_connection() {
        let base = EntryPassToken {
            game_session_key: GameSessionStamp::new(SESSION_A, 1),
            seat_bytes: [5; 16],
            link_number: 1,
            opening_number: 1,
        };
        let reconnect = EntryPassToken {
            link_number: 2,
            ..base
        };
        let set: HashSet<EntryPassToken> = [base, reconnect, base].into();
        assert_eq!(set.len(), 2);
    }
}
