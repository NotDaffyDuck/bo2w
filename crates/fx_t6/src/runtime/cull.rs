//! Deciding whether an element is worth spawning or drawing at all.

use crate::runtime::mode_bits::elem_spawn_frustum_cull;
use crate::runtime::angle_graph::FX_RECIP_255;

/// The flag that limits drawing culls to five of the view's planes.
const CULL_AGAINST_FIVE_PLANES: i32 = 0x400;

/// How many planes a draw cull tests.
const FIVE_PLANES: u32 = 5;

/// Whether an element may spawn, given how far it is from the camera and
/// whether a sphere around its spawn point lies outside the view. An element
/// with a spawn range only spawns while the camera is inside it.
pub fn cull_elem_for_spawn_allows(
    birth_span_floor: f32,
    birth_span_swing: f32,
    eye_gap: f32,
    flags: i32,
    ball_outside: bool,
) -> bool {
    let past_start = eye_gap - birth_span_floor;
    let out_of_range =
        birth_span_swing != 0.0 && (past_start < 0.0 || birth_span_swing < past_start);
    let culled_by_view = elem_spawn_frustum_cull(flags) && ball_outside;
    !(out_of_range || culled_by_view)
}

/// How many of the view's planes an element is tested against.
const fn cull_plane_count(elem_flags: i32, sight_wall_total: u32) -> u32 {
    match elem_flags & CULL_AGAINST_FIVE_PLANES {
        0 => sight_wall_total,
        _ => FIVE_PLANES,
    }
}

/// The radius of a cloud element's culling sphere: its larger size plus the
/// scale.
fn cull_cloud_radius(start_extent: f32, second_extent: f32, factor: f32) -> f32 {
    let larger = if start_extent > second_extent { start_extent } else { second_extent };
    larger + factor
}

/// Whether a sphere lies wholly behind any of the first `plane_tally`
/// planes. Each plane is `[facing_x, facing_y, facing_z, d]` with its normal pointing inside.
fn cull_sphere(planes: &[[f32; 4]], plane_tally: u32, pos: [f32; 3], reach: f32) -> bool {
    planes[..planes.len().min(plane_tally as usize)]
        .iter()
        .map(|&[a, b, c, d]| a * pos[0] + b * pos[1] + c * pos[2] + d)
        .any(|signed_distance| signed_distance < -reach)
}

/// Whether a cloud element is outside the view and need not be drawn.
/// `cull_each` is the system-wide switch for culling.
#[allow(clippy::too_many_arguments)]
pub fn cull_cloud(
    cull_each: bool,
    walls: &[[f32; 4]],
    sight_wall_total: u32,
    elem_bits: i32,
    pos: [f32; 3],
    start_extent: f32,
    second_extent: f32,
    scale: f32,
) -> bool {
    cull_each
        && cull_sphere(
            walls,
            cull_plane_count(elem_bits, sight_wall_total),
            pos,
            cull_cloud_radius(start_extent, second_extent, scale),
        )
}

/// Whether a light element is outside the view and need not be drawn.
pub fn cull_elem_light(
    cull_each: bool,
    walls: &[[f32; 4]],
    sight_wall_total: u32,
    elem_bits: i32,
    pos: [f32; 3],
    start_extent: f32,
) -> bool {
    cull_each
        && cull_sphere(
            walls,
            cull_plane_count(elem_bits, sight_wall_total),
            pos,
            start_extent,
        )
}

