//! Spinning an axis frame about its own forward vector.
//!
//! Used for elements that start at a random roll and for the frame an impact
//! mark is stamped with.

use crate::runtime::random::{FX_RAND_CH_INITIAL_ROTATION, random_table_f32};
use crate::runtime::vec::vec3_cross;

/// Degrees in a full turn.
const FULL_TURN_DEGREES: f64 = 360.0;
/// Radians per degree, as the f32 the engine used widened to f64.
pub(super) const FX_DEG_TO_RAD: f64 = 0.017_453_292_384_743_69;
/// Degrees per radian, likewise.
const RAD_TO_DEG: f64 = 57.295_776_367_187_5;

/// The random starting roll, in degrees, for this seed.
pub fn runner_rand_rot_degrees(random_seed: u32) -> f32 {
    let unit = f64::from(random_table_f32(random_seed, FX_RAND_CH_INITIAL_ROTATION));
    (unit * FULL_TURN_DEGREES) as f32
}

/// `point` turned `degrees` about the unit vector `dir`.
fn rotate_point_around_vector(dir: [f32; 3], point: [f32; 3], degrees: f32) -> [f32; 3] {
    let radians = (f64::from(degrees) * FX_DEG_TO_RAD) as f32;
    let cos = libm::cosf(radians);
    let sin = libm::sinf(radians);
    let rest = 1.0 - cos;
    let [x, y, z] = dir;
    let rows = [
        [
            rest * x * x + cos,
            rest * y * x - sin * z,
            y * sin + rest * x * z,
        ],
        [
            sin * z + rest * y * x,
            cos + y * y * rest,
            rest * y * z - sin * x,
        ],
        [
            rest * x * z - y * sin,
            rest * y * z + sin * x,
            rest * z * z + cos,
        ],
    ];
    rows.map(|row| row[0] * point[0] + row[1] * point[1] + row[2] * point[2])
}

/// The frame an impact mark is drawn in: the surface frame's `right` turned
/// about its forward by `orientation_radians`, with a texture axis crossed
/// from the two. Returns `[forward, texture axis, turned right]`.
pub fn impact_mark_axis(axis_in: [[f32; 3]; 3], orientation_radians: f32) -> [[f32; 3]; 3] {
    let forward = axis_in[0];
    let degrees = (f64::from(orientation_radians) * RAD_TO_DEG) as f32;
    let turned = rotate_point_around_vector(forward, axis_in[1], degrees);
    [forward, vec3_cross(forward, turned), turned]
}

/// The frame with `right` and `up` spun about forward by a random roll.
pub fn randomly_rotate_axis(axis_in: [[f32; 3]; 3], random_seed: u32) -> [[f32; 3]; 3] {
    let forward = axis_in[0];
    let right =
        rotate_point_around_vector(forward, axis_in[1], runner_rand_rot_degrees(random_seed));
    [forward, right, vec3_cross(forward, right)]
}

#[cfg(test)]
mod tests {
    use super::*;

    const IDENTITY: [[f32; 3]; 3] = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];

    #[test]
    fn a_quarter_turn_about_forward_takes_right_to_up() {
        let turned = rotate_point_around_vector([1.0, 0.0, 0.0], [0.0, 1.0, 0.0], 90.0);
        assert!(turned[0].abs() < 1e-6);
        assert!(turned[1].abs() < 1e-6, "{turned:?}");
        assert!((turned[2] - 1.0).abs() < 1e-6, "{turned:?}");
    }

    #[test]
    fn turning_about_forward_leaves_forward_alone() {
        let frame = randomly_rotate_axis(IDENTITY, 9);
        assert_eq!(frame[0], IDENTITY[0]);
    }

    #[test]
    fn the_rolled_frame_stays_square() {
        let frame = randomly_rotate_axis(IDENTITY, 123);
        let dot = |a: [f32; 3], b: [f32; 3]| a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
        assert!(dot(frame[0], frame[1]).abs() < 1e-5);
        assert!(dot(frame[1], frame[2]).abs() < 1e-5);
        assert!((dot(frame[1], frame[1]) - 1.0).abs() < 1e-5);
    }

    #[test]
    fn an_unrotated_mark_keeps_the_frame() {
        let frame = impact_mark_axis(IDENTITY, 0.0);
        assert_eq!(frame[0], [1.0, 0.0, 0.0]);
        assert_eq!(frame[2], [0.0, 1.0, 0.0]);
        assert_eq!(frame[1], [0.0, 0.0, 1.0]);
    }

    #[test]
    fn the_starting_roll_is_under_a_full_turn() {
        for seed in 0..64 {
            assert!((0.0..360.0).contains(&runner_rand_rot_degrees(seed)));
        }
    }
}
