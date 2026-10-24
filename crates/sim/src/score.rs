//! The clock the match steps by, and the free-for-all defaults a match opens with.

use crate::frame::FrameWorld;
use crate::identities::MatchPhase;
use gamemode_t6::dm::{SCORE_LIMIT, TIME_LIMIT_MS};

/// Milliseconds of match time one tick stands for.
pub const MATCH_TICK_MS: u32 = 50;

/// Ends the pre-match wait: play begins.
pub(crate) fn finish_prematch(world: &mut FrameWorld) {
    world.set_phase(MatchPhase::Playing);
}

/// The score limit and the time limit (in ms) a freshly set-up match starts with.
pub fn bootstrap_score_defaults() -> (i32, u32) {
    (SCORE_LIMIT, TIME_LIMIT_MS)
}
