//! Commands the client has built but the host has not acknowledged yet.

use std::collections::VecDeque;

use bevy::prelude::*;
use playerstate_t6::game::InputCommand;

use crate::host_tick::peer_mail::{COMMAND_TIME_STEP_MS, MOVE_BACKLOG_SPAN_LIMIT, MOVE_RESEND_WINDOW};
use crate::client::run_ahead::InputCommandNumber;

/// The commands that are still waiting for an acknowledgement, oldest first. Each send repeats
/// all of them, so a lost packet costs nothing.
#[derive(Resource, Default, Debug, Clone)]
pub struct QueuedOutgoingPlayerMessages {
    queue: VecDeque<(InputCommandNumber, InputCommand, sim::ShotRecordOrigin)>,

    /// Commands that were dropped because the queue was already full.
    turned_away: u32,

    acked_through: Option<InputCommandNumber>,
}

impl QueuedOutgoingPlayerMessages {
    pub fn len(&self) -> usize {
        self.queue.len()
    }

    pub fn is_empty(&self) -> bool {
        self.queue.is_empty()
    }

    /// Queues a command; when the queue is full the command is dropped and counted.
    pub fn enqueue(&mut self, seq: InputCommandNumber, cmd: InputCommand, sample: sim::ShotRecordOrigin) {
        if self.queue.len() >= MOVE_RESEND_WINDOW {
            self.turned_away = self.turned_away.saturating_add(1);
        } else {
            self.queue.push_back((seq, cmd, sample));
        }
    }

    pub fn displaced_unconfirmed(&self) -> u32 {
        self.turned_away
    }

    /// The highest sequence the host has said it has, if it said anything yet.
    pub fn acked_through(&self) -> Option<InputCommandNumber> {
        self.acked_through
    }

    /// Drops every queued command up to and including `receipt_confirmed`.
    pub fn retire_through(&mut self, receipt_confirmed: InputCommandNumber) {
        self.acked_through = self.acked_through.max(Some(receipt_confirmed));
        let keep_from = self.queue.partition_point(|(seq, _, _)| seq.0 <= receipt_confirmed.0);
        self.queue.drain(..keep_from);
    }

    pub fn iter(&self) -> impl Iterator<Item = &(InputCommandNumber, InputCommand, sim::ShotRecordOrigin)> {
        self.queue.iter()
    }

    pub fn oldest_seq(&self) -> Option<InputCommandNumber> {
        self.queue.front().map(|(seq, _, _)| *seq)
    }

    pub fn newest_seq(&self) -> Option<InputCommandNumber> {
        self.queue.back().map(|(seq, _, _)| *seq)
    }

    /// Whether a command stamped `authority_clock_ms` can be queued without the host's backlog of
    /// unconsumed work going over its limit. One whole step is held back for the gap between
    /// the host's last consumed command and the oldest queued one.
    pub fn has_command_capacity(&self, authority_clock_ms: i32) -> bool {
        if self.queue.len() >= MOVE_RESEND_WINDOW {
            return false;
        }
        let stamps = self
            .queue
            .iter()
            .map(|(_, cmd, _)| cmd.authority_clock_ms)
            .chain(std::iter::once(authority_clock_ms));
        COMMAND_TIME_STEP_MS.saturating_add(capped_gap_sum(stamps)) <= MOVE_BACKLOG_SPAN_LIMIT
    }
}

