//! The match clock and the opening limits of a free-for-all game.

use crate::step_view::StepWorldView;
use crate::match_ids::GameRoundStage;
use crate::ruleset::FFA;

/// Milliseconds of match time in one tick.
pub const SIM_STEP_MILLIS: u32 = 50;

/// Pre-match wait is over; play starts.
pub(crate) fn finish_prematch(world: &mut StepWorldView) {
    world.set_phase(GameRoundStage::Playing)
}

/// `(score limit, time limit in ms)` for a match that has just been set up.
pub fn opening_limits() -> (i32, u32) {
    (FFA.win_score, FFA.round_ms)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_tick_is_a_twentieth_of_a_second() {
        assert_eq!(1000 / SIM_STEP_MILLIS, 20);
    }

    #[test]
    fn a_new_match_opens_with_the_gametype_limits() {
        let (score, time) = opening_limits();
        assert_eq!((score, time), (FFA.win_score, FFA.round_ms));
    }
}
