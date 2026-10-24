//! Space conversions and plain data shared by the first-person pose code.
//!
//! The tag space a viewmodel skeleton is authored in has x forward, y left and
//! z up. The renderer's camera looks down -Z with +X to the right and +Y up.

use bevy::math::{Mat3, Mat4, Quat, Vec3};
use xmodel_runtime::AnimClip;

pub use crate::anim::xmodel_pose::{FpvSurfOwner, PosedModelSurface};

/// Rotation taking tag-space directions into the renderer's camera space.
pub fn tag_view_to_bevy_mat3() -> Mat3 {
    let forward_to = Vec3::NEG_Z;
    let left_to = Vec3::NEG_X;
    let up_to = Vec3::Y;
    Mat3::from_cols(forward_to, left_to, up_to)
}

/// The same rotation as a 4x4 with no translation.
pub fn tag_view_to_bevy_camera() -> Mat4 {
    Mat4::from_mat3(tag_view_to_bevy_mat3())
}

/// Turns the Euler angles (degrees) of a placement into the camera-space
/// rotation it describes.
pub fn placement_angles_to_bevy_camera_quat(angles_deg: [f32; 3]) -> Quat {
    let [forward, left, up] = game_math::angles_to_axis(angles_deg);
    let placed = Mat3::from_cols(
        Vec3::from_array(forward),
        Vec3::from_array(left),
        Vec3::from_array(up),
    );
    let fold = tag_view_to_bevy_mat3();
    Quat::from_mat3(&(fold * placed * fold.inverse()))
}

/// Where the camera tag sits relative to the view tag, in camera space.
#[inline]
pub fn tag_camera_lens_local(view_world: Mat4, camera_world: Mat4) -> Mat4 {
    let fold = tag_view_to_bevy_camera();
    let eye_from_world = fold * view_world.inverse();
    eye_from_world * camera_world * fold.inverse()
}

/// Bone indices of the attachment points a weapon fires from or shows things
/// on. All optional: a gun carries the ones its model has.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FpvBoltTags {
    pub flash: Option<u16>,
    pub flash_silenced: Option<u16>,
    pub brass: Option<u16>,
    pub knife: Option<u16>,
    pub laser: Option<u16>,
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
}

/// One hand's bones in eye space for a frame, with the tags that index them.
#[derive(Clone, Debug, Default)]
pub struct FpvBoltFrame {
    pub bones: Vec<Mat4>,
    pub tags: FpvBoltTags,
}

/// One clip a hand is playing this frame. `node` is the scheduler slot it is
/// playing in, which is what the rig binds its bone tracks against: a clip
/// that stayed in its node keeps the binding it was given.
#[derive(Clone, Copy)]
pub struct PosedClip<'a> {
    pub node: usize,
    pub clip: &'a AnimClip,
    pub time: f32,
    pub weight: f32,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: Vec3, b: Vec3) -> bool {
        (a - b).length() < 1e-5
    }

    #[test]
    fn tag_directions_land_on_camera_axes() {
        let fold = tag_view_to_bevy_mat3();
        assert!(close(fold * Vec3::X, Vec3::NEG_Z), "forward looks down -Z");
        assert!(close(fold * Vec3::Y, Vec3::NEG_X), "left is -X");
        assert!(close(fold * Vec3::Z, Vec3::Y), "up is +Y");
    }

    #[test]
    fn the_4x4_fold_matches_the_3x3_and_moves_nothing() {
        let fold = tag_view_to_bevy_camera();
        let point = Vec3::new(3.0, -2.0, 7.0);
        assert!(close(
            fold.transform_point3(point),
            tag_view_to_bevy_mat3() * point
        ));
        assert_eq!(fold.w_axis.truncate(), Vec3::ZERO);
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
        let view = Mat4::from_translation(Vec3::new(1.0, 2.0, 3.0));
        let lens = tag_camera_lens_local(view, view);
        assert!(lens.abs_diff_eq(Mat4::IDENTITY, 1e-5));
    }

    #[test]
    fn a_camera_ahead_of_the_view_tag_sits_ahead_in_camera_space() {
        let lens = tag_camera_lens_local(
            Mat4::IDENTITY,
            Mat4::from_translation(Vec3::new(4.0, 0.0, 0.0)),
        );
        assert!(close(lens.w_axis.truncate(), Vec3::new(0.0, 0.0, -4.0)));
    }

    #[test]
    fn bolt_tags_default_to_nothing_attached() {
        let tags = FpvBoltTags::default();
        assert!(tags.flash.is_none() && tags.hud.is_none());
        assert_eq!(tags.tracker_screen, [None; 3]);
        assert!(FpvBoltFrame::default().bones.is_empty());
    }
}
