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
    pub fn xyz(self) -> [f32; 3] {
        [self.x, self.y, self.z]
    }
}

/// Adds two view-space vectors part by part.
fn sum3(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

/// The gun's origin moved by the hand-set offset.
pub(crate) fn apply_cg_gun_offset_view(origin: [f32; 3], gun: [f32; 3]) -> [f32; 3] {
    sum3(origin, gun)
}

/// The gun's origin moved by the landing dip (already spread over the view's axes).
pub(crate) fn apply_viewweapon_land_view(origin: [f32; 3], land_view: [f32; 3]) -> [f32; 3] {
    sum3(origin, land_view)
}

/// A placement given as (forward, right, up) plus angles, as a transform in the camera's own space.
///
/// The camera looks down its negative z, so forward becomes minus z.
pub(crate) fn iw_view_placement_to_bevy_camera_local(
    origin: [f32; 3],
    angles_deg: [f32; 3],
) -> Transform {
    let [forward, right, up] = origin;
    Transform {
        translation: Vec3::new(right, up, -forward),
        rotation: crate::anim::fpv_pose::placement_angles_to_bevy_camera_quat(angles_deg),
        ..Default::default()
    }
}
