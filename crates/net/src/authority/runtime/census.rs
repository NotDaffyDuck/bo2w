//! Per-frame measurements of the authority loop: how many fixed steps ran, what each phase
//! allocated, and what the listen-server fan-out sent to its own player.

use bevy::prelude::*;

use super::lifecycle::authority_should_tick;

/// Number of fixed steps that ran in the last rendered frame.
#[derive(Resource, Debug, Default, Clone)]
pub struct FixedUpdateCensus {
    pub steps: u32,
    running: u32,
    span_open: bool,
}

impl FixedUpdateCensus {
    fn open_span(&mut self) {
        if !self.span_open {
            perf::Span::FramesFixedMs.begin();
            self.span_open = true;
        }
    }

    fn count_step(&mut self) {
        self.running = self.running.saturating_add(1);
    }

    fn close_frame(&mut self) {
        if self.span_open {
            perf::Span::FramesFixedMs.end();
            self.span_open = false;
        }
        self.steps = self.running;
        self.running = 0;
    }
}

pub(super) fn begin_fixed_census(mut census: ResMut<FixedUpdateCensus>) {
    census.open_span();
}

pub(super) fn end_fixed_census(mut census: ResMut<FixedUpdateCensus>) {
    census.count_step();
}

pub(super) fn publish_fixed_census(mut census: ResMut<FixedUpdateCensus>) {
    census.close_frame();
}

const PHASES: usize = frame::AUTHORITY_TOC.len();

/// Allocation counts and bytes per authority phase, published once per rendered frame.
#[derive(Resource, Debug, Clone, Default)]
pub struct AuthorityPhaseCensus {
    pub n: [Option<u64>; PHASES],

    pub bytes: [Option<u64>; PHASES],
    pending_n: [Option<u64>; PHASES],
    pending_bytes: [Option<u64>; PHASES],
    last_count: Option<u64>,
    last_bytes: Option<u64>,
}

impl AuthorityPhaseCensus {
    pub fn alloc_n(&self) -> Option<String> {
        join_by_phase(&self.n)
    }

    pub fn alloc_bytes(&self) -> Option<String> {
        join_by_phase(&self.bytes)
    }

    pub fn alloc_sum_n(&self) -> Option<u64> {
        total(&self.n)
    }

    pub fn alloc_sum_bytes(&self) -> Option<u64> {
        total(&self.bytes)
    }

    /// Credit the allocations since the previous seam to the phase that just ended.
    fn close_phase(&mut self, slot: usize, count_now: u64, bytes_now: u64) {
        if let Some(before) = self.last_count {
            add_to(&mut self.pending_n[slot], count_now.saturating_sub(before));
        }
        if let Some(before) = self.last_bytes {
            add_to(&mut self.pending_bytes[slot], bytes_now.saturating_sub(before));
        }
    }

    fn mark_seam(&mut self, count_now: u64, bytes_now: u64) {
        self.last_count = Some(count_now);
        self.last_bytes = Some(bytes_now);
    }

    fn roll_frame(&mut self) {
        self.n = std::mem::take(&mut self.pending_n);
        self.bytes = std::mem::take(&mut self.pending_bytes);
        self.last_count = None;
        self.last_bytes = None;
    }
}

fn add_to(slot: &mut Option<u64>, amount: u64) {
    *slot = Some(slot.unwrap_or(0) + amount);
}

/// `Name:value` pairs in phase order, comma separated; phases with no value are skipped.
fn join_by_phase(values: &[Option<u64>; PHASES]) -> Option<String> {
    let parts: Vec<String> = frame::AUTHORITY_TOC
        .iter()
        .zip(values)
        .filter_map(|(phase, value)| Some(format!("{}:{}", frame::authority_set_name(phase), (*value)?)))
        .collect();
    (!parts.is_empty()).then(|| parts.join(","))
}

fn total(values: &[Option<u64>; PHASES]) -> Option<u64> {
    values.iter().flatten().copied().reduce(|a, b| a + b)
}

fn phase_span(slot: usize) -> perf::Span {
    const SPANS: [perf::Span; 7] = [
        perf::Span::FixedTocAdvance,
        perf::Span::FixedTocIngress,
        perf::Span::FixedTocGather,
        perf::Span::FixedTocStep,
        perf::Span::FixedTocSnapshot,
        perf::Span::FixedTocFanout,
        perf::Span::FixedTocBookkeeping,
    ];
    SPANS
        .get(slot)
        .copied()
        .unwrap_or_else(|| unreachable!("AUTHORITY_TOC slot {slot}"))
}

