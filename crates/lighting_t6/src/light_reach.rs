//! Deciding whether a light can reach a box. A light reaches a box if the box
//! touches its sphere (and, for a spot, its cone) and is not wholly outside
//! the volumes the level marked as the light's region.
//!
//! Every test here answers "is this box out of reach?": `true` means culled.

use core::borrow::Borrow;

use crate::stage_light::is_cone_lamp;

/// What the culling needs to know about a primary light.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LampReach {
    pub kind: u8,
    pub origin: [f32; 3],
    /// The axis of a spot's cone. The cone opens away from this direction.
    pub direction: [f32; 3],
    pub radius: f32,
    /// Cosine of the cone's half angle, widened a little. Negative or NaN
    /// means the light has no usable cone.
    pub widened_half_view_cosine: f32,
}

/// One extra bounding slab of a light region: a unit direction and the range
/// along it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LightVolumeSpan {
    pub axis: [f32; 3],
    pub centre_dist: f32,
    pub half: f32,
}

/// A convex piece of a light's region. The first nine slabs are fixed: x, y,
/// z, then x+y, x-y, x+z, x-z, y+z, y-z. `axes` adds more.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LightVolumeHull<'a> {
    pub bounds_center_point: [f32; 9],
    pub bounds_half_extent: [f32; 9],
    pub axes: &'a [LightVolumeSpan],
}

