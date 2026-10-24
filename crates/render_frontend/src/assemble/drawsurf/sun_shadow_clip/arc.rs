//! The planes that bound the arc the camera frustum sweeps around the sun.

use bevy::math::Vec3;
use render_frame::SunShadowClipPlanes;

use super::plane::{push_finite, unit2};
use super::rays::SunShadowFrustumRays;
use super::{ARC_CLASSIFICATION_EPSILON, FAR_PARTITION_SAMPLE_SCALE, SunShadowClipInput};

/// The two sides of the arc, one plane each in both cascades.
///
/// A frustum that sweeps no arc adds nothing, unless its outline winds the
/// other way round the sun: then both cascades are cut off by depth along the
/// camera's forward axis instead, the near one at the near clip plane and the
/// far one where the near cascade stops resolving.
pub(super) fn add(
    sets: &mut [SunShadowClipPlanes; 2],
    input: &SunShadowClipInput,
    rays: &SunShadowFrustumRays,
) {
    let Some((first, last)) = rays.bounding_arc else {
        if rays.sin_max < -ARC_CLASSIFICATION_EPSILON {
            let forward = Vec3::from_array(input.camera_forward);
            let base = -forward.dot(Vec3::from_array(input.camera_origin));
            push_finite(&mut sets[0], forward, base - input.camera_near_distance);
            push_finite(&mut sets[1], forward, base - input.near_shadow_min_distance);
        }
        return;
    };
    let (Some(start), Some(end)) = (
        unit2(rays.shadow_rays[first]),
        unit2(rays.shadow_rays[last]),
    ) else {
        return;
    };
    add_side(sets, input, [-start.y, start.x]);
    add_side(sets, input, [end.y, -end.x]);
}

/// One side of the arc, pushed outwards by half a texel (four times that for
/// the far cascade) so a caster straddling the boundary still reaches the
/// cascade it shadows. `normal` is in the sun basis and points inwards.
fn add_side(sets: &mut [SunShadowClipPlanes; 2], input: &SunShadowClipInput, normal: [f32; 2]) {
    let world_normal = input.axis(0) * normal[0] + input.axis(1) * normal[1];
    let base = -world_normal.dot(Vec3::from_array(input.camera_origin));
    let padding = 0.5 * input.sample_size[0] * (normal[0].abs() + normal[1].abs());
    push_finite(&mut sets[0], world_normal, base + padding);
    push_finite(
        &mut sets[1],
        world_normal,
        base + FAR_PARTITION_SAMPLE_SCALE * padding,
    );
}

#[cfg(test)]
mod tests {
    use super::super::tests::{centred_input, rays_over};
    use super::*;

    fn empty() -> [SunShadowClipPlanes; 2] {
        [SunShadowClipPlanes::EMPTY; 2]
    }

    #[test]
    fn a_frustum_round_the_sun_adds_nothing() {
        let rays = rays_over([[1.0, 1.0], [-1.0, 1.0], [-1.0, -1.0], [1.0, -1.0]]);
        let mut sets = empty();
        add(&mut sets, &centred_input(), &rays);
        assert!(sets[0].is_empty() && sets[1].is_empty());
    }

    #[test]
    fn a_frustum_the_other_way_round_is_cut_by_depth() {
        let rays = rays_over([[1.0, 1.0], [1.0, -1.0], [-1.0, -1.0], [-1.0, 1.0]]);
        let mut input = centred_input();
        input.camera_origin = [10.0, 0.0, 0.0];
        input.camera_near_distance = 0.5;
        input.near_shadow_min_distance = 20.0;
        let mut sets = empty();
        add(&mut sets, &input, &rays);
        assert_eq!(sets[0].as_slice(), [[1.0, 0.0, 0.0, -10.5]]);
        assert_eq!(sets[1].as_slice(), [[1.0, 0.0, 0.0, -30.0]]);
    }

    #[test]
    fn the_two_sides_of_an_arc_make_a_wedge_in_both_cascades() {
        let rays = rays_over([[1.0, -1.0], [1.0, 1.0], [2.0, 1.0], [2.0, -1.0]]);
        let input = centred_input();
        let mut sets = empty();
        add(&mut sets, &input, &rays);
        for set in sets {
            assert_eq!(set.len(), 2);
        }

        let inside = |set: &SunShadowClipPlanes, point: Vec3| {
            set.as_slice()
                .iter()
                .all(|&bound| super::super::plane::signed_distance(bound, point) >= 0.0)
        };
        for set in &sets {
            assert!(inside(set, Vec3::new(10.0, 0.0, 0.0)));
            // The sun shines along z, so height does not matter.
            assert!(inside(set, Vec3::new(10.0, 5.0, 900.0)));
            assert!(!inside(set, Vec3::new(0.0, 10.0, 0.0)));
            assert!(!inside(set, Vec3::new(0.0, -10.0, 0.0)));
            assert!(!inside(set, Vec3::new(-10.0, 0.0, 0.0)));
        }
    }

    #[test]
    fn the_far_cascade_is_padded_four_times_as_far() {
        let rays = rays_over([[1.0, 0.0], [1.0, 1.0], [2.0, 1.0], [2.0, 0.0]]);
        let input = centred_input();
        let mut sets = empty();
        add(&mut sets, &input, &rays);
        let near = sets[0].as_slice();
        let far = sets[1].as_slice();
        assert_eq!(near.len(), far.len());
        for (near, far) in near.iter().zip(far) {
            assert_eq!(near[..3], far[..3]);
            // The camera is at the origin, so the offset is all padding.
            assert!(near[3] > 0.0);
            assert!((far[3] - 4.0 * near[3]).abs() < 1e-6, "{near:?} {far:?}");
        }
    }

    #[test]
    fn a_ray_without_a_direction_adds_no_side() {
        let mut rays = rays_over([[1.0, -1.0], [1.0, 1.0], [2.0, 1.0], [2.0, -1.0]]);
        rays.shadow_rays[1] = [0.0, 0.0];
        let mut sets = empty();
        add(&mut sets, &centred_input(), &rays);
        assert!(sets[0].is_empty() && sets[1].is_empty());
    }
}
