//! Turns frame time into authority-sized steps for the client.

use bevy::prelude::*;

use crate::authority::inbox::AUTHORITY_MS;

/// What one call to [`ClientClock::tick`] hands back.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ClockTick {
    /// Steps the caller should run now: one, or none if a full step has not built up yet.
    pub released: u32,

    /// Whole steps that went by in the same call and were skipped to catch up.
    pub dropped: u32,
}

/// Collects frame time and lets one authority step out each time a full step has built up.
#[derive(Resource, Clone, Copy, Debug, Default)]
pub struct ClientClock {
    pub accumulator_ms: f32,

    pub dropped_slots: u32,
}

/// A leftover this close to a whole step is float noise from the modulo, not real time.
const LEFTOVER_SNAP_MS: f32 = 0.0005;

impl ClientClock {
    pub fn tick(&mut self, delta_ms: f32) -> ClockTick {
        let step = AUTHORITY_MS as f32;
        let carried = self.accumulator_ms;
        self.accumulator_ms += delta_ms;
        if self.accumulator_ms < step {
            return ClockTick::default();
        }

        let whole_steps = (self.accumulator_ms / step).floor() as u32;
        let dropped = whole_steps.saturating_sub(1);
        self.dropped_slots = self.dropped_slots.wrapping_add(dropped);

        let leftover = self.accumulator_ms % step;
        self.accumulator_ms = if leftover >= step - LEFTOVER_SNAP_MS {
            0.0
        } else {
            leftover
        };
        debug_assert!(
            (0.0..step).contains(&self.accumulator_ms),
            "clock debt after tick: before={carried} dt={delta_ms} after={}",
            self.accumulator_ms
        );

        ClockTick {
            released: 1,
            dropped,
        }
    }

    /// Time that has built up toward the next step.
    pub fn debt_ms(&self) -> f32 {
        self.accumulator_ms
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const STEP: f32 = AUTHORITY_MS as f32;

    #[test]
    fn nothing_is_released_before_a_full_step() {
        let mut clock = ClientClock::default();
        assert_eq!(clock.tick(STEP - 1.0), ClockTick::default());
        assert_eq!(clock.debt_ms(), STEP - 1.0);
        assert_eq!(clock.dropped_slots, 0);
    }

    #[test]
    fn small_ticks_add_up_to_one_release() {
        let mut clock = ClientClock::default();
        let mut released = 0;
        for _ in 0..5 {
            released += clock.tick(STEP / 4.0).released;
        }
        assert_eq!(released, 1);
        assert_eq!(clock.debt_ms(), STEP / 4.0);
    }

    #[test]
    fn the_part_past_a_step_stays_in_the_debt() {
        let mut clock = ClientClock::default();
        let tick = clock.tick(STEP + 7.0);
        assert_eq!(tick, ClockTick { released: 1, dropped: 0 });
        assert_eq!(clock.debt_ms(), 7.0);
    }

    #[test]
    fn a_long_frame_releases_one_step_and_counts_the_rest_as_dropped() {
        let mut clock = ClientClock::default();
        let tick = clock.tick(STEP * 4.0 + 3.0);
        assert_eq!(tick, ClockTick { released: 1, dropped: 3 });
        assert_eq!(clock.dropped_slots, 3);
        assert_eq!(clock.debt_ms(), 3.0);

        clock.tick(STEP * 2.0);
        assert_eq!(clock.dropped_slots, 4);
    }

    #[test]
    fn a_leftover_within_float_noise_of_a_step_snaps_to_zero() {
        let mut clock = ClientClock::default();
        clock.accumulator_ms = STEP - 0.0001;
        let tick = clock.tick(STEP);
        assert_eq!(tick.released, 1);
        assert_eq!(clock.debt_ms(), 0.0);
    }

    #[test]
    fn the_dropped_counter_wraps_instead_of_overflowing() {
        let mut clock = ClientClock {
            accumulator_ms: 0.0,
            dropped_slots: u32::MAX,
        };
        clock.tick(STEP * 3.0);
        assert_eq!(clock.dropped_slots, 1);
    }
}
