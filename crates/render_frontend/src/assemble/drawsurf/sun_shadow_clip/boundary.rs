//! The four sides of a cascade's own rectangle.

use render_frame::SunShadowClipPlanes;

use super::plane::{Plane, clip_segment, normal_of, plane};
use super::rays::SunShadowFrustumRays;
use super::{ARC_CLASSIFICATION_EPSILON, BOUNDARY_LENGTH_FRACTION, SunShadowClipInput};

/// The sides of the cascade's rectangle as planes holding inside it: two
/// opposite pairs, the pair across the axis where the frustum's footprint is
/// narrower first, and in each pair the side the eye is nearer to first.
fn sides(input: &SunShadowClipInput, rays: &SunShadowFrustumRays, partition: usize) -> [Plane; 4] {
    let sample = input.sample_size[partition];
    let size = input.useful_size as f32 * sample;
    let snapped = input.snapped_shadow_origin[partition];
    let axis1_narrower = rays.maxs[1] - rays.mins[1] < rays.maxs[0] - rays.mins[0];

    let mut sides = [[0.0; 4]; 4];
    for pair in 0..2 {
        let axis = usize::from(axis1_narrower) ^ pair;
        let normal = input.axis(axis);
        let from_min = input.shadow_origin_pixel_center[axis] * sample - snapped[axis];
        let low_side = plane(normal, from_min);
        let high_side = plane(-normal, size - from_min);
        let ordered = if from_min >= size * 0.5 {
            [high_side, low_side]
        } else {
            [low_side, high_side]
        };
        sides[pair * 2..pair * 2 + 2].copy_from_slice(&ordered);
    }
    sides
}

/// Adds the sides of the cascade's rectangle that are worth a plane slot.
///
/// A set that holds a plane running along the sun's direction already bounds
/// the volume sideways, so a side whose edge is cut away by it, or left a
/// sliver, adds nothing and is skipped. Without one, every side is added, the
/// pair with the smaller offset first.
pub(super) fn add(
    set: &mut SunShadowClipPlanes,
    input: &SunShadowClipInput,
    rays: &SunShadowFrustumRays,
    partition: usize,
) {
    let sides = sides(input, rays, partition);
    let light = input.axis(2);
    let first_along_the_sun = set
        .as_slice()
        .iter()
        .position(|&bound| light.dot(normal_of(bound)).abs() < ARC_CLASSIFICATION_EPSILON);
    match first_along_the_sun {
        None => add_all(set, &sides),
        Some(first) => add_those_that_cut(set, input, partition, &sides, first),
    }
}

fn add_all(set: &mut SunShadowClipPlanes, sides: &[Plane; 4]) {
    let first_pair = if sides[0][3] >= sides[2][3] { 2 } else { 0 };
    for pair in [first_pair, first_pair ^ 2] {
        set.push(sides[pair]);
        set.push(sides[pair + 1]);
    }
}

