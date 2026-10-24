//! An element's angles over its life, as an axis frame.
//!
//! The def authors a start angle and an angular velocity per axis, each a
//! `base + amplitude * random` range; the angle at a given age is the start
//! plus the age times the velocity.

use crate::runtime::origin::sample_float_range;
use crate::runtime::random::{
    FX_RAND_CH_ANG_VEL_PITCH, FX_RAND_CH_ANG_VEL_ROLL, FX_RAND_CH_ANG_VEL_YAW,
    FX_RAND_CH_SPAWN_ANGLES_PITCH, FX_RAND_CH_SPAWN_ANGLES_ROLL, FX_RAND_CH_SPAWN_ANGLES_YAW,
    random_table_f32,
};

/// Pitch, yaw and roll (radians) at `age_msec`.
fn elem_angles(
    spawn_angles: [[f32; 2]; 3],
    angular_velocity: [[f32; 2]; 3],
    seed: u32,
    age_msec: f32,
) -> [f32; 3] {
    let start_channels = [
        FX_RAND_CH_SPAWN_ANGLES_PITCH,
        FX_RAND_CH_SPAWN_ANGLES_YAW,
        FX_RAND_CH_SPAWN_ANGLES_ROLL,
    ];
    let spin_channels = [
        FX_RAND_CH_ANG_VEL_PITCH,
        FX_RAND_CH_ANG_VEL_YAW,
        FX_RAND_CH_ANG_VEL_ROLL,
    ];
    let mut angles = [0.0; 3];
    for i in 0..3 {
        let start = sample_float_range(
            spawn_angles[i][0],
            spawn_angles[i][1],
            random_table_f32(seed, start_channels[i]),
        );
        let spin = sample_float_range(
            angular_velocity[i][0],
            angular_velocity[i][1],
            random_table_f32(seed, spin_channels[i]),
        );
        angles[i] = start + age_msec * spin;
    }
    angles
}

/// The frame `[forward, right, up]` for pitch, yaw and roll in radians.
fn angles_to_axis(angles: [f32; 3]) -> [[f32; 3]; 3] {
    let [pitch, yaw, roll] = angles;
    let (sy, cy) = (libm::sinf(yaw), libm::cosf(yaw));
    let (sp, cp) = (libm::sinf(pitch), libm::cosf(pitch));
    let (sr, cr) = (libm::sinf(roll), libm::cosf(roll));
    [
        [cy * cp, sy * cp, -sp],
        [sr * sp * cy - cr * sy, sr * sp * sy + cr * cy, sr * cp],
        [cr * sp * cy + sr * sy, cr * sp * sy - sr * cy, cr * cp],
    ]
}

/// `a` times `b`.
pub fn mat3_mul(a: [[f32; 3]; 3], b: [[f32; 3]; 3]) -> [[f32; 3]; 3] {
    let mut product = [[0.0; 3]; 3];
    for (row, out_row) in a.iter().zip(product.iter_mut()) {
        for (col, cell) in out_row.iter_mut().enumerate() {
            *cell = row[0] * b[0][col] + row[1] * b[1][col] + row[2] * b[2][col];
        }
    }
    product
}

/// The element's frame at `age_msec`: its own angles applied inside the
/// effect's axes.
pub fn get_elem_angles_axis(
    spawn_angles: [[f32; 2]; 3],
    angular_velocity: [[f32; 2]; 3],
    seed: u32,
    age_msec: f32,
    effect_axis: [[f32; 3]; 3],
) -> [[f32; 3]; 3] {
    let own = angles_to_axis(elem_angles(spawn_angles, angular_velocity, seed, age_msec));
    mat3_mul(own, effect_axis)
}

#[cfg(test)]
mod tests {
    use super::*;

    const IDENTITY: [[f32; 3]; 3] = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
    const NO_RANGE: [[f32; 2]; 3] = [[0.0; 2]; 3];

    #[test]
    fn no_angles_is_the_identity_frame() {
        assert_eq!(angles_to_axis([0.0; 3]), IDENTITY);
    }

    #[test]
    fn a_quarter_yaw_points_forward_along_y() {
        let frame = angles_to_axis([0.0, core::f32::consts::FRAC_PI_2, 0.0]);
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
}