/// A light element's colour as blue, green, red floats, scaled by `factor`.
pub fn elem_light_color_bgr(color_rgba: [u8; 4], factor: f32) -> [f32; 3] {
    let per_step = factor * (FX_RECIP_255 as f32);
    [2, 1, 0].map(|channel| f32::from(color_rgba[channel]) * per_step)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A single plane facing +x through x = 10.
    const WALL: [[f32; 4]; 1] = [[1.0, 0.0, 0.0, -10.0]];

    #[test]
    fn a_sphere_wholly_behind_a_plane_is_culled() {
        assert!(cull_sphere(&WALL, 1, [0.0, 0.0, 0.0], 5.0));
        assert!(!cull_sphere(&WALL, 1, [0.0, 0.0, 0.0], 10.5));
        assert!(!cull_sphere(&WALL, 1, [20.0, 0.0, 0.0], 5.0));
    }

    #[test]
    fn no_planes_cull_nothing() {
        assert!(!cull_sphere(&WALL, 0, [0.0; 3], 1.0));
        assert!(!cull_sphere(&[], 3, [0.0; 3], 1.0));
    }

    #[test]
    fn only_the_first_plane_count_planes_are_tested() {
        let planes = [[0.0, 0.0, 1.0, 100.0], [1.0, 0.0, 0.0, -10.0]];
        assert!(!cull_sphere(&planes, 1, [0.0; 3], 1.0));
        assert!(cull_sphere(&planes, 2, [0.0; 3], 1.0));
    }

    #[test]
    fn the_five_plane_flag_overrides_the_view_count() {
        assert_eq!(cull_plane_count(0, 6), 6);
        assert_eq!(cull_plane_count(CULL_AGAINST_FIVE_PLANES, 6), 5);
    }

    #[test]
    fn a_cloud_is_never_culled_when_culling_is_off() {
        assert!(!cull_cloud(false, &WALL, 1, 0, [0.0; 3], 1.0, 1.0, 0.0));
        assert!(cull_cloud(true, &WALL, 1, 0, [0.0; 3], 1.0, 1.0, 0.0));
    }

    #[test]
    fn a_cloud_is_culled_by_its_larger_size_plus_scale() {
        assert_eq!(cull_cloud_radius(2.0, 3.0, 1.0), 4.0);
        assert_eq!(cull_cloud_radius(5.0, 3.0, 1.0), 6.0);
        // Radius 6 reaches past the wall at 10 from the origin? No: 6 < 10.
        assert!(cull_cloud(true, &WALL, 1, 0, [0.0; 3], 5.0, 3.0, 1.0));
        assert!(!cull_cloud(true, &WALL, 1, 0, [0.0; 3], 5.0, 12.0, 1.0));
    }

    #[test]
    fn a_light_is_culled_by_its_size() {
        assert!(cull_elem_light(true, &WALL, 1, 0, [0.0; 3], 9.0));
        assert!(!cull_elem_light(true, &WALL, 1, 0, [0.0; 3], 11.0));
        assert!(!cull_elem_light(false, &WALL, 1, 0, [0.0; 3], 9.0));
    }

    #[test]
    fn spawn_range_keeps_out_cameras_before_it_and_past_it() {
        let allows = |dist| cull_elem_for_spawn_allows(100.0, 500.0, dist, 0, false);
        assert!(!allows(50.0));
        assert!(allows(100.0));
        assert!(allows(600.0));
        assert!(!allows(601.0));
    }

    #[test]
    fn no_spawn_range_allows_any_distance() {
        assert!(cull_elem_for_spawn_allows(100.0, 0.0, 1.0e9, 0, false));
    }

    #[test]
    fn the_frustum_test_only_applies_when_the_element_asks_for_it() {
        use crate::runtime::mode_bits::ELEM_CULL_AT_BIRTH;
        assert!(cull_elem_for_spawn_allows(0.0, 0.0, 0.0, 0, true));
        assert!(!cull_elem_for_spawn_allows(
            0.0,
            0.0,
            0.0,
            ELEM_CULL_AT_BIRTH,
            true
        ));
        assert!(cull_elem_for_spawn_allows(
            0.0,
            0.0,
            0.0,
            ELEM_CULL_AT_BIRTH,
            false
        ));
    }

    #[test]
    fn light_colour_comes_out_blue_green_red() {
        let got = elem_light_color_bgr([255, 0, 51, 7], 1.0);
        assert!((got[0] - 0.2).abs() < 1e-6);
        assert_eq!(got[1], 0.0);
        assert!((got[2] - 1.0).abs() < 1e-6);
    }
}
