//! Black Ops II's PM_DoSlideAdjustments: on sliding ground the player is
//! pushed downhill, and on forced slick ground (movement_state_bits 0x40000000) the
//! push mixes the slope with the way the player is already going. Both stop
//! adding speed past a cap.

use crate::bridge::{TickLocals, accelerate};
use playerstate_t6::game::AvatarState;

use crate::ground_drag::SURF_SLIDE;
use crate::project::{ground_normal, length, scale_to_unit};

/// The forced slick flag (movement_state_bits).
pub(crate) const PMF_SLICK: u32 = 0x4000_0000;

/// player_sliding_wishspeed.
const SLIDING_WISHSPEED: f32 = 4000.0;

/// How hard sliding ground pushes downhill.
const SLIDING_ACCEL: f32 = 2.0;

/// player_slick_wishspeed.
const SLICK_WISHSPEED: f32 = 1000.0;

/// player_slick_invspeed: how much the speed you already have steers the
/// slick push (bigger is less).
const SLICK_INVSPEED: f32 = 150.0;

/// player_slick_accel.
const SLICK_ACCEL: f32 = 0.25;

/// How strongly the slope steers the slick push, against the speed's 1.
const SLICK_SLOPE_WEIGHT: f32 = 2.0;

/// The slick push's mix is scaled by this (about a third).
const SLICK_MIX_SCALE: f32 = 0.333;

/// player_sliding_velocity_cap. BO2 checks the speed against this number
/// squared (not the speed squared), so in practice it only bites at very
/// high speeds; kept as BO2 has it.
const SLIDING_VELOCITY_CAP: f32 = 500.0;

pub(crate) fn slide_adjust(avatar: &mut AvatarState, tick_vars: &TickLocals) {
    let normal = ground_normal(&tick_vars.floor_probe);
    let slope_scale = {
        let len = libm::sqrtf(normal[1] * normal[1] + normal[0] * normal[0]);
        1.0 / if len <= 0.0 { 1.0 } else { len }
    };

    if tick_vars.floor_probe[4] & SURF_SLIDE != 0 {
        let downhill = [normal[0] * slope_scale, normal[1] * slope_scale, slope_scale * 0.0];
        accelerate(avatar, tick_vars, &downhill, SLIDING_WISHSPEED, SLIDING_ACCEL);
        let speed = length(&avatar.velocity);
        if SLIDING_VELOCITY_CAP * SLIDING_VELOCITY_CAP < speed {
            let scale = 1.0 / speed;
            for axis in &mut avatar.velocity {
                *axis = *axis * scale * SLIDING_VELOCITY_CAP;
            }
        }
        return;
    }

    if avatar.movement_state_bits & PMF_SLICK == 0 || tick_vars.walking == 0 {
        return;
    }
    let stow_kind = 1.0 / SLICK_INVSPEED;
    let push = [
        (normal[0] * slope_scale * SLICK_SLOPE_WEIGHT + avatar.velocity[0] * stow_kind) * SLICK_MIX_SCALE,
        (normal[1] * slope_scale * SLICK_SLOPE_WEIGHT + stow_kind * avatar.velocity[1]) * SLICK_MIX_SCALE,
        slope_scale * 0.0 * SLICK_SLOPE_WEIGHT * SLICK_MIX_SCALE,
    ];
    accelerate(avatar, tick_vars, &push, SLICK_WISHSPEED, SLICK_ACCEL);
    if SLIDING_VELOCITY_CAP * SLIDING_VELOCITY_CAP < length(&avatar.velocity) {
        let mut dir = avatar.velocity;
        scale_to_unit(&mut dir);
        avatar.velocity = [
            dir[0] * SLIDING_VELOCITY_CAP,
            dir[1] * SLIDING_VELOCITY_CAP,
            dir[2] * SLIDING_VELOCITY_CAP,
        ];
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tick_vars(grounded: bool, surf: u32, normal_in: [f32; 3]) -> TickLocals {
        let mut floor_probe = [0; 11];
        floor_probe[1] = normal_in[0].to_bits();
        floor_probe[2] = normal_in[1].to_bits();
        floor_probe[3] = normal_in[2].to_bits();
        floor_probe[4] = surf;
        TickLocals {
            fwd_axis: Default::default(),
            right_axis: Default::default(),
            up_axis: [0.0; 3],
            step_millis_elapsed: 0.05,
            frame_ms: 50,
            walking: u32::from(grounded),
            standing_on_floor: u32::from(grounded),
            near_floor_contact: u32::from(grounded),
            floor_probe,
            prior_tick_position: [0.0; 3],
            prior_tick_speed_vec: [0.0; 3],
            frame_seed: 0_i32,
        }
    }

    #[test]
    fn sliding_ground_pushes_downhill() {
        let mut avatar = AvatarState::ZERO;
        let n = 1.0 / libm::sqrtf(2.0);
        slide_adjust(&mut avatar, &tick_vars(true, SURF_SLIDE, [n, 0.0, n]));
        // 2.0 x 4000 x 0.05 = 400 along +x (the slope's downhill side).
        assert!((avatar.velocity[0] - 400.0).abs() < 1e-2, "{:?}", avatar.velocity);
        assert_eq!(avatar.velocity[1], 0.0);
        assert_eq!(avatar.velocity[2], 0.0);
    }

    #[test]
    fn flat_sliding_ground_does_not_push() {
        let mut avatar = AvatarState::ZERO;
        slide_adjust(&mut avatar, &tick_vars(true, SURF_SLIDE, [0.0, 0.0, 1.0]));
        assert_eq!(avatar.velocity, [0.0; 3]);
    }

    #[test]
    fn plain_ground_is_left_alone() {
        let mut avatar = AvatarState::ZERO;
        avatar.velocity = [100.0, 0.0, 0.0];
        let n = 1.0 / libm::sqrtf(2.0);
        slide_adjust(&mut avatar, &tick_vars(true, 0, [n, 0.0, n]));
        assert_eq!(avatar.velocity, [100.0, 0.0, 0.0]);
    }

    #[test]
    fn slick_needs_the_ground() {
        let mut avatar = AvatarState::ZERO;
        avatar.movement_state_bits = PMF_SLICK;
        avatar.velocity = [100.0, 0.0, 0.0];
        let n = 1.0 / libm::sqrtf(2.0);
        slide_adjust(&mut avatar, &tick_vars(false, 0, [n, 0.0, n]));
        assert_eq!(avatar.velocity, [100.0, 0.0, 0.0]);
    }

    #[test]
    fn slick_mixes_slope_and_speed() {
        let mut avatar = AvatarState::ZERO;
        avatar.movement_state_bits = PMF_SLICK;
        avatar.velocity = [0.0_f32, 150.0, 0.0];
        let n = 1.0 / libm::sqrtf(2.0);
        slide_adjust(&mut avatar, &tick_vars(true, 0, [n, 0.0, n]));
        // Push = (2, 1, 0) x 0.333; along it the player has 50 of the
        // 1000 wanted, so it adds 0.25 x 1000 x 0.05 = 12.5 along the push.
        let push = [2.0 * 0.333_f32, 0.333, 0.0];
        assert!((avatar.velocity[0] - push[0] * 12.5).abs() < 1e-3, "{:?}", avatar.velocity);
        assert!((avatar.velocity[1] - (150.0 + push[1] * 12.5)).abs() < 1e-3, "{:?}", avatar.velocity);
    }
}
