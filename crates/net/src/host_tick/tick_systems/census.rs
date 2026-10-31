//! Per-frame measurements of the authority loop: how many fixed steps ran, what each phase
//! allocated, and what the listen-server fan-out sent to its own player.

use bevy::prelude::*;

use super::run_gate::host_due_for_step;

/// Number of fixed steps that ran in the last rendered frame.
#[derive(Resource, Debug, Default, Clone)]
pub struct StepUpdateTally {
    pub steps_last_frame: u32,
    running: u32,
    timing_on: bool,
}

impl StepUpdateTally {
    fn open_span(&mut self) {
        if !self.timing_on {
            perf::Span::FrameTicksMillis.begin();
            self.timing_on = true;
        }
    }

    fn count_step(&mut self) {
        self.running = self.running.saturating_add(1);
    }

    fn close_frame(&mut self) {
        if self.timing_on {
            perf::Span::FrameTicksMillis.end();
            self.timing_on = false;
        }
        self.steps_last_frame = self.running;
        self.running = 0;
    }
}

pub(super) fn begin_fixed_census(mut census: ResMut<StepUpdateTally>) {
    census.open_span();
}

pub(super) fn finish_steady_tally(mut census: ResMut<StepUpdateTally>) {
    census.count_step();
}

pub(super) fn post_step_census(mut census: ResMut<StepUpdateTally>) {
    census.close_frame();
}

const PHASES: usize = frame::SERVER_STAGES.len();

/// Allocation counts and bytes per authority phase, published once per rendered frame.
#[derive(Resource, Debug, Clone, Default)]
pub struct HostStageTally {
    pub alloc_counts: [Option<u64>; PHASES],

    pub alloc_bytes: [Option<u64>; PHASES],
    pending_n: [Option<u64>; PHASES],
    pending_bytes: [Option<u64>; PHASES],
    last_count: Option<u64>,
    last_bytes: Option<u64>,
}

impl HostStageTally {
    pub fn heap_count_report(&self) -> Option<String> {
        join_by_phase(&self.alloc_counts)
    }

    pub fn heap_bytes_report(&self) -> Option<String> {
        join_by_phase(&self.alloc_bytes)
    }

    pub fn heap_count_total(&self) -> Option<u64> {
        total(&self.alloc_counts)
    }

    pub fn heap_bytes_total(&self) -> Option<u64> {
        total(&self.alloc_bytes)
    }

    /// Credit the allocations since the previous seam to the phase that just ended.
    fn close_phase(&mut self, idx: usize, count_now: u64, bytes_now: u64) {
        if let Some(before) = self.last_count {
            add_to(&mut self.pending_n[idx], count_now.saturating_sub(before));
        }
        if let Some(before) = self.last_bytes {
            add_to(&mut self.pending_bytes[idx], bytes_now.saturating_sub(before));
        }
    }

    fn mark_seam(&mut self, count_now: u64, bytes_now: u64) {
        self.last_count = Some(count_now);
        self.last_bytes = Some(bytes_now);
    }

    fn roll_frame(&mut self) {
        self.alloc_counts = std::mem::take(&mut self.pending_n);
        self.alloc_bytes = std::mem::take(&mut self.pending_bytes);
        self.last_count = None;
        self.last_bytes = None;
    }
}

fn add_to(slot: &mut Option<u64>, amount: u64) {
    *slot = Some(slot.unwrap_or(0) + amount);
}

/// `Name:value` pairs in phase order, comma separated; phases with no value are skipped.
fn join_by_phase(values: &[Option<u64>; PHASES]) -> Option<String> {
    let parts: Vec<String> = frame::SERVER_STAGES
        .iter()
        .zip(values)
        .filter_map(|(phase, value)| Some(format!("{}:{}", frame::server_stage_name(phase), (*value)?)))
        .collect();
    (!parts.is_empty()).then(|| parts.join(","))
}

fn total(values: &[Option<u64>; PHASES]) -> Option<u64> {
    values.iter().flatten().copied().reduce(|a, b| a + b)
}

fn phase_span(idx: usize) -> perf::Span {
    const SPANS: [perf::Span; 7] = [
        perf::Span::TickPhaseAdvance,
        perf::Span::TickPhaseIntake,
        perf::Span::TickPhaseCollect,
        perf::Span::TickPhaseStep,
        perf::Span::TickPhaseCapture,
        perf::Span::TickPhaseSpread,
        perf::Span::TickPhaseHousekeeping,
    ];
    SPANS
        .get(idx)
        .copied()
        .unwrap_or_else(|| unreachable!("AUTHORITY_TOC slot {idx}"))
}

/// Seam system: edge `PANEL_MARGIN` closes phase `PANEL_MARGIN - 1` and opens phase `PANEL_MARGIN`.
fn stamp_edge<const PANEL_MARGIN: u8>(mut census: ResMut<HostStageTally>) {
    let heap = diag::process_allocations();
    let (count_now, bytes_now) = (heap.allocation_count, heap.heap_bytes_handed_out);
    if let Some(finished) = (PANEL_MARGIN as usize).checked_sub(1) {
        phase_span(finished).end();
        census.close_phase(finished, count_now, bytes_now);
    }
    if (PANEL_MARGIN as usize) < PHASES {
        phase_span(PANEL_MARGIN as usize).begin();
    }
    census.mark_seam(count_now, bytes_now);
}

pub(super) fn publish_phase_census(mut census: ResMut<HostStageTally>) {
    census.roll_frame();
}

