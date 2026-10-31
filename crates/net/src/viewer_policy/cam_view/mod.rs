//! What a client sees while their killcam runs: the world as the archive held it, with the
//! viewer's own row and HUD kept live.

mod follow;
mod hud;
mod reading;
mod runs;
mod lens;
mod world;

pub use reading::{ViewerSeatReading, stamp_seat_into_world_state, read_deathreplay_seat};
pub use runs::{LiveDeathReplays, DeathReplayRun};
pub use lens::{state_and_reading_for_viewer, state_for_viewer};

use crate::host_tick::peer_mail::HOST_STEP_MILLIS;

/// A killcam length in seconds as milliseconds, rounded to the nearest whole authority tick.
pub const fn deathreplay_secs_to_millis(seconds: f32) -> i32 {
    let ticks = (seconds * 1000.0 / (HOST_STEP_MILLIS as f32) + 0.5) as i32;
    ticks.saturating_mul(HOST_STEP_MILLIS)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn whole_ticks_come_out_unchanged() {
        assert_eq!(deathreplay_secs_to_millis(5.0), 5000);
        assert_eq!(deathreplay_secs_to_millis(0.0), 0);
        assert_eq!(deathreplay_secs_to_millis(0.05), HOST_STEP_MILLIS);
    }

    #[test]
    fn a_length_between_ticks_rounds_to_the_nearer_one() {
        assert_eq!(deathreplay_secs_to_millis(0.07), HOST_STEP_MILLIS);
        assert_eq!(deathreplay_secs_to_millis(0.08), 2 * HOST_STEP_MILLIS);
    }
}
