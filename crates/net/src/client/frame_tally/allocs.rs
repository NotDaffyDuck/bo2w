//! Allocation counts between the edges of the client's phases, one slot per phase.

use std::fmt::Write as _;

use bevy::prelude::*;
use frame::{CLIENT_STAGES, PlayerHandoffLink, client_stage_name};

const PHASES: usize = CLIENT_STAGES.len();

/// Allocations (count and bytes) made during each client phase, as of the last frame. A phase
/// whose slot is `None` has not been measured yet.
#[derive(Resource, Debug, Clone, Default)]
pub struct PlayerStageTally {
    pub counts: [Option<u64>; PHASES],
    pub byte_counts: [Option<u64>; PHASES],
    /// The process totals at the previous edge.
    junction_count: Option<u64>,
    junction_bytes: Option<u64>,
}

impl PlayerStageTally {
    /// `phase:count` pairs for the measured phases, comma separated; `None` when none are.
    pub fn heap_count_report(&self) -> Option<String> {
        list_by_phase(&self.counts)
    }

    pub fn heap_bytes_report(&self) -> Option<String> {
        list_by_phase(&self.byte_counts)
    }

    /// The total over the measured phases; `None` when none are.
    pub fn heap_count_total(&self) -> Option<u64> {
        total(&self.counts)
    }

    pub fn heap_bytes_total(&self) -> Option<u64> {
        total(&self.byte_counts)
    }
}

fn list_by_phase(values: &[Option<u64>; PHASES]) -> Option<String> {
    let mut summary_text = String::new();
    for (stage, value) in CLIENT_STAGES.iter().zip(values) {
        let Some(value) = value else { continue };
        if !summary_text.is_empty() {
            summary_text.push(',');
        }
        let _ = write!(summary_text, "{}:{value}", client_stage_name(stage));
    }
    (!summary_text.is_empty()).then_some(summary_text)
}

fn total(values: &[Option<u64>; PHASES]) -> Option<u64> {
    values.iter().flatten().copied().reduce(|a, b| a + b)
}

/// The perf span that times phase `slot`.
fn span_of(slot: usize) -> perf::Span {
    use perf::Span::*;
    match slot {
        0 => TocLoad,
        1 => TocIntake,
        2 => TickPhaseSettle,
        3 => TocInput,
        4 => FrameForecastMillis,
        5 => TocSend,
        6 => FrameShowMillis,
        7 => TocUi,
        8 => FrameEffectsMillis,
        9 => FrameMetricsMillis,
        _ => unreachable!("CLIENT_TOC slot {slot}"),
    }
}

/// Runs at edge `PANEL_MARGIN`: the boundary before phase `PANEL_MARGIN` and after phase `PANEL_MARGIN - 1`. Edge 0 is
/// the start of the frame and edge `PHASES` its end.
fn stamp_edge<const PANEL_MARGIN: usize>(mut census: ResMut<PlayerStageTally>) {
    let now = diag::process_allocations();
    if PANEL_MARGIN == 0 {
        perf::Span::FrameRefreshMillis.begin();
    } else {
        let phase = PANEL_MARGIN - 1;
        span_of(phase).end();
        if let Some(before) = census.junction_count {
            census.counts[phase] = Some(now.allocation_count.saturating_sub(before));
        }
        if let Some(before) = census.junction_bytes {
            census.byte_counts[phase] = Some(now.heap_bytes_handed_out.saturating_sub(before));
        }
    }
    if PANEL_MARGIN < PHASES {
        span_of(PANEL_MARGIN).begin();
    } else {
        perf::Span::FrameRefreshMillis.end();
    }
    census.junction_count = Some(now.allocation_count);
    census.junction_bytes = Some(now.heap_bytes_handed_out);
}

pub(super) fn add_to_app(app: &mut App) {
    app.init_resource::<PlayerStageTally>();
    macro_rules! stamp_at {
        ($($edge:literal)*) => {$(
            app.add_systems(Update, stamp_edge::<$edge>.in_set(PlayerHandoffLink($edge)));
        )*};
    }
    // One edge before each phase and one after the last.
    stamp_at!(0 1 2 3 4 5 6 7 8 9 10);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn name(slot: usize) -> &'static str {
        client_stage_name(&CLIENT_STAGES[slot])
    }

    #[test]
    fn nothing_measured_lists_and_sums_to_none() {
        let census = PlayerStageTally::default();
        assert_eq!(census.heap_count_report(), None);
        assert_eq!(census.heap_bytes_report(), None);
        assert_eq!(census.heap_count_total(), None);
        assert_eq!(census.heap_bytes_total(), None);
    }

    #[test]
    fn only_measured_phases_are_listed_in_phase_order() {
        let mut census = PlayerStageTally::default();
        census.counts[2] = Some(5);
        census.counts[0] = Some(3);
        assert_eq!(census.heap_count_report(), Some(format!("{}:3,{}:5", name(0), name(2))));
        assert_eq!(census.heap_count_total(), Some(8));
    }

    #[test]
    fn bytes_are_kept_apart_from_counts() {
        let mut census = PlayerStageTally::default();
        census.byte_counts[9] = Some(4096);
        assert_eq!(census.heap_count_report(), None);
        assert_eq!(census.heap_bytes_report(), Some(format!("{}:4096", name(9))));
        assert_eq!(census.heap_bytes_total(), Some(4096));
    }

    #[test]
    fn a_measured_zero_still_counts_as_measured() {
        let mut census = PlayerStageTally::default();
        census.counts[1] = Some(0);
        assert_eq!(census.heap_count_total(), Some(0));
        assert!(census.heap_count_report().is_some());
    }

    #[test]
    fn every_phase_has_a_span() {
        for slot in 0..PHASES {
            let _ = span_of(slot);
        }
    }
}
