//! Counters and per-snapshot results of reconciliation.

use sim::AdoptReport;

use crate::transport::netfields::Deviation;

/// Running totals for one client's prediction, shown in the periodic log line.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PredictionMetrics {
    pub predicted_moves: u64,
    pub snapshots: u64,
    pub acks_matched: u64,
    /// Snapshots whose acknowledged move disagreed with the authority.
    pub deviations: u64,
    pub replayed_moves: u64,
    /// Times the history was thrown away and the next snapshot taken whole.
    pub forced_adopts: u64,
    pub retired_acks: u64,
    /// Moves dropped because the history was full.
    pub evicted_moves: u64,
    pub deepest_history: usize,
    /// Field name and distance of the latest miss.
    pub last_deviation: Option<(&'static str, f32)>,
}

impl PredictionMetrics {
    pub fn report_line(&self) -> String {
        let replay_ratio = match self.snapshots {
            0 => 0.0,
            snapshots => self.replayed_moves as f64 / snapshots as f64,
        };
        let last = match self.last_deviation {
            Some((field, distance)) => format!("{field}@{distance:.1}"),
            None => "none".to_string(),
        };
        format!(
            "pred moves={} snaps={} acks={} retired={} dev={} replay={} ratio={replay_ratio:.2} forced={} evicted={} depth={} last={last}",
            self.predicted_moves,
            self.snapshots,
            self.acks_matched,
            self.retired_acks,
            self.deviations,
            self.replayed_moves,
            self.forced_adopts,
            self.evicted_moves,
            self.deepest_history,
        )
    }
}

/// What one snapshot did to the prediction.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ReconcileOutcome {
    pub adopted: bool,
    pub acked: bool,
    pub retired_ack: bool,
    pub deviation: Option<Deviation>,
    pub replayed_moves: usize,
    pub forced_adopt: bool,
    pub report: AdoptReport,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_idle_client_reports_zeros_and_no_miss() {
        assert_eq!(
            PredictionMetrics::default().report_line(),
            "pred moves=0 snaps=0 acks=0 retired=0 dev=0 replay=0 ratio=0.00 forced=0 evicted=0 depth=0 last=none"
        );
    }

    #[test]
    fn the_line_carries_every_counter_in_order() {
        let metrics = PredictionMetrics {
            predicted_moves: 100,
            snapshots: 4,
            acks_matched: 3,
            deviations: 2,
            replayed_moves: 6,
            forced_adopts: 1,
            retired_acks: 5,
            evicted_moves: 7,
            deepest_history: 9,
            last_deviation: Some(("origin", 12.34)),
        };
        assert_eq!(
            metrics.report_line(),
            "pred moves=100 snaps=4 acks=3 retired=5 dev=2 replay=6 ratio=1.50 forced=1 evicted=7 depth=9 last=origin@12.3"
        );
    }

    #[test]
    fn a_default_outcome_reports_nothing_happened() {
        let outcome = ReconcileOutcome::default();
        assert!(!outcome.adopted && !outcome.acked && !outcome.retired_ack);
        assert!(!outcome.forced_adopt);
        assert_eq!(outcome.replayed_moves, 0);
        assert!(outcome.deviation.is_none());
    }
}
