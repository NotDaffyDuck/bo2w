//! Black Ops II's PM_AirMove: friction (only a dive's slide meets the
//! ground here), the keys' push at air acceleration 1, the sliding and slick
//! ground's push, then the slide with plain gravity.

use crate::bridge::{AirMoveContext, Bo2Lunge, MoveBounds, Pml, accelerate, step_slide_move};
use playerstate_t6::game::{PlayerState, UserCmd};

use crate::gjk::Bo2Collision;
use crate::friction::friction_scaled;
use crate::movement_dir::set_movement_dir;
use crate::project::{clip_velocity, ground_normal, wish_dir};
use crate::slide_adjust::slide_adjust;

/// The keys' push in the air.
const AIR_ACCEL: f32 = 1.0;

/// Walking slowly (pm_flags 0x40), leaning or diving, the keys push at this
/// share.
const SLOW_PUSH_SCALE: f32 = 0.4;

/// Noclip and ufo fly this many times faster.
const NOCLIP_SCALE: f32 = 3.0;
const UFO_SCALE: f32 = 6.0;

#[allow(clippy::too_many_arguments)]
pub(crate) fn air_move<C: Bo2Collision>(
    ps: &mut PlayerState,
    pml: &Pml,
    cmd: &UserCmd,
    context: AirMoveContext,
    bounds: MoveBounds,
    collision: &C,
    zombies: bool,
    lunge: Option<Bo2Lunge>,
) -> f32 {
    air_move_with(ps, pml, cmd, context, bounds, collision, zombies, lunge, |_| 1.0)
}

/// The air move with the ground's grip scaled by `ground_scale` (a dive's
/// slide runs this move); `lunge` is the knife lunge's target, if any.
/// Returns a stair step's height for the camera.
#[allow(clippy::too_many_arguments)]
pub(crate) fn air_move_with<C: Bo2Collision>(
    ps: &mut PlayerState,
    pml: &Pml,
    cmd: &UserCmd,
    context: AirMoveContext,
    bounds: MoveBounds,
    collision: &C,
    zombies: bool,
    lunge: Option<Bo2Lunge>,
    ground_scale: impl FnOnce(&mut PlayerState) -> f32,
) -> f32 {
    friction_scaled(ps, pml, true, lunge, ground_scale);

    let command_scale = cmd_scale(ps, cmd, context.player_spectate_speed_scale);
    let (wishdir, wishspeed) = wish_dir(pml.forward, pml.right, cmd.forwardmove, cmd.rightmove);
    accelerate(ps, pml, &wishdir, wishspeed * command_scale, AIR_ACCEL);

    slide_adjust(ps, pml);

    if pml.ground_plane != 0 {
        ps.velocity = clip_velocity(&ps.velocity, &ground_normal(&pml.ground_trace));
    }

    let step = if collision.gjk_on() {
        crate::gjk::step::step_slide_move(
            ps,
            pml,
            collision,
            bounds.mins,
            bounds.maxs,
            bounds.tracemask,
            true,
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
            Some(ps.gravity as f32),
            zombies,
        )
    };
    set_movement_dir(ps, pml, cmd);
    step
}

/// PM_CmdScale: how far the keys go toward the player's speed.
pub(crate) fn cmd_scale(ps: &PlayerState, cmd: &UserCmd, spectate_speed_scale: f32) -> f32 {
    let forward = cmd.forwardmove as f32;
    let right = cmd.rightmove as f32;
    let largest = forward.abs().max(right.abs());
    if largest == 0.0 {
        return 0.0;
    }
    let magnitude = libm::sqrtf(forward * forward + right * right);
    let mut scale = (ps.speed as f32 * largest) / (magnitude * 127.0);
    if ps.pm_flags & 0x40 != 0 || ps.leanf != 0.0 || crate::dive::active(ps) {
        scale *= SLOW_PUSH_SCALE;
    }
    scale
        * match ps.pm_type {
            2 => NOCLIP_SCALE,
            3 => UFO_SCALE,
            5 => spectate_speed_scale,
            _ => 1.0,
        }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A dive (in the air and sliding) steers at 0.4.
    #[test]
    fn dive_steers_at_four_tenths() {
        let cmd = UserCmd {
            forwardmove: 127,
            ..UserCmd::default()
        };
        let mut ps = PlayerState::ZERO;
        ps.speed = 190;
        // The keys' push (127) times the scale is the player's speed.
        let full = cmd_scale(&ps, &cmd, 1.0);
        assert!((full * 127.0 - 190.0).abs() < 1e-3);
        ps.pm_flags = crate::dive::PMF_DIVE;
        assert!((cmd_scale(&ps, &cmd, 1.0) / full - 0.4).abs() < 1e-6);
        ps.pm_flags = crate::dive::PMF_DIVE | 0x40;
        ps.jump_time = -1;
        assert!((cmd_scale(&ps, &cmd, 1.0) / full - 0.4).abs() < 1e-6);
    }

    #[test]
    fn diagonal_keys_go_no_faster() {
        let mut ps = PlayerState::ZERO;
        ps.speed = 190;
        let both = UserCmd {
            forwardmove: 127,
            rightmove: 127,
            ..UserCmd::default()
        };
        // The push's size is sqrt(2) x 127, so the scale takes that back.
        let push = libm::sqrtf(2.0) * 127.0;
        assert!((cmd_scale(&ps, &both, 1.0) * push - 190.0).abs() < 1e-2);
    }
}
