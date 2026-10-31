//! Ladders: what grabs onto one and how the player is held against it.

use playerstate_t6::game::AvatarState;

/// How long after letting go of a ladder a jump is refused, in milliseconds.
pub const RUNG_LEAP_LOCKOUT_MS: i32 = 300;

/// How far from the player to look for a ladder while in the air.
pub const RUNG_PROBE_REACH_AIRBORNE: f32 = 30.0;

/// How far from the player to look for a ladder while walking.
pub const RUNG_PROBE_REACH_FOOTED: f32 = 8.0;

/// The surface flag a ladder brush carries.
pub const FLOOR_IS_RUNGS: u32 = 0x8;

/// How hard the player is pulled in toward the rungs while climbing.
const ATTRACT_SPEED: f32 = 50.0;

/// What the world answers when asked whether a ladder is within reach.
pub trait RungGripSource {
    /// Trace from `origin` along `dir` for `dist` units; `None` when nothing
    /// is hit.
    fn rung_probe(
        &mut self,
        at_pos: [f32; 3],
        dir: [f32; 3],
        dist: f32,
    ) -> Option<RungProbeContact>;
}

/// A surface the ladder probe touched.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RungProbeContact {
    pub travelled: f32,
    pub facing: [f32; 3],
    pub surf_bits: u32,
}

/// What the ladder check needs to know about this frame's input.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RungGrabArgs {
    pub authority_clock_ms: i32,
    pub on_foot: bool,
    /// The view's forward direction flattened onto the floor.
    pub ahead_planar: [f32; 2],
    pub thrust_ahead: i8,
}

/// Hold the climber against the ladder. Sideways drift along the rungs is
/// cancelled; when the climb is mostly vertical the player is also pulled in
/// toward the ladder face (`ladder_normal_dir` points out of it).
pub fn ladder_attract_velocity(avatar: &mut AvatarState) {
    let out = avatar.ladder_normal_dir;
    let drift = avatar.velocity[0] * out[0] + avatar.velocity[1] * out[1];
    avatar.velocity[0] += -drift * out[0];
    avatar.velocity[1] += -drift * out[1];

    if avatar.velocity == [0.0; 3] {
        return;
    }
    let across = avatar.velocity[0] * avatar.velocity[0] + avatar.velocity[1] * avatar.velocity[1];
    let up_down = avatar.velocity[2] * avatar.velocity[2];
    if up_down >= across {
        avatar.velocity[0] += -ATTRACT_SPEED * out[0];
        avatar.velocity[1] += -ATTRACT_SPEED * out[1];
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn on_ladder_facing_north(velocity: [f32; 3]) -> AvatarState {
        let mut avatar = AvatarState::ZERO;
        avatar.ladder_normal_dir = [0.0, 1.0, 0.0];
        avatar.velocity = velocity;
        avatar
    }

    #[test]
    fn a_vertical_climb_is_pulled_in_toward_the_rungs() {
        let mut avatar = on_ladder_facing_north([0.0, 0.0, 100.0]);
        ladder_attract_velocity(&mut avatar);
        assert_eq!(avatar.velocity, [0.0, -ATTRACT_SPEED, 100.0]);
    }

    #[test]
    fn drift_away_from_the_ladder_is_cancelled() {
        let mut avatar = on_ladder_facing_north([30.0, 20.0, 10.0]);
        ladder_attract_velocity(&mut avatar);
        assert_eq!(avatar.velocity, [30.0, 0.0, 10.0]);
    }

    #[test]
    fn a_mostly_sideways_slide_is_not_pulled_in() {
        let mut avatar = on_ladder_facing_north([80.0, 0.0, 10.0]);
        ladder_attract_velocity(&mut avatar);
        assert_eq!(avatar.velocity, [80.0, 0.0, 10.0]);
    }

    #[test]
    fn standing_still_stays_still() {
        let mut avatar = on_ladder_facing_north([0.0; 3]);
        ladder_attract_velocity(&mut avatar);
        assert_eq!(avatar.velocity, [0.0; 3]);
    }
}
