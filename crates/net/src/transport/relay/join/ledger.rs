use std::collections::BTreeMap;

use sim::Snapshot;

/// How many applied snapshots are kept as possible baselines for the authority's deltas.
const KEPT_LIMIT: usize = 64;

/// The snapshots a joined client has applied, by the sequence number the authority gave them.
///
/// The authority encodes against a snapshot it believes the client holds, so the client keeps
/// recent ones. Two are never thrown away: the newest applied, which is what gets
/// acknowledged, and the highest one any delta has been based on.
#[derive(Debug, Default)]
pub(super) struct Ledger {
    kept: BTreeMap<u32, Snapshot>,
    newest: Option<u32>,
    highest_base: u32,
}

impl Ledger {
    pub(super) fn holds(&self, snapshot_seq: u32) -> bool {
        self.kept.contains_key(&snapshot_seq)
    }

    pub(super) fn get(&self, snapshot_seq: u32) -> Option<&Snapshot> {
        self.kept.get(&snapshot_seq)
    }

    /// The highest sequence applied so far.
    pub(super) fn newest(&self) -> Option<u32> {
        self.newest
    }

    pub(super) fn note_applied(&mut self, snapshot_seq: u32) {
        self.newest = Some(
            self.newest
                .map_or(snapshot_seq, |newest| newest.max(snapshot_seq)),
        );
    }

    /// Files an applied snapshot that was encoded against `baseline_seq` (zero for none).
    pub(super) fn record(&mut self, snapshot_seq: u32, baseline_seq: u32, snapshot: Snapshot) {
        self.note_applied(snapshot_seq);
        self.kept.insert(snapshot_seq, snapshot);
        self.highest_base = self.highest_base.max(baseline_seq);
        self.trim();
    }

    fn trim(&mut self) {
        while self.kept.len() > KEPT_LIMIT {
            let victim = self
                .kept
                .keys()
                .copied()
                .find(|&seq| Some(seq) != self.newest && seq != self.highest_base);
            match victim {
                Some(seq) => {
                    self.kept.remove(&seq);
                }
                None => break,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use sim::Tick;

    use super::*;

    fn shot(tick: u32) -> Snapshot {
        Snapshot::unpublished(Tick(tick))
    }

    #[test]
    fn a_recorded_snapshot_can_be_found_again() {
        let mut ledger = Ledger::default();
        assert!(!ledger.holds(3));
        ledger.record(3, 0, shot(30));
        assert!(ledger.holds(3));
        assert_eq!(ledger.get(3).unwrap().tick, Tick(30));
        assert!(ledger.get(4).is_none());
    }

    #[test]
    fn the_newest_sequence_never_goes_backwards() {
        let mut ledger = Ledger::default();
        assert_eq!(ledger.newest(), None);
        ledger.record(5, 0, shot(1));
        ledger.record(3, 0, shot(2));
        assert_eq!(ledger.newest(), Some(5));
        ledger.note_applied(9);
        ledger.note_applied(7);
        assert_eq!(ledger.newest(), Some(9));
    }

    #[test]
    fn the_oldest_snapshots_are_dropped_past_the_limit() {
        let mut ledger = Ledger::default();
        for seq in 1..=70u32 {
            ledger.record(seq, 0, shot(seq));
        }
        assert_eq!(ledger.kept.len(), KEPT_LIMIT);
        assert!(!ledger.holds(1));
        assert!(!ledger.holds(6));
        assert!(ledger.holds(7));
        assert!(ledger.holds(70));
    }

    #[test]
    fn the_baseline_deltas_are_built_on_survives_trimming() {
        let mut ledger = Ledger::default();
        ledger.record(1, 0, shot(1));
        for seq in 2..=70u32 {
            ledger.record(seq, if seq == 2 { 1 } else { 0 }, shot(seq));
        }
        assert!(ledger.holds(1), "snapshot 1 is the highest base in use");
        assert!(!ledger.holds(2));
        assert_eq!(ledger.kept.len(), KEPT_LIMIT);
    }

    #[test]
    fn late_arrivals_with_low_numbers_are_the_first_to_go() {
        let mut ledger = Ledger::default();
        for seq in 10..=80u32 {
            ledger.record(seq, 0, shot(seq));
        }
        ledger.record(2, 0, shot(2));
        ledger.record(1, 0, shot(1));
        assert!(ledger.holds(80), "the newest stays");
        assert!(!ledger.holds(1) && !ledger.holds(2));
        assert_eq!(ledger.kept.len(), KEPT_LIMIT);
    }

    #[test]
    fn trimming_gives_up_rather_than_dropping_the_two_protected_snapshots() {
        let mut ledger = Ledger::default();
        ledger.kept.insert(1, shot(1));
        ledger.kept.insert(2, shot(2));
        ledger.newest = Some(1);
        ledger.highest_base = 2;
        ledger.trim();
        assert_eq!(ledger.kept.len(), 2);
    }
}
