use crate::frame::FrameWorld;
use crate::identities::MatchPhase;
use gamemode_t6::dm::{SCORE_LIMIT, TIME_LIMIT_MS};

pub const MATCH_TICK_MS: u32 = 50;

pub(crate) fn finish_prematch(world: &mut FrameWorld) {
    world.set_phase(MatchPhase::Playing);
    if world.bootstrap_ref().kind == gamemode_t6::GameModeKind::Demolition {
        world.set_use_start_spawns(false);
    }
}

pub fn bootstrap_score_defaults() -> (i32, u32) {
    (SCORE_LIMIT, TIME_LIMIT_MS)
}