/// Seam system: edge `EDGE` closes phase `EDGE - 1` and opens phase `EDGE`.
fn stamp_edge<const EDGE: u8>(mut census: ResMut<AuthorityPhaseCensus>) {
    let alloc = diag::process_allocations();
    let (count_now, bytes_now) = (alloc.process_allocations, alloc.process_allocation_bytes);
    if let Some(finished) = (EDGE as usize).checked_sub(1) {
        phase_span(finished).end();
        census.close_phase(finished, count_now, bytes_now);
    }
    if (EDGE as usize) < PHASES {
        phase_span(EDGE as usize).begin();
    }
    census.mark_seam(count_now, bytes_now);
}

pub(super) fn publish_phase_census(mut census: ResMut<AuthorityPhaseCensus>) {
    census.roll_frame();
}

/// Install one seam system per edge, each gated on the authority actually ticking.
pub(super) fn install_phase_seams(app: &mut App) {
    macro_rules! install {
        ($($edge:literal),+) => {$(
            app.add_systems(
                FixedUpdate,
                stamp_edge::<$edge>
                    .in_set(frame::AuthorityEdge($edge))
                    .run_if(authority_should_tick),
            );
        )+};
    }
    install!(0, 1, 2, 3, 4, 5, 6, 7);
}

/// What the listen fan-out handed the local player on the last tick (for the debug dump).
#[derive(Resource, Debug, Clone, Default)]
pub struct ListenFanoutCensus {
    pub tick: Option<u32>,
    pub world_origin: Option<[f32; 3]>,
    pub snapshot_origin: Option<[f32; 3]>,
    pub sent_origin: Option<[f32; 3]>,
    pub seat_applied: Option<i32>,
    pub world_other_flags: Option<i32>,
    pub sent_other_flags: Option<i32>,
    pub sent_delta_time: Option<i32>,

    pub seat_archivetime_ms: Option<i32>,

    pub seat_focus_client: Option<i32>,

    pub seat_lookup_tick: Option<i32>,

    pub seat_attained_ms: Option<i32>,

    pub seat_rebase_ms: Option<i32>,

    pub seat_focus_live_origin: Option<[f32; 3]>,

    pub seat_focus_lifecycle: Option<&'static str>,

    pub seat_world_archived: Option<i32>,
}

impl ListenFanoutCensus {
    /// Drop every field that only means something while a killcam seat is applied.
    pub(super) fn clear_seat(&mut self) {
        self.seat_archivetime_ms = None;
        self.seat_focus_client = None;
        self.seat_lookup_tick = None;
        self.seat_attained_ms = None;
        self.seat_rebase_ms = None;
        self.seat_focus_live_origin = None;
        self.seat_focus_lifecycle = None;
        self.seat_world_archived = None;
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
        let mut census = AuthorityPhaseCensus::default();
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
        let mut census = AuthorityPhaseCensus::default();
        census.mark_seam(100, 1000);
        census.close_phase(3, 90, 900);
        assert_eq!(census.pending_n[3], Some(0));
        assert_eq!(census.pending_bytes[3], Some(0));
    }

    #[test]
    fn rolling_a_frame_publishes_and_resets() {
        let mut census = AuthorityPhaseCensus::default();
        census.mark_seam(1, 1);
        census.close_phase(1, 5, 50);
        census.roll_frame();
        assert_eq!(census.n[1], Some(4));
        assert_eq!(census.bytes[1], Some(49));
        assert_eq!(census.pending_n[1], None);
        assert_eq!(census.last_count, None);
        assert_eq!(census.alloc_sum_n(), Some(4));
        assert!(census.alloc_bytes().is_some_and(|s| s.ends_with(":49")));
    }

    #[test]
    fn fixed_census_counts_steps_per_frame() {
        let mut census = FixedUpdateCensus::default();
        census.count_step();
        census.count_step();
        assert_eq!(census.steps, 0, "published only at the end of the frame");
        census.close_frame();
        assert_eq!(census.steps, 2);
        census.close_frame();
        assert_eq!(census.steps, 0);
    }

    #[test]
    fn clearing_the_seat_leaves_the_other_fields() {
        let mut census = ListenFanoutCensus {
            tick: Some(3),
            seat_applied: Some(1),
            seat_lookup_tick: Some(2),
            seat_focus_lifecycle: Some("Alive"),
            seat_world_archived: Some(1),
            ..Default::default()
        };
        census.clear_seat();
        assert_eq!(census.tick, Some(3));
        assert_eq!(census.seat_applied, Some(1));
        assert_eq!(census.seat_lookup_tick, None);
        assert_eq!(census.seat_focus_lifecycle, None);
        assert_eq!(census.seat_world_archived, None);
    }

    #[test]
    fn every_toc_slot_has_a_span() {
        for slot in 0..PHASES {
            let _ = phase_span(slot);
        }
    }
}
