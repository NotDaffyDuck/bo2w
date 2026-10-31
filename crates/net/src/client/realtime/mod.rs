//! The client's wall clock in whole milliseconds, built from the frame deltas the app reports.

use bevy::prelude::*;

/// Whole milliseconds since the first frame, and how long the last frame took, with the
/// sub-millisecond remainder carried to the next frame so no time is lost.
#[derive(Resource, Clone, Copy, Debug, Default, PartialEq)]
pub struct ClientRealtime {
    elapsed_ms: i32,
    step_millis_elapsed: i32,
    has_started: bool,
    subtick_millis: f32,
}

impl ClientRealtime {
    /// Milliseconds since the first frame; wraps like the 32-bit clock the wire speaks.
    pub fn now_ms(&self) -> i32 {
        self.elapsed_ms
    }

    /// Whole milliseconds the last frame took.
    pub fn step_millis_elapsed(&self) -> i32 {
        self.step_millis_elapsed
    }

    pub fn step_duration_seconds(&self) -> f32 {
        self.step_millis_elapsed as f32 / 1000.0
    }

    /// The part of a millisecond not yet counted.
    pub fn subtick_millis(&self) -> f32 {
        self.subtick_millis
    }

    pub fn has_begun(&self) -> bool {
        self.has_started
    }

    /// Move the clock on by `step_secs`. The first call only starts it, so the clock reads zero
    /// until the second frame.
    pub fn advance_host_clock(&mut self, step_secs: f32) {
        if !self.has_started {
            *self = Self { has_started: true, ..Self::default() };
            return;
        }
        self.subtick_millis += step_secs.max(0.0) * 1000.0;
        let whole_ms = self.subtick_millis as i32;
        self.subtick_millis -= whole_ms as f32;
        self.step_millis_elapsed = whole_ms;
        self.elapsed_ms = self.elapsed_ms.wrapping_add(whole_ms);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_first_frame_only_starts_the_clock() {
        let mut clock = ClientRealtime::default();
        clock.advance_host_clock(0.5);
        assert!(clock.has_begun());
        assert_eq!((clock.now_ms(), clock.step_millis_elapsed()), (0, 0));
        assert_eq!(clock.subtick_millis(), 0.0);
    }

    #[test]
    fn whole_milliseconds_count_and_the_remainder_carries() {
        let mut clock = ClientRealtime::default();
        clock.advance_host_clock(0.0);
        clock.advance_host_clock(0.0105);
        assert_eq!(clock.step_millis_elapsed(), 10);
        assert_eq!(clock.now_ms(), 10);
        assert!((clock.subtick_millis() - 0.5).abs() < 0.01, "{}", clock.subtick_millis());
        clock.advance_host_clock(0.0006);
        assert_eq!(clock.step_millis_elapsed(), 1);
        assert_eq!(clock.now_ms(), 11);
    }

    #[test]
    fn a_negative_delta_counts_as_no_time() {
        let mut clock = ClientRealtime::default();
        clock.advance_host_clock(0.0);
        clock.advance_host_clock(-3.0);
        assert_eq!((clock.now_ms(), clock.step_millis_elapsed()), (0, 0));
    }

    #[test]
    fn the_clock_wraps_instead_of_overflowing() {
        let mut clock = ClientRealtime { elapsed_ms: i32::MAX, has_started: true, ..Default::default() };
        clock.advance_host_clock(0.002);
        assert_eq!(clock.now_ms(), i32::MIN + 1);
    }

    #[test]
    fn frametime_in_seconds_follows_the_whole_milliseconds() {
        let clock = ClientRealtime { step_millis_elapsed: 16, has_started: true, ..Default::default() };
        assert!((clock.step_duration_seconds() - 0.016).abs() < 1e-6);
    }
}
