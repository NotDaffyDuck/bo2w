//! Black Ops II's PM_DoSlideAdjustments: on sliding ground the player is
//! pushed downhill, and on forced slick ground (pm_flags 0x40000000) the
//! push mixes the slope with the way the player is already going. Both stop
//! adding speed past a cap.

use crate::bridge::{Pml, accelerate};
use playerstate_t6::game::PlayerState;

use crate::friction::SURF_SLIDE;
use crate::project::{ground_normal, length, normalize};

/// The forced slick flag (pm_flags).
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

pub(crate) fn slide_adjust(ps: &mut PlayerState, pml: &Pml) {
    let normal = ground_normal(&pml.ground_trace);
    let slope_scale = {
        let len = libm::sqrtf(normal[1] * normal[1] + normal[0] * normal[0]);
        1.0 / if len <= 0.0 { 1.0 } else { len }
    };

    if pml.ground_trace[4] & SURF_SLIDE != 0 {
        let downhill = [normal[0] * slope_scale, normal[1] * slope_scale, slope_scale * 0.0];
        accelerate(ps, pml, &downhill, SLIDING_WISHSPEED, SLIDING_ACCEL);
        let speed = length(&ps.velocity);
        if SLIDING_VELOCITY_CAP * SLIDING_VELOCITY_CAP < speed {
            let scale = 1.0 / speed;
            for axis in &mut ps.velocity {
                *axis = *axis * scale * SLIDING_VELOCITY_CAP;
            }
        }
        return;
    }

    if ps.pm_flags & PMF_SLICK == 0 || pml.walking == 0 {
        return;
    }
    let inv = 1.0 / SLICK_INVSPEED;
    let push = [
        (normal[0] * slope_scale * SLICK_SLOPE_WEIGHT + ps.velocity[0] * inv) * SLICK_MIX_SCALE,
        (normal[1] * slope_scale * SLICK_SLOPE_WEIGHT + inv * ps.velocity[1]) * SLICK_MIX_SCALE,
        slope_scale * 0.0 * SLICK_SLOPE_WEIGHT * SLICK_MIX_SCALE,
    ];
    accelerate(ps, pml, &push, SLICK_WISHSPEED, SLICK_ACCEL);
    if SLIDING_VELOCITY_CAP * SLIDING_VELOCITY_CAP < length(&ps.velocity) {
        let mut dir = ps.velocity;
        normalize(&mut dir);
        ps.velocity = [
            dir[0] * SLIDING_VELOCITY_CAP,
            dir[1] * SLIDING_VELOCITY_CAP,
            dir[2] * SLIDING_VELOCITY_CAP,
        ];
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pml(walking: bool, surface: u32, normal: [f32; 3]) -> Pml {
        let mut ground_trace = [0; 11];
        ground_trace[1] = normal[0].to_bits();
        ground_trace[2] = normal[1].to_bits();
        ground_trace[3] = normal[2].to_bits();
        ground_trace[4] = surface;
        Pml {
            forward: [0.0; 3],
            right: [0.0; 3],
            up: [0.0; 3],
            frametime: 0.05,
            msec: 50,
            walking: u32::from(walking),
            ground_plane: u32::from(walking),
            almost_ground_plane: u32::from(walking),
            ground_trace,
            previous_origin: [0.0; 3],
            previous_velocity: [0.0; 3],
            holdrand: 0,
        }
    }

    #[test]
    fn sliding_ground_pushes_downhill() {
        let mut ps = PlayerState::ZERO;
        let n = 1.0 / libm::sqrtf(2.0);
        slide_adjust(&mut ps, &pml(true, SURF_SLIDE, [n, 0.0, n]));
        // 2.0 x 4000 x 0.05 = 400 along +x (the slope's downhill side).
        assert!((ps.velocity[0] - 400.0).abs() < 1e-2, "{:?}", ps.velocity);
        assert_eq!(ps.velocity[1], 0.0);
        assert_eq!(ps.velocity[2], 0.0);
    }

    #[test]
    fn flat_sliding_ground_does_not_push() {
        let mut ps = PlayerState::ZERO;
        slide_adjust(&mut ps, &pml(true, SURF_SLIDE, [0.0, 0.0, 1.0]));
        assert_eq!(ps.velocity, [0.0; 3]);
    }

    #[test]
    fn plain_ground_is_left_alone() {
        let mut ps = PlayerState::ZERO;
        ps.velocity = [100.0, 0.0, 0.0];
        let n = 1.0 / libm::sqrtf(2.0);
        slide_adjust(&mut ps, &pml(true, 0, [n, 0.0, n]));
        assert_eq!(ps.velocity, [100.0, 0.0, 0.0]);
    }

    #[test]
    fn slick_needs_the_ground() {
        let mut ps = PlayerState::ZERO;
        ps.pm_flags = PMF_SLICK;
        ps.velocity = [100.0, 0.0, 0.0];
        let n = 1.0 / libm::sqrtf(2.0);
        slide_adjust(&mut ps, &pml(false, 0, [n, 0.0, n]));
        assert_eq!(ps.velocity, [100.0, 0.0, 0.0]);
    }

    #[test]
    fn slick_mixes_slope_and_speed() {
        let mut ps = PlayerState::ZERO;
        ps.pm_flags = PMF_SLICK;
        ps.velocity = [0.0, 150.0, 0.0];
        let n = 1.0 / libm::sqrtf(2.0);
        slide_adjust(&mut ps, &pml(true, 0, [n, 0.0, n]));
        // Push = (2, 1, 0) x 0.333; along it the player has 50 of the
        // 1000 wanted, so it adds 0.25 x 1000 x 0.05 = 12.5 along the push.
        let push = [2.0 * 0.333_f32, 0.333, 0.0];
        assert!((ps.velocity[0] - push[0] * 12.5).abs() < 1e-3, "{:?}", ps.velocity);
        assert!((ps.velocity[1] - (150.0 + push[1] * 12.5)).abs() < 1e-3, "{:?}", ps.velocity);
    }
}
