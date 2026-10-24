//! The commands a client has run but the authority has not yet acknowledged.

use playerstate_t6::game::{PlayerState, UserCmd};
use sim::Tick;
use std::collections::VecDeque;

/// Sequence number a client stamps on each command it sends.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CmdSeq(pub u32);

impl CmdSeq {
    pub const fn next(self) -> Self {
        Self(self.0.wrapping_add(1))
    }
}

/// One predicted command with the player state before and after it ran.
#[derive(Clone, Debug, PartialEq)]
pub struct MoveRecord {
    pub seq: CmdSeq,
    pub tick: Tick,
    pub cmd: UserCmd,
    pub input: PlayerState,
    pub output: PlayerState,
}

/// Most unacknowledged commands kept unless a caller asks for another limit.
pub const DEFAULT_HISTORY_CAP: usize = 256;

/// What the authority's acknowledgement turned out to refer to.
#[derive(Clone, Debug, PartialEq)]
pub enum AckMatch {
    /// The acknowledged command is still held; this is its record.
    Matched(MoveRecord),
    /// Nothing held is that old or the history is empty: the ack only retires old records.
    Retired,
    /// The ack names a command newer than anything sent, or one older than the history reaches.
    Broken,
}

/// Unacknowledged commands in the order they were run, oldest first.
#[derive(Clone, Debug)]
pub struct MoveHistory {
    records: VecDeque<MoveRecord>,
    cap: usize,
}

impl Default for MoveHistory {
    fn default() -> Self {
        Self::with_cap(DEFAULT_HISTORY_CAP)
    }
}

impl MoveHistory {
    pub fn new() -> Self {
        Self::default()
    }

    /// A history that refuses records beyond `cap` (at least one).
    pub fn with_cap(cap: usize) -> Self {
        let cap = cap.max(1);
        Self {
            records: VecDeque::with_capacity(cap.min(DEFAULT_HISTORY_CAP)),
            cap,
        }
    }

    pub fn len(&self) -> usize {
        self.records.len()
    }

    pub fn is_empty(&self) -> bool {
        self.records.is_empty()
    }

    pub fn cap(&self) -> usize {
        self.cap
    }

    pub fn oldest_seq(&self) -> Option<CmdSeq> {
        self.records.front().map(|record| record.seq)
    }

    pub fn newest_seq(&self) -> Option<CmdSeq> {
        self.records.back().map(|record| record.seq)
    }

    pub fn newest(&self) -> Option<&MoveRecord> {
        self.records.back()
    }

    pub fn iter(&self) -> impl Iterator<Item = &MoveRecord> {
        self.records.iter()
    }

    /// Appends `record`. A full history drops the new record instead and returns `true`.
    pub fn push(&mut self, record: MoveRecord) -> bool {
        let full = self.records.len() >= self.cap;
        if !full {
            self.records.push_back(record);
        }
        full
    }

    pub fn clear(&mut self) {
        self.records.clear();
    }

    /// Mutable access by position, for the replay that rewrites each record in turn.
    pub(super) fn record_mut(&mut self, index: usize) -> Option<&mut MoveRecord> {
        self.records.get_mut(index)
    }

