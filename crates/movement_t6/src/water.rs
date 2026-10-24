//! bo2mp: Black Ops II's water: how deep the player stands in it
//! (PM_SetWaterLevel, PM_GetWaterLevel, CM_GetWaterHeight) and the swim
//! (PM_SwimMove). Not part of the upstream engine.
//!
//! The water's surface is found straight over or under the player's feet: a
//! point from 200 units above them to 200 below that only water stops. How
//! far the surface sits above the feet is the water level, out of a standing
//! eye height of 60: 1 from 3 units (footsteps splash), 2 from 24 (waist
//! deep: the ground grips 1.7 times as hard, no sprint, no prone), 3 from 38
//! (no crouch), 4 from 45 (the player swims), 5 from 54 and 6 from 60 (the
//! head is under). The level is worked out after the view each frame.
//!
//! Swimming, the keys push the player along the view, up and down too,
//! though at level 4 not up: 1.3 times the walking speed
//! (`player_waterSpeedScale`), at acceleration 2 (more for frames shorter
//! than 1/120 s, as BO2 off a console). No gravity. Deeper than 4 with no
//! key held the water lifts the player at half of `player_floatSpeed` (20);
//! a script's movement scale above 1 floats them faster (20 times it), below
//! 1 slower, and at a half they sink at 10. The water's own push
//! (CM_GetWaterForce) comes only from "undertow_volume" entities, which no
//! multiplayer map has, so there is none.

use crate::bridge::{AirMoveContext, Bo2Lunge, MoveBounds, Pml, accelerate, step_slide_move};
use playerstate_t6::game::{PlayerState, UserCmd};

use crate::gjk::Bo2Collision;
use crate::movement_dir::set_movement_dir;

/// The contents bit that marks water.
pub const CONTENTS_WATER: u32 = 0x20;

/// No water over or under the player.
pub const NO_WATER: f32 = -131_072.0;

/// How far above and below the feet the water's surface is looked for.
pub const WATER_SEARCH: f32 = 200.0;

/// Swimming goes this many times the walking speed
/// (`player_waterSpeedScale`).
const WATER_SPEED_SCALE: f32 = 1.3;

/// The swim's acceleration.
const SWIM_ACCEL: f32 = 2.0;

/// `player_floatSpeed`: the float's speed scale.
const FLOAT_SPEED: f32 = 20.0;

/// The level from which the player swims.
pub const SWIM_LEVEL: i32 = 4;

/// The water level for a surface at `height` and feet at `feet_z`: 0 to 6.
pub fn water_level(height: f32, feet_z: f32) -> i32 {
    // Fractions of the standing eye height, 60 (player_standingViewHeight).
    let depth = height - feet_z;
    if depth >= 60.0 {
        6
    } else if depth >= 54.0 {
        5
    } else if depth >= 45.0 {
        4
    } else if depth >= 38.0 {
        3
    } else if depth >= 24.0 {
        2
    } else if depth >= 3.0 {
        1
    } else {
        0
    }
}

/// PM_SetWaterLevel: how deep the player stands this frame.
pub(crate) fn set_water_level<C: Bo2Collision>(ps: &mut PlayerState, collision: &C) {
    ps.waterlevel = water_level(collision.water_height(ps.origin), ps.origin[2]);
}

/// The float's speed for a script's movement scale (negative: sinking).
fn float_speed(move_speed_scale: f32) -> f32 {
    let scale = move_speed_scale.clamp(0.5, 1.5);
    if scale <= 1.0 {
        (1.0 - scale) * -2.0 * FLOAT_SPEED + FLOAT_SPEED * 0.5
    } else {
        FLOAT_SPEED * scale
    }
}

/// The swim's acceleration for a frame this long.
fn swim_accel(frametime: f32) -> f32 {
    if frametime < 1.0 / 120.0 {
        SWIM_ACCEL * (3.0 - frametime * 240.0).clamp(1.0, 3.0)
    } else {
        SWIM_ACCEL
    }
}

