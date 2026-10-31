//! Messages that announce a match or a life starting, ending or failing.

use bevy::ecs::message::Message;

use super::keys::{OwnLevelLoadTicket, GameSessionStamp, LevelGeneration};

/// A world finished installing for the load request `request_id`.
#[derive(Debug, Clone, PartialEq, Eq, Message)]
pub struct GameSessionArmed {
    pub ask_number: u64,
    pub decrypt_secret: OwnLevelLoadTicket,
    pub zone: String,
    pub spawns_made: usize,
}

/// A match's world was taken down.
#[derive(Message, Clone, Copy, Debug, PartialEq, Eq)]
pub struct GameSessionClosed {
    pub reason: ShutdownCause,
    pub level_generation: LevelGeneration,
    pub game_session_key: GameSessionStamp,
    pub game_generation: u32,
}

/// The app went back to the main menu.
#[derive(Message, Clone, Copy, Debug, PartialEq, Eq)]
pub struct ReturnedToMenu {
    pub swap_id: u64,
    pub had_world: bool,
}

/// The authority accepted a map-load request.
#[derive(Debug, Clone, PartialEq, Eq, Message)]
pub struct LevelOpenGranted {
    pub ask_number: u64,
    pub decrypt_secret: OwnLevelLoadTicket,
    pub zone: String,
}

/// A map-load request could not be completed.
#[derive(Debug, Clone, PartialEq, Eq, Message)]
pub struct LevelOpenRejected {
    pub ask_number: u64,
    pub decrypt_secret: OwnLevelLoadTicket,
    pub zone: String,
    pub error: String,
}

/// Why a match's world was taken down.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShutdownCause {
    /// The connection ended; the session goes with it.
    Disconnect,
    /// A newer match is taking this one's place.
    Replaced,
    /// The match ran its course.
    RoundOver,
}

impl ShutdownCause {
    /// The session outlives the teardown.
    pub fn keeps_session(self) -> bool {
        let cause = self;
        matches!(cause, Self::RoundOver | Self::Replaced)
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Disconnect => "Disconnect",
            Self::Replaced => "Replaced",
            Self::RoundOver => "RoundOver",
        }
    }
}

/// A client began a life.
#[derive(Message, Clone, Copy, Debug, PartialEq, Eq)]
pub struct SpawnLifeBegun {
    pub client: u32,
    pub life: u32,
    pub reason: SpawnCause,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SpawnCause {
    JustRespawned,
}

/// A client's life ended.
#[derive(Message, Clone, Copy, Debug, PartialEq, Eq)]
pub struct SpawnLifeOver {
    pub client: u32,
    pub life: u32,
    pub cause: DeathCircumstance,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DeathCircumstance {
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
        assert!(!ShutdownCause::Disconnect.keeps_session());
        assert!(ShutdownCause::Replaced.keeps_session());
        assert!(ShutdownCause::RoundOver.keeps_session());
    }

    #[test]
    fn teardown_labels_are_the_variant_names() {
        for cause in [
            ShutdownCause::Disconnect,
            ShutdownCause::Replaced,
            ShutdownCause::RoundOver,
        ] {
            assert_eq!(cause.label(), format!("{cause:?}"));
        }
    }

    #[test]
    fn a_teardown_message_keeps_what_it_was_told() {
        let key = GameSessionStamp::new([3; 16], 2);
        let torn = GameSessionClosed {
            reason: ShutdownCause::Replaced,
            level_generation: LevelGeneration::out_of_setup_dir(4),
            game_session_key: key,
            game_generation: 2,
        };
        let copy = torn;
        assert_eq!(copy, torn);
        assert_eq!(copy.level_generation, LevelGeneration(Some(4)));
        assert_eq!(copy.game_session_key, key);
    }

    #[test]
    fn load_messages_compare_by_content() {
        let key = OwnLevelLoadTicket::from_request(1, GameSessionStamp::new([1; 16], 1), 1);
        let approved = LevelOpenGranted {
            ask_number: key.own_level_ticket_no,
            decrypt_secret: key,
            zone: "zone_a".into(),
        };
        assert_eq!(approved.clone(), approved);
        let failed = LevelOpenRejected {
            ask_number: key.own_level_ticket_no,
            decrypt_secret: key,
            zone: "zone_a".into(),
            error: "missing".into(),
        };
        assert_ne!(
            failed.clone(),
            LevelOpenRejected {
                error: "other".into(),
                ..failed
            }
        );
    }

    #[test]
    fn life_messages_carry_client_and_life() {
        let started = SpawnLifeBegun {
            client: 3,
            life: 2,
            reason: SpawnCause::JustRespawned,
        };
        let ended = SpawnLifeOver {
            client: 3,
            life: 2,
            cause: DeathCircumstance::Dropped,
        };
        assert_eq!((started.client, started.life), (ended.client, ended.life));
        assert_ne!(ended.cause, DeathCircumstance::LeftAlive);
    }
}
