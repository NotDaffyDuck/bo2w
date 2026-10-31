//! Black Ops II's PM_AirMove: friction (only a dive's slide meets the
//! ground here), the keys' push at air acceleration 1, the sliding and slick
//! ground's push, then the slide with plain gravity.

use crate::bridge::{AirborneStepArgs, Bo2Lunge, HullExtents, TickLocals, accelerate, step_slide_move};
use playerstate_t6::game::{AvatarState, InputCommand};

use crate::gjk::Bo2Collision;
use crate::ground_drag::friction_scaled;
use crate::movement_dir::set_movement_dir;
use crate::project::{clip_velocity, ground_normal, wish_dir};
use crate::slide_adjust::slide_adjust;

/// The keys' push in the air.
const AIR_ACCEL: f32 = 1.0;

/// Walking slowly (movement_state_bits 0x40), leaning or diving, the keys push at this
/// share.
const SLOW_PUSH_SCALE: f32 = 0.4;

/// Noclip and ufo fly this many times faster.
const NOCLIP_SCALE: f32 = 3.0;
const UFO_SCALE: f32 = 6.0;

#[allow(clippy::too_many_arguments)]
pub(crate) fn air_move<C: Bo2Collision>(
    avatar: &mut AvatarState,
    tick_vars: &TickLocals,
    ucmd: &InputCommand,
    frame_ctx: AirborneStepArgs,
    bounds: HullExtents,
    solids: &C,
    zombies: bool,
    lunge: Option<Bo2Lunge>,
) -> f32 {
    air_move_with(avatar, tick_vars, ucmd, frame_ctx, bounds, solids, zombies, lunge, |_| 1.0)
}

/// The air move with the ground's grip scaled by `ground_scale` (a dive's
/// slide runs this move); `lunge` is the knife lunge's target, if any.
/// Returns a stair step's height for the camera.
#[allow(clippy::too_many_arguments)]
pub(crate) fn air_move_with<C: Bo2Collision>(
    avatar: &mut AvatarState,
    tick_vars: &TickLocals,
    ucmd: &InputCommand,
    frame_ctx: AirborneStepArgs,
    bounds: HullExtents,
    solids: &C,
    zombies: bool,
    lunge: Option<Bo2Lunge>,
    ground_scale: impl FnOnce(&mut AvatarState) -> f32,
) -> f32 {
    friction_scaled(avatar, tick_vars, true, lunge, ground_scale);

    let command_scale = cmd_scale(avatar, ucmd, frame_ctx.observer_pace_ratio);
    let (intended_heading, intended_pace) = wish_dir(tick_vars.fwd_axis, tick_vars.right_axis, ucmd.thrust_ahead, ucmd.thrust_lateral);
    accelerate(avatar, tick_vars, &intended_heading, intended_pace * command_scale, AIR_ACCEL);

    slide_adjust(avatar, tick_vars);

    if tick_vars.standing_on_floor != 0 {
        avatar.velocity = clip_velocity(&avatar.velocity, &ground_normal(&tick_vars.floor_probe));
    }

    let HullExtents { lo_corner: lo, hi_corner: hi, .. } = bounds;
    let step = if solids.gjk_on() {
        crate::gjk::step::step_slide_move(
            avatar,
            tick_vars,
            solids,
            lo,
            hi,
            bounds.collide_filter,
            true,
            zombies,
        )
    } else {
        step_slide_move(
            avatar,
            tick_vars,
            solids,
            lo,
            hi,
            bounds.collide_filter,
            Some(avatar.gravity as f32),
            zombies,
        )
    };
    set_movement_dir(avatar, tick_vars, ucmd);
    step
}

/// PM_CmdScale: how far the keys go toward the player's speed.
pub(crate) fn cmd_scale(avatar: &AvatarState, ucmd: &InputCommand, observer_pace: f32) -> f32 {
    let ahead = ucmd.thrust_ahead as f32;
    let side = ucmd.thrust_lateral as f32;
    let largest = ahead.abs().max(side.abs());
    if 0.0 == largest {
        return 0.0;
    }
    let magnitude = libm::sqrtf(ahead * ahead + side * side);
    let mut scale = (avatar.speed as f32 * largest) / (magnitude * 127.0);
    if avatar.movement_state_bits & 0x40 != 0 || avatar.tilt_sideways != 0.0 || crate::dive::active(avatar) {
        scale *= SLOW_PUSH_SCALE;
    }
    scale
        * match avatar.movement_mode {
            2 => NOCLIP_SCALE,
            3 => UFO_SCALE,
            5 => observer_pace,
            _ => 1.0,
        }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A dive (in the air and sliding) steers at 0.4.
    #[test]
    fn dive_steers_at_four_tenths() {
        let ucmd = InputCommand {
            thrust_ahead: 127,
            ..InputCommand::default()
        };
        let mut avatar = AvatarState::ZERO;
        avatar.speed = 190;
        // The keys' push (127) times the scale is the player's speed.
        let full = cmd_scale(&avatar, &ucmd, 1.0);
        assert!((full * 127.0 - 190.0).abs() < 1e-3);
        avatar.movement_state_bits = crate::dive::PMF_DIVE;
        assert!((cmd_scale(&avatar, &ucmd, 1.0) / full - 0.4).abs() < 1e-6);
        avatar.movement_state_bits = crate::dive::PMF_DIVE | 0x40;
        avatar.hop_start_ms = -1;
        assert!((cmd_scale(&avatar, &ucmd, 1.0) / full - 0.4).abs() < 1e-6);
    }

    #[test]
    fn diagonal_keys_go_no_faster() {
        let mut avatar = AvatarState::ZERO;
        avatar.speed = 190;
        let both = InputCommand {
            thrust_ahead: 127,
            thrust_lateral: 127,
            ..InputCommand::default()
        };
        // The push's size is sqrt(2) x 127, so the scale takes that back.
        let push = libm::sqrtf(2.0) * 127.0;
        assert!((cmd_scale(&avatar, &both, 1.0) * push - 190.0).abs() < 1e-2);
    }
}
