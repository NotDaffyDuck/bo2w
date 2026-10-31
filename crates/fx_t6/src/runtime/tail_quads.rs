//! Laying out a tail sprite: a quad stretched along the element's velocity
//! and turned to face the camera.

use crate::runtime::vec::{vec3_cross, vec3_length_sq, vec3_normalize};

/// A cross product shorter than this (squared) means the velocity points at
/// the camera and the quad has no sideways direction.
const DEGENERATE_SQ: f32 = 1e-12;

/// The point the tail is drawn from: back from the element along its
/// velocity direction by `second_extent`.
pub fn tail_anchor_origin(origin: [f32; 3], heading: [f32; 3], second_extent: f32) -> [f32; 3] {
    let back = -second_extent;
    [
        origin[0] + back * heading[0],
        origin[1] + back * heading[1],
        origin[2] + back * heading[2],
    ]
}

/// The frame `[sideways, along, facing]` of a tail sprite, or `None` when
/// the velocity points straight at (or away from) the camera. `sideways` is
/// at right angles to both the velocity and the line to the camera.
pub fn tail_sprite_axes(
    heading: [f32; 3],
    eye_at: [f32; 3],
    scene_point: [f32; 3],
) -> Option<[[f32; 3]; 3]> {
    let to_camera = [
        eye_at[0] - scene_point[0],
        eye_at[1] - scene_point[1],
        eye_at[2] - scene_point[2],
    ];
    let sideways = vec3_cross(heading, to_camera);
    if vec3_length_sq(sideways) < DEGENERATE_SQ {
        return None;
    }
    let sideways = vec3_normalize(sideways);
    Some([sideways, heading, vec3_cross(sideways, heading)])
}

/// The full width of a sprite given its half width.
pub fn tail_sprite_full_extent(half: f32) -> f32 {
    half * 2.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_anchor_sits_behind_the_element() {
        assert_eq!(
            tail_anchor_origin([1.0, 2.0, 3.0], [1.0, 0.0, 0.0], 4.0),
            [-3.0, 2.0, 3.0]
        );
    }

    #[test]
    fn the_axes_are_square_to_each_other() {
        let axes = tail_sprite_axes([1.0, 0.0, 0.0], [0.0, 0.0, 10.0], [0.0; 3]).unwrap();
        assert_eq!(axes[1], [1.0, 0.0, 0.0]);
        let dot = |a: [f32; 3], b: [f32; 3]| a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
        assert!(dot(axes[0], axes[1]).abs() < 1e-6);
        assert!(dot(axes[0], axes[2]).abs() < 1e-6);
        assert!((dot(axes[0], axes[0]) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn the_facing_axis_points_at_the_camera_side() {
        let axes = tail_sprite_axes([1.0, 0.0, 0.0], [0.0, 0.0, 10.0], [0.0; 3]).unwrap();
        assert!(axes[2][2] > 0.99, "{:?}", axes[2]);
    }

    #[test]
    fn a_velocity_straight_at_the_camera_has_no_sprite() {
        assert_eq!(
            tail_sprite_axes([0.0, 0.0, 1.0], [0.0, 0.0, 10.0], [0.0; 3]),
            None
        );
    }

    #[test]
    fn the_full_extent_is_twice_the_half() {
        assert_eq!(tail_sprite_full_extent(3.5), 7.0);
    }
}