/// The sum of the gaps between neighbouring stamps, each gap held between 0 and one step.
fn capped_gap_sum(stamps: impl Iterator<Item = i32>) -> i32 {
    let mut total = 0i32;
    let mut last = None;
    for stamp in stamps {
        if let Some(last) = last {
            total = total.saturating_add(stamp.saturating_sub(last).clamp(0, COMMAND_TIME_STEP_MS));
        }
        last = Some(stamp);
    }
    total
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cmd_at(authority_clock_ms: i32) -> InputCommand {
        InputCommand {
            authority_clock_ms,
            ..InputCommand::default()
        }
    }

    fn queue_of(seqs: std::ops::Range<u32>) -> QueuedOutgoingPlayerMessages {
        let mut sends = QueuedOutgoingPlayerMessages::default();
        for n in seqs {
            sends.enqueue(InputCommandNumber(n), cmd_at(n as i32 * 10), Default::default());
        }
        sends
    }

    #[test]
    fn acknowledging_drops_everything_up_to_and_including_that_sequence() {
        let mut sends = queue_of(1..8);
        sends.retire_through(InputCommandNumber(4));
        assert_eq!(sends.oldest_seq(), Some(InputCommandNumber(5)));
        assert_eq!(sends.newest_seq(), Some(InputCommandNumber(7)));
        assert_eq!(sends.len(), 3);
        assert_eq!(sends.acked_through(), Some(InputCommandNumber(4)));
    }

    #[test]
    fn an_old_acknowledgement_does_not_move_the_ack_back() {
        let mut sends = queue_of(1..8);
        sends.retire_through(InputCommandNumber(6));
        sends.retire_through(InputCommandNumber(2));
        assert_eq!(sends.acked_through(), Some(InputCommandNumber(6)));
        assert_eq!(sends.len(), 1);
    }

    #[test]
    fn acknowledging_past_the_newest_empties_the_queue() {
        let mut sends = queue_of(1..4);
        sends.retire_through(InputCommandNumber(99));
        assert!(sends.is_empty());
        assert_eq!(sends.oldest_seq(), None);
        assert_eq!(sends.newest_seq(), None);
    }

    #[test]
    fn a_full_queue_turns_new_commands_away_and_counts_them() {
        let mut sends = queue_of(0..MOVE_RESEND_WINDOW as u32);
        assert_eq!(sends.len(), MOVE_RESEND_WINDOW);
        sends.enqueue(InputCommandNumber(1000), cmd_at(0), Default::default());
        sends.enqueue(InputCommandNumber(1001), cmd_at(0), Default::default());
        assert_eq!(sends.len(), MOVE_RESEND_WINDOW);
        assert_eq!(sends.displaced_unconfirmed(), 2);
        assert_eq!(sends.newest_seq(), Some(InputCommandNumber(MOVE_RESEND_WINDOW as u32 - 1)));
    }

    #[test]
    fn an_empty_queue_has_room_for_a_command() {
        assert!(QueuedOutgoingPlayerMessages::default().has_command_capacity(12_345));
    }

    #[test]
    fn capacity_runs_out_when_the_stamped_work_reaches_the_limit() {
        let step = COMMAND_TIME_STEP_MS;
        let room = (MOVE_BACKLOG_SPAN_LIMIT - step) / step;
        let mut sends = QueuedOutgoingPlayerMessages::default();
        let mut stamp = 0;
        sends.enqueue(InputCommandNumber(0), cmd_at(stamp), Default::default());
        for n in 1..=room {
            stamp += step;
            assert!(sends.has_command_capacity(stamp), "step {n} should still fit");
            sends.enqueue(InputCommandNumber(n as u32), cmd_at(stamp), Default::default());
        }
        assert!(sends.has_command_capacity(stamp));
        assert!(!sends.has_command_capacity(stamp + step));
    }

    #[test]
    fn gaps_are_held_between_zero_and_one_step() {
        let step = COMMAND_TIME_STEP_MS;
        assert_eq!(capped_gap_sum([100, 100 + step * 5].into_iter()), step);
        assert_eq!(capped_gap_sum([100, 90, 80].into_iter()), 0);
        assert_eq!(capped_gap_sum([0, 30, 50].into_iter()), 50);
        assert_eq!(capped_gap_sum(std::iter::empty()), 0);
        assert_eq!(capped_gap_sum(std::iter::once(7)), 0);
    }
}
