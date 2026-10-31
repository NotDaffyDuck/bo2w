//! Space conversions and plain data shared by the first-person pose code.
//!
//! The tag space a viewmodel skeleton is authored in has x forward, y left and
//! z up. The renderer's camera looks down -Z with +X to the right and +Y up.

use bevy::math::{Mat3, Mat4, Quat, Vec3};
use xmodel_runtime::SkeletalClip;

pub use crate::motion::figure_pose::{SurfaceDrawOwner, SkinnedMeshFacet};

/// Where the tag-space x, y and z axes land in camera space.
const TAG_AXES_IN_CAMERA: Mat3 = Mat3::from_cols(Vec3::NEG_Z, Vec3::NEG_X, Vec3::Y);

/// Rotation taking tag-space directions into the renderer's camera space.
pub fn bone_tag_view_to_bevy_mat3() -> Mat3 {
    TAG_AXES_IN_CAMERA
}

/// The same rotation as a 4x4 with no translation.
pub fn bone_tag_view_to_bevy_camera() -> Mat4 {
    Mat4::from_mat3(TAG_AXES_IN_CAMERA)
}

/// Turns the Euler angles (degrees) of a placement into the camera-space
/// rotation it describes.
pub fn placement_angles_to_bevy_camera_quat(rotation_degrees: [f32; 3]) -> Quat {
    let [forward, left, up] = game_math::euler_to_rows(rotation_degrees).map(Vec3::from_array);
    let in_tag_space = Mat3::from_cols(forward, left, up);
    // A rotation's inverse is its transpose.
    let back = TAG_AXES_IN_CAMERA.transpose();
    Quat::from_mat3(&(TAG_AXES_IN_CAMERA * in_tag_space * back))
}

/// Where the camera tag sits relative to the view tag, in camera space.
#[inline]
pub fn tag_camera_lens_local(vw_world: Mat4, eye_world_matrix: Mat4) -> Mat4 {
    let camera_from_view = vw_world.inverse() * eye_world_matrix;
    let into = bone_tag_view_to_bevy_camera();
    into * camera_from_view * into.inverse()
}

/// Bone indices of the attachment points a weapon fires from or shows things
/// on. All optional: a gun carries the ones its model has.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AttachTagNames {
    pub flash_tag: Option<u16>,
    pub muzzle_flash_suppressed: Option<u16>,
    pub brass_tag: Option<u16>,
    pub knife_tag: Option<u16>,
    pub laser_tag: Option<u16>,
    pub tracker_screen: [Option<u16>; 3],
    pub tracker_light: Option<u16>,
    /// bo2zm: the left gun's flash and brass when both guns hang on one set
    /// of arms (Black Ops II dual wield).
    pub flash_left: Option<u16>,
    pub brass_left: Option<u16>,
    /// bo2mp: the gun's `tag_target` (BO2's `playviewmodelfx`).
    pub target: Option<u16>,
    /// bo2mp: the gun's `tag_hud` (the Target Finder screen plane).
    pub hud: Option<u16>,
    /// bo2zm: the wrists and gun tag (Mob's Afterlife ghost glow).
    pub wrist: [Option<u16>; 2],
    pub weapon: Option<u16>,
}

/// One hand's bones in eye space for a frame, with the tags that index them.
#[derive(Clone, Debug, Default)]
pub struct GunTagPoseFrame {
    pub joints: Vec<Mat4>,
    pub tags: AttachTagNames,
}

/// One clip a hand is playing this frame. `node` is the scheduler slot it is
/// playing in, which is what the rig binds its bone tracks against: a clip
/// that stayed in its node keeps the binding it was given.
#[derive(Clone, Copy)]
pub struct ClipPoseBinding<'a> {
    pub node_ix: usize,
    pub reel: &'a SkeletalClip,
    pub time: f32,
    pub wt: f32,
}

mod tests {
    #![cfg(test)]
    use super::*;

    fn close(a: Vec3, b: Vec3) -> bool {
        (a - b).length() < 1e-5
    }

    #[test]
    fn tag_directions_land_on_camera_axes() {
        let turn = TAG_AXES_IN_CAMERA;
        for (tag_axis, camera_axis, what) in [
            (Vec3::X, Vec3::NEG_Z, "forward looks down -Z"),
            (Vec3::Y, Vec3::NEG_X, "left is -X"),
            (Vec3::Z, Vec3::Y, "up is +Y"),
        ] {
            assert!(close(turn * tag_axis, camera_axis), "{what}");
        }
    }

    #[test]
    fn the_4x4_fold_matches_the_3x3_and_moves_nothing() {
        let probe = Vec3::new(3.0, -2.0, 7.0);
        let as_4x4 = bone_tag_view_to_bevy_camera();
        assert!(close(
            as_4x4.transform_point3(probe),
            bone_tag_view_to_bevy_mat3() * probe
        ));
        assert_eq!(as_4x4.w_axis.truncate(), Vec3::ZERO);
    }

    #[test]
    fn no_placement_angles_mean_no_rotation() {
        let quat = placement_angles_to_bevy_camera_quat([0.0; 3]);
        assert!(quat.angle_between(Quat::IDENTITY) < 1e-5);
    }

    #[test]
    fn a_yawed_placement_turns_about_the_camera_up_axis() {
        let quat = placement_angles_to_bevy_camera_quat([0.0, 90.0, 0.0]);
        let (axis, angle) = quat.to_axis_angle();
        assert!((angle - std::f32::consts::FRAC_PI_2).abs() < 1e-4);
        assert!(close(axis.abs(), Vec3::Y), "axis {axis:?}");
    }

    #[test]
    fn a_camera_on_the_view_tag_has_no_lens_offset() {
        let tag = Mat4::from_translation(Vec3::new(1.0, 2.0, 3.0));
        let lens = tag_camera_lens_local(tag, tag);
        assert!(lens.abs_diff_eq(Mat4::IDENTITY, 1e-5));
    }

    #[test]
    fn a_camera_ahead_of_the_view_tag_sits_ahead_in_camera_space() {
        let ahead = Mat4::from_translation(Vec3::X * 4.0);
        let lens = tag_camera_lens_local(Mat4::IDENTITY, ahead);
        assert!(close(lens.w_axis.truncate(), Vec3::NEG_Z * 4.0));
    }

    #[test]
    fn bolt_tags_default_to_nothing_attached() {
        let tags = AttachTagNames::default();
        assert!(tags.flash_tag.is_none() && tags.hud.is_none());
        assert_eq!(tags.tracker_screen, [None; 3]);
        assert!(GunTagPoseFrame::default().joints.is_empty());
    }
}
