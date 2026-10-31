//! The camera frustum as the sun sees it.

use bevy::math::Vec3;

use super::ARC_CLASSIFICATION_EPSILON;

/// The four corner rays of the camera frustum seen from the sun, and how they
/// turn around it.
#[derive(Clone, Copy, Debug)]
pub struct SunShadowFrustumRays {
    pub world_rays: [[f32; 3]; 4],

    /// Each world ray projected onto the sun basis.
    pub shadow_rays: [[f32; 2]; 4],

    /// Cross product of each projected ray with the next one round.
    pub sines: [f32; 4],
    pub sin_min: f32,
    pub sin_max: f32,

    /// The box round the projected rays and the eye.
    pub lo_corner: [f32; 2],
    pub hi_corner: [f32; 2],

    /// The rays where the swept arc starts and ends, or `None` when the frustum
    /// does not straddle the sun at all.
    pub bounding_arc: Option<(usize, usize)>,
}

fn cross(left: [f32; 2], right_v: [f32; 2]) -> f32 {
    left[0] * right_v[1] - left[1] * right_v[0]
}

/// The last edge whose sine, times `sign`, turns clearly positive after one
/// that was not.
fn last_turn(sines: &[f32; 4], sign: f32) -> Option<usize> {
    (0..4usize).rev().find(|&edge| {
        let here = sign * sines[edge];
        let before = sign * sines[(edge + 3) & 3];
        here > ARC_CLASSIFICATION_EPSILON && before <= ARC_CLASSIFICATION_EPSILON
    })
}

/// Projects the camera's corner rays into the sun basis and classifies the arc
/// they sweep.
///
/// `packed_axes` is `[axis0, axis1, light_direction]`.
#[must_use]
pub fn sun_shadow_frustum_rays(
    world_rays: [[f32; 3]; 4],
    packed_axes: [[f32; 3]; 3],
) -> SunShadowFrustumRays {
    let axis0 = Vec3::from_array(packed_axes[0]);
    let axis1 = Vec3::from_array(packed_axes[1]);
    let shadow_rays = world_rays.map(|ray| {
        let ray = Vec3::from_array(ray);
        [ray.dot(axis0), ray.dot(axis1)]
    });

    // The eye sits at the origin of the projection, so the box covers it too.
    let (mins, maxs) = shadow_rays
        .iter()
        .fold(([0.0f32; 2], [0.0f32; 2]), |(low, high), ray| {
            (
                [low[0].min(ray[0]), low[1].min(ray[1])],
                [high[0].max(ray[0]), high[1].max(ray[1])],
            )
        });

    let sines: [f32; 4] =
        std::array::from_fn(|edge| cross(shadow_rays[edge], shadow_rays[(edge + 1) & 3]));
    let sin_min = sines.iter().copied().fold(f32::INFINITY, f32::min);
    let sin_max = sines.iter().copied().fold(f32::NEG_INFINITY, f32::max);

    // The outline only sweeps an arc when it turns both ways round the sun.
    let turns_both_ways = (-sin_min).min(sin_max) >= ARC_CLASSIFICATION_EPSILON;
    let bounding_arc = if turns_both_ways {
        last_turn(&sines, 1.0).zip(last_turn(&sines, -1.0))
    } else {
        None
    };

    SunShadowFrustumRays {
        world_rays,
        shadow_rays,
        sines,
        sin_min,
        sin_max,
        lo_corner: mins,
        hi_corner: maxs,
        bounding_arc,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const IDENTITY_AXES: [[f32; 3]; 3] = [Vec3::X.to_array(), Vec3::Y.to_array(), Vec3::Z.to_array()];

    /// Rays whose projection on the identity axes is `outline`.
    fn rays_over(outline: [[f32; 2]; 4]) -> SunShadowFrustumRays {
        sun_shadow_frustum_rays(outline.map(|[x, y]| [x, y, 5.0]), IDENTITY_AXES)
    }

    #[test]
    fn rays_are_projected_on_the_two_sun_axes() {
        let rays = sun_shadow_frustum_rays(
            [[1.0, 2.0, 3.0], [4.0, 5.0, 6.0], [0.0; 3], [0.0; 3]],
            [[0.0, 1.0, 0.0], [0.0, 0.0, 1.0], [1.0, 0.0, 0.0]],
        );
        assert_eq!(rays.shadow_rays[0], [2.0, 3.0]);
        assert_eq!(rays.shadow_rays[1], [5.0, 6.0]);
        assert_eq!(rays.world_rays[1], [4.0, 5.0, 6.0]);
    }

    #[test]
    fn the_box_always_holds_the_eye() {
        let rays = rays_over([[2.0, 3.0], [3.0, 3.0], [3.0, 4.0], [2.0, 4.0]]);
        assert_eq!(rays.lo_corner, [0.0, 0.0]);
        assert_eq!(rays.hi_corner, [3.0, 4.0]);

        let rays = rays_over([[-2.0, -3.0], [-1.0, -3.0], [-1.0, -1.0], [-2.0, -1.0]]);
        assert_eq!(rays.lo_corner, [-2.0, -3.0]);
        assert_eq!(rays.hi_corner, [0.0, 0.0]);
    }

    #[test]
    fn an_outline_round_the_sun_sweeps_no_arc() {
        let rays = rays_over([[1.0, 1.0], [-1.0, 1.0], [-1.0, -1.0], [1.0, -1.0]]);
        assert_eq!(rays.sines, [2.0; 4]);
        assert_eq!((rays.sin_min, rays.sin_max), (2.0, 2.0));
        assert_eq!(rays.bounding_arc, None);
    }

    #[test]
    fn an_outline_the_other_way_round_sweeps_no_arc_either() {
        let rays = rays_over([[1.0, 1.0], [1.0, -1.0], [-1.0, -1.0], [-1.0, 1.0]]);
        assert_eq!(rays.sines, [-2.0; 4]);
        assert_eq!(rays.bounding_arc, None);
    }

    #[test]
    fn an_outline_beside_the_sun_sweeps_the_arc_between_its_edges() {
        let rays = rays_over([[1.0, -1.0], [1.0, 1.0], [2.0, 1.0], [2.0, -1.0]]);
        assert_eq!(rays.sines, [2.0, -1.0, -4.0, -1.0]);
        assert_eq!((rays.sin_min, rays.sin_max), (-4.0, 2.0));
        assert_eq!(rays.bounding_arc, Some((0, 1)));
    }

    #[test]
    fn the_arc_follows_the_winding_not_the_numbering() {
        // The same outline starting from another corner.
        let rays = rays_over([[2.0, 1.0], [2.0, -1.0], [1.0, -1.0], [1.0, 1.0]]);
        assert_eq!(rays.bounding_arc, Some((2, 3)));
    }

    #[test]
    fn a_flat_outline_is_not_an_arc() {
        let rays = rays_over([[1.0, 0.0], [2.0, 0.0], [3.0, 0.0], [4.0, 0.0]]);
        assert_eq!(rays.sines, [0.0; 4]);
        assert_eq!(rays.bounding_arc, None);
    }

    #[test]
    fn a_turn_must_clear_the_epsilon() {
        // Barely positive and barely negative sines are neither.
        let rays = rays_over([[1.0, 0.0], [1.0, 0.0005], [1.0, 0.0], [1.0, -0.0005]]);
        assert!(rays.sin_max < ARC_CLASSIFICATION_EPSILON && rays.sin_min > -ARC_CLASSIFICATION_EPSILON);
        assert_eq!(rays.bounding_arc, None);
    }
}
