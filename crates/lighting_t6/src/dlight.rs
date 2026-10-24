//! Choosing which dynamic lights are sent to the backend this frame.

use core::cmp::Ordering;

use crate::scene_light::{GfxLightPack, SCENE_DLIGHT_LIMIT, gfx_light_is_spot};

/// The `r_dlightLimit` default: omni lights sent along with the world.
pub const R_DLIGHT_LIMIT_DEFAULT: u32 = 4;

/// The most omni lights the backend takes however high the limit is set.
const OMNI_LIMIT_MAX: u32 = 4;

/// Room for one spot light plus the omni lights.
pub const R_DLIGHT_BACKEND_MAX: usize = 5;

/// A dynamic light in the scene's list.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SceneDlight {
    pub light: GfxLightPack,
    /// Already taken by another pass; skipped here.
    pub used: bool,
}

fn distance_squared(a: [f32; 3], b: [f32; 3]) -> f32 {
    let (dx, dy, dz) = (a[0] - b[0], a[1] - b[1], a[2] - b[2]);
    dx * dx + dy * dy + dz * dz
}

/// Whether `first` should be taken before `second` when the view is at
/// `view`. Spot lights come before omni lights; within a kind, the one whose
/// distance is smaller compared with its radius comes first.
fn is_preferred(first: &GfxLightPack, second: &GfxLightPack, view: [f32; 3]) -> bool {
    let first_spot = gfx_light_is_spot(first.light_type);
    if first_spot != gfx_light_is_spot(second.light_type) {
        return first_spot;
    }
    // distance_a / radius_a^2 < distance_b / radius_b^2, without dividing.
    let first_scaled = distance_squared(view, first.origin) * second.radius * second.radius;
    let second_scaled = distance_squared(view, second.origin) * first.radius * first.radius;
    first_scaled < second_scaled
}

/// Whether a light's sphere reaches the box `mid` +- `half`. A light with no
/// radius reaches nothing.
pub fn dlight_hits_aabb(origin: [f32; 3], radius: f32, mid: [f32; 3], half: [f32; 3]) -> bool {
    if radius <= 0.0 {
        return false;
    }
    let mut gap_squared = 0.0_f32;
    for axis in 0..3 {
        let gap = ((origin[axis] - mid[axis]).abs() - half[axis]).max(0.0);
        gap_squared += gap * gap;
    }
    gap_squared <= radius * radius
}

/// Whether a sphere lies wholly behind one of the planes (each is
/// `[nx, ny, nz, d]`, facing into the view).
fn sphere_outside_planes(planes: &[[f32; 4]], origin: [f32; 3], radius: f32) -> bool {
    planes.iter().any(|plane| {
        plane[0] * origin[0] + plane[1] * origin[1] + plane[2] * origin[2] + plane[3] < -radius
    })
}

/// The unused spot light that is most preferred and inside the frustum.
fn best_visible_spot<'a>(
    slots: &'a [SceneDlight],
    view: [f32; 3],
    planes: &[[f32; 4]],
) -> Option<&'a GfxLightPack> {
    let mut best: Option<&GfxLightPack> = None;
    for slot in slots {
        let light = &slot.light;
        let usable = !slot.used
            && gfx_light_is_spot(light.light_type)
            && light.radius > 0.0
            && !sphere_outside_planes(planes, light.origin, light.radius);
        if usable && best.is_none_or(|current| is_preferred(light, current, view)) {
            best = Some(light);
        }
    }
    best
}

