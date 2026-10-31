//! Small helpers that place the first-person gun relative to the camera.

use bevy::prelude::*;

/// A hand-set nudge of the gun, in view units (forward, right, up), set from the console.
#[derive(Resource, Clone, Copy, Debug, Default)]
pub struct GunOffset {
    /// Forward.
    pub x: f32,
    /// Right.
    pub y: f32,
    /// Up.
    pub z: f32,
}

impl GunOffset {
    /// The three parts as one array, forward first.
    pub fn as_xyz(self) -> [f32; 3] {
        let Self { x, y, z } = self;
        [x, y, z]
    }
}

/// Adds two view-space vectors part by part.
pub(crate) fn sum3(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    std::array::from_fn(|axis| a[axis] + b[axis])
}

/// A placement given as (forward, right, up) plus angles, as a transform in the camera's own space.
///
/// The camera looks down its negative z, so forward becomes minus z.
pub(crate) fn placement_to_camera_local(
    org: [f32; 3],
    rotation_degrees: [f32; 3],
) -> Transform {
    let [forward, right, up] = org;
    let facing = crate::motion::gun_pose::placement_angles_to_bevy_camera_quat(rotation_degrees);
    Transform::from_translation(Vec3::new(right, up, -forward)).with_rotation(facing)
}
