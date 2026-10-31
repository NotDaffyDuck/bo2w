//! The scoreboard the server sends to clients that are looking at it.

mod codec;
mod command;
mod pacing;

use bevy::prelude::Resource;

use crate::parse_scores::DecodedScoreboard;

pub use codec::{parse_scoreboard_message, encode_svc_scores};
pub use command::{compose_standings_command, compose_standings_from_state, read_board_command};
pub use pacing::StagedScoreboardSend;

/// Wire tag for the scoreboard command (ASCII `b`).
pub const HOST_MSG_SCOREBOARD: u8 = b'b';

/// How often, in milliseconds, scores are sent while a client holds the scoreboard open.
pub const SCOREBOARD_POLL_MILLIS: i32 = 2000;

/// The scoreboard as the client last received it.
#[derive(Resource, Clone, Debug, Default)]
pub struct Scoreboard {
    pub command_text: Option<String>,
    pub parsed: DecodedScoreboard,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_tag_is_a_lowercase_b_and_scores_refresh_every_two_seconds() {
        assert_eq!(HOST_MSG_SCOREBOARD, 0x62);
        assert_eq!(SCOREBOARD_POLL_MILLIS, 2000);
    }

    #[test]
    fn an_empty_scoreboard_has_no_command() {
        let board = Scoreboard::default();
        assert_eq!(board.command_text, None);
        assert_eq!(board.parsed.num, 0);
    }
}
