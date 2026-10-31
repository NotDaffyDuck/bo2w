//! The view offset left behind when a snapshot corrects a prediction, faded out over a
//! short time so the camera does not jump.

mod correction;
mod vector;

use crate::client::frame_clock::FrameClock;

/// A prediction miss that is still fading out of the view.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ForecastDrift {
    error: [f32; 3],
    drift_age_millis: i32,
    clock_stamp_ms: i32,
    earlier_clock_millis: i32,
    latest_divergence_distance: f32,
    snap_back_total: u32,
    spawned_entity_total: u32,
}

impl ForecastDrift {
    /// How long a miss takes to fade to nothing.
    pub const DRIFT_FADE_MILLIS: f32 = 100.0;
    /// Misses this short or shorter are not worth smoothing.
    pub const SNAP_THRESHOLD_FLOOR: f32 = 0.1;

    pub fn is_blending(&self) -> bool {
        vector::length_sq(self.error) > 1e-9
    }

    pub fn error(&self) -> [f32; 3] {
        self.error
    }

    pub fn drift_age_millis(&self) -> i32 {
        self.drift_age_millis
    }

    pub fn latest_divergence_distance(&self) -> f32 {
        self.latest_divergence_distance
    }

    pub fn snap_back_total(&self) -> u32 {
        self.snap_back_total
    }

    pub fn spawned_entity_total(&self) -> u32 {
        self.spawned_entity_total
    }

    /// Take this frame's times from the game clock.
    pub fn follow_clock(&mut self, clock: &FrameClock) {
        self.clock_stamp_ms = clock.time();
        self.earlier_clock_millis = clock.prior_ms();
    }

    /// Forget the pending miss, as when the player is placed somewhere new.
    pub fn start_over(&mut self) {
        self.error = [0.0; 3];
        self.drift_age_millis = 0;
        self.snap_back_total = self.snap_back_total.saturating_add(1);
    }

    /// [`Self::reset`] for a player entity that has just appeared.
    pub fn initialize_fresh_object(&mut self) {
        self.start_over();
        self.spawned_entity_total = self.spawned_entity_total.saturating_add(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::HostInstant;

    /// A clock whose current time is 1000 ms.
    pub(super) fn clock_at_1000() -> FrameClock {
        let mut clock = FrameClock::default();
        clock.step_local_host_clock(0.05, true, Some(HostInstant::from_millis_value(1000)));
        clock
    }

    /// Move `clock` on by `secs` without a new server time.
    pub(super) fn drive_frames(clock: &mut FrameClock, secs: f32) {
        clock.step_local_host_clock(secs, true, Some(HostInstant::from_millis_value(1000)));
    }

    #[test]
    fn a_new_one_is_quiet() {
        let error = ForecastDrift::default();
        assert!(!error.is_blending());
        assert_eq!(error.error(), [0.0; 3]);
        assert_eq!((error.snap_back_total(), error.spawned_entity_total()), (0, 0));
        assert_eq!(error.latest_divergence_distance(), 0.0);
    }

    #[test]
    fn sync_frame_copies_both_times_from_the_clock() {
        let mut clock = clock_at_1000();
        drive_frames(&mut clock, 0.03125); // 31.25 ms: the clock moves 31
        let mut error = ForecastDrift::default();
        error.follow_clock(&clock);
        assert_eq!((error.clock_stamp_ms, error.earlier_clock_millis), (1031, 1000));
    }

    #[test]
    fn reset_clears_the_miss_and_counts() {
        let mut error = ForecastDrift::default();
        error.start_blend([2.0, 0.0, 0.0], [0.0; 3]);
        assert!(error.is_blending());
        error.start_over();
        assert!(!error.is_blending());
        assert_eq!((error.drift_age_millis(), error.snap_back_total()), (0, 1));
        assert_eq!(error.spawned_entity_total(), 0);
    }

    #[test]
    fn reset_new_entity_is_a_reset_that_also_counts_the_entity() {
        let mut error = ForecastDrift::default();
        error.initialize_fresh_object();
        error.initialize_fresh_object();
        assert_eq!((error.snap_back_total(), error.spawned_entity_total()), (2, 2));
    }

    #[test]
    fn the_counters_stop_at_the_top_instead_of_wrapping() {
        let mut error = ForecastDrift { snap_back_total: u32::MAX, spawned_entity_total: u32::MAX, ..Default::default() };
        error.initialize_fresh_object();
        assert_eq!((error.snap_back_total(), error.spawned_entity_total()), (u32::MAX, u32::MAX));
    }
}
