//! Deciding whether a light can reach a box. A light reaches a box if the box
//! touches its sphere (and, for a spot, its cone) and is not wholly outside
//! the volumes the level marked as the light's region.
//!
//! Every test here answers "is this box out of reach?": `true` means culled.

use core::borrow::Borrow;

use crate::scene_light::gfx_light_is_spot;

/// What the culling needs to know about a primary light.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PrimaryLightCull {
    pub light_type: u8,
    pub origin: [f32; 3],
    /// The axis of a spot's cone. The cone opens away from this direction.
    pub direction: [f32; 3],
    pub radius: f32,
    /// Cosine of the cone's half angle, widened a little. Negative or NaN
    /// means the light has no usable cone.
    pub cos_half_fov_expanded: f32,
}

/// One extra bounding slab of a light region: a unit direction and the range
/// along it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LightRegionAxis {
    pub dir: [f32; 3],
    pub mid: f32,
    pub half: f32,
}

/// A convex piece of a light's region. The first nine slabs are fixed: x, y,
/// z, then x+y, x-y, x+z, x-z, y+z, y-z. `axes` adds more.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LightRegionHull<'a> {
    pub kdop_mid: [f32; 9],
    pub kdop_half: [f32; 9],
    pub axes: &'a [LightRegionAxis],
}