/// Install one seam system per edge, each gated on the authority actually ticking.
pub(super) fn install_phase_seams(app: &mut App) {
    macro_rules! install {
        ($($seam:literal),+) => {$(
            app.add_systems(
                FixedUpdate,
                stamp_edge::<$seam>
                    .in_set(frame::ServerHandoffLink($seam))
                    .run_if(host_due_for_step),
            );
        )+};
    }
    install!(0, 1, 2, 3, 4, 5, 6, 7);
}

/// What the listen fan-out handed the local player on the last tick (for the debug dump).
#[derive(Resource, Debug, Clone, Default)]
pub struct LocalHostFanoutTally {
    pub fanned_tick: Option<u32>,
    pub scene_anchor: Option<[f32; 3]>,
    pub state_position: Option<[f32; 3]>,
    pub transmitted_position: Option<[f32; 3]>,
    pub viewer_seat_installed: Option<i32>,
    pub world_misc_bits: Option<i32>,
    pub transmitted_misc_flags: Option<i32>,
    pub transmitted_step_gap: Option<i32>,

    pub seat_history_clock_millis: Option<i32>,

    pub seat_watched_receiver: Option<i32>,

    pub seat_query_tick: Option<i32>,

    pub seat_reached_millis: Option<i32>,

    pub seat_rebaseline_millis: Option<i32>,

    pub seat_watched_live_position: Option<[f32; 3]>,

    pub seat_watched_life_stage: Option<&'static str>,

    pub seat_scene_in_history: Option<i32>,
}

impl LocalHostFanoutTally {
    /// Drop every field that only means something while a killcam seat is applied.
    pub(super) fn clear_seat(&mut self) {
        self.seat_history_clock_millis = None;
        self.seat_watched_receiver = None;
        self.seat_query_tick = None;
        self.seat_reached_millis = None;
        self.seat_rebaseline_millis = None;
        self.seat_watched_live_position = None;
        self.seat_watched_life_stage = None;
        self.seat_scene_in_history = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn totals_ignore_empty_slots() {
        let mut values = [None; PHASES];
        assert_eq!(total(&values), None);
        values[1] = Some(4);
        values[5] = Some(6);
        assert_eq!(total(&values), Some(10));
    }

    #[test]
    fn join_lists_only_filled_phases_in_order() {
        let mut values = [None; PHASES];
        assert_eq!(join_by_phase(&values), None);
        values[2] = Some(9);
        values[0] = Some(1);
        let text = join_by_phase(&values).expect("two phases filled");
        let pieces: Vec<&str> = text.split(',').collect();
        assert_eq!(pieces.len(), 2);
        assert!(pieces[0].ends_with(":1"));
        assert!(pieces[1].ends_with(":9"));
    }

    #[test]
    fn closing_a_phase_credits_the_growth_since_the_last_seam() {
        let mut census = HostStageTally::default();
        census.close_phase(0, 50, 500);
        assert_eq!(census.pending_n[0], None, "no seam yet, nothing to credit");
        census.mark_seam(10, 100);
        census.close_phase(0, 25, 160);
        census.close_phase(0, 30, 200);
        assert_eq!(census.pending_n[0], Some(15 + 20));
        assert_eq!(census.pending_bytes[0], Some(60 + 100));
    }

    #[test]
    fn counters_that_went_backwards_credit_zero() {
        let mut census = HostStageTally::default();
        census.mark_seam(100, 1000);
        census.close_phase(3, 90, 900);
        assert_eq!(census.pending_n[3], Some(0));
        assert_eq!(census.pending_bytes[3], Some(0));
    }

    #[test]
    fn rolling_a_frame_publishes_and_resets() {
        let mut census = HostStageTally::default();
        census.mark_seam(1, 1);
        census.close_phase(1, 5, 50);
        census.roll_frame();
        assert_eq!(census.alloc_counts[1], Some(4));
        assert_eq!(census.alloc_bytes[1], Some(49));
        assert_eq!(census.pending_n[1], None);
        assert_eq!(census.last_count, None);
        assert_eq!(census.heap_count_total(), Some(4));
        assert!(census.heap_bytes_report().is_some_and(|s| s.ends_with(":49")));
    }

    #[test]
    fn fixed_census_counts_steps_per_frame() {
        let mut census = StepUpdateTally::default();
        census.count_step();
        census.count_step();
        assert_eq!(census.steps_last_frame, 0, "published only at the end of the frame");
        census.close_frame();
        assert_eq!(census.steps_last_frame, 2);
        census.close_frame();
        assert_eq!(census.steps_last_frame, 0);
    }

    #[test]
    fn clearing_the_seat_leaves_the_other_fields() {
        let mut census = LocalHostFanoutTally {
            fanned_tick: Some(3),
            viewer_seat_installed: Some(1),
            seat_query_tick: Some(2),
            seat_watched_life_stage: Some("Alive"),
            seat_scene_in_history: Some(1),
            ..Default::default()
        };
        census.clear_seat();
        assert_eq!(census.fanned_tick, Some(3));
        assert_eq!(census.viewer_seat_installed, Some(1));
        assert_eq!(census.seat_query_tick, None);
        assert_eq!(census.seat_watched_life_stage, None);
        assert_eq!(census.seat_scene_in_history, None);
    }

    #[test]
    fn every_toc_slot_has_a_span() {
        for slot in 0..PHASES {
            let _ = phase_span(slot);
        }
    }
}