/// PM_SwimMove. Returns a stair step's height for the camera.
#[allow(clippy::too_many_arguments)]
pub(crate) fn swim_move<C: Bo2Collision>(
    ps: &mut PlayerState,
    pml: &mut Pml,
    cmd: &UserCmd,
    context: AirMoveContext,
    bounds: MoveBounds,
    collision: &C,
    zombies: bool,
    lunge: Option<Bo2Lunge>,
) -> f32 {
    crate::friction::friction(ps, pml, true, lunge);

    let scale =
        crate::air::cmd_scale(ps, cmd, context.player_spectate_speed_scale) * WATER_SPEED_SCALE;
    let accel = swim_accel(pml.frametime);

    // Along the view, up and down too; at level 4 not up.
    let (f, r) = (cmd.forwardmove as f32, cmd.rightmove as f32);
    let mut wishvel = [0, 1, 2].map(|i| pml.forward[i] * f + pml.right[i] * r);
    if ps.waterlevel == SWIM_LEVEL && wishvel[2] >= 0.0 {
        wishvel[2] = 0.0;
    }
    let len = libm::sqrtf(wishvel[0] * wishvel[0] + wishvel[1] * wishvel[1] + wishvel[2] * wishvel[2]);
    let wishspeed = len * scale;
    let divisor = if len > 0.0 { len } else { 1.0 };
    let wishdir = wishvel.map(|v| v / divisor);

    // Deep, with no key held, the water lifts (or sinks) the player.
    if ps.waterlevel > SWIM_LEVEL && cmd.forwardmove == 0 && cmd.rightmove == 0 {
        let mut speed = float_speed(ps.move_speed_scale_multiplier);
        let mut dir = [0.0, 0.0, 1.0];
        if speed < 0.0 {
            dir[2] = -1.0;
            speed = -speed;
        }
        accelerate(ps, pml, &dir, speed, accel);
    }
    accelerate(ps, pml, &wishdir, wishspeed, accel);

    pml.walking = 0;
    pml.ground_plane = 0;
    let step = if collision.gjk_on() {
        crate::gjk::step::step_slide_move(
            ps,
            pml,
            collision,
            bounds.mins,
            bounds.maxs,
            bounds.tracemask,
            false,
            zombies,
        )
    } else {
        step_slide_move(
            ps,
            pml,
            collision,
            bounds.mins,
            bounds.maxs,
            bounds.tracemask,
            None,
            zombies,
        )
    };
    set_movement_dir(ps, pml, cmd);
    step
}