/// Appends the lights for this frame to `dest` and returns how many. First
/// comes the best on-screen spot light (only when `sm3` is set and a frustum
/// is given), then up to `limit` omni lights (at most four): the unused ones
/// in list order if they all fit, else the most preferred.
pub fn append_scene_dlights_to_backend(
    slots: &[SceneDlight],
    view: [f32; 3],
    limit: u32,
    sm3: bool,
    planes: Option<&[[f32; 4]]>,
    dest: &mut [GfxLightPack],
) -> usize {
    if limit == 0 {
        return 0;
    }
    let mut written = 0;
    let mut push = |light: GfxLightPack| {
        if written < dest.len() {
            dest[written] = light;
            written += 1;
        }
    };

    if sm3
        && let Some(planes) = planes.filter(|planes| !planes.is_empty())
        && let Some(spot) = best_visible_spot(slots, view, planes)
    {
        push(*spot);
    }

    let mut omni: Vec<usize> = slots
        .iter()
        .enumerate()
        .filter(|(_, slot)| !slot.used && !gfx_light_is_spot(slot.light.light_type))
        .map(|(index, _)| index)
        .take(SCENE_DLIGHT_LIMIT as usize)
        .collect();
    let omni_limit = limit.min(OMNI_LIMIT_MAX) as usize;
    if omni.len() > omni_limit {
        omni.sort_by(|&a, &b| {
            let (first, second) = (&slots[a].light, &slots[b].light);
            match (
                is_preferred(first, second, view),
                is_preferred(second, first, view),
            ) {
                (true, false) => Ordering::Less,
                (false, true) => Ordering::Greater,
                _ => a.cmp(&b),
            }
        });
        omni.truncate(omni_limit);
    }
    for index in omni {
        push(slots[index].light);
    }
    written
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene_light::{GFX_LIGHT_TYPE_SPOT, omni_light_pack};

    const ORIGIN: [f32; 3] = [0.0; 3];

    fn omni_at(x: f32, radius: f32) -> SceneDlight {
        SceneDlight {
            light: omni_light_pack([x, 0.0, 0.0], radius, [1.0; 3]),
            used: false,
        }
    }

    fn spot_at(x: f32, radius: f32) -> SceneDlight {
        let mut light = omni_light_pack([x, 0.0, 0.0], radius, [1.0; 3]);
        light.light_type = GFX_LIGHT_TYPE_SPOT;
        SceneDlight { light, used: false }
    }

    fn origins(lights: &[GfxLightPack], count: usize) -> Vec<f32> {
        lights[..count]
            .iter()
            .map(|light| light.origin[0])
            .collect()
    }

    fn blank() -> [GfxLightPack; R_DLIGHT_BACKEND_MAX] {
        [omni_light_pack(ORIGIN, 1.0, ORIGIN); R_DLIGHT_BACKEND_MAX]
    }

    /// Everything with z >= 0 is in view.
    const FLOOR: [[f32; 4]; 1] = [[0.0, 0.0, 1.0, 0.0]];

    #[test]
    fn a_light_touching_the_box_edge_hits_it() {
        assert!(dlight_hits_aabb(
            [10.0, 0.0, 0.0],
            5.0,
            ORIGIN,
            [5.0, 5.0, 5.0]
        ));
        assert!(!dlight_hits_aabb(
            [10.1, 0.0, 0.0],
            5.0,
            ORIGIN,
            [5.0, 5.0, 5.0]
        ));
    }

    #[test]
    fn a_light_inside_the_box_hits_it() {
        assert!(dlight_hits_aabb([1.0, 1.0, 1.0], 0.5, ORIGIN, [5.0; 3]));
    }

    #[test]
    fn the_gap_is_measured_across_all_axes() {
        // Corner gap is (3, 4, 0): 5 away.
        assert!(dlight_hits_aabb([4.0, 5.0, 0.0], 5.0, ORIGIN, [1.0; 3]));
        assert!(!dlight_hits_aabb([4.0, 5.0, 0.0], 4.9, ORIGIN, [1.0; 3]));
    }

    #[test]
    fn a_light_with_no_radius_hits_nothing() {
        assert!(!dlight_hits_aabb(ORIGIN, 0.0, ORIGIN, [5.0; 3]));
        assert!(!dlight_hits_aabb(ORIGIN, -1.0, ORIGIN, [5.0; 3]));
    }

    #[test]
    fn a_zero_limit_sends_nothing() {
        let slots = [omni_at(1.0, 10.0)];
        let mut dest = blank();
        assert_eq!(
            append_scene_dlights_to_backend(&slots, ORIGIN, 0, true, None, &mut dest),
            0
        );
    }

    #[test]
    fn omni_lights_that_fit_keep_the_list_order() {
        let slots = [
            omni_at(30.0, 10.0),
            omni_at(10.0, 10.0),
            omni_at(20.0, 10.0),
        ];
        let mut dest = blank();
        let n = append_scene_dlights_to_backend(&slots, ORIGIN, 4, false, None, &mut dest);
        assert_eq!(origins(&dest, n), [30.0, 10.0, 20.0]);
    }

    #[test]
    fn used_lights_are_skipped() {
        let mut slots = [omni_at(1.0, 10.0), omni_at(2.0, 10.0)];
        slots[0].used = true;
        let mut dest = blank();
        let n = append_scene_dlights_to_backend(&slots, ORIGIN, 4, false, None, &mut dest);
        assert_eq!(origins(&dest, n), [2.0]);
    }

    #[test]
    fn too_many_omni_lights_keep_the_ones_nearest_for_their_size() {
        let slots = [
            omni_at(50.0, 10.0),
            omni_at(10.0, 10.0),
            omni_at(40.0, 10.0),
            omni_at(20.0, 10.0),
            omni_at(30.0, 10.0),
        ];
        let mut dest = blank();
        let n = append_scene_dlights_to_backend(&slots, ORIGIN, 4, false, None, &mut dest);
        assert_eq!(origins(&dest, n), [10.0, 20.0, 30.0, 40.0]);
    }

    #[test]
    fn a_bigger_light_beats_a_nearer_small_one() {
        // 40 away with radius 40 (ratio 1) against 10 away with radius 2 (25).
        let slots = [omni_at(10.0, 2.0), omni_at(40.0, 40.0)];
        let mut dest = blank();
        let n = append_scene_dlights_to_backend(&slots, ORIGIN, 1, false, None, &mut dest);
        assert_eq!(origins(&dest, n), [40.0]);
    }

    #[test]
    fn equally_good_omni_lights_keep_list_order() {
        let slots = [omni_at(10.0, 5.0), omni_at(-10.0, 5.0), omni_at(10.0, 5.0)];
        let mut dest = blank();
        let n = append_scene_dlights_to_backend(&slots, ORIGIN, 2, false, None, &mut dest);
        assert_eq!(origins(&dest, n), [10.0, -10.0]);
    }

    #[test]
    fn the_limit_is_capped_at_four_omni_lights() {
        let slots: Vec<SceneDlight> = (1..=6).map(|i| omni_at(i as f32, 10.0)).collect();
        let mut dest = blank();
        let n = append_scene_dlights_to_backend(&slots, ORIGIN, 99, false, None, &mut dest);
        assert_eq!(n, 4);
    }

    #[test]
    fn a_lower_limit_sends_fewer_omni_lights() {
        let slots: Vec<SceneDlight> = (1..=6).map(|i| omni_at(i as f32, 10.0)).collect();
        let mut dest = blank();
        let n = append_scene_dlights_to_backend(&slots, ORIGIN, 2, false, None, &mut dest);
        assert_eq!(origins(&dest, n), [1.0, 2.0]);
    }

    #[test]
    fn a_visible_spot_goes_first() {
        let mut spot = spot_at(5.0, 10.0);
        spot.light.origin[2] = 3.0;
        let slots = [omni_at(1.0, 10.0), spot];
        let mut dest = blank();
        let n = append_scene_dlights_to_backend(&slots, ORIGIN, 4, true, Some(&FLOOR), &mut dest);
        assert_eq!(n, 2);
        assert_eq!(origins(&dest, n), [5.0, 1.0]);
        assert_eq!(dest[0].light_type, GFX_LIGHT_TYPE_SPOT);
    }

    #[test]
    fn spot_lights_need_sm3_and_a_frustum() {
        let slots = [spot_at(5.0, 10.0)];
        let mut dest = blank();
        assert_eq!(
            append_scene_dlights_to_backend(&slots, ORIGIN, 4, false, Some(&FLOOR), &mut dest),
            0
        );
        assert_eq!(
            append_scene_dlights_to_backend(&slots, ORIGIN, 4, true, None, &mut dest),
            0
        );
        assert_eq!(
            append_scene_dlights_to_backend(&slots, ORIGIN, 4, true, Some(&[]), &mut dest),
            0
        );
    }

    #[test]
    fn a_spot_wholly_behind_a_plane_is_dropped() {
        let mut behind = spot_at(5.0, 10.0);
        behind.light.origin[2] = -100.0;
        let mut grazing = spot_at(6.0, 10.0);
        grazing.light.origin[2] = -5.0;
        let mut dest = blank();
        let n =
            append_scene_dlights_to_backend(&[behind], ORIGIN, 4, true, Some(&FLOOR), &mut dest);
        assert_eq!(n, 0);
        let n =
            append_scene_dlights_to_backend(&[grazing], ORIGIN, 4, true, Some(&FLOOR), &mut dest);
        assert_eq!(n, 1);
    }

    #[test]
    fn only_the_best_spot_is_sent_and_a_dead_one_is_ignored() {
        let mut dead = spot_at(1.0, 0.0);
        dead.light.origin[2] = 1.0;
        let mut far = spot_at(80.0, 10.0);
        far.light.origin[2] = 1.0;
        let mut near = spot_at(20.0, 10.0);
        near.light.origin[2] = 1.0;
        let slots = [dead, far, near];
        let mut dest = blank();
        let n = append_scene_dlights_to_backend(&slots, ORIGIN, 4, true, Some(&FLOOR), &mut dest);
        assert_eq!(origins(&dest, n), [20.0]);
    }

    #[test]
    fn a_used_spot_is_not_sent() {
        let mut spot = spot_at(5.0, 10.0);
        spot.light.origin[2] = 1.0;
        spot.used = true;
        let mut dest = blank();
        let n = append_scene_dlights_to_backend(&[spot], ORIGIN, 4, true, Some(&FLOOR), &mut dest);
        assert_eq!(n, 0);
    }

    #[test]
    fn a_short_destination_is_filled_and_no_more() {
        let slots = [omni_at(1.0, 10.0), omni_at(2.0, 10.0), omni_at(3.0, 10.0)];
        let mut dest = blank();
        let n = append_scene_dlights_to_backend(&slots, ORIGIN, 4, false, None, &mut dest[..2]);
        assert_eq!(n, 2);
        assert_eq!(origins(&dest, 2), [1.0, 2.0]);
    }

    #[test]
    fn only_the_first_thirty_two_omni_lights_are_considered() {
        let mut slots: Vec<SceneDlight> = (0..33).map(|_| omni_at(100.0, 10.0)).collect();
        // The nearest light is the 33rd, past the scan limit.
        slots[32] = omni_at(1.0, 10.0);
        let mut dest = blank();
        let n = append_scene_dlights_to_backend(&slots, ORIGIN, 1, false, None, &mut dest);
        assert_eq!(origins(&dest, n), [100.0]);
    }
}
