//! Deciding whether an element is worth spawning or drawing at all.

use crate::runtime::flags::elem_spawn_frustum_cull;
use crate::runtime::rotation::FX_RECIP_255;

/// The flag that limits drawing culls to five of the view's planes.
const CULL_AGAINST_FIVE_PLANES: i32 = 0x400;

/// How many planes a draw cull tests.
const FIVE_PLANES: u32 = 5;

/// Whether an element may spawn, given how far it is from the camera and
/// whether a sphere around its spawn point lies outside the view. An element
/// with a spawn range only spawns while the camera is inside it.
pub fn cull_elem_for_spawn_allows(
    spawn_range_base: f32,
    spawn_range_amplitude: f32,
    camera_distance_inches: f32,
    flags: i32,
    sphere_outside_frustum: bool,
) -> bool {
    if spawn_range_amplitude != 0.0 {
        let past_start = camera_distance_inches - spawn_range_base;
        if past_start < 0.0 || spawn_range_amplitude < past_start {
            return false;
        }
    }
    !(elem_spawn_frustum_cull(flags) && sphere_outside_frustum)
}

/// How many of the view's planes an element is tested against.
const fn cull_plane_count(elem_flags: i32, view_plane_count: u32) -> u32 {
    if elem_flags & CULL_AGAINST_FIVE_PLANES != 0 {
        FIVE_PLANES
    } else {
        view_plane_count
    }
}

/// The radius of a cloud element's culling sphere: its larger size plus the
/// scale.
fn cull_cloud_radius(size0: f32, size1: f32, scale: f32) -> f32 {
    let mut radius = size1;
    if radius < size0 {
        radius = size0;
    }
    radius + scale
}

/// Whether a sphere lies wholly behind any of the first `plane_count`
/// planes. Each plane is `[nx, ny, nz, d]` with its normal pointing inside.
fn cull_sphere(planes: &[[f32; 4]], plane_count: u32, pos: [f32; 3], radius: f32) -> bool {
    let limit = -radius;
    planes.iter().take(plane_count as usize).any(|p| {
        let side = p[0] * pos[0] + p[1] * pos[1] + p[2] * pos[2] + p[3];
        side < limit
    })
}

/// Whether a cloud element is outside the view and need not be drawn.
/// `cull_elem_draw` is the system-wide switch for culling.
#[allow(clippy::too_many_arguments)]
pub fn cull_cloud(
    cull_elem_draw: bool,
    planes: &[[f32; 4]],
    view_plane_count: u32,
    elem_flags: i32,
    pos: [f32; 3],
    size0: f32,
    size1: f32,
    scale: f32,
) -> bool {
    if !cull_elem_draw {
        return false;
    }
    let count = cull_plane_count(elem_flags, view_plane_count);
    cull_sphere(planes, count, pos, cull_cloud_radius(size0, size1, scale))
}

/// Whether a light element is outside the view and need not be drawn.
pub fn cull_elem_light(
    cull_elem_draw: bool,
    planes: &[[f32; 4]],
    view_plane_count: u32,
    elem_flags: i32,
    pos: [f32; 3],
    size0: f32,
) -> bool {
    if !cull_elem_draw {
        return false;
    }
    let count = cull_plane_count(elem_flags, view_plane_count);
    cull_sphere(planes, count, pos, size0)
}

/// A light element's colour as blue, green, red floats, scaled by `scale`.
pub fn elem_light_color_bgr(color_rgba: [u8; 4], scale: f32) -> [f32; 3] {
    let per_step = scale * (FX_RECIP_255 as f32);
    [
        f32::from(color_rgba[2]) * per_step,
        f32::from(color_rgba[1]) * per_step,
        f32::from(color_rgba[0]) * per_step,
    ]
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
        use crate::runtime::flags::FX_ELEM_SPAWN_FRUSTUM_CULL;
        assert!(cull_elem_for_spawn_allows(0.0, 0.0, 0.0, 0, true));
        assert!(!cull_elem_for_spawn_allows(
            0.0,
            0.0,
            0.0,
            FX_ELEM_SPAWN_FRUSTUM_CULL,
            true
        ));
        assert!(cull_elem_for_spawn_allows(
            0.0,
            0.0,
            0.0,
            FX_ELEM_SPAWN_FRUSTUM_CULL,
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
