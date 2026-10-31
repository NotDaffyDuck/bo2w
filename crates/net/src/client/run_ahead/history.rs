//! The commands a client has run but the authority has not yet acknowledged.

use playerstate_t6::game::{AvatarState, InputCommand};
use sim::Tick;
use std::collections::VecDeque;

/// Sequence number a client stamps on each command it sends.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct InputCommandNumber(pub u32);

impl InputCommandNumber {
    pub const fn next(self) -> Self {
        Self(self.0.wrapping_add(1))
    }
}

/// One predicted command with the player state before and after it ran.
#[derive(Clone, Debug, PartialEq)]
pub struct ForecastMoveEntry {
    pub seq: InputCommandNumber,
    pub at_tick: Tick,
    pub cmd: InputCommand,
    pub input: AvatarState,
    pub output: AvatarState,
}

/// Most unacknowledged commands kept unless a caller asks for another limit.
pub const DEFAULT_MOVE_LOG_LIMIT: usize = 256;

/// What the authority's acknowledgement turned out to refer to.
#[derive(Clone, Debug, PartialEq)]
pub enum ReceiptLookup {
    /// The acknowledged command is still held; this is its record.
    Matched(ForecastMoveEntry),
    /// Nothing held is that old or the history is empty: the confirm_receipt only retires old records.
    Retired,
    /// The confirm_receipt names a command newer than anything sent, or one older than the history reaches.
    Broken,
}

/// Unacknowledged commands in the order they were run, oldest first.
#[derive(Clone, Debug)]
pub struct ForecastMoveLog {
    records: VecDeque<ForecastMoveEntry>,
    cap_len: usize,
}

impl Default for ForecastMoveLog {
    fn default() -> Self {
        Self::holding_at_most(DEFAULT_MOVE_LOG_LIMIT)
    }
}

impl ForecastMoveLog {
    pub fn new() -> Self {
        Self::default()
    }

    /// A history that refuses records beyond `limit` (at least one).
    pub fn holding_at_most(limit: usize) -> Self {
        let limit = limit.max(1);
        Self {
            records: VecDeque::with_capacity(limit.min(DEFAULT_MOVE_LOG_LIMIT)),
            cap_len: limit,
        }
    }

    pub fn len(&self) -> usize {
        self.records.len()
    }

    pub fn is_empty(&self) -> bool {
        self.records.is_empty()
    }

    pub fn capacity_limit(&self) -> usize {
        self.cap_len
    }

    pub fn oldest_seq(&self) -> Option<InputCommandNumber> {
        self.records.front().map(|record| record.seq)
    }

    pub fn newest_seq(&self) -> Option<InputCommandNumber> {
        self.records.back().map(|record| record.seq)
    }

    pub fn newest(&self) -> Option<&ForecastMoveEntry> {
        self.records.back()
    }

    pub fn iter(&self) -> impl Iterator<Item = &ForecastMoveEntry> {
        self.records.iter()
    }

    /// Appends `record`. A full history drops the new record instead and returns `true`.
    pub fn add_entry(&mut self, record: ForecastMoveEntry) -> bool {
        let full = self.records.len() >= self.cap_len;
        if !full {
            self.records.push_back(record);
        }
        full
    }

    pub fn clear(&mut self) {
        self.records.clear();
    }

    /// Mutable access by position, for the replay that rewrites each record in turn.
    pub(super) fn record_mut(&mut self, index: usize) -> Option<&mut ForecastMoveEntry> {
        self.records.get_mut(index)
    }

