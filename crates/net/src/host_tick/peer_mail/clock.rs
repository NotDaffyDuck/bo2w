//! The authority's fixed tick rate and the clock that counts it.

use bevy::prelude::Resource;
use playerstate_t6::game::InputCommand;
use sim::{ClientId, Tick};

pub const HOST_TICK_FREQUENCY: f64 = 20.0;

pub const HOST_STEP_MILLIS: i32 = 50;

/// A point on the authority's timeline, in milliseconds.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct HostInstant(i32);

impl HostInstant {
    pub const fn from_millis_value(ms: i32) -> Self {
        Self(ms)
    }

    pub fn from_step_number(tick: Tick) -> Self {
        let ticks = i32::try_from(tick.0).unwrap_or(0);
        Self(ticks.saturating_mul(HOST_STEP_MILLIS))
    }

    pub const fn as_millis(self) -> i32 {
        self.0
    }
}

/// The tick number and the time it stands for. Both wrap rather than saturate.
#[derive(Resource, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct HostTimeKeeper {
    pub tick: u32,
    pub clock_stamp_ms: i32,
}

impl HostTimeKeeper {
    /// Move to the next tick and return its number.
    pub fn step_forward(&mut self) -> u32 {
        let upcoming = self.tick.wrapping_add(1);
        self.tick = upcoming;
        self.clock_stamp_ms = self.clock_stamp_ms.wrapping_add(HOST_STEP_MILLIS);
        self.tick
    }
}

/// Run `ticks` authority frames against `arena`, feeding each one the commands `feed` returns
/// for that tick and time, and collect the snapshots.
pub fn execute_steady_host_loop(
    arena: &mut sim::SimWorld,
    ticks: u32,
    mut feed: impl FnMut(u32, i32) -> Vec<(ClientId, InputCommand)>,
) -> Vec<sim::Snapshot> {
    let mut clock = HostTimeKeeper::default();
    (0..ticks)
        .map(|_| {
            let step_no = clock.step_forward();
            let input = sim::StepInputs::from_cmds(feed(step_no, clock.clock_stamp_ms));
            sim::step(
                arena,
                sim::Tick(step_no),
                &input,
                HOST_STEP_MILLIS,
                sim::AdvanceCause::ServerStepFrame,
            )
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_clock_counts_ticks_and_milliseconds_together() {
        let mut clock = HostTimeKeeper::default();
        assert_eq!(clock.step_forward(), 1);
        assert_eq!(clock.step_forward(), 2);
        assert_eq!((clock.tick, clock.clock_stamp_ms), (2, 2 * HOST_STEP_MILLIS));
    }

    #[test]
    fn the_clock_wraps_instead_of_overflowing() {
        let mut clock = HostTimeKeeper {
            tick: u32::MAX,
            clock_stamp_ms: i32::MAX,
        };
        assert_eq!(clock.step_forward(), 0);
        assert_eq!(clock.clock_stamp_ms, i32::MAX.wrapping_add(HOST_STEP_MILLIS));
    }

    #[test]
    fn a_tick_converts_to_server_time_by_the_frame_length() {
        assert_eq!(HostInstant::from_step_number(Tick(0)).as_millis(), 0);
        assert_eq!(HostInstant::from_step_number(Tick(3)).as_millis(), 3 * HOST_STEP_MILLIS);
        assert_eq!(HostInstant::from_millis_value(120).as_millis(), 120);
    }

    #[test]
    fn a_tick_too_large_for_milliseconds_reads_as_zero() {
        assert_eq!(HostInstant::from_step_number(Tick(u32::MAX)).as_millis(), 0);
    }

    #[test]
    fn authority_rate_and_frame_length_agree() {
        assert_eq!(f64::from(HOST_STEP_MILLIS) * HOST_TICK_FREQUENCY, 1000.0);
    }
}
