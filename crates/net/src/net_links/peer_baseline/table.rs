//! The snapshots sent to one peer, keyed by the sequence number they went out under.

use std::collections::BTreeMap;

use sim::{Snapshot, Tick};

/// Sent snapshots kept for use as a delta baseline once the peer acknowledges one.
///
/// Sequence 0 always means "no baseline": a full snapshot with nothing to diff against.
#[derive(Debug, Default)]
pub struct ConfirmedReferenceTable {
    dispatched: BTreeMap<u32, Snapshot>,
    newest_confirmed: Option<u32>,
    retention_cap: usize,
}

impl ConfirmedReferenceTable {
    /// A table keeping at most `retention_cap` snapshots (never fewer than one).
    pub fn new(retention_cap: usize) -> Self {
        Self { dispatched: BTreeMap::new(), newest_confirmed: None, retention_cap: retention_cap.max(1) }
    }

    /// Keep a snapshot just sent as `frame_number_on_wire`, dropping old ones if over the limit.
    pub fn keep_frame(&mut self, frame_number_on_wire: u32, snapshot: Snapshot) {
        self.dispatched.insert(frame_number_on_wire, snapshot);
        self.trim();
    }

    /// The peer confirms it has `frame_number_on_wire`. Returns that snapshot's tick, or `None` when
    /// the sequence is not held (nothing changes then). Older snapshots are dropped, and an
    /// acknowledgement lower than one already made does not move the baseline back.
    pub fn confirm_receipt(&mut self, frame_number_on_wire: u32) -> Option<Tick> {
        let acked_tick = self.dispatched.get(&frame_number_on_wire)?.tick;
        let receipt_confirmed = self.newest_confirmed.map_or(frame_number_on_wire, |prev| prev.max(frame_number_on_wire));
        self.newest_confirmed = Some(receipt_confirmed);
        self.dispatched.retain(|&seq, _| seq >= receipt_confirmed);
        Some(acked_tick)
    }

    pub fn newest_confirmed(&self) -> Option<u32> {
        self.newest_confirmed
    }

    /// The sequence a new snapshot should be encoded against: the acknowledged one while it
    /// is still held, otherwise 0.
    pub fn reference_number_for_packing(&self) -> u32 {
        self.held_ack().unwrap_or(0)
    }

    /// The snapshot to encode against, when there is one.
    pub fn reference_state_for_packing(&self) -> Option<&Snapshot> {
        self.dispatched.get(&self.newest_confirmed?)
    }

    /// Whether `reference_frame_number` is usable: 0 always is; anything else must be the held
    /// acknowledged snapshot.
    pub fn can_diff_from(&self, reference_frame_number: u32) -> bool {
        reference_frame_number == 0 || self.held_ack() == Some(reference_frame_number)
    }

    /// Forget an acknowledgement whose snapshot is no longer held. True when one was
    /// forgotten, meaning the peer needs a fresh full snapshot.
    pub fn forget_absent_receipt(&mut self) -> bool {
        if self.newest_confirmed.is_some() && self.held_ack().is_none() {
            self.newest_confirmed = None;
            return true;
        }
        false
    }

    /// The acknowledged sequence, if its snapshot is still here.
    fn held_ack(&self) -> Option<u32> {
        self.newest_confirmed.filter(|seq| self.dispatched.contains_key(seq))
    }

    /// Never dropped while trimming: the acknowledged one, the newest, and, before anything
    /// is acknowledged, the oldest.
    fn is_held_back(&self, move_seq: u32) -> bool {
        self.newest_confirmed == Some(move_seq)
            || self.dispatched.keys().next_back() == Some(&move_seq)
            || (self.newest_confirmed.is_none() && self.dispatched.keys().next() == Some(&move_seq))
    }

