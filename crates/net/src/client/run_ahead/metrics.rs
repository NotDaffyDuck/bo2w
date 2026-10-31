//! Counters and per-snapshot results of reconciliation.

use sim::TakeoverReport;

use crate::net_links::crossing_fields::FieldDrift;

/// Running totals for one client's prediction, shown in the periodic log line.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ForecastCounters {
    pub forecast_inputs: u64,
    pub frames_seen: u64,
    pub receipts_paired: u64,
    /// Snapshots whose acknowledged move disagreed with the authority.
    pub drift_samples: u64,
    pub rerun_move_total: u64,
    /// Times the history was thrown away and the next snapshot taken whole.
    pub compelled_takeovers: u64,
    pub discarded_receipts: u64,
    /// Moves dropped because the history was full.
    pub displaced_inputs: u64,
    pub oldest_retained_frame: usize,
    /// Field name and distance of the latest miss.
    pub latest_drift: Option<(&'static str, f32)>,
}

impl ForecastCounters {
    pub fn summary_text(&self) -> String {
        let replay_ratio = match self.frames_seen {
            0 => 0.0,
            snapshots => self.rerun_move_total as f64 / snapshots as f64,
        };
        let last = match self.latest_drift {
            Some((field, distance)) => format!("{field}@{distance:.1}"),
            None => "none".to_string(),
        };
        format!(
            "pred moves={} snaps={} acks={} retired={} dev={} replay={} ratio={replay_ratio:.2} forced={} evicted={} depth={} last={last}",
            self.forecast_inputs,
            self.frames_seen,
            self.receipts_paired,
            self.discarded_receipts,
            self.drift_samples,
            self.rerun_move_total,
            self.compelled_takeovers,
            self.displaced_inputs,
            self.oldest_retained_frame,
        )
    }
}

/// What one snapshot did to the prediction.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct CorrectionResult {
    pub took_host_state: bool,
    pub receipt_confirmed: bool,
    pub discarded_receipt: bool,
    pub deviation: Option<FieldDrift>,
    pub rerun_move_total: usize,
    pub compelled_takeover: bool,
    pub report: TakeoverReport,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_idle_client_reports_zeros_and_no_miss() {
        assert_eq!(
            ForecastCounters::default().summary_text(),
            "pred moves=0 snaps=0 acks=0 retired=0 dev=0 replay=0 ratio=0.00 forced=0 evicted=0 depth=0 last=none"
        );
    }

    #[test]
    fn the_line_carries_every_counter_in_order() {
        let metrics = ForecastCounters {
            forecast_inputs: 100,
            frames_seen: 4,
            receipts_paired: 3,
            drift_samples: 2,
            rerun_move_total: 6,
            compelled_takeovers: 1,
            discarded_receipts: 5,
            displaced_inputs: 7,
            oldest_retained_frame: 9,
            latest_drift: Some(("origin", 12.34)),
        };
        assert_eq!(
            metrics.summary_text(),
            "pred moves=100 snaps=4 acks=3 retired=5 dev=2 replay=6 ratio=1.50 forced=1 evicted=7 depth=9 last=origin@12.3"
        );
    }

    #[test]
    fn a_default_outcome_reports_nothing_happened() {
        let outcome = CorrectionResult::default();
        assert!(!outcome.took_host_state && !outcome.receipt_confirmed && !outcome.discarded_receipt);
        assert!(!outcome.compelled_takeover);
        assert_eq!(outcome.rerun_move_total, 0);
        assert!(outcome.deviation.is_none());
    }
}
