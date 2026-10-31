//! Planes as `[facing_x, facing_y, facing_z, offset]`: a point is inside one when
//! `n . p + offset` is not negative.

use bevy::math::{Vec2, Vec3};
use render_frame::SunShadowClipPlanes;

pub(super) fn normal_of(plane: [f32; 4]) -> Vec3 {
    Vec3::new(plane[0], plane[1], plane[2])
}

pub(super) fn signed_distance(plane: [f32; 4], pt: Vec3) -> f32 {
    normal_of(plane).dot(pt) + plane[3]
}

/// The direction of `value`, or `None` when it has no usable length.
pub(super) fn unit2(value: [f32; 2]) -> Option<Vec2> {
    let value = Vec2::from_array(value);
    let length = value.length();
    (length > 0.0 && length.is_finite()).then(|| value / length)
}

/// Adds a plane unless a number in it is not finite. A full set refuses the
/// plane on its own.
pub(super) fn push_finite(set: &mut SunShadowClipPlanes, normal: Vec3, offset: f32) {
    let candidate = normal.extend(offset).to_array();
    if candidate.iter().all(|number| number.is_finite()) {
        set.push(candidate);
    }
}

/// Trims the segment to the inside of every plane in turn; `None` when a plane
/// leaves nothing of it.
pub(super) fn clip_segment(
    mut from_pt: Vec3,
    mut end: Vec3,
    cut_planes: &[[f32; 4]],
) -> Option<(Vec3, Vec3)> {
    for &bound in cut_planes {
        let start_distance = signed_distance(bound, from_pt);
        let end_distance = signed_distance(bound, end);
        match (start_distance <= 0.0, end_distance <= 0.0) {
            (true, true) => return None,
            (true, false) => {
                from_pt =
                    (from_pt * end_distance - end * start_distance) / (end_distance - start_distance);
            }
            (false, true) => {
                end =
                    (end * start_distance - from_pt * end_distance) / (start_distance - end_distance);
            }
            (false, false) => {}
        }
    }
    Some((from_pt, end))
}

#[cfg(test)]
mod tests {
    use super::*;

    const ABOVE_ZERO: [f32; 4] = [0.0, 0.0, 1.0, 0.0];

    #[test]
    fn distance_is_positive_inside() {
        let bound = Vec3::X.extend(-2.0).to_array();
        assert_eq!(signed_distance(bound, Vec3::new(5.0, 7.0, 9.0)), 3.0);
        assert_eq!(signed_distance(bound, Vec3::new(1.0, 0.0, 0.0)), -1.0);
        assert_eq!(normal_of(bound), Vec3::X);
    }

    #[test]
    fn a_direction_needs_a_length() {
        assert_eq!(unit2([3.0, 4.0]), Some(Vec2::new(0.6, 0.8)));
        assert_eq!(unit2([0.0, 0.0]), None);
        assert_eq!(unit2([f32::INFINITY, 0.0]), None);
        assert_eq!(unit2([f32::NAN, 1.0]), None);
    }

    #[test]
    fn only_finite_planes_are_added() {
        let mut set = SunShadowClipPlanes::BLANK;
        push_finite(&mut set, Vec3::X, 1.0);
        push_finite(&mut set, Vec3::new(f32::NAN, 0.0, 0.0), 1.0);
        push_finite(&mut set, Vec3::Y, f32::INFINITY);
        assert_eq!(set.as_slice(), [[1.0, 0.0, 0.0, 1.0]]);
    }

    #[test]
    fn a_segment_wholly_inside_is_left_alone() {
        let from_pt = Vec3::new(0.0, 0.0, 1.0);
        let end = Vec3::new(4.0, 0.0, 3.0);
        assert_eq!(clip_segment(from_pt, end, &[ABOVE_ZERO]), Some((from_pt, end)));
        assert_eq!(clip_segment(from_pt, end, &[]), Some((from_pt, end)));
    }

    #[test]
    fn a_segment_wholly_outside_is_gone() {
        let start = Vec3::new(0.0, 0.0, -1.0);
        let end = Vec3::new(4.0, 0.0, -3.0);
        assert_eq!(clip_segment(start, end, &[ABOVE_ZERO]), None);
        // Touching the plane is not inside it.
        assert_eq!(
            clip_segment(Vec3::ZERO, Vec3::new(1.0, 0.0, 0.0), &[ABOVE_ZERO]),
            None
        );
    }

    #[test]
    fn a_straddling_segment_is_cut_at_the_plane() {
        let (start, end) = clip_segment(
            Vec3::new(0.0, 0.0, -1.0),
            Vec3::new(4.0, 0.0, 3.0),
            &[ABOVE_ZERO],
        )
        .expect("half of it is inside");
        assert!(
            (start - Vec3::new(1.0, 0.0, 0.0)).length() < 1e-6,
            "{start}"
        );
        assert_eq!(end, Vec3::new(4.0, 0.0, 3.0));

        let (start, end) = clip_segment(
            Vec3::new(4.0, 0.0, 3.0),
            Vec3::new(0.0, 0.0, -1.0),
            &[ABOVE_ZERO],
        )
        .expect("half of it is inside");
        assert_eq!(start, Vec3::new(4.0, 0.0, 3.0));
        assert!((end - Vec3::new(1.0, 0.0, 0.0)).length() < 1e-6, "{end}");
    }

    #[test]
    fn each_plane_trims_what_the_last_left() {
        let left_of_two = [-1.0, 0.0, 0.0, 2.0];
        let (start, end) = clip_segment(
            Vec3::new(-4.0, 0.0, 0.0),
            Vec3::new(6.0, 0.0, 0.0),
            &[[1.0, 0.0, 0.0, 0.0], left_of_two],
        )
        .expect("the middle survives");
        assert!((start.x - 0.0).abs() < 1e-6 && (end.x - 2.0).abs() < 1e-6);
    }
}