    /// Drop the oldest unpinned snapshots until within the limit (or only pinned ones are left).
    fn trim(&mut self) {
        while self.dispatched.len() > self.retention_cap {
            let Some(victim) = self.dispatched.keys().copied().find(|&seq| !self.is_held_back(seq)) else {
                break;
            };
            self.dispatched.remove(&victim);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snap(tick: u32) -> Snapshot {
        Snapshot::unpublished(Tick(tick))
    }

    fn table_with(max: usize, seqs: &[u32]) -> ConfirmedReferenceTable {
        let mut table = ConfirmedReferenceTable::new(max);
        for &seq in seqs {
            table.keep_frame(seq, snap(seq * 10));
        }
        table
    }

    fn held(table: &ConfirmedReferenceTable) -> Vec<u32> {
        table.dispatched.keys().copied().collect()
    }

    #[test]
    fn a_fresh_table_has_no_baseline() {
        let table = ConfirmedReferenceTable::new(4);
        assert_eq!(table.newest_confirmed(), None);
        assert_eq!(table.reference_number_for_packing(), 0);
        assert!(table.reference_state_for_packing().is_none());
        assert!(table.can_diff_from(0));
        assert!(!table.can_diff_from(1));
    }

    #[test]
    fn the_limit_is_at_least_one_but_pinned_snapshots_may_exceed_it() {
        let mut table = ConfirmedReferenceTable::new(0);
        assert_eq!(table.retention_cap, 1);
        table.keep_frame(1, snap(1));
        table.keep_frame(2, snap(2));
        assert_eq!(held(&table), [1, 2], "with no ack the oldest and the newest are both kept");
    }

    #[test]
    fn before_any_ack_the_oldest_and_newest_survive_trimming() {
        let table = table_with(3, &[1, 2, 3, 4, 5]);
        assert_eq!(held(&table), [1, 4, 5]);
    }

    #[test]
    fn an_ack_returns_the_tick_and_drops_older_snapshots() {
        let mut table = table_with(8, &[1, 2, 3, 4]);
        assert_eq!(table.confirm_receipt(3), Some(Tick(30)));
        assert_eq!(held(&table), [3, 4]);
        assert_eq!(table.newest_confirmed(), Some(3));
        assert_eq!(table.reference_number_for_packing(), 3);
        assert_eq!(table.reference_state_for_packing().map(|s| s.tick), Some(Tick(30)));
    }

    #[test]
    fn an_ack_for_a_sequence_not_held_changes_nothing() {
        let mut table = table_with(8, &[2, 3]);
        assert_eq!(table.confirm_receipt(9), None);
        assert_eq!(table.newest_confirmed(), None);
        assert_eq!(held(&table), [2, 3]);
    }

    #[test]
    fn a_late_lower_ack_does_not_move_the_baseline_back() {
        let mut table = table_with(8, &[1, 2, 3]);
        table.confirm_receipt(3);
        table.keep_frame(2, snap(20));
        assert_eq!(table.confirm_receipt(2), Some(Tick(20)));
        assert_eq!(table.newest_confirmed(), Some(3));
        assert_eq!(held(&table), [3]);
    }

    #[test]
    fn the_acked_snapshot_survives_trimming_while_newer_ones_come_and_go() {
        let mut table = table_with(3, &[1, 2]);
        table.confirm_receipt(1);
        for seq in 3..=8 {
            table.keep_frame(seq, snap(seq * 10));
        }
        assert_eq!(held(&table), [1, 7, 8]);
        assert_eq!(table.reference_number_for_packing(), 1);
    }

    #[test]
    fn encoding_is_allowed_only_against_zero_or_the_held_ack() {
        let mut table = table_with(8, &[1, 2, 3]);
        table.confirm_receipt(2);
        assert!(table.can_diff_from(0));
        assert!(table.can_diff_from(2));
        assert!(!table.can_diff_from(1));
        assert!(!table.can_diff_from(3));
    }

    #[test]
    fn a_lost_ack_is_cleared_once_and_reported() {
        let mut table = table_with(8, &[1, 2]);
        table.confirm_receipt(2);
        table.dispatched.remove(&2);
        assert_eq!(table.reference_number_for_packing(), 0);
        assert!(!table.can_diff_from(2));
        assert!(table.reference_state_for_packing().is_none());
        assert!(table.forget_absent_receipt());
        assert_eq!(table.newest_confirmed(), None);
        assert!(!table.forget_absent_receipt());
    }

    #[test]
    fn nothing_is_cleared_while_the_ack_is_held_or_absent() {
        let mut table = table_with(8, &[1]);
        assert!(!table.forget_absent_receipt());
        table.confirm_receipt(1);
        assert!(!table.forget_absent_receipt());
        assert_eq!(table.newest_confirmed(), Some(1));
    }
}
