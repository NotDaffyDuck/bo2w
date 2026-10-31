//! Choosing which dynamic lights are sent to the backend this frame.

use core::cmp::Ordering;

use crate::stage_light::{LampRecord, SCENE_DLIGHT_LIMIT, is_cone_lamp};

/// The `r_dlightLimit` default: omni lights sent along with the world.
pub const TRANSIENT_LAMP_INITIAL_CAP: u32 = 4;

/// The most omni lights the backend takes however high the limit is set.
const OMNI_LIMIT_MAX: u32 = 4;

/// Room for one spot light plus the omni lights.
pub const TRANSIENT_LAMP_GPU_CEILING: usize = 5;

/// A dynamic light in the scene's list.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BriefLamp {
    pub light: LampRecord,
    /// Already taken by another pass; skipped here.
    pub claimed: bool,
}

fn distance_squared(a: [f32; 3], b: [f32; 3]) -> f32 {
    let (step_along_x, dy, dz) = (a[0] - b[0], a[1] - b[1], a[2] - b[2]);
    step_along_x * step_along_x + dy * dy + dz * dz
}

/// Whether `first` should be taken before `second` when the view is at
/// `view`. Spot lights come before omni lights; within a kind, the one whose
/// distance is smaller compared with its radius comes first.
fn is_preferred(first: &LampRecord, second: &LampRecord, view: [f32; 3]) -> bool {
    let first_spot = is_cone_lamp(first.kind);
    if first_spot != is_cone_lamp(second.kind) {
        return first_spot;
    }
    // distance_a / radius_a^2 < distance_b / radius_b^2, without dividing.
    let first_scaled = distance_squared(view, first.origin) * second.radius * second.radius;
    let second_scaled = distance_squared(view, second.origin) * first.radius * first.radius;
    first_scaled < second_scaled
}

/// Whether a light's sphere reaches the box `mid` +- `half`. A light with no
/// radius reaches nothing.
pub fn brief_lamp_hits_box(centre: [f32; 3], range: f32, box_at: [f32; 3], half_ext: [f32; 3]) -> bool {
    // Squared distance from the light to the nearest point of the box.
    let reach = (0..3)
        .map(|k| ((centre[k] - box_at[k]).abs() - half_ext[k]).max(0.0))
        .fold(0.0_f32, |sum, outside| sum + outside * outside);
    range > 0.0 && reach <= range * range
}

/// Whether a sphere lies wholly behind one of the planes (each is
/// `[facing_x, facing_y, facing_z, d]`, facing into the view).
fn sphere_outside_planes(planes: &[[f32; 4]], spot: [f32; 3], reach: f32) -> bool {
    let [x, y, z] = spot;
    planes.iter().any(|&[nx, ny, nz, d]| nx * x + ny * y + nz * z + d < -reach)
}

/// The unused spot light that is most preferred and inside the frustum.
fn best_visible_spot<'a>(
    lamps: &'a [BriefLamp],
    eye: [f32; 3],
    sides: &[[f32; 4]],
) -> Option<&'a LampRecord> {
    let mut best: Option<&LampRecord> = None;
    for lamp in lamps {
        let light = &lamp.light;
        let usable = !lamp.claimed
            && is_cone_lamp(light.kind)
            && light.radius > 0.0
            && !sphere_outside_planes(sides, light.origin, light.radius);
        if usable && best.is_none_or(|current| is_preferred(light, current, eye)) {
            best = Some(light);
        }
    }
    best
}