/// Each side's edge is a segment on the rectangle, cut by the planes from
/// `first` on as the set stood before any side was added.
fn add_those_that_cut(
    set: &mut SunShadowClipPlanes,
    input: &SunShadowClipInput,
    partition: usize,
    sides: &[Plane; 4],
    first: usize,
) {
    let sample = input.sample_size[partition];
    let half = input.useful_size as f32 * sample * 0.5;
    let snapped = input.snapped_shadow_origin[partition];
    let [pixel_x, pixel_y] = input.shadow_origin_pixel_center;
    let middle = input.axis(0) * (half - sample * pixel_x + snapped[0])
        + input.axis(1) * (half - sample * pixel_y + snapped[1]);
    let shortest = half * BOUNDARY_LENGTH_FRACTION;

    let before = *set;
    let cutting = &before.as_slice()[first..];
    for (index, &side) in sides.iter().enumerate() {
        // The other pair's normal runs along this side.
        let along = normal_of(sides[index ^ 2]) * half;
        let centre = middle - normal_of(side) * half;
        let Some((start, end)) = clip_segment(centre - along, centre + along, cutting) else {
            continue;
        };
        if start.distance_squared(end) >= shortest * shortest {
            set.push(side);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::tests::{centred_input, rays_over};
    use super::*;

    const ROUND_THE_SUN: [[f32; 2]; 4] = [[1.0, 1.0], [-1.0, 1.0], [-1.0, -1.0], [1.0, -1.0]];

    fn set_of(planes: &[Plane]) -> SunShadowClipPlanes {
        let mut set = SunShadowClipPlanes::EMPTY;
        for &bound in planes {
            assert!(set.push(bound));
        }
        set
    }

    #[test]
    fn a_centred_cascade_has_four_equal_sides() {
        let sides = sides(&centred_input(), &rays_over(ROUND_THE_SUN), 0);
        assert_eq!(
            sides,
            [
                [-1.0, 0.0, 0.0, 32.0],
                [1.0, 0.0, 0.0, 32.0],
                [0.0, -1.0, 0.0, 32.0],
                [0.0, 1.0, 0.0, 32.0],
            ]
        );
        let far = super::sides(&centred_input(), &rays_over(ROUND_THE_SUN), 1);
        assert_eq!(far[0], [-1.0, 0.0, 0.0, 128.0]);
        assert_eq!(far[3], [0.0, 1.0, 0.0, 128.0]);
    }

    #[test]
    fn the_axis_where_the_footprint_is_narrower_comes_first() {
        // Three wide in x, two high in y.
        let rays = rays_over([[-3.0, 1.0], [3.0, 1.0], [3.0, 2.0], [-3.0, 2.0]]);
        let sides = sides(&centred_input(), &rays, 0);
        assert_eq!(sides[0], [0.0, -1.0, 0.0, 32.0]);
        assert_eq!(sides[2], [-1.0, 0.0, 0.0, 32.0]);
    }

    #[test]
    fn each_pair_starts_with_the_side_the_eye_is_nearer_to() {
        let mut input = centred_input();
        // The rectangle sits 10 further along +x, so the eye is 10 nearer its
        // low side.
        input.snapped_shadow_origin[0] = [-10.0, 10.0];
        let sides = sides(&input, &rays_over(ROUND_THE_SUN), 0);
        assert_eq!(sides[0], [-1.0, 0.0, 0.0, 22.0]);
        assert_eq!(sides[1], [1.0, 0.0, 0.0, 42.0]);
        assert_eq!(sides[2], [0.0, 1.0, 0.0, 22.0]);
        assert_eq!(sides[3], [0.0, -1.0, 0.0, 42.0]);
    }

    #[test]
    fn with_nothing_along_the_sun_every_side_is_added() {
        let mut set = SunShadowClipPlanes::EMPTY;
        add(&mut set, &centred_input(), &rays_over(ROUND_THE_SUN), 0);
        // Equal offsets: the second pair goes first.
        assert_eq!(
            set.as_slice(),
            [
                [0.0, -1.0, 0.0, 32.0],
                [0.0, 1.0, 0.0, 32.0],
                [-1.0, 0.0, 0.0, 32.0],
                [1.0, 0.0, 0.0, 32.0],
            ]
        );
    }

    #[test]
    fn the_pair_with_the_smaller_offset_goes_first() {
        let mut input = centred_input();
        input.snapped_shadow_origin[0] = [0.0, 10.0];
        let mut set = SunShadowClipPlanes::EMPTY;
        add(&mut set, &input, &rays_over(ROUND_THE_SUN), 0);
        assert_eq!(
            set.as_slice(),
            [
                [0.0, 1.0, 0.0, 22.0],
                [0.0, -1.0, 0.0, 42.0],
                [-1.0, 0.0, 0.0, 32.0],
                [1.0, 0.0, 0.0, 32.0],
            ]
        );
    }

    #[test]
    fn a_plane_not_along_the_sun_does_not_count() {
        // Its normal has a part along the sun (z).
        let mut set = set_of(&[[0.0, 0.6, 0.8, 0.0]]);
        add(&mut set, &centred_input(), &rays_over(ROUND_THE_SUN), 0);
        assert_eq!(set.len(), 5);
    }

    #[test]
    fn sides_behind_a_plane_along_the_sun_are_dropped() {
        // Only y >= 0 is left.
        let mut set = set_of(&[[0.0, 1.0, 0.0, 0.0]]);
        add(&mut set, &centred_input(), &rays_over(ROUND_THE_SUN), 0);
        assert_eq!(
            set.as_slice(),
            [
                [0.0, 1.0, 0.0, 0.0],
                [-1.0, 0.0, 0.0, 32.0],
                [1.0, 0.0, 0.0, 32.0],
                [0.0, -1.0, 0.0, 32.0],
            ],
            "the side at y = -32 is entirely behind y >= 0"
        );
    }

    #[test]
    fn a_sliver_of_a_side_is_not_worth_a_plane() {
        // y >= 31.9 leaves 0.1 of each x side, under 2% of the half extent.
        let mut set = set_of(&[[0.0, 1.0, 0.0, -31.9]]);
        add(&mut set, &centred_input(), &rays_over(ROUND_THE_SUN), 0);
        assert_eq!(
            set.as_slice(),
            [[0.0, 1.0, 0.0, -31.9], [0.0, -1.0, 0.0, 32.0]]
        );
    }

    #[test]
    fn sides_are_judged_by_the_planes_from_the_first_along_the_sun() {
        // The first plane would remove everything, but only the ones from the
        // second on take part.
        let mut set = set_of(&[[0.0, 0.0, 1.0, -1.0e6], [0.0, 1.0, 0.0, 0.0]]);
        add(&mut set, &centred_input(), &rays_over(ROUND_THE_SUN), 0);
        assert_eq!(set.len(), 2 + 3);
    }
}