/// All the collision_volumes of one light's region.
pub type LightVolumeHullList<'a> = &'a [LightVolumeHull<'a>];

/// The half sizes of the box used to light a model of the given radius.
pub fn illum_query_half_size(animated_object_bound_radius: f32) -> [f32; 3] {
    [animated_object_bound_radius; 3]
}

/// `a - b`, per axis.
fn minus(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    core::array::from_fn(|k| a[k] - b[k])
}

/// `a + s * b`, per axis.
fn plus_scaled(lhs: [f32; 3], s: f32, rhs: [f32; 3]) -> [f32; 3] {
    core::array::from_fn(|k| lhs[k] + s * rhs[k])
}

/// `a - s * b`, per axis.
fn minus_scaled(lhs: [f32; 3], s: f32, rhs: [f32; 3]) -> [f32; 3] {
    core::array::from_fn(|k| lhs[k] - s * rhs[k])
}

/// Sum over the axes of `term(k)`, added in axis order.
fn sum3(term: impl Fn(usize) -> f32) -> f32 {
    term(0) + term(1) + term(2)
}

fn dot(lhs: [f32; 3], rhs: [f32; 3]) -> f32 {
    sum3(|k| lhs[k] * rhs[k])
}

/// How far a box of half size `half` reaches along `dir` from its middle.
fn box_reach(aim: [f32; 3], half_ext: [f32; 3]) -> f32 {
    sum3(|k| aim[k].abs() * half_ext[k])
}

/// Distance from the sphere's centre to the box, squared, against the radius
/// squared.
fn cull_volume_for_ball(
    centre: [f32; 3],
    reach: f32,
    mid_at: [f32; 3],
    half_ext: [f32; 3],
) -> bool {
    let gap = |k: usize| ((centre[k] - mid_at[k]).abs() - half_ext[k]).max(0.0);
    let squared = (0..3).fold(0.0_f32, |sum, k| sum + gap(k) * gap(k));
    reach * reach < squared
}

/// A point or box seen from a cone's apex: its offset, how far that runs along
/// the axis, and the part of the offset off the axis.
struct ConeView {
    offset: [f32; 3],
    along: f32,
    sideways: [f32; 3],
}

impl ConeView {
    fn of(apex: [f32; 3], axis: [f32; 3], at: [f32; 3]) -> Self {
        let offset = minus(at, apex);
        let along = dot(offset, axis);
        ConeView { offset, along, sideways: minus_scaled(offset, along, axis) }
    }
}

/// Whether the box lies wholly outside a cone with its apex at `apex`.
///
/// `axis` is the cone's reference direction and the cone opens against it: a
/// box on the `axis` side of the apex is out. Past that, the cone is a
/// separating-axis problem: the box is out when some plane through the apex
/// and tangent to the cone has the box wholly on its outer side.
pub(crate) fn cone_reach_box(
    apex: [f32; 3],
    axis: [f32; 3],
    cos_half_angle: f32,
    mid_at: [f32; 3],
    half_ext: [f32; 3],
) -> bool {
    let offset = minus(mid_at, apex);
    let along = dot(offset, axis);
    if along >= box_reach(axis, half_ext) {
        return true;
    }
    let view = ConeView { sideways: minus_scaled(offset, along, axis), offset, along };
    let off_axis_sq = dot(view.sideways, view.sideways);
    let cos_sq = cos_half_angle * cos_half_angle;
    let sine_sq = 1.0 - cos_sq;
    // The middle of the box is inside the cone: nothing more to check.
    let middle_inside = off_axis_sq * cos_sq <= sine_sq * view.along * view.along;
    if middle_inside {
        return false;
    }
    let scaled = libm::sqrtf(off_axis_sq * sine_sq);
    if scaled == 0.0 {
        return true;
    }
    // The normal of the tangent plane nearest the box; the box is out when its
    // nearest corner is not behind that plane.
    let normal = plus_scaled(axis, cos_half_angle / scaled, view.sideways);
    let nearest = (0..3).fold(dot(normal, view.offset), |s, k| s - (normal[k] * half_ext[k]).abs());
    nearest >= 0.0 || nearest.is_nan()
}

/// Whether a point, grown by `slack`, lies wholly outside the cone at `apex`.
///
/// Same convention as the box test: the cone opens against `axis`, and a point
/// on the `axis` side (less `slack`) is out. Growing the point makes the cone
/// effectively wider, which keeps things that merely touch it.
pub(crate) fn cone_reach_point(
    apex: [f32; 3],
    axis: [f32; 3],
    cos_half_angle: f32,
    at: [f32; 3],
    slack: f32,
) -> bool {
    let view = ConeView::of(apex, axis, at);
    if view.along >= slack {
        return true;
    }
    let off_axis_sq = dot(view.sideways, view.sideways);
    let gap = libm::sqrtf(off_axis_sq) * view.along - slack;
    gap * gap <= off_axis_sq * cos_half_angle * cos_half_angle
}

/// Whether a primary light cannot reach the box. Spots are tested against
/// their cone as well as their sphere.
pub fn cull_volume_for_main_light(
    lamp: &LampReach,
    mid_at: [f32; 3],
    half_ext: [f32; 3],
) -> bool {
    if cull_volume_for_ball(lamp.origin, lamp.radius, mid_at, half_ext) {
        return true;
    }
    let cos_half = lamp.widened_half_view_cosine;
    // NaN fails the comparison, so a NaN cone is no cone.
    let usable_cone = cos_half >= 0.0 && is_cone_lamp(lamp.kind);
    usable_cone && cone_reach_box(lamp.origin, lamp.direction, cos_half, mid_at, half_ext)
}

/// Whether the box and a slab (`mid` +- `half`) cannot overlap, given the
/// box's own middle and half size along the slab's direction.
fn slab_misses(box_mid: f32, box_half: f32, slab_mid: f32, slab_half: f32) -> bool {
    box_half + slab_half <= (box_mid - slab_mid).abs()
}

/// The nine fixed slab directions of a hull, as the axes they add: x, y, z,
/// then x+y, x-y, x+z, x-z, y+z, y-z. A second axis comes with its sign.
const KDOP_DIRS: [(usize, Option<(usize, f32)>); 9] = [
    (0, None),
    (1, None),
    (2, None),
    (0, Some((1, 1.0))),
    (0, Some((1, -1.0))),
    (0, Some((2, 1.0))),
    (0, Some((2, -1.0))),
    (1, Some((2, 1.0))),
    (1, Some((2, -1.0))),
];

/// Whether the box (in the light's local space) misses a region hull.
fn cull_box_from_light_region_hull(
    hull: &LightVolumeHull<'_>,
    mid_at: [f32; 3],
    half_ext: [f32; 3],
) -> bool {
    let fixed_miss = KDOP_DIRS.iter().enumerate().any(|(slot, &(a, b))| {
        let (mid, half) = match b {
            None => (mid_at[a], half_ext[a]),
            Some((b, sign)) => (mid_at[a] + sign * mid_at[b], half_ext[a] + half_ext[b]),
        };
        slab_misses(mid, half, hull.bounds_center_point[slot], hull.bounds_half_extent[slot])
    });
    fixed_miss
        || hull.axes.iter().any(|extra| {
            slab_misses(dot(mid_at, extra.axis), box_reach(extra.axis, half_ext), extra.centre_dist, extra.half)
        })
}

/// Whether a light's region excludes the box. The region is the union of its
/// collision_volumes, positioned relative to the light's origin; a light with no collision_volumes
/// excludes nothing.
pub fn hulls_reject_box<'a, H: Borrow<LightVolumeHull<'a>>>(
    collision_volumes: impl IntoIterator<Item = H>,
    lit_sample_point: [f32; 3],
    mid_at: [f32; 3],
    half_ext: [f32; 3],
) -> bool {
    let rel = minus(mid_at, lit_sample_point);
    let mut collision_volumes = collision_volumes.into_iter().peekable();
    collision_volumes.peek().is_some()
        && collision_volumes.all(|shell| cull_box_from_light_region_hull(shell.borrow(), rel, half_ext))
}

/// The light a box is lit by, found by walking the lights after the sun.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SecondaryLightWalkLog {
    /// Index of the first light that reaches the box; 0 when none does.
    pub first_hit: u32,
}