/// All the hulls of one light's region.
pub type LightRegionHulls<'a> = &'a [LightRegionHull<'a>];

/// The half sizes of the box used to light a model of the given radius.
pub fn lighting_query_box_half(dobj_bounds_radius: f32) -> [f32; 3] {
    [dobj_bounds_radius; 3]
}

/// Distance from the sphere's centre to the box, squared, against the radius
/// squared.
fn cull_box_from_sphere(
    center: [f32; 3],
    radius: f32,
    box_mid: [f32; 3],
    box_half: [f32; 3],
) -> bool {
    let mut distance_squared = 0.0_f32;
    for axis in 0..3 {
        let outside = ((center[axis] - box_mid[axis]).abs() - box_half[axis]).max(0.0);
        distance_squared += outside * outside;
    }
    distance_squared > radius * radius
}

fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

/// Whether the box lies wholly outside a cone with its apex at `apex`.
///
/// `axis` is the cone's reference direction and the cone opens against it: a
/// box on the `axis` side of the apex is out. Past that, the cone is a
/// separating-axis problem: the box is out when some plane through the apex
/// and tangent to the cone has the box wholly on its outer side.
pub(crate) fn cull_box_from_cone(
    apex: [f32; 3],
    axis: [f32; 3],
    cos_half_angle: f32,
    box_mid: [f32; 3],
    box_half: [f32; 3],
) -> bool {
    let delta = [
        box_mid[0] - apex[0],
        box_mid[1] - apex[1],
        box_mid[2] - apex[2],
    ];
    let along = dot(delta, axis);
    // How far the box reaches along the axis from its middle.
    let reach =
        axis[0].abs() * box_half[0] + axis[1].abs() * box_half[1] + axis[2].abs() * box_half[2];
    if reach <= along {
        return true;
    }
    let sideways = [
        delta[0] - along * axis[0],
        delta[1] - along * axis[1],
        delta[2] - along * axis[2],
    ];
    let sideways_squared = dot(sideways, sideways);
    let cos_squared = cos_half_angle * cos_half_angle;
    let sin_squared = 1.0 - cos_squared;
    // The middle of the box is inside the cone: nothing more to check.
    if sideways_squared * cos_squared <= sin_squared * along * along {
        return false;
    }
    let sideways_scaled = libm::sqrtf(sideways_squared * sin_squared);
    if sideways_scaled == 0.0 {
        return true;
    }
    // Normal of the tangent plane nearest the box.
    let k = cos_half_angle / sideways_scaled;
    let normal = [
        axis[0] + k * sideways[0],
        axis[1] + k * sideways[1],
        axis[2] + k * sideways[2],
    ];
    let separation = dot(normal, delta)
        - (normal[0] * box_half[0]).abs()
        - (normal[1] * box_half[1]).abs()
        - (normal[2] * box_half[2]).abs();
    !(separation < 0.0)
}

/// Whether a point, grown by `slack`, lies wholly outside the cone at `apex`.
///
/// Same convention as the box test: the cone opens against `axis`, and a point
/// on the `axis` side (less `slack`) is out. Growing the point makes the cone
/// effectively wider, which keeps things that merely touch it.
pub(crate) fn cull_point_from_cone_expanded(
    apex: [f32; 3],
    axis: [f32; 3],
    cos_half_angle: f32,
    point: [f32; 3],
    slack: f32,
) -> bool {
    let delta = [point[0] - apex[0], point[1] - apex[1], point[2] - apex[2]];
    let along = dot(delta, axis);
    if slack <= along {
        return true;
    }
    let sideways = [
        delta[0] - along * axis[0],
        delta[1] - along * axis[1],
        delta[2] - along * axis[2],
    ];
    let sideways_squared = dot(sideways, sideways);
    let gap = libm::sqrtf(sideways_squared) * along - slack;
    sideways_squared * cos_half_angle * cos_half_angle >= gap * gap
}

/// Whether a primary light cannot reach the box. Spots are tested against
/// their cone as well as their sphere.
pub fn cull_box_from_primary_light(
    light: &PrimaryLightCull,
    box_mid: [f32; 3],
    box_half: [f32; 3],
) -> bool {
    let cos = light.cos_half_fov_expanded;
    let has_cone = gfx_light_is_spot(light.light_type) && !cos.is_nan() && cos >= 0.0;
    cull_box_from_sphere(light.origin, light.radius, box_mid, box_half)
        || (has_cone && cull_box_from_cone(light.origin, light.direction, cos, box_mid, box_half))
}

/// Whether the box and a slab (`mid` +- `half`) cannot overlap, given the
/// box's own middle and half size along the slab's direction.
fn slab_misses(box_mid: f32, box_half: f32, slab_mid: f32, slab_half: f32) -> bool {
    (box_mid - slab_mid).abs() >= box_half + slab_half
}

/// Whether the box (in the light's local space) misses a region hull.
fn cull_box_from_light_region_hull(
    hull: &LightRegionHull<'_>,
    box_mid: [f32; 3],
    box_half: [f32; 3],
) -> bool {
    let [mx, my, mz] = box_mid;
    let [hx, hy, hz] = box_half;
    let (kmid, khalf) = (&hull.kdop_mid, &hull.kdop_half);
    let fixed_slabs = [
        (mx, hx),
        (my, hy),
        (mz, hz),
        (mx + my, hx + hy),
        (mx - my, hx + hy),
        (mx + mz, hx + hz),
        (mx - mz, hx + hz),
        (my + mz, hy + hz),
        (my - mz, hy + hz),
    ];
    for (slot, &(mid, half)) in fixed_slabs.iter().enumerate() {
        if slab_misses(mid, half, kmid[slot], khalf[slot]) {
            return true;
        }
    }
    hull.axes.iter().any(|axis| {
        let reach = box_half[0] * axis.dir[0].abs()
            + box_half[1] * axis.dir[1].abs()
            + box_half[2] * axis.dir[2].abs();
        slab_misses(dot(box_mid, axis.dir), reach, axis.mid, axis.half)
    })
}

/// Whether a light's region excludes the box. The region is the union of its
/// hulls, positioned relative to the light's origin; a light with no hulls
/// excludes nothing.
pub fn light_region_culls_box<'a, H: Borrow<LightRegionHull<'a>>>(
    hulls: impl IntoIterator<Item = H>,
    light_origin: [f32; 3],
    box_mid: [f32; 3],
    box_half: [f32; 3],
) -> bool {
    let local = [
        box_mid[0] - light_origin[0],
        box_mid[1] - light_origin[1],
        box_mid[2] - light_origin[2],
    ];
    let mut any_hull = false;
    for hull in hulls {
        any_hull = true;
        if !cull_box_from_light_region_hull(hull.borrow(), local, box_half) {
            return false;
        }
    }
    any_hull
}

/// The light a box is lit by, found by walking the lights after the sun.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct NonSunPrimaryWalkTrace {
    /// Index of the first light that reaches the box; 0 when none does.
    pub walk: u32,
}

