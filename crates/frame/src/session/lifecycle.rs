//! Messages that announce a match or a life starting, ending or failing.

use bevy::prelude::Message;

use super::keys::{LocalLoadKey, MatchKey, WorldGeneration};

/// A world finished installing for the load request `request_id`.
#[derive(Message, Clone, Debug, PartialEq, Eq)]
pub struct MatchInstalled {
    pub request_id: u64,
    pub load_key: LocalLoadKey,
    pub zone: String,
    pub spawn_count: usize,
}

/// A match's world was taken down.
#[derive(Message, Clone, Copy, Debug, PartialEq, Eq)]
pub struct MatchTornDown {
    pub reason: TeardownReason,
    pub world_generation: WorldGeneration,
    pub match_key: MatchKey,
    pub match_epoch: u32,
}

/// The app went back to the main menu.
#[derive(Message, Clone, Copy, Debug, PartialEq, Eq)]
pub struct ReturnedToMenu {
    pub swap_id: u64,
    pub had_world: bool,
}

/// The authority accepted a map-load request.
#[derive(Message, Clone, Debug, PartialEq, Eq)]
pub struct MapLoadApproved {
    pub request_id: u64,
    pub load_key: LocalLoadKey,
    pub zone: String,
}

/// A map-load request could not be completed.
#[derive(Message, Clone, Debug, PartialEq, Eq)]
pub struct MapLoadFailed {
    pub request_id: u64,
    pub load_key: LocalLoadKey,
    pub zone: String,
    pub error: String,
}

/// Why a match's world was taken down.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TeardownReason {
    /// The connection ended; the session goes with it.
    Disconnect,
    /// A newer match is taking this one's place.
    Replaced,
    /// The match ran its course.
    MatchEnded,
}

impl TeardownReason {
    /// The session outlives the teardown.
    pub fn keeps_session(self) -> bool {
        matches!(self, Self::Replaced | Self::MatchEnded)
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Disconnect => "Disconnect",
            Self::Replaced => "Replaced",
            Self::MatchEnded => "MatchEnded",
        }
    }
}

/// A client began a life.
#[derive(Message, Clone, Copy, Debug, PartialEq, Eq)]
pub struct LifeStarted {
    pub client: u32,
    pub life: u32,
    pub reason: LifeStartReason,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LifeStartReason {
    BecameAlive,
}

/// A client's life ended.
#[derive(Message, Clone, Copy, Debug, PartialEq, Eq)]
pub struct LifeEnded {
    pub client: u32,
    pub life: u32,
    pub cause: LifeEndCause,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LifeEndCause {
    /// The client stopped being alive.
    LeftAlive,
    /// A new life took its place.
    Replaced,
    /// The client dropped out of the match.
    Dropped,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_disconnect_ends_the_session_and_the_other_reasons_keep_it() {
        assert!(!TeardownReason::Disconnect.keeps_session());
        assert!(TeardownReason::Replaced.keeps_session());
        assert!(TeardownReason::MatchEnded.keeps_session());
    }

    #[test]
    fn teardown_labels_are_the_variant_names() {
        for reason in [
            TeardownReason::Disconnect,
            TeardownReason::Replaced,
            TeardownReason::MatchEnded,
        ] {
            assert_eq!(reason.label(), format!("{reason:?}"));
        }
    }

    #[test]
    fn a_teardown_message_keeps_what_it_was_told() {
        let key = MatchKey::new([3; 16], 2);
        let torn = MatchTornDown {
            reason: TeardownReason::Replaced,
            world_generation: WorldGeneration::from_install(4),
            match_key: key,
            match_epoch: 2,
        };
        let copy = torn;
        assert_eq!(copy, torn);
        assert_eq!(copy.world_generation, WorldGeneration(Some(4)));
        assert_eq!(copy.match_key, key);
    }

    #[test]
    fn load_messages_compare_by_content() {
        let key = LocalLoadKey::from_request(1, MatchKey::new([1; 16], 1), 1);
        let approved = MapLoadApproved {
            request_id: 1,
            load_key: key,
            zone: "zone_a".into(),
        };
        assert_eq!(approved.clone(), approved);
        let failed = MapLoadFailed {
            request_id: 1,
            load_key: key,
            zone: "zone_a".into(),
            error: "missing".into(),
        };
        assert_ne!(
            failed.clone(),
            MapLoadFailed {
                error: "other".into(),
                ..failed
            }
        );
    }

    #[test]
    fn life_messages_carry_client_and_life() {
        let started = LifeStarted {
            client: 3,
            life: 2,
            reason: LifeStartReason::BecameAlive,
        };
        let ended = LifeEnded {
            client: 3,
            life: 2,
            cause: LifeEndCause::Dropped,
        };
        assert_eq!((started.client, started.life), (ended.client, ended.life));
        assert_ne!(ended.cause, LifeEndCause::LeftAlive);
    }
}