/// Appends the lights for this frame to `write_target` and returns how many. First
/// comes the best on-screen spot light (only when `sm3` is set and a frustum
/// is given), then up to `limit` omni lights (at most four): the unused ones
/// in list order if they all fit, else the most preferred.
pub fn forward_scene_dynamic_lights_to_renderer(
    slots: &[BriefLamp],
    eye: [f32; 3],
    limit: u32,
    sm3: bool,
    frustum: Option<&[[f32; 4]]>,
    write_target: &mut [LampRecord],
) -> usize {
    if limit == 0 {
        return 0;
    }
    let mut stored = 0;
    let mut push = |light: LampRecord| {
        if stored < write_target.len() {
            write_target[stored] = light;
            stored += 1;
        }
    };

    if sm3
        && let Some(sides) = frustum.filter(|sides| !sides.is_empty())
        && let Some(spot) = best_visible_spot(slots, eye, sides)
    {
        push(*spot);
    }

    let mut omni: Vec<usize> = slots
        .iter()
        .enumerate()
        .filter_map(|(light_no, lamp)| (!lamp.claimed && !is_cone_lamp(lamp.light.kind)).then_some(light_no))
        .take(SCENE_DLIGHT_LIMIT as usize)
        .collect();
    let omni_limit = limit.min(OMNI_LIMIT_MAX) as usize;
    if omni.len() > omni_limit {
        omni.sort_by(|&i, &j| {
            let (first, second) = (&slots[i].light, &slots[j].light);
            match (is_preferred(first, second, eye), is_preferred(second, first, eye)) {
                (true, false) => Ordering::Less,
                (false, true) => Ordering::Greater,
                _ => i.cmp(&j),
            }
        });
        omni.truncate(omni_limit);
    }
    for light_no in omni {
        push(slots[light_no].light);
    }
    stored
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stage_light::{CONE_LAMP, pack_point_lamp};

    const ORIGIN: [f32; 3] = [0.0; 3];

    fn omni_at(x: f32, radius: f32) -> BriefLamp {
        let lamp = pack_point_lamp([x, 0.0, 0.0], radius, [1.0; 3]);
        BriefLamp { light: lamp, claimed: false }
    }

    fn spot_at(x: f32, radius: f32) -> BriefLamp {
        let mut lamp = pack_point_lamp([x, 0.0, 0.0], radius, [1.0; 3]);
        lamp.kind = CONE_LAMP;
        BriefLamp { light: lamp, claimed: false }
    }

    fn origins(lights: &[LampRecord], count: usize) -> Vec<f32> {
        lights[..count]
            .iter()
            .map(|light| light.origin[0])
            .collect()
    }

    fn blank() -> [LampRecord; TRANSIENT_LAMP_GPU_CEILING] {
        [pack_point_lamp(ORIGIN, 1.0, ORIGIN); TRANSIENT_LAMP_GPU_CEILING]
    }

    /// Everything with z >= 0 is in view.
    const FLOOR: [[f32; 4]; 1] = [[0.0, 0.0, 1.0, 0.0]];

    #[test]
    fn a_light_touching_the_box_edge_hits_it() {
        assert!(brief_lamp_hits_box(
            [10.0, 0.0, 0.0],
            5.0,
            ORIGIN,
            [5.0, 5.0, 5.0]
        ));
        assert!(!brief_lamp_hits_box(
            [10.1, 0.0, 0.0],
            5.0,
            ORIGIN,
            [5.0, 5.0, 5.0]
        ));
    }

    #[test]
    fn a_light_inside_the_box_hits_it() {
        assert!(brief_lamp_hits_box([1.0, 1.0, 1.0], 0.5, ORIGIN, [5.0; 3]));
    }

    #[test]
    fn the_gap_is_measured_across_all_axes() {
        // Corner gap is (3, 4, 0): 5 away.
        assert!(brief_lamp_hits_box([4.0, 5.0, 0.0], 5.0, ORIGIN, [1.0; 3]));
        assert!(!brief_lamp_hits_box([4.0, 5.0, 0.0], 4.9, ORIGIN, [1.0; 3]));
    }

    #[test]
    fn a_light_with_no_radius_hits_nothing() {
        assert!(!brief_lamp_hits_box(ORIGIN, 0.0, ORIGIN, [5.0; 3]));
        assert!(!brief_lamp_hits_box(ORIGIN, -1.0, ORIGIN, [5.0; 3]));
    }

    #[test]
    fn a_zero_limit_sends_nothing() {
        let slots = [omni_at(1.0, 10.0)];
        let mut write_target = blank();
        assert_eq!(
            forward_scene_dynamic_lights_to_renderer(&slots, ORIGIN, 0, true, None, &mut write_target),
            0
        );
    }

    #[test]
    fn omni_lights_that_fit_keep_the_list_order() {
        let lamps = [
            omni_at(30.0, 10.0),
            omni_at(10.0, 10.0),
            omni_at(20.0, 10.0),
        ];
        let mut write_target = blank();
        let n = forward_scene_dynamic_lights_to_renderer(&lamps, ORIGIN, 4, false, None, &mut write_target);
        assert_eq!(origins(&write_target, n), [30.0, 10.0, 20.0]);
    }

    #[test]
    fn used_lights_are_skipped() {
        let mut lamps = [omni_at(1.0, 10.0), omni_at(2.0, 10.0)];
        lamps[0].claimed = true;
        let mut write_target = blank();
        let n = forward_scene_dynamic_lights_to_renderer(&lamps, ORIGIN, 4, false, None, &mut write_target);
        assert_eq!(origins(&write_target, n), [2.0]);
    }

    #[test]
    fn too_many_omni_lights_keep_the_ones_nearest_for_their_size() {
        let lamps = [
            omni_at(50.0, 10.0),
            omni_at(10.0, 10.0),
            omni_at(40.0, 10.0),
            omni_at(20.0, 10.0),
            omni_at(30.0, 10.0),
        ];
        let mut write_target = blank();
        let n = forward_scene_dynamic_lights_to_renderer(&lamps, ORIGIN, 4, false, None, &mut write_target);
        assert_eq!(origins(&write_target, n), [10.0, 20.0, 30.0, 40.0]);
    }

    #[test]
    fn a_bigger_light_beats_a_nearer_small_one() {
        // 40 away with radius 40 (ratio 1) against 10 away with radius 2 (25).
        let lamps = [omni_at(10.0, 2.0), omni_at(40.0, 40.0)];
        let mut write_target = blank();
        let n = forward_scene_dynamic_lights_to_renderer(&lamps, ORIGIN, 1, false, None, &mut write_target);
        assert_eq!(origins(&write_target, n), [40.0]);
    }

    #[test]
    fn equally_good_omni_lights_keep_list_order() {
        let lamps = [omni_at(10.0, 5.0), omni_at(-10.0, 5.0), omni_at(10.0, 5.0)];
        let mut write_target = blank();
        let n = forward_scene_dynamic_lights_to_renderer(&lamps, ORIGIN, 2, false, None, &mut write_target);
        assert_eq!(origins(&write_target, n), [10.0, -10.0]);
    }

    #[test]
    fn the_limit_is_capped_at_four_omni_lights() {
        let lamps: Vec<BriefLamp> = (1..=6).map(|i| omni_at(i as f32, 10.0)).collect();
        let mut write_target = blank();
        let n = forward_scene_dynamic_lights_to_renderer(&lamps, ORIGIN, 99, false, None, &mut write_target);
        assert_eq!(n, 4);
    }

    #[test]
    fn a_lower_limit_sends_fewer_omni_lights() {
        let lamps: Vec<BriefLamp> = (1..=6).map(|i| omni_at(i as f32, 10.0)).collect();
        let mut write_target = blank();
        let n = forward_scene_dynamic_lights_to_renderer(&lamps, ORIGIN, 2, false, None, &mut write_target);
        assert_eq!(origins(&write_target, n), [1.0, 2.0]);
    }

    #[test]
    fn a_visible_spot_goes_first() {
        let mut spot = spot_at(5.0, 10.0);
        spot.light.origin[2] = 3.0;
        let lamps = [omni_at(1.0, 10.0), spot];
        let mut write_target = blank();
        let n = forward_scene_dynamic_lights_to_renderer(&lamps, ORIGIN, 4, true, Some(&FLOOR), &mut write_target);
        assert_eq!(n, 2);
        assert_eq!(origins(&write_target, n), [5.0, 1.0]);
        assert_eq!(write_target[0].kind, CONE_LAMP);
    }

    #[test]
    fn spot_lights_need_sm3_and_a_frustum() {
        let lamps = [spot_at(5.0, 10.0)];
        let mut write_target = blank();
        assert_eq!(
            forward_scene_dynamic_lights_to_renderer(&lamps, ORIGIN, 4, false, Some(&FLOOR), &mut write_target),
            0
        );
        assert_eq!(
            forward_scene_dynamic_lights_to_renderer(&lamps, ORIGIN, 4, true, None, &mut write_target),
            0
        );
        assert_eq!(
            forward_scene_dynamic_lights_to_renderer(&lamps, ORIGIN, 4, true, Some(&[]), &mut write_target),
            0
        );
    }

    #[test]
    fn a_spot_wholly_behind_a_plane_is_dropped() {
        let mut behind = spot_at(5.0, 10.0);
        behind.light.origin[2] = -100.0;
        let mut grazing = spot_at(6.0, 10.0);
        grazing.light.origin[2] = -5.0;
        let mut write_target = blank();
        let n =
            forward_scene_dynamic_lights_to_renderer(&[behind], ORIGIN, 4, true, Some(&FLOOR), &mut write_target);
        assert_eq!(n, 0);
        let n =
            forward_scene_dynamic_lights_to_renderer(&[grazing], ORIGIN, 4, true, Some(&FLOOR), &mut write_target);
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
        let lamps = [dead, far, near];
        let mut write_target = blank();
        let n = forward_scene_dynamic_lights_to_renderer(&lamps, ORIGIN, 4, true, Some(&FLOOR), &mut write_target);
        assert_eq!(origins(&write_target, n), [20.0]);
    }

    #[test]
    fn a_used_spot_is_not_sent() {
        let mut spot = spot_at(5.0, 10.0);
        spot.light.origin[2] = 1.0;
        spot.claimed = true;
        let mut write_target = blank();
        let n = forward_scene_dynamic_lights_to_renderer(&[spot], ORIGIN, 4, true, Some(&FLOOR), &mut write_target);
        assert_eq!(n, 0);
    }

    #[test]
    fn a_short_destination_is_filled_and_no_more() {
        let lamps = [omni_at(1.0, 10.0), omni_at(2.0, 10.0), omni_at(3.0, 10.0)];
        let mut write_target = blank();
        let n = forward_scene_dynamic_lights_to_renderer(&lamps, ORIGIN, 4, false, None, &mut write_target[..2]);
        assert_eq!(n, 2);
        assert_eq!(origins(&write_target, 2), [1.0, 2.0]);
    }

    #[test]
    fn only_the_first_thirty_two_omni_lights_are_considered() {
        let mut lamps: Vec<BriefLamp> = (0..33).map(|_| omni_at(100.0, 10.0)).collect();
        // The nearest light is the 33rd, past the scan limit.
        lamps[32] = omni_at(1.0, 10.0);
        let mut write_target = blank();
        let n = forward_scene_dynamic_lights_to_renderer(&lamps, ORIGIN, 1, false, None, &mut write_target);
        assert_eq!(origins(&write_target, n), [100.0]);
    }
}