    /// Drops every record up to and including `acked` and says what the ack referred to.
    pub fn clear_acknowledged(&mut self, acked: CmdSeq) -> AckMatch {
        let (Some(oldest), Some(newest)) = (self.oldest_seq(), self.newest_seq()) else {
            return AckMatch::Retired;
        };
        let too_new = acked.0 > newest.0;
        let too_old = acked.0 < oldest.0.saturating_sub(1);
        if too_new || too_old {
            return AckMatch::Broken;
        }

        let mut matched = None;
        while let Some(front) = self.records.front() {
            if front.seq.0 > acked.0 {
                break;
            }
            let record = self.records.pop_front().expect("front was just seen");
            if record.seq == acked {
                matched = Some(record);
            }
        }
        matched.map_or(AckMatch::Retired, AckMatch::Matched)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(seq: u32) -> MoveRecord {
        MoveRecord {
            seq: CmdSeq(seq),
            tick: Tick(seq),
            cmd: UserCmd::default(),
            input: PlayerState::ZERO,
            output: PlayerState::ZERO,
        }
    }

    fn filled(seqs: &[u32]) -> MoveHistory {
        let mut history = MoveHistory::new();
        for &seq in seqs {
            assert!(!history.push(record(seq)));
        }
        history
    }

    #[test]
    fn sequence_numbers_wrap() {
        assert_eq!(CmdSeq(4).next(), CmdSeq(5));
        assert_eq!(CmdSeq(u32::MAX).next(), CmdSeq(0));
    }

    #[test]
    fn a_new_history_is_empty_with_the_default_cap() {
        let history = MoveHistory::new();
        assert!(history.is_empty());
        assert_eq!(history.cap(), DEFAULT_HISTORY_CAP);
        assert_eq!(history.oldest_seq(), None);
        assert_eq!(history.newest_seq(), None);
        assert!(history.newest().is_none());
    }

    #[test]
    fn a_cap_of_zero_still_holds_one_record() {
        let mut history = MoveHistory::with_cap(0);
        assert_eq!(history.cap(), 1);
        assert!(!history.push(record(1)));
        assert!(history.push(record(2)));
        assert_eq!(history.newest_seq(), Some(CmdSeq(1)));
    }

    #[test]
    fn a_full_history_refuses_the_new_record() {
        let mut history = MoveHistory::with_cap(2);
        assert!(!history.push(record(1)));
        assert!(!history.push(record(2)));
        assert!(history.push(record(3)));
        let held: Vec<_> = history.iter().map(|r| r.seq.0).collect();
        assert_eq!(held, [1, 2]);
    }

    #[test]
    fn oldest_and_newest_follow_the_ends() {
        let history = filled(&[4, 5, 9]);
        assert_eq!(history.oldest_seq(), Some(CmdSeq(4)));
        assert_eq!(history.newest_seq(), Some(CmdSeq(9)));
        assert_eq!(history.newest().map(|r| r.tick), Some(Tick(9)));
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
        match history.clear_acknowledged(CmdSeq(2)) {
            AckMatch::Matched(found) => assert_eq!(found.seq, CmdSeq(2)),
            other => panic!("expected a match, got {other:?}"),
        }
        let held: Vec<_> = history.iter().map(|r| r.seq.0).collect();
        assert_eq!(held, [3, 4]);
    }

    #[test]
    fn an_ack_for_the_newest_record_empties_the_history() {
        let mut history = filled(&[1, 2]);
        assert!(matches!(
            history.clear_acknowledged(CmdSeq(2)),
            AckMatch::Matched(_)
        ));
        assert!(history.is_empty());
    }

    #[test]
    fn an_ack_that_falls_in_a_gap_retires_without_a_match() {
        let mut history = filled(&[2, 3, 7, 8]);
        assert_eq!(history.clear_acknowledged(CmdSeq(5)), AckMatch::Retired);
        let held: Vec<_> = history.iter().map(|r| r.seq.0).collect();
        assert_eq!(held, [7, 8]);
    }

    #[test]
    fn an_ack_just_below_the_oldest_retires_and_keeps_everything() {
        let mut history = filled(&[5, 6]);
        assert_eq!(history.clear_acknowledged(CmdSeq(4)), AckMatch::Retired);
        assert_eq!(history.len(), 2);
    }

    #[test]
    fn an_ack_from_the_future_or_the_far_past_is_broken_and_changes_nothing() {
        let mut history = filled(&[5, 6]);
        assert_eq!(history.clear_acknowledged(CmdSeq(7)), AckMatch::Broken);
        assert_eq!(history.clear_acknowledged(CmdSeq(3)), AckMatch::Broken);
        assert_eq!(history.len(), 2);
    }

    #[test]
    fn an_ack_against_an_empty_history_retires() {
        let mut history = MoveHistory::new();
        assert_eq!(history.clear_acknowledged(CmdSeq(9)), AckMatch::Retired);
    }

    #[test]
    fn records_can_be_rewritten_in_place() {
        let mut history = filled(&[1, 2]);
        history.record_mut(1).unwrap().tick = Tick(40);
        assert_eq!(history.newest().map(|r| r.tick), Some(Tick(40)));
        assert!(history.record_mut(2).is_none());
    }
}
