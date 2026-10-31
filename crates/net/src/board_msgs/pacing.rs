//! How often scores go out.

use bevy::prelude::Resource;

use super::SCOREBOARD_POLL_MILLIS;

/// Times the scoreboard last went out; -1 means not yet.
#[derive(Resource, Debug)]
pub struct StagedScoreboardSend {
    latest_transmit_millis: i32,
    latest_fanout_millis: i32,
}

impl Default for StagedScoreboardSend {
    fn default() -> Self {
        Self { latest_transmit_millis: -1, latest_fanout_millis: -1 }
    }
}

/// True when never done, or more than the refresh interval ago.
fn is_due(last_ms: i32, clock_millis: i32) -> bool {
    last_ms < 0 || last_ms + SCOREBOARD_POLL_MILLIS < clock_millis
}

impl StagedScoreboardSend {
    /// Should the scoreboard go to everyone now? Marks it sent when it says yes.
    pub fn fanout_ready(&mut self, clock_millis: i32) -> bool {
        let due = is_due(self.latest_fanout_millis, clock_millis);
        if due {
            self.latest_fanout_millis = clock_millis;
        }
        due
    }

    /// Should a client holding the scoreboard open get it now? Letting go of the key starts
    /// the count again, so the next press sends at once.
    pub fn held_board_ready(&mut self, scoreboard_held_open: bool, clock_millis: i32) -> bool {
        if !scoreboard_held_open {
            self.latest_transmit_millis = -1;
            return false;
        }
        let due = is_due(self.latest_transmit_millis, clock_millis);
        if due {
            self.latest_transmit_millis = clock_millis;
        }
        due
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_first_broadcast_is_due_straight_away() {
        let mut board = StagedScoreboardSend::default();
        assert!(board.fanout_ready(0));
    }

    #[test]
    fn the_next_broadcast_waits_more_than_two_seconds() {
        let mut board = StagedScoreboardSend::default();
        assert!(board.fanout_ready(1000));
        assert!(!board.fanout_ready(1001));
        assert!(!board.fanout_ready(3000), "exactly two seconds is not enough");
        assert!(board.fanout_ready(3001));
        assert!(!board.fanout_ready(3002));
    }

    #[test]
    fn a_held_scoreboard_is_sent_on_the_first_check_then_on_the_interval() {
        let mut board = StagedScoreboardSend::default();
        assert!(board.held_board_ready(true, 500));
        assert!(!board.held_board_ready(true, 600));
        assert!(!board.held_board_ready(true, 2500));
        assert!(board.held_board_ready(true, 2501));
    }

    #[test]
    fn letting_go_resets_so_the_next_press_sends_at_once() {
        let mut board = StagedScoreboardSend::default();
        assert!(board.held_board_ready(true, 500));
        assert!(!board.held_board_ready(false, 600));
        assert!(board.held_board_ready(true, 700));
    }

    #[test]
    fn a_broadcast_and_a_held_request_keep_separate_clocks() {
        let mut board = StagedScoreboardSend::default();
        assert!(board.fanout_ready(100));
        assert!(board.held_board_ready(true, 100));
    }

    #[test]
    fn not_holding_the_scoreboard_never_sends() {
        let mut board = StagedScoreboardSend::default();
        assert!(!board.held_board_ready(false, 0));
        assert!(!board.held_board_ready(false, 100_000));
    }
}
