//! The frame clock and the schedule spans it encloses.
//!
//! The clock closes the row for the frame that just ran and opens the next at
//! the very top of `First`. Closing it from inside `Update` would cut the row
//! while `Update` is still open, and that schedule would report a fraction of
//! itself with the rest carried into the next row.

use bevy::app::{RunFixedMainLoop, RunFixedMainLoopSystems};
use bevy::prelude::*;
use bevy::transform::TransformSystems;
use perf::Span;

/// Closes the previous frame's row, opens this one's, and publishes the
/// process-wide allocation count since the last bookend, so the count and the
/// frame it belongs to are one interval.
fn frame_clock(mut open: Local<bool>, mut last_total: Local<Option<u64>>) {
    if std::mem::replace(&mut *open, true) {
        Span::FrameClockMillis.end();
    }
    // When the allocator is not counting the total is a constant zero, and a
    // delta of it would claim "no allocations" where the truth is "unmeasured".
    if perf::stats::enabled() && diag::counting_enabled() {
        let total = diag::process_allocations().allocation_count;
        if let Some(before) = last_total.replace(total) {
            perf::Counter::TallyHeapRequests.emit(total.saturating_sub(before) as f64);
        }
    }
    Span::FrameClockMillis.begin();
}

fn span_start(span: Span) -> impl Fn() {
    move || span.begin()
}

fn span_end(span: Span) -> impl Fn() {
    move || span.end()
}

/// The clock and the pre-update and post-update spans, registered together
/// because their order against one another is what is being measured.
pub(crate) fn register_frame_spans(app: &mut App) {
    app.add_systems(
        First,
        (frame_clock, span_start(Span::FrameBeforeRefreshMillis)).chain(),
    )
    .add_systems(
        RunFixedMainLoop,
        span_end(Span::FrameBeforeRefreshMillis).in_set(RunFixedMainLoopSystems::BeforeFixedMainLoop),
    )
    .add_systems(
        PostUpdate,
        span_start(Span::FrameAfterRefreshMillis).before(TransformSystems::Propagate),
    )
    .add_systems(Last, span_end(Span::FrameAfterRefreshMillis));
}