/// Walks the lights after `sun_primary_index` and picks the first that
/// neither its volume nor its region rules out. Without region data nothing
/// is picked.
pub fn non_sun_primary_light_walk_trace(
    lights: &[PrimaryLightCull],
    sun_primary_index: u32,
    regions: Option<&[LightRegionHulls<'_>]>,
    box_mid: [f32; 3],
    box_half: [f32; 3],
) -> NonSunPrimaryWalkTrace {
    let Some(regions) = regions else {
        return NonSunPrimaryWalkTrace::default();
    };
    let first = sun_primary_index.saturating_add(1) as usize;
    let candidates = lights.len().min(regions.len());
    let walk = (first..candidates)
        .find(|&i| {
            !cull_box_from_primary_light(&lights[i], box_mid, box_half)
                && !light_region_culls_box(regions[i], lights[i].origin, box_mid, box_half)
        })
        .map_or(0, |i| i as u32);
    NonSunPrimaryWalkTrace { walk }
}

/// Starting value for "closest light so far" when linking a dynamic entity.
pub const DYN_ENT_PRIMARY_LIGHT_LINK_DIST2_INIT: f32 = f32::MAX;

/// Squared distance from a light to a box's middle.
pub fn dyn_ent_primary_light_link_dist2(origin: [f32; 3], box_mid: [f32; 3]) -> f32 {
    let delta = [
        box_mid[0] - origin[0],
        box_mid[1] - origin[1],
        box_mid[2] - origin[2],
    ];
    dot(delta, delta)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene_light::{GFX_LIGHT_TYPE_OMNI, GFX_LIGHT_TYPE_SPOT};

    const ONE: [f32; 3] = [1.0; 3];

    fn light(light_type: u8, origin: [f32; 3], radius: f32, cos: f32) -> PrimaryLightCull {
        PrimaryLightCull {
            light_type,
            origin,
            direction: [0.0, 0.0, 1.0],
            radius,
            cos_half_fov_expanded: cos,
        }
    }

    fn hull(half: f32) -> LightRegionHull<'static> {
        LightRegionHull {
            kdop_mid: [0.0; 9],
            kdop_half: [half; 9],
            axes: &[],
        }
    }

    #[test]
    fn the_box_query_half_is_the_radius_on_every_axis() {
        assert_eq!(lighting_query_box_half(12.5), [12.5; 3]);
    }

    #[test]
    fn a_box_beyond_the_sphere_is_culled() {
        assert!(cull_box_from_sphere([0.0; 3], 10.0, [16.0, 0.0, 0.0], ONE));
    }

    #[test]
    fn a_box_inside_the_sphere_is_not_culled() {
        assert!(!cull_box_from_sphere([0.0; 3], 10.0, [10.0, 0.0, 0.0], ONE));
        assert!(!cull_box_from_sphere([0.0; 3], 10.0, [0.0; 3], ONE));
    }

    #[test]
    fn a_box_whose_nearest_point_is_exactly_the_radius_away_is_kept() {
        assert!(!cull_box_from_sphere([0.0; 3], 10.0, [11.0, 0.0, 0.0], ONE));
    }

    #[test]
    fn the_sphere_distance_counts_every_axis() {
        // Nearest point is (8, 8, 0) away: 128 > 100.
        assert!(cull_box_from_sphere([0.0; 3], 10.0, [9.0, 9.0, 0.0], ONE));
        // (6, 6, 0) away: 72 < 100.
        assert!(!cull_box_from_sphere([0.0; 3], 10.0, [7.0, 7.0, 0.0], ONE));
    }

    #[test]
    fn a_box_on_the_axis_side_of_the_apex_is_outside_the_cone() {
        assert!(cull_box_from_cone(
            [0.0; 3],
            [0.0, 0.0, 1.0],
            0.7071,
            [0.0, 0.0, 10.0],
            ONE
        ));
    }

    #[test]
    fn a_box_straight_down_the_cone_is_inside_it() {
        assert!(!cull_box_from_cone(
            [0.0; 3],
            [0.0, 0.0, 1.0],
            0.7071,
            [0.0, 0.0, -10.0],
            ONE
        ));
    }

    #[test]
    fn a_box_far_off_to_the_side_is_outside_the_cone() {
        assert!(cull_box_from_cone(
            [0.0; 3],
            [0.0, 0.0, 1.0],
            0.7071,
            [50.0, 0.0, -10.0],
            ONE
        ));
    }

    #[test]
    fn a_box_grazing_the_cone_edge_is_inside_it() {
        assert!(!cull_box_from_cone(
            [0.0; 3],
            [0.0, 0.0, 1.0],
            0.7071,
            [11.0, 0.0, -10.0],
            ONE
        ));
    }

    #[test]
    fn a_point_on_the_axis_side_is_outside_the_expanded_cone() {
        assert!(cull_point_from_cone_expanded(
            [0.0; 3],
            [0.0, 0.0, 1.0],
            0.7071,
            [0.0, 0.0, 10.0],
            4.0
        ));
    }

    #[test]
    fn a_point_straight_down_the_expanded_cone_is_kept() {
        assert!(!cull_point_from_cone_expanded(
            [0.0; 3],
            [0.0, 0.0, 1.0],
            0.7071,
            [3.0, 0.0, -10.0],
            4.0
        ));
    }

    #[test]
    fn a_point_beside_the_apex_plane_is_outside_the_cone_until_it_is_grown() {
        let point = [3.0, 0.0, -0.5];
        assert!(cull_point_from_cone_expanded(
            [0.0; 3],
            [0.0, 0.0, 1.0],
            0.7071,
            point,
            0.0
        ));
        assert!(!cull_point_from_cone_expanded(
            [0.0; 3],
            [0.0, 0.0, 1.0],
            0.7071,
            point,
            4.0
        ));
    }

    #[test]
    fn slack_pulls_a_point_beside_the_axis_side_back_in() {
        // Just past the apex plane: out with no slack, kept once it is grown.
        let apex = [0.0; 3];
        let axis = [0.0, 0.0, 1.0];
        assert!(cull_point_from_cone_expanded(
            apex,
            axis,
            0.7071,
            [1.0, 0.0, 0.5],
            0.5
        ));
        assert!(!cull_point_from_cone_expanded(
            apex,
            axis,
            0.7071,
            [1.0, 0.0, 0.5],
            40.0
        ));
    }

    #[test]
    fn an_omni_light_ignores_its_cone() {
        let omni = light(GFX_LIGHT_TYPE_OMNI, [0.0; 3], 100.0, 0.5);
        assert!(!cull_box_from_primary_light(&omni, [0.0, 0.0, 50.0], ONE));
    }

    #[test]
    fn a_spot_light_is_cut_by_its_cone() {
        let spot = light(GFX_LIGHT_TYPE_SPOT, [0.0; 3], 100.0, 0.5);
        assert!(cull_box_from_primary_light(&spot, [0.0, 0.0, 50.0], ONE));
        assert!(!cull_box_from_primary_light(&spot, [0.0, 0.0, -50.0], ONE));
    }

    #[test]
    fn a_spot_is_still_cut_by_its_sphere() {
        let spot = light(GFX_LIGHT_TYPE_SPOT, [0.0; 3], 100.0, 0.5);
        assert!(cull_box_from_primary_light(&spot, [0.0, 0.0, -150.0], ONE));
    }

    #[test]
    fn a_spot_with_no_usable_cone_acts_like_an_omni() {
        let negative = light(GFX_LIGHT_TYPE_SPOT, [0.0; 3], 100.0, -1.0);
        let nan = light(GFX_LIGHT_TYPE_SPOT, [0.0; 3], 100.0, f32::NAN);
        assert!(!cull_box_from_primary_light(
            &negative,
            [0.0, 0.0, 50.0],
            ONE
        ));
        assert!(!cull_box_from_primary_light(&nan, [0.0, 0.0, 50.0], ONE));
    }

    #[test]
    fn a_hull_culls_a_box_outside_one_of_its_axes() {
        assert!(cull_box_from_light_region_hull(
            &hull(10.0),
            [15.0, 0.0, 0.0],
            ONE
        ));
        assert!(!cull_box_from_light_region_hull(
            &hull(10.0),
            [5.0, 0.0, 0.0],
            ONE
        ));
    }

    #[test]
    fn a_hull_culls_a_box_that_only_misses_a_diagonal() {
        // Inside every axis slab, but x+y = 12 is as far as 2 + 10.
        assert!(cull_box_from_light_region_hull(
            &hull(10.0),
            [6.0, 6.0, 0.0],
            ONE
        ));
        assert!(!cull_box_from_light_region_hull(
            &hull(10.0),
            [5.0, 5.0, 0.0],
            ONE
        ));
    }

    #[test]
    fn every_diagonal_slab_is_checked() {
        for mid in [
            [6.0, -6.0, 0.0],
            [6.0, 0.0, 6.0],
            [6.0, 0.0, -6.0],
            [0.0, 6.0, 6.0],
            [0.0, 6.0, -6.0],
        ] {
            assert!(
                cull_box_from_light_region_hull(&hull(10.0), mid, ONE),
                "{mid:?}"
            );
        }
    }

    #[test]
    fn extra_axes_can_cull_what_the_fixed_slabs_keep() {
        let axes = [LightRegionAxis {
            dir: [0.0, 0.0, 1.0],
            mid: 0.0,
            half: 3.0,
        }];
        let with_axis = LightRegionHull {
            axes: &axes,
            ..hull(10.0)
        };
        assert!(cull_box_from_light_region_hull(
            &with_axis,
            [0.0, 0.0, 5.0],
            ONE
        ));
        assert!(!cull_box_from_light_region_hull(
            &hull(10.0),
            [0.0, 0.0, 5.0],
            ONE
        ));
    }

    #[test]
    fn a_region_with_no_hulls_culls_nothing() {
        let none: [LightRegionHull; 0] = [];
        assert!(!light_region_culls_box(&none, [0.0; 3], [99.0; 3], ONE));
    }

    #[test]
    fn a_region_culls_only_when_every_hull_does() {
        let hulls = [hull(1.0), hull(100.0)];
        assert!(!light_region_culls_box(
            &hulls,
            [0.0; 3],
            [50.0, 0.0, 0.0],
            ONE
        ));
        let both = [hull(1.0), hull(2.0)];
        assert!(light_region_culls_box(
            &both,
            [0.0; 3],
            [50.0, 0.0, 0.0],
            ONE
        ));
    }

    #[test]
    fn a_region_is_measured_from_the_light() {
        let hulls = [hull(5.0)];
        assert!(light_region_culls_box(
            &hulls,
            [0.0; 3],
            [100.0, 0.0, 0.0],
            ONE
        ));
        assert!(!light_region_culls_box(
            &hulls,
            [100.0, 0.0, 0.0],
            [100.0, 0.0, 0.0],
            ONE
        ));
    }

    #[test]
    fn a_region_takes_hulls_by_value_or_by_reference() {
        let owned = vec![hull(1.0)];
        assert!(light_region_culls_box(
            owned.clone(),
            [0.0; 3],
            [50.0, 0.0, 0.0],
            ONE
        ));
        assert!(light_region_culls_box(
            owned.iter(),
            [0.0; 3],
            [50.0, 0.0, 0.0],
            ONE
        ));
    }

    fn omni(origin: [f32; 3], radius: f32) -> PrimaryLightCull {
        light(GFX_LIGHT_TYPE_OMNI, origin, radius, -1.0)
    }

    fn walk(
        lights: &[PrimaryLightCull],
        sun: u32,
        regions: Option<&[LightRegionHulls<'_>]>,
    ) -> u32 {
        non_sun_primary_light_walk_trace(lights, sun, regions, [0.0; 3], ONE).walk
    }

    #[test]
    fn the_walk_starts_after_the_sun_and_takes_the_first_light_in_reach() {
        let lights = [
            omni([0.0; 3], 100.0),
            omni([500.0, 0.0, 0.0], 10.0),
            omni([0.0; 3], 100.0),
            omni([0.0; 3], 100.0),
        ];
        let regions: [LightRegionHulls; 4] = [&[]; 4];
        assert_eq!(walk(&lights, 0, Some(&regions)), 2);
        assert_eq!(walk(&lights, 2, Some(&regions)), 3);
    }

    #[test]
    fn the_walk_skips_a_light_whose_region_excludes_the_box() {
        let lights = [
            omni([0.0; 3], 100.0),
            omni([0.0; 3], 100.0),
            omni([0.0; 3], 100.0),
        ];
        let tight = [hull(0.5)];
        let box_mid = [50.0, 0.0, 0.0];
        let open: [LightRegionHulls; 3] = [&[], &[], &[]];
        let one_tight: [LightRegionHulls; 3] = [&[], &tight, &[]];
        let pick = |regions: &[LightRegionHulls]| {
            non_sun_primary_light_walk_trace(&lights, 0, Some(regions), box_mid, ONE).walk
        };
        assert_eq!(pick(&open), 1);
        assert_eq!(pick(&one_tight), 2);
    }

    #[test]
    fn the_walk_finds_nothing_when_every_light_is_ruled_out() {
        let lights = [omni([0.0; 3], 100.0), omni([900.0, 0.0, 0.0], 10.0)];
        let regions: [LightRegionHulls; 2] = [&[], &[]];
        assert_eq!(walk(&lights, 0, Some(&regions)), 0);
    }

    #[test]
    fn the_walk_without_region_data_finds_nothing() {
        let lights = [omni([0.0; 3], 100.0), omni([0.0; 3], 100.0)];
        assert_eq!(walk(&lights, 0, None), 0);
    }

    #[test]
    fn the_walk_stops_at_the_shorter_of_lights_and_regions() {
        let lights = [omni([0.0; 3], 100.0), omni([0.0; 3], 100.0)];
        let regions: [LightRegionHulls; 1] = [&[]];
        assert_eq!(walk(&lights, 0, Some(&regions)), 0);
    }

    #[test]
    fn the_link_distance_is_squared() {
        assert_eq!(
            dyn_ent_primary_light_link_dist2([1.0, 2.0, 3.0], [4.0, 6.0, 3.0]),
            25.0
        );
        assert!(
            dyn_ent_primary_light_link_dist2([0.0; 3], [1.0; 3])
                < DYN_ENT_PRIMARY_LIGHT_LINK_DIST2_INIT
        );
    }
}
