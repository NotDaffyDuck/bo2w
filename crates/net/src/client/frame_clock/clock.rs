//! The clock's state and what can be read from it.

use bevy::prelude::Resource;

/// Client game time in milliseconds, with the step since the previous reading.
#[derive(Resource, Clone, Copy, Debug, Default, PartialEq)]
pub struct FrameClock {
    pub(super) now_ms: i32,
    pub(super) prior_ms: i32,
    pub(super) step_millis_elapsed: i32,
    pub(super) ticking: bool,
    /// Milliseconds carried over between frames, below one whole millisecond.
    pub(super) subtick_millis: f32,
    pub(super) previous_host_state: Option<i32>,
    pub(super) wipe_count: u32,
    pub(super) quick_count: u32,
    pub(super) adjust_count: u32,
    pub(super) latest_correction_note: u8,
    pub(super) predicted_ahead: bool,
}

/// How the last advance corrected for the server clock.
pub(super) const ADJUST_RESET: u8 = 1;
pub(super) const ADJUST_FAST: u8 = 2;
pub(super) const ADJUST_NUDGE: u8 = 3;

impl FrameClock {
    /// A gap to the server clock past this snaps the clock to the server.
    /// bo2zm: BO2's CL_AdjustTimeDelta resets past timescale x 1000 ms; between 100 and 1000
    /// it halves the gap.
    pub const CLOCK_NUDGE_SNAP_MILLIS: i32 = 1000;

    /// A gap past this (and under the reset gap) closes half the difference at once.
    pub const CLOCK_NUDGE_QUICK_MILLIS: i32 = 100;

    pub fn time(&self) -> i32 {
        self.now_ms
    }

    pub fn prior_ms(&self) -> i32 {
        self.prior_ms
    }

    pub fn step_millis_elapsed(&self) -> i32 {
        self.step_millis_elapsed
    }

    pub fn step_duration_seconds(&self) -> f32 {
        self.step_millis_elapsed as f32 / 1000.0
    }

    pub fn started(&self) -> bool {
        self.ticking
    }

    pub fn restart_blank(&mut self) {
        *self = Self::default();
    }

    /// How the latest advance corrected the clock: "reset", "fast" or "nudge"; none if it did not.
    pub fn latest_correction_note(&self) -> Option<&'static str> {
        match self.latest_correction_note {
            ADJUST_RESET => Some("reset"),
            ADJUST_FAST => Some("fast"),
            ADJUST_NUDGE => Some("nudge"),
            _ => None,
        }
    }

    /// Whether the clock is running at or past the latest server time (within 8 ms) rather than
    /// behind it.
    pub fn predicted_ahead(&self) -> bool {
        self.predicted_ahead
    }

    pub fn subtick_millis(&self) -> f32 {
        self.subtick_millis
    }

    /// Set the time directly. A time before the previous one gives a zero step.
    pub fn set_host_ms(&mut self, authority_clock_ms: i32) {
        self.prior_ms = self.now_ms;
        self.now_ms = authority_clock_ms;
        let step = self.now_ms.wrapping_sub(self.prior_ms);
        if step < 0 {
            self.step_millis_elapsed = 0;
            self.prior_ms = self.now_ms;
        } else {
            self.step_millis_elapsed = step;
        }
        self.ticking = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_new_clock_has_not_started() {
        let clock = FrameClock::default();
        assert!(!clock.started());
        assert_eq!((clock.time(), clock.prior_ms(), clock.step_millis_elapsed()), (0, 0, 0));
        assert_eq!(clock.latest_correction_note(), None);
    }

    #[test]
    fn assigning_a_later_time_gives_the_step() {
        let mut fake_clock = FrameClock::default();
        fake_clock.set_host_ms(1000);
        fake_clock.set_host_ms(1050);
        assert_eq!((fake_clock.prior_ms(), fake_clock.time(), fake_clock.step_millis_elapsed()), (1000, 1050, 50));
        assert!(fake_clock.started());
        assert_eq!(fake_clock.step_duration_seconds(), 0.05);
    }

    #[test]
    fn assigning_an_earlier_time_gives_no_step_and_forgets_the_old_time() {
        let mut fake_clock = FrameClock::default();
        fake_clock.set_host_ms(1000);
        fake_clock.set_host_ms(900);
        assert_eq!((fake_clock.prior_ms(), fake_clock.time(), fake_clock.step_millis_elapsed()), (900, 900, 0));
    }

    #[test]
    fn resetting_goes_back_to_not_started() {
        let mut clock = FrameClock::default();
        clock.set_host_ms(1000);
        clock.restart_blank();
        assert_eq!(clock, FrameClock::default());
    }

    #[test]
    fn adjustments_have_their_log_names() {
        let named = |code| FrameClock { latest_correction_note: code, ..Default::default() }.latest_correction_note();
        assert_eq!(named(ADJUST_RESET), Some("reset"));
        assert_eq!(named(ADJUST_FAST), Some("fast"));
        assert_eq!(named(ADJUST_NUDGE), Some("nudge"));
        assert_eq!(named(0), None);
    }
}
