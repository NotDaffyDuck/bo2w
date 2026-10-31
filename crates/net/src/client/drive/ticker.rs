//! Turns frame time into authority-sized steps for the client.

use bevy::prelude::*;

use crate::host_tick::peer_mail::HOST_STEP_MILLIS;

/// What one call to [`PlayerTimeStepper::tick`] hands back.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TimerStepResult {
    /// Steps the caller should run now: one, or none if a full step has not built up yet.
    pub fired: u32,

    /// Whole steps that went by in the same call and were skipped to catch up.
    pub lost_steps: u32,
}

/// Collects frame time and lets one authority step out each time a full step has built up.
#[derive(Resource, Clone, Copy, Debug, Default)]
pub struct PlayerTimeStepper {
    pub carryover_millis: f32,

    pub discarded_entries: u32,
}

/// A leftover this close to a whole step is float noise from the modulo, not real time.
const LEFTOVER_SNAP_MS: f32 = 0.0005;

impl PlayerTimeStepper {
    pub fn advance_by(&mut self, delta_ms: f32) -> TimerStepResult {
        let step = HOST_STEP_MILLIS as f32;
        let carried = self.carryover_millis;
        self.carryover_millis += delta_ms;
        if self.carryover_millis < step {
            return TimerStepResult::default();
        }

        let whole_steps = (self.carryover_millis / step).floor() as u32;
        let lost_steps = whole_steps.saturating_sub(1);
        self.discarded_entries = self.discarded_entries.wrapping_add(lost_steps);

        let leftover = self.carryover_millis % step;
        self.carryover_millis = if leftover >= step - LEFTOVER_SNAP_MS {
            0.0
        } else {
            leftover
        };
        debug_assert!(
            (0.0..step).contains(&self.carryover_millis),
            "clock debt after tick: before={carried} dt={delta_ms} after={}",
            self.carryover_millis
        );

        TimerStepResult {
            fired: 1,
            lost_steps,
        }
    }

    /// Time that has built up toward the next step.
    pub fn owed_millis(&self) -> f32 {
        self.carryover_millis
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const STEP: f32 = HOST_STEP_MILLIS as f32;

    #[test]
    fn nothing_is_released_before_a_full_step() {
        let mut clock = PlayerTimeStepper::default();
        assert_eq!(clock.advance_by(STEP - 1.0), TimerStepResult::default());
        assert_eq!(clock.owed_millis(), STEP - 1.0);
        assert_eq!(clock.discarded_entries, 0);
    }

    #[test]
    fn small_ticks_add_up_to_one_release() {
        let mut clock = PlayerTimeStepper::default();
        let mut fired = 0;
        for _ in 0..5 {
            fired += clock.advance_by(STEP / 4.0).fired;
        }
        assert_eq!(fired, 1);
        assert_eq!(clock.owed_millis(), STEP / 4.0);
    }

    #[test]
    fn the_part_past_a_step_stays_in_the_debt() {
        let mut clock = PlayerTimeStepper::default();
        let tick = clock.advance_by(STEP + 7.0);
        assert_eq!(tick, TimerStepResult { fired: 1, lost_steps: 0 });
        assert_eq!(clock.owed_millis(), 7.0);
    }

    #[test]
    fn a_long_frame_releases_one_step_and_counts_the_rest_as_dropped() {
        let mut clock = PlayerTimeStepper::default();
        let tick = clock.advance_by(STEP * 4.0 + 3.0);
        assert_eq!(tick, TimerStepResult { fired: 1, lost_steps: 3 });
        assert_eq!(clock.discarded_entries, 3);
        assert_eq!(clock.owed_millis(), 3.0);

        clock.advance_by(STEP * 2.0);
        assert_eq!(clock.discarded_entries, 4);
    }

    #[test]
    fn a_leftover_within_float_noise_of_a_step_snaps_to_zero() {
        let mut clock = PlayerTimeStepper::default();
        clock.carryover_millis = STEP - 0.0001;
        let tick = clock.advance_by(STEP);
        assert_eq!(tick.fired, 1);
        assert_eq!(clock.owed_millis(), 0.0);
    }

    #[test]
    fn the_dropped_counter_wraps_instead_of_overflowing() {
        let mut clock = PlayerTimeStepper {
            carryover_millis: 0.0,
            discarded_entries: u32::MAX,
        };
        clock.advance_by(STEP * 3.0);
        assert_eq!(clock.discarded_entries, 1);
    }
}
