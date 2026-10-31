//! Starting a miss and reading how much of it is left.

use playerstate_t6::game::{MOVE_KIND_DEAD_TETHERED, MOVE_MODE_WALK_ATTACHED};

use super::ForecastDrift;
use super::vector::{add, remaining, scaled_by, sub};

impl ForecastDrift {
    /// Record that the view was at `view_before` and the server now says `submit_host_truth`.
    /// A tiny miss is ignored; a bigger one is added to what is still fading.
    pub fn start_blend(&mut self, view_before: [f32; 3], submit_host_truth: [f32; 3]) {
        let delta = sub(view_before, submit_host_truth);
        let length = game_math::vec3_norm(delta);
        if length <= Self::SNAP_THRESHOLD_FLOOR {
            return;
        }
        self.latest_divergence_distance = length;
        let still_fading = if Self::DRIFT_FADE_MILLIS == 0.0 {
            [0.0; 3]
        } else {
            let left = remaining(Self::DRIFT_FADE_MILLIS, self.clock_stamp_ms, self.drift_age_millis);
            scaled_by(self.error, left)
        };
        self.error = add(delta, still_fading);
        self.drift_age_millis = self.earlier_clock_millis;
    }

    /// Note how far the view jumped without smoothing it (a teleport).
    pub fn record_jump(&mut self, view_before: [f32; 3], submit_host_truth: [f32; 3]) {
        self.latest_divergence_distance = game_math::vec3_norm(sub(view_before, submit_host_truth));
    }

    /// The view offset this frame, for a player that is not linked to a mover.
    pub fn offset(&self) -> [f32; 3] {
        self.correction_for_move_mode(0)
    }

    /// The view offset this frame. Linked movement types ride on something else and get none.
    pub fn correction_for_move_mode(&self, movement_mode: i32) -> [f32; 3] {
        if movement_mode == MOVE_MODE_WALK_ATTACHED || movement_mode == MOVE_KIND_DEAD_TETHERED {
            return [0.0; 3];
        }
        if Self::DRIFT_FADE_MILLIS <= 0.0 {
            return [0.0; 3];
        }
        let left = remaining(Self::DRIFT_FADE_MILLIS, self.clock_stamp_ms, self.drift_age_millis);
        // Exactly 1 (the miss began this very frame) or 0 (it has faded) shows nothing.
        if !(0.0 < left && left < 1.0) {
            return [0.0; 3];
        }
        scaled_by(self.error, left)
    }
}

#[cfg(test)]
mod tests {
    use super::super::tests::{clock_at_1000, drive_frames};
    use super::*;

    fn close(a: [f32; 3], b: [f32; 3]) -> bool {
        a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-5)
    }

    /// A miss of `x` along the first axis that began at 1000 ms, then 31 ms passed.
    fn miss_31_ms_old(x: f32) -> ForecastDrift {
        let mut clock = clock_at_1000();
        let mut error = ForecastDrift::default();
        error.follow_clock(&clock);
        error.start_blend([x, 0.0, 0.0], [0.0; 3]);
        drive_frames(&mut clock, 0.03125);
        error.follow_clock(&clock);
        error
    }

    #[test]
    fn a_miss_of_a_tenth_or_less_is_ignored() {
        let mut error = ForecastDrift::default();
        error.start_blend([0.1, 0.0, 0.0], [0.0; 3]);
        assert!(!error.is_blending());
        assert_eq!(error.latest_divergence_distance(), 0.0);
    }

    #[test]
    fn a_bigger_miss_is_kept_and_remembered() {
        let mut error = ForecastDrift::default();
        error.start_blend([0.0, 3.0, 4.0], [0.0, 0.0, 0.0]);
        assert_eq!(error.error(), [0.0, 3.0, 4.0]);
        assert_eq!(error.latest_divergence_distance(), 5.0);
    }

    #[test]
    fn the_miss_is_view_minus_server() {
        let mut error = ForecastDrift::default();
        error.start_blend([1.0, 1.0, 1.0], [3.0, 1.0, 0.0]);
        assert_eq!(error.error(), [-2.0, 0.0, 1.0]);
    }

    #[test]
    fn a_miss_that_began_this_frame_shows_no_offset_yet() {
        let mut clock = clock_at_1000();
        let mut error = ForecastDrift::default();
        error.follow_clock(&clock);
        error.start_blend([1.0, 0.0, 0.0], [0.0; 3]);
        assert_eq!(error.drift_age_millis(), 1000);
        assert_eq!(error.offset(), [0.0; 3]);
        drive_frames(&mut clock, 0.0);
        error.follow_clock(&clock);
        assert_eq!(error.offset(), [0.0; 3]);
    }

    #[test]
    fn the_offset_fades_with_the_time_since_the_miss() {
        let error = miss_31_ms_old(1.0);
        assert!(close(error.offset(), [0.69, 0.0, 0.0]), "{:?}", error.offset());
    }

    #[test]
    fn after_the_decay_time_the_offset_is_gone() {
        let mut clock = clock_at_1000();
        let mut error = ForecastDrift::default();
        error.follow_clock(&clock);
        error.start_blend([1.0, 0.0, 0.0], [0.0; 3]);
        drive_frames(&mut clock, 0.125);
        error.follow_clock(&clock);
        assert_eq!(error.offset(), [0.0; 3]);
    }

    #[test]
    fn linked_movement_types_get_no_offset() {
        let error = miss_31_ms_old(1.0);
        assert_eq!(error.correction_for_move_mode(MOVE_MODE_WALK_ATTACHED), [0.0; 3]);
        assert_eq!(error.correction_for_move_mode(MOVE_KIND_DEAD_TETHERED), [0.0; 3]);
        assert!(error.correction_for_move_mode(0)[0] > 0.0);
    }

    #[test]
    fn a_second_miss_adds_to_what_was_still_fading() {
        let mut clock = clock_at_1000();
        let mut error = ForecastDrift::default();
        error.follow_clock(&clock);
        error.start_blend([1.0, 0.0, 0.0], [0.0; 3]);
        drive_frames(&mut clock, 0.03125); // time 1031, old 1000
        error.follow_clock(&clock);
        error.start_blend([1.0, 0.0, 0.0], [0.0; 3]);
        // 0.69 of the first miss is left; the second adds a full 1.0.
        assert!(close(error.error(), [1.69, 0.0, 0.0]), "{:?}", error.error());
        assert_eq!(error.drift_age_millis(), 1000, "the second began at the frame's old time");
    }

    #[test]
    fn a_teleport_only_records_how_far_the_view_moved() {
        let mut error = ForecastDrift::default();
        error.record_jump([0.0, 0.0, 12.0], [0.0, 0.0, 0.0]);
        assert_eq!(error.latest_divergence_distance(), 12.0);
        assert!(!error.is_blending());
        assert_eq!(error.snap_back_total(), 0);
    }
}
