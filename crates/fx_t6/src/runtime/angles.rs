//! An element's angles over its life, as an axis frame.
//!
//! The def authors a start angle and an angular velocity per axis, each a
//! `base + amplitude * random` range; the angle at a given age is the start
//! plus the age times the velocity.

use crate::runtime::birth_spot::sample_float_range;
use crate::runtime::dice::{
    random_table_f32,

};
use crate::runtime::dice::DICE_LANE_SPIN_TILT;
use crate::runtime::dice::DICE_LANE_SPIN_ROLL;
use crate::runtime::dice::DICE_LANE_SPIN_TURN;
use crate::runtime::dice::DICE_LANE_BIRTH_TILT;
use crate::runtime::dice::DICE_LANE_BIRTH_ROLL;
use crate::runtime::dice::DICE_LANE_BIRTH_TURN;

/// Pitch, yaw and roll (radians) at `lived_ms`.
fn elem_angles(
    birth_orientation: [[f32; 2]; 3],
    twirl_rate: [[f32; 2]; 3],
    seed: u32,
    lived_ms: f32,
) -> [f32; 3] {
    // Per axis (pitch, yaw, roll): the channel for its start, the one for its spin.
    const CHANNELS: [(u32, u32); 3] = [
        (DICE_LANE_BIRTH_TILT, DICE_LANE_SPIN_TILT),
        (DICE_LANE_BIRTH_TURN, DICE_LANE_SPIN_TURN),
        (DICE_LANE_BIRTH_ROLL, DICE_LANE_SPIN_ROLL),
    ];
    let range_at = |[base, amp]: [f32; 2], channel| {
        sample_float_range(base, amp, random_table_f32(seed, channel))
    };
    core::array::from_fn::<f32, 3, _>(|axis| {
        let (start_ch, spin_ch) = CHANNELS[axis];
        range_at(birth_orientation[axis], start_ch)
            + lived_ms * range_at(twirl_rate[axis], spin_ch)
    })
}

/// The frame `[forward, right, up]` for pitch, yaw and roll in radians.
fn angles_to_axis(pitch: f32, yaw: f32, roll: f32) -> [[f32; 3]; 3] {
    let (sp, codepoint) = libm::sincosf(pitch);
    let (sr, cr) = libm::sincosf(roll);
    let (sy, cy) = libm::sincosf(yaw);
    // The heading on the ground and the direction a quarter turn left of it.
    let heading = [cy, sy];
    let left = [-sy, cy];
    let grounded_now = |along: f32, across: f32, z: f32| {
        [
            along * heading[0] + across * left[0],
            along * heading[1] + across * left[1],
            z,
        ]
    };
    [
        grounded_now(codepoint, 0.0, -sp),
        grounded_now(sr * sp, cr, sr * codepoint),
        grounded_now(cr * sp, -sr, cr * codepoint),
    ]
}

/// `a` times `rhs`.
pub fn mat3_mul(a: [[f32; 3]; 3], rhs: [[f32; 3]; 3]) -> [[f32; 3]; 3] {
    a.map(|row| {
        core::array::from_fn(|col| row[0] * rhs[0][col] + row[1] * rhs[1][col] + row[2] * rhs[2][col])
    })
}

/// The element's frame at `lived_ms`: its own angles applied inside the
/// effect's axes.
pub fn get_elem_angles_axis(
    birth_orientation: [[f32; 2]; 3],
    twirl_rate: [[f32; 2]; 3],
    seed: u32,
    lived_ms: f32,
    fx_basis: [[f32; 3]; 3],
) -> [[f32; 3]; 3] {
    let [pitch, yaw, roll] = elem_angles(birth_orientation, twirl_rate, seed, lived_ms);
    let own = angles_to_axis(pitch, yaw, roll);
    mat3_mul(own, fx_basis)
}

#[cfg(test)]
mod tests {
    use super::*;

    const IDENTITY: [[f32; 3]; 3] = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
    const NO_RANGE: [[f32; 2]; 3] = [[0.0; 2]; 3];

    #[test]
    fn no_angles_is_the_identity_frame() {
        assert_eq!(angles_to_axis(0.0, 0.0, 0.0), IDENTITY);
    }

    #[test]
    fn a_quarter_yaw_points_forward_along_y() {
        let frame = angles_to_axis(0.0, core::f32::consts::FRAC_PI_2, 0.0);
        assert!(frame[0][0].abs() < 1e-6);
        assert!((frame[0][1] - 1.0).abs() < 1e-6);
    }

    #[test]
    fn multiplying_by_the_identity_changes_nothing() {
        let m = [[1.0, 2.0, 3.0], [4.0, 5.0, 6.0], [7.0, 8.0, 9.0]];
        assert_eq!(mat3_mul(m, IDENTITY), m);
        assert_eq!(mat3_mul(IDENTITY, m), m);
    }

    #[test]
    fn a_fixed_spin_grows_with_age() {
        let spin = [[0.0; 2], [0.5, 0.0], [0.0; 2]];
        let angles = elem_angles(NO_RANGE, spin, 11, 4.0);
        assert_eq!(angles, [0.0, 2.0, 0.0]);
    }

    #[test]
    fn an_element_with_no_angles_takes_the_effect_frame() {
        let effect = [[0.0, 1.0, 0.0], [-1.0, 0.0, 0.0], [0.0, 0.0, 1.0]];
        assert_eq!(
            get_elem_angles_axis(NO_RANGE, NO_RANGE, 3, 100.0, effect),
            effect
        );
    }

    #[test]
    fn any_angles_give_an_orthonormal_frame_with_the_known_forward() {
        let (pitch, yaw, roll) = (0.4f32, -1.1f32, 2.3f32);
        let f = angles_to_axis(pitch, yaw, roll);
        let dot = |a: [f32; 3], b: [f32; 3]| a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
        for i in 0..3 {
            for j in 0..3 {
                let want = if i == j { 1.0 } else { 0.0 };
                assert!((dot(f[i], f[j]) - want).abs() < 1e-5, "{i} {j}");
            }
        }
        assert!((f[0][2] + pitch.sin()).abs() < 1e-6);
        assert!((f[0][0] - yaw.cos() * pitch.cos()).abs() < 1e-6);
        // right x forward = up (a right-handed turn from right to up about forward)
        let cross = [
            f[1][1] * f[0][2] - f[1][2] * f[0][1],
            f[1][2] * f[0][0] - f[1][0] * f[0][2],
            f[1][0] * f[0][1] - f[1][1] * f[0][0],
        ];
        assert!(dot(cross, f[2]).abs() > 0.999);
    }
}
