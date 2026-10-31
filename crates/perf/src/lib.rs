//! Hooks the engine calls so a profiler could see where a frame goes: spans
//! around work, counters for sampled values, and one call per notable game or
//! render event.
//!
//! No recorder is attached in this build. Every hook is empty and
//! [`recording`] is false, so a caller that gathers values only to hand them
//! to a counter skips that work as well.

mod events;
mod hooks;

pub use events::*;
pub use hooks::{Counter, Span, TimedStretchCloser};

/// Whether anything is listening to the hooks.
#[inline]
pub fn recording() -> bool {
    false
}

/// The per-frame statistics side.
pub mod stats {
    /// Whether per-frame statistics are being gathered.
    #[inline]
    pub fn enabled() -> bool {
        crate::recording()
    }
}

/// The frame table a recorder keeps: which frame is open, and what state it
/// was drawn in.
pub mod frames {
    /// Conditions a frame is tagged with.
    pub mod flag {
        /// A screenshot was taken while the frame was open.
        pub const SCREENSHOT: u32 = 1;
    }

    /// The index of the frame open now. Work that finishes in a later frame
    /// carries it, so its numbers land on the frame that started it. Stays 0
    /// while nothing records.
    #[inline]
    pub fn open_index() -> u64 {
        0
    }

    /// Tag the open frame with `flag`.
    #[inline]
    pub fn mark(_flag: u32) {}

    /// Tag every frame with `flag` until it is set off again.
    #[inline]
    pub fn set_state(_flag: u32, _on: bool) {}

    /// The replay tick the open frame shows.
    #[inline]
    pub fn set_replay_tick(_tick: u64) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nothing_records_and_the_frame_index_stays_put() {
        assert!(!recording());
        assert!(!stats::enabled());
        frames::mark(frames::flag::SCREENSHOT);
        frames::set_state(frames::flag::SCREENSHOT, true);
        frames::set_replay_tick(40);
        assert_eq!(frames::open_index(), 0);
    }

    #[test]
    fn hooks_take_their_values_and_return() {
        let open_span = Span::FrameClockMillis.enter();
        Span::TocLoad.begin();
        Span::TocLoad.end();
        Counter::TallyDrawCalls.emit(12.0);
        Counter::DrawGpuWholeMillis.emit_at(4.5, frames::open_index());
        drop(open_span);
        level_ready(1);
        swap(7, "begin", "mp_raid");
    }
}