/// PM_Friction's water part: standing on the ground in water, it grips by
/// the level (waist deep 1.7 times, `player_waistWaterFrictionScale`); off
/// the ground the water does not slow the player. Added to the frame's other
/// slowing.
pub(crate) fn friction_drop(ps: &PlayerState, pml: &Pml, speed: f32) -> f32 {
    /// How hard each level grips (level 2 is the waist's own scale).
    const LEVEL_SCALE: [f32; 7] = [0.0, 1.0, 2.0, 2.0, 3.0, 3.0, 3.0];
    const WAIST_SCALE: f32 = 1.7;
    if ps.waterlevel == 0 || pml.walking == 0 {
        return 0.0;
    }
    let scale = if ps.waterlevel == 2 {
        WAIST_SCALE
    } else {
        LEVEL_SCALE[ps.waterlevel.clamp(0, 6) as usize]
    };
    crate::friction::GROUND_FRICTION * speed * scale * pml.frametime
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bridge::{CollisionBackend, GroundTraceInput};
    use trace_t6::Trace;

    #[test]
    fn the_levels_step_with_the_depth() {
        let feet = 100.0;
        for (depth, level) in [
            (70.0, 6),
            (60.0, 6),
            (59.9, 5),
            (54.0, 5),
            (53.9, 4),
            (45.0, 4),
            (44.9, 3),
            (38.0, 3),
            (37.9, 2),
            (24.0, 2),
            (23.9, 1),
            (3.0, 1),
            (2.9, 0),
            (-10.0, 0),
        ] {
            assert_eq!(water_level(feet + depth, feet), level, "depth {depth}");
        }
        assert_eq!(water_level(NO_WATER, feet), 0);
    }

    #[test]
    fn the_float_follows_the_script_speed() {
        assert_eq!(float_speed(1.0), 10.0);
        assert_eq!(float_speed(0.5), -10.0);
        // Unset (0) counts as a half: the player sinks.
        assert_eq!(float_speed(0.0), -10.0);
        assert_eq!(float_speed(0.75), 0.0);
        assert!((float_speed(1.2) - 24.0).abs() < 1e-4);
        assert_eq!(float_speed(3.0), 30.0);
    }

    #[test]
    fn short_frames_swim_harder() {
        assert_eq!(swim_accel(0.05), 2.0);
        assert_eq!(swim_accel(1.0 / 120.0), 2.0);
        assert!((swim_accel(1.0 / 240.0) - 4.0).abs() < 1e-4);
        assert_eq!(swim_accel(0.0), 6.0);
    }

    /// Nothing to hit; water up to `surface`.
    struct Pool {
        surface: f32,
    }

    impl Bo2Collision for Pool {
        fn water_height(&self, _origin: [f32; 3]) -> f32 {
            self.surface
        }
    }

    impl CollisionBackend for Pool {
        fn trace(&self, _input: GroundTraceInput) -> Trace {
            Trace {
                fraction: 1.0,
                ..Trace::default()
            }
        }
    }

    fn air() -> AirMoveContext {
        AirMoveContext {
            player_spectate_speed_scale: 1.0,
            shellshock_gravity_scale: 1.0,
            shellshock_gravity_bias: 0.0,
        }
    }

    fn bounds() -> MoveBounds {
        MoveBounds {
            mins: [-15.0, -15.0, 0.0],
            maxs: [15.0, 15.0, 70.0],
            tracemask: 0x0281_0011,
        }
    }

    fn swimmer(level: i32) -> PlayerState {
        let mut ps = PlayerState::ZERO;
        ps.speed = 190;
        ps.gravity = 800;
        ps.move_speed_scale_multiplier = 1.0;
        ps.waterlevel = level;
        ps
    }

    #[test]
    fn the_level_is_read_at_the_feet() {
        let mut ps = swimmer(0);
        ps.origin = [0.0, 0.0, -50.0];
        set_water_level(&mut ps, &Pool { surface: 0.0 });
        assert_eq!(ps.waterlevel, 4);
    }

    #[test]
    fn deep_and_still_the_water_lifts() {
        let mut ps = swimmer(5);
        let mut pml = crate::testing::pml(false);
        let cmd = UserCmd::default();
        swim_move(&mut ps, &mut pml, &cmd, air(), bounds(), &Pool { surface: 1000.0 }, false, None);
        // No gravity; the float's 10 a second at acceleration 2.
        assert!((ps.velocity[2] - 10.0).abs() < 1e-4, "{:?}", ps.velocity);
        // At level 4 nothing lifts, and nothing pulls down.
        let mut ps = swimmer(4);
        swim_move(&mut ps, &mut pml, &cmd, air(), bounds(), &Pool { surface: 1000.0 }, false, None);
        assert_eq!(ps.velocity, [0.0; 3]);
    }

    #[test]
    fn the_keys_swim_along_the_view_but_not_up_at_four() {
        let cmd = UserCmd {
            forwardmove: 127,
            ..UserCmd::default()
        };
        // Looking 45 degrees up.
        let h = libm::sqrtf(0.5);
        let mut pml = crate::testing::pml(false);
        pml.forward = [h, 0.0, h];
        let mut ps = swimmer(5);
        swim_move(&mut ps, &mut pml, &cmd, air(), bounds(), &Pool { surface: 1000.0 }, false, None);
        // 190 x 1.3 wished; the push is 0.05 x 2 x that a frame.
        let push = 0.05 * 2.0 * 190.0 * 1.3;
        assert!((ps.velocity[0] - push * h).abs() < 1e-2, "{:?}", ps.velocity);
        assert!((ps.velocity[2] - push * h).abs() < 1e-2, "{:?}", ps.velocity);
        assert_eq!(pml.walking, 0);

        let mut pml = crate::testing::pml(false);
        pml.forward = [h, 0.0, h];
        let mut ps = swimmer(4);
        swim_move(&mut ps, &mut pml, &cmd, air(), bounds(), &Pool { surface: 1000.0 }, false, None);
        // The wish is measured after the clamp: only the level part pushes.
        assert!((ps.velocity[0] - push * h).abs() < 1e-2, "{:?}", ps.velocity);
        assert_eq!(ps.velocity[2], 0.0);
        // Down still goes down at four.
        let mut pml = crate::testing::pml(false);
        pml.forward = [h, 0.0, -h];
        let mut ps = swimmer(4);
        swim_move(&mut ps, &mut pml, &cmd, air(), bounds(), &Pool { surface: 1000.0 }, false, None);
        assert!(ps.velocity[2] < -1.0, "{:?}", ps.velocity);
    }

    #[test]
    fn standing_in_water_grips_by_the_level() {
        let pml = crate::testing::pml(true);
        let mut ps = swimmer(1);
        assert!((friction_drop(&ps, &pml, 100.0) - 5.5 * 100.0 * 0.05).abs() < 1e-4);
        ps.waterlevel = 2;
        assert!((friction_drop(&ps, &pml, 100.0) - 5.5 * 100.0 * 1.7 * 0.05).abs() < 1e-3);
        ps.waterlevel = 3;
        assert!((friction_drop(&ps, &pml, 100.0) - 5.5 * 100.0 * 2.0 * 0.05).abs() < 1e-3);
        ps.waterlevel = 6;
        assert!((friction_drop(&ps, &pml, 100.0) - 5.5 * 100.0 * 3.0 * 0.05).abs() < 1e-3);
        // Off the ground the water does not slow.
        assert_eq!(friction_drop(&ps, &crate::testing::pml(false), 100.0), 0.0);
    }
}