    /// Drops every record up to and including `receipt_confirmed` and says what the confirm_receipt referred to.
    pub fn drop_confirmed_moves(&mut self, receipt_confirmed: InputCommandNumber) -> ReceiptLookup {
        let (Some(eldest), Some(newest)) = (self.oldest_seq(), self.newest_seq()) else {
            return ReceiptLookup::Retired;
        };
        let too_new = receipt_confirmed.0 > newest.0;
        let too_old = receipt_confirmed.0 < eldest.0.saturating_sub(1);
        if too_new || too_old {
            return ReceiptLookup::Broken;
        }

        let mut matched = None;
        while let Some(front) = self.records.front() {
            if front.seq.0 > receipt_confirmed.0 {
                break;
            }
            let record = self.records.pop_front().expect("front was just seen");
            if record.seq == receipt_confirmed {
                matched = Some(record);
            }
        }
        matched.map_or(ReceiptLookup::Retired, ReceiptLookup::Matched)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(seq: u32) -> ForecastMoveEntry {
        ForecastMoveEntry {
            seq: InputCommandNumber(seq),
            at_tick: Tick(seq),
            cmd: InputCommand::default(),
            input: AvatarState::ZERO,
            output: AvatarState::ZERO,
        }
    }

    fn filled(seqs: &[u32]) -> ForecastMoveLog {
        let mut history = ForecastMoveLog::new();
        for &seq in seqs {
            assert!(!history.add_entry(record(seq)));
        }
        history
    }

    #[test]
    fn sequence_numbers_wrap() {
        assert_eq!(InputCommandNumber(4).next(), InputCommandNumber(5));
        assert_eq!(InputCommandNumber(u32::MAX).next(), InputCommandNumber(0));
    }

    #[test]
    fn a_new_history_is_empty_with_the_default_cap() {
        let history = ForecastMoveLog::new();
        assert!(history.is_empty());
        assert_eq!(history.capacity_limit(), DEFAULT_MOVE_LOG_LIMIT);
        assert_eq!(history.oldest_seq(), None);
        assert_eq!(history.newest_seq(), None);
        assert!(history.newest().is_none());
    }

    #[test]
    fn a_cap_of_zero_still_holds_one_record() {
        let mut history = ForecastMoveLog::holding_at_most(0);
        assert_eq!(history.capacity_limit(), 1);
        assert!(!history.add_entry(record(1)));
        assert!(history.add_entry(record(2)));
        assert_eq!(history.newest_seq(), Some(InputCommandNumber(1)));
    }

    #[test]
    fn a_full_history_refuses_the_new_record() {
        let mut history = ForecastMoveLog::holding_at_most(2);
        assert!(!history.add_entry(record(1)));
        assert!(!history.add_entry(record(2)));
        assert!(history.add_entry(record(3)));
        let held: Vec<_> = history.iter().map(|r| r.seq.0).collect();
        assert_eq!(held, [1, 2]);
    }

    #[test]
    fn oldest_and_newest_follow_the_ends() {
        let history = filled(&[4, 5, 9]);
        assert_eq!(history.oldest_seq(), Some(InputCommandNumber(4)));
        assert_eq!(history.newest_seq(), Some(InputCommandNumber(9)));
        assert_eq!(history.newest().map(|r| r.at_tick), Some(Tick(9)));
        assert_eq!(history.len(), 3);
    }

    #[test]
    fn clearing_empties_the_history() {
        let mut history = filled(&[1, 2]);
        history.clear();
        assert!(history.is_empty());
    }

    #[test]
    fn an_ack_in_the_middle_hands_back_its_record_and_keeps_the_newer_ones() {
        let mut history = filled(&[1, 2, 3, 4]);
        match history.drop_confirmed_moves(InputCommandNumber(2)) {
            ReceiptLookup::Matched(found) => assert_eq!(found.seq, InputCommandNumber(2)),
            other => panic!("expected a match, got {other:?}"),
        }
        let held: Vec<_> = history.iter().map(|r| r.seq.0).collect();
        assert_eq!(held, [3, 4]);
    }

    #[test]
    fn an_ack_for_the_newest_record_empties_the_history() {
        let mut history = filled(&[1, 2]);
        assert!(matches!(
            history.drop_confirmed_moves(InputCommandNumber(2)),
            ReceiptLookup::Matched(_)
        ));
        assert!(history.is_empty());
    }

    #[test]
    fn an_ack_that_falls_in_a_gap_retires_without_a_match() {
        let mut history = filled(&[2, 3, 7, 8]);
        assert_eq!(history.drop_confirmed_moves(InputCommandNumber(5)), ReceiptLookup::Retired);
        let held: Vec<_> = history.iter().map(|r| r.seq.0).collect();
        assert_eq!(held, [7, 8]);
    }

    #[test]
    fn an_ack_just_below_the_oldest_retires_and_keeps_everything() {
        let mut history = filled(&[5, 6]);
        assert_eq!(history.drop_confirmed_moves(InputCommandNumber(4)), ReceiptLookup::Retired);
        assert_eq!(history.len(), 2);
    }

    #[test]
    fn an_ack_from_the_future_or_the_far_past_is_broken_and_changes_nothing() {
        let mut history = filled(&[5, 6]);
        assert_eq!(history.drop_confirmed_moves(InputCommandNumber(7)), ReceiptLookup::Broken);
        assert_eq!(history.drop_confirmed_moves(InputCommandNumber(3)), ReceiptLookup::Broken);
        assert_eq!(history.len(), 2);
    }

    #[test]
    fn an_ack_against_an_empty_history_retires() {
        let mut history = ForecastMoveLog::new();
        assert_eq!(history.drop_confirmed_moves(InputCommandNumber(9)), ReceiptLookup::Retired);
    }

    #[test]
    fn records_can_be_rewritten_in_place() {
        let mut history = filled(&[1, 2]);
        history.record_mut(1).unwrap().at_tick = Tick(40);
        assert_eq!(history.newest().map(|r| r.at_tick), Some(Tick(40)));
        assert!(history.record_mut(2).is_none());
    }
}
