//! Ladders: what grabs onto one and how the player is held against it.

use playerstate_t6::game::PlayerState;

/// How long after letting go of a ladder a jump is refused, in milliseconds.
pub const LADDER_JUMP_BLOCK_MS: i32 = 300;

/// How far from the player to look for a ladder while in the air.
pub const LADDER_TRACE_DIST_AIR: f32 = 30.0;

/// How far from the player to look for a ladder while walking.
pub const LADDER_TRACE_DIST_WALK: f32 = 8.0;

/// The surface flag a ladder brush carries.
pub const SURF_LADDER: u32 = 0x8;

/// How hard the player is pulled in toward the rungs while climbing.
const ATTRACT_SPEED: f32 = 50.0;

/// What the world answers when asked whether a ladder is within reach.
pub trait LadderAttachBackend {
    /// Trace from `origin` along `dir` for `dist` units; `None` when nothing
    /// is hit.
    fn ladder_trace(
        &mut self,
        origin: [f32; 3],
        dir: [f32; 3],
        dist: f32,
    ) -> Option<LadderTraceHit>;
}

/// A surface the ladder probe touched.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LadderTraceHit {
    pub fraction: f32,
    pub normal: [f32; 3],
    pub surface_flags: u32,
}

/// What the ladder check needs to know about this frame's input.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CheckLadderContext {
    pub server_time: i32,
    pub walking: bool,
    /// The view's forward direction flattened onto the floor.
    pub forward_xy: [f32; 2],
    pub forwardmove: i8,
}

/// Hold the climber against the ladder. Sideways drift along the rungs is
/// cancelled; when the climb is mostly vertical the player is also pulled in
/// toward the ladder face (`v_ladder_vec` points out of it).
pub fn ladder_attract_velocity(ps: &mut PlayerState) {
    let out = ps.v_ladder_vec;
    let drift = ps.velocity[0] * out[0] + ps.velocity[1] * out[1];
    ps.velocity[0] += -drift * out[0];
    ps.velocity[1] += -drift * out[1];

    if ps.velocity == [0.0; 3] {
        return;
    }
    let across = ps.velocity[0] * ps.velocity[0] + ps.velocity[1] * ps.velocity[1];
    let up_down = ps.velocity[2] * ps.velocity[2];
    if up_down >= across {
        ps.velocity[0] += -ATTRACT_SPEED * out[0];
        ps.velocity[1] += -ATTRACT_SPEED * out[1];
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn on_ladder_facing_north(velocity: [f32; 3]) -> PlayerState {
        let mut ps = PlayerState::ZERO;
        ps.v_ladder_vec = [0.0, 1.0, 0.0];
        ps.velocity = velocity;
        ps
    }

    #[test]
    fn a_vertical_climb_is_pulled_in_toward_the_rungs() {
        let mut ps = on_ladder_facing_north([0.0, 0.0, 100.0]);
        ladder_attract_velocity(&mut ps);
        assert_eq!(ps.velocity, [0.0, -ATTRACT_SPEED, 100.0]);
    }

    #[test]
    fn drift_away_from_the_ladder_is_cancelled() {
        let mut ps = on_ladder_facing_north([30.0, 20.0, 10.0]);
        ladder_attract_velocity(&mut ps);
        assert_eq!(ps.velocity, [30.0, 0.0, 10.0]);
    }

    #[test]
    fn a_mostly_sideways_slide_is_not_pulled_in() {
        let mut ps = on_ladder_facing_north([80.0, 0.0, 10.0]);
        ladder_attract_velocity(&mut ps);
        assert_eq!(ps.velocity, [80.0, 0.0, 10.0]);
    }

    #[test]
    fn standing_still_stays_still() {
        let mut ps = on_ladder_facing_north([0.0; 3]);
        ladder_attract_velocity(&mut ps);
        assert_eq!(ps.velocity, [0.0; 3]);
    }
}