/// Walks the lights after `first_sun` and picks the first that
/// neither its volume nor its region rules out. Without region data nothing
/// is picked.
pub fn secondary_main_light_walk_trace(
    lights: &[LampReach],
    first_sun: u32,
    regions: Option<&[LightVolumeHullList<'_>]>,
    mid_at: [f32; 3],
    half_ext: [f32; 3],
) -> SecondaryLightWalkLog {
    let skip = first_sun.saturating_add(1) as usize;
    let reaching = regions.and_then(|regions| {
        lights
            .iter()
            .zip(regions)
            .enumerate()
            .skip(skip)
            .find(|(_, (lamp, region))| {
                !cull_volume_for_main_light(lamp, mid_at, half_ext)
                    && !hulls_reject_box(region.iter(), lamp.origin, mid_at, half_ext)
            })
    });
    SecondaryLightWalkLog { first_hit: reaching.map_or(0, |(at, _)| at as u32) }
}

/// Starting value for "closest light so far" when linking a dynamic entity.
pub const PROP_KEY_LIGHT_LINK_FARTHEST_SQUARED: f32 = f32::MAX;

/// Squared distance from a light to a box's middle.
pub fn mover_relink_dist_sq(point: [f32; 3], mid_at: [f32; 3]) -> f32 {
    let apart = minus(mid_at, point);
    dot(apart, apart)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stage_light::{POINT_LAMP, CONE_LAMP};

    const ONE: [f32; 3] = [1.0; 3];

    fn light(kind: u8, spot: [f32; 3], reach: f32, cos_half: f32) -> LampReach {
        LampReach {
            kind,
            origin: spot,
            direction: UP,
            radius: reach,
            widened_half_view_cosine: cos_half,
        }
    }

    fn hull(half: f32) -> LightVolumeHull<'static> {
        LightVolumeHull {
            bounds_center_point: [0.0; 9],
            bounds_half_extent: [half; 9],
            axes: &[],
        }
    }

    #[test]
    fn the_box_query_half_is_the_radius_on_every_axis() {
        assert_eq!(illum_query_half_size(12.5), [12.5; 3]);
    }

    #[test]
    fn a_box_beyond_the_sphere_is_culled() {
        assert!(cull_volume_for_ball([0.0; 3], 10.0, [16.0, 0.0, 0.0], ONE));
    }

    #[test]
    fn a_box_inside_the_sphere_is_not_culled() {
        assert!(!cull_volume_for_ball([0.0; 3], 10.0, [10.0, 0.0, 0.0], ONE));
        assert!(!cull_volume_for_ball([0.0; 3], 10.0, [0.0; 3], ONE));
    }

    #[test]
    fn a_box_whose_nearest_point_is_exactly_the_radius_away_is_kept() {
        assert!(!cull_volume_for_ball([0.0; 3], 10.0, [11.0, 0.0, 0.0], ONE));
    }

    #[test]
    fn the_sphere_distance_counts_every_axis() {
        // Nearest point is (8, 8, 0) away: 128 > 100.
        assert!(cull_volume_for_ball([0.0; 3], 10.0, [9.0, 9.0, 0.0], ONE));
        // (6, 6, 0) away: 72 < 100.
        assert!(!cull_volume_for_ball([0.0; 3], 10.0, [7.0, 7.0, 0.0], ONE));
    }

    /// The cone at the origin opening down -z, about 45 degrees wide.
    const UP: [f32; 3] = [0.0, 0.0, 1.0];
    const COS_45: f32 = 0.7071;

    fn box_out(mid: [f32; 3]) -> bool {
        cone_reach_box([0.0; 3], UP, COS_45, mid, ONE)
    }

    fn point_out(point: [f32; 3], slack: f32) -> bool {
        cone_reach_point([0.0; 3], UP, COS_45, point, slack)
    }

    #[test]
    fn a_box_on_the_axis_side_of_the_apex_is_outside_the_cone() {
        assert!(box_out([0.0, 0.0, 10.0]));
    }

    #[test]
    fn a_box_straight_down_the_cone_is_inside_it() {
        assert!(!box_out([0.0, 0.0, -10.0]));
    }

    #[test]
    fn a_box_far_off_to_the_side_is_outside_the_cone() {
        assert!(box_out([50.0, 0.0, -10.0]));
    }

    #[test]
    fn a_box_grazing_the_cone_edge_is_inside_it() {
        assert!(!box_out([11.0, 0.0, -10.0]));
    }

    #[test]
    fn a_point_on_the_axis_side_is_outside_the_expanded_cone() {
        assert!(point_out([0.0, 0.0, 10.0], 4.0));
    }

    #[test]
    fn a_point_straight_down_the_expanded_cone_is_kept() {
        assert!(!point_out([3.0, 0.0, -10.0], 4.0));
    }

    #[test]
    fn a_point_beside_the_apex_plane_is_outside_the_cone_until_it_is_grown() {
        let point = [3.0, 0.0, -0.5];
        assert!(point_out(point, 0.0));
        assert!(!point_out(point, 4.0));
    }

    #[test]
    fn slack_pulls_a_point_beside_the_axis_side_back_in() {
        // Just past the apex plane: out with no slack, kept once it is grown.
        let apex = [0.0; 3];
        let axis = UP;
        assert!(cone_reach_point(
            apex,
            axis,
            0.7071,
            [1.0, 0.0, 0.5],
            0.5
        ));
        assert!(!cone_reach_point(
            apex,
            axis,
            0.7071,
            [1.0, 0.0, 0.5],
            40.0
        ));
    }

    #[test]
    fn an_omni_light_ignores_its_cone() {
        let omni = light(POINT_LAMP, [0.0; 3], 100.0, 0.5);
        assert!(!cull_volume_for_main_light(&omni, [0.0, 0.0, 50.0], ONE));
    }

    #[test]
    fn a_spot_light_is_cut_by_its_cone() {
        let spot = light(CONE_LAMP, [0.0; 3], 100.0, 0.5);
        assert!(cull_volume_for_main_light(&spot, [0.0, 0.0, 50.0], ONE));
        assert!(!cull_volume_for_main_light(&spot, [0.0, 0.0, -50.0], ONE));
    }

    #[test]
    fn a_spot_is_still_cut_by_its_sphere() {
        let spot = light(CONE_LAMP, [0.0; 3], 100.0, 0.5);
        assert!(cull_volume_for_main_light(&spot, [0.0, 0.0, -150.0], ONE));
    }

    #[test]
    fn a_spot_with_no_usable_cone_acts_like_an_omni() {
        let negative = light(CONE_LAMP, [0.0; 3], 100.0, -1.0);
        let nan = light(CONE_LAMP, [0.0; 3], 100.0, f32::NAN);
        assert!(!cull_volume_for_main_light(
            &negative,
            [0.0, 0.0, 50.0],
            ONE
        ));
        assert!(!cull_volume_for_main_light(&nan, [0.0, 0.0, 50.0], ONE));
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
        let axes = [LightVolumeSpan {
            axis: [0.0, 0.0, 1.0],
            centre_dist: 0.0,
            half: 3.0,
        }];
        let with_axis = LightVolumeHull {
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
        let none: [LightVolumeHull; 0] = [];
        assert!(!hulls_reject_box(&none, [0.0; 3], [99.0; 3], ONE));
    }

    #[test]
    fn a_region_culls_only_when_every_hull_does() {
        let collision_volumes = [hull(1.0), hull(100.0)];
        assert!(!hulls_reject_box(
            &collision_volumes,
            [0.0; 3],
            [50.0, 0.0, 0.0],
            ONE
        ));
        let both = [hull(1.0), hull(2.0)];
        assert!(hulls_reject_box(
            &both,
            [0.0; 3],
            [50.0, 0.0, 0.0],
            ONE
        ));
    }

    #[test]
    fn a_region_is_measured_from_the_light() {
        let collision_volumes = [hull(5.0)];
        assert!(hulls_reject_box(
            &collision_volumes,
            [0.0; 3],
            [100.0, 0.0, 0.0],
            ONE
        ));
        assert!(!hulls_reject_box(
            &collision_volumes,
            [100.0, 0.0, 0.0],
            [100.0, 0.0, 0.0],
            ONE
        ));
    }

    #[test]
    fn a_region_takes_hulls_by_value_or_by_reference() {
        let owned = vec![hull(1.0)];
        assert!(hulls_reject_box(
            owned.clone(),
            [0.0; 3],
            [50.0, 0.0, 0.0],
            ONE
        ));
        assert!(hulls_reject_box(
            owned.iter(),
            [0.0; 3],
            [50.0, 0.0, 0.0],
            ONE
        ));
    }

    fn omni(origin: [f32; 3], radius: f32) -> LampReach {
        light(POINT_LAMP, origin, radius, -1.0)
    }

    fn walk(
        lights: &[LampReach],
        sun: u32,
        regions: Option<&[LightVolumeHullList<'_>]>,
    ) -> u32 {
        secondary_main_light_walk_trace(lights, sun, regions, [0.0; 3], ONE).first_hit
    }

    #[test]
    fn the_walk_starts_after_the_sun_and_takes_the_first_light_in_reach() {
        let lights = [
            omni([0.0; 3], 100.0),
            omni([500.0, 0.0, 0.0], 10.0),
            omni([0.0; 3], 100.0),
            omni([0.0; 3], 100.0),
        ];
        let regions: [LightVolumeHullList; 4] = [&[]; 4];
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
        let open: [LightVolumeHullList; 3] = [&[], &[], &[]];
        let one_tight: [LightVolumeHullList; 3] = [&[], &tight, &[]];
        let pick = |regions: &[LightVolumeHullList]| {
            secondary_main_light_walk_trace(&lights, 0, Some(regions), box_mid, ONE).first_hit
        };
        assert_eq!(pick(&open), 1);
        assert_eq!(pick(&one_tight), 2);
    }

    #[test]
    fn the_walk_finds_nothing_when_every_light_is_ruled_out() {
        let lights = [omni([0.0; 3], 100.0), omni([900.0, 0.0, 0.0], 10.0)];
        let regions: [LightVolumeHullList; 2] = [&[], &[]];
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
        let regions: [LightVolumeHullList; 1] = [&[]];
        assert_eq!(walk(&lights, 0, Some(&regions)), 0);
    }

    #[test]
    fn the_link_distance_is_squared() {
        assert_eq!(
            mover_relink_dist_sq([1.0, 2.0, 3.0], [4.0, 6.0, 3.0]),
            25.0
        );
        assert!(
            mover_relink_dist_sq([0.0; 3], [1.0; 3])
                < PROP_KEY_LIGHT_LINK_FARTHEST_SQUARED
        );
    }
}
