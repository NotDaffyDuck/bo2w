//! The sun's two cascades as the engine forces them: both in one atlas, the
//! near one four times as fine as the far one, fitted to the camera.

use bevy::math::{Mat4, Vec3};
use render_frame::{DAYLIGHT_SHADOW_FORCED_PROFILE, DaylightShadowForcedFrame, SunCascadeSlice};

use super::axes::{
    stage_extents_along_dir, snap_daylight_proj_origin, daylight_axes_from_dir,
    daylight_shadow_compact_axes_from_light_dir, sun_shadow_view_matrix, eye_org_in_daylight_proj,
};
use super::proj_m::{
    DaylightShadowMapMetrics, DaylightShadowProjectionSetup, shadow_map_polygon_offset,
    daylight_shadow_lookup_matrix, daylight_shadow_ortho_projection, sun_shadow_projection_fit,
    sun_shadow_projection_m22, daylight_shadow_map_pixel_adjust,
};
use super::{
    SM_DAYLIGHT_PARTITION_RATIO, SM_DAYLIGHT_SAMPLE_SIZE_NEAR_DEFAULT, SunShadowClipInput,
    sun_shadow_clip_planes, sun_shadow_frustum_rays,
};

/// The camera a cascade set is fitted to.
#[derive(Clone, Copy, Debug)]
pub struct DaylightShadowCamera {
    pub at_point: [f32; 3],
    pub fwd: [f32; 3],
    pub right_v: [f32; 3],
    pub up_v: [f32; 3],
    pub fov_half_tan_x: f32,
    pub fov_half_tan_y: f32,

    /// Distance from the eye to the near clip plane.
    pub front_clip_depth: f32,
}

/// The camera frustum's four corner rays, unit length, round the view
/// direction in the order (right, up), (left, up), (left, down), (right, down).
fn sun_shadow_corner_rays(camera: DaylightShadowCamera) -> [[f32; 3]; 4] {
    let forward = Vec3::from_array(camera.fwd);
    let right = Vec3::from_array(camera.right_v) * camera.fov_half_tan_x;
    let up = Vec3::from_array(camera.up_v) * camera.fov_half_tan_y;
    [(1.0, 1.0), (-1.0, 1.0), (-1.0, -1.0), (1.0, -1.0)].map(|(across, rise)| {
        let ray = forward + right * across + up * rise;
        // A degenerate camera looks straight ahead.
        let unit = if ray.length_squared() > 1e-12 {
            ray.normalize()
        } else {
            forward
        };
        unit.to_array()
    })
}

/// How the forced atlas is cut up.
pub const fn forced_fallback_metrics() -> DaylightShadowMapMetrics {
    DaylightShadowMapMetrics {
        tile_pixel_span: DAYLIGHT_SHADOW_FORCED_PROFILE.split_size(),
        tiles_per_atlas_page: DAYLIGHT_SHADOW_FORCED_PROFILE.atlas_tiles_across(),
        low_coord: 0,
    }
}

/// Everything a frame needs to draw the sun's shadow: both cascades fitted to
/// `camera`, a scene box (`level_mid` and `level_half`) to cover, and the
/// constants the receivers read it back with.
///
/// `daylight_direction` points at the sun.
pub fn forced_backup_frame(
    daylight_direction: [f32; 3],
    camera: DaylightShadowCamera,
    level_mid: [f32; 3],
    level_half: [f32; 3],
) -> DaylightShadowForcedFrame {
    let profile = DAYLIGHT_SHADOW_FORCED_PROFILE;
    let tile = profile.split_size();
    let to_light = Vec3::from_array(daylight_direction);

    let axes = daylight_axes_from_dir(to_light);
    let packed_axes = daylight_shadow_compact_axes_from_light_dir(-to_light);
    let eye_on_plane = eye_org_in_daylight_proj(camera.at_point, axes);

    let near_sample = SM_DAYLIGHT_SAMPLE_SIZE_NEAR_DEFAULT;
    let tap_far = near_sample * SM_DAYLIGHT_PARTITION_RATIO;
    let tap_size = [near_sample, tap_far];

    let rays = sun_shadow_frustum_rays(sun_shadow_corner_rays(camera), packed_axes);
    let fit = sun_shadow_projection_fit(&rays, camera.fwd, tile as f32);
    let snapped_to = [
        snap_daylight_proj_origin(eye_on_plane, near_sample),
        snap_daylight_proj_origin(eye_on_plane, tap_far),
    ];
    let proj_offset_clip = fit
        .pixel_center
        .map(|centre| centre * 2.0 / tile as f32 - 1.0);

    // Where the near cascade stops: its coarsest axis, along the view.
    let near_shadow_floor = near_sample * fit.size_k[0].min(fit.size_k[1]) * fit.min_forward_dot;
    assert!(
        near_shadow_floor > 0.0,
        "sun-shadow near partition reaches nothing ahead of the eye"
    );

    let [near_cut, far_clip] = stage_extents_along_dir(axes[0], level_mid, level_half);
    let depth_scale = sun_shadow_projection_m22(near_cut, far_clip);
    let frustum_clip_planes_for = sun_shadow_clip_planes(
        &SunShadowClipInput {
            packed_axes,
            camera_origin: camera.at_point,
            camera_forward: camera.fwd,
            camera_near_distance: camera.front_clip_depth,
            near_shadow_min_distance: near_shadow_floor,
            shadow_origin: eye_on_plane,
            shadow_origin_pixel_center: fit.pixel_center,
            snapped_shadow_origin: snapped_to,
            probe_size: tap_size,
            useful_size: tile,
        },
        &rays,
    );
    let parts: [SunCascadeSlice; 2] = std::array::from_fn(|index| {
        let view_pos = sun_shadow_view_matrix(axes, snapped_to[index]);
        let proj_m = daylight_shadow_ortho_projection(
            proj_offset_clip,
            tap_size[index],
            near_cut,
            far_clip,
        );
        SunCascadeSlice {
            world_to_clip: proj_m * view_pos,
            view: view_pos,
            projection: proj_m,
            depth_bias: shadow_map_polygon_offset(depth_scale),
            viewport: profile
                .slice_rect(index as u32)
                .expect("the forced fallback lays out two partitions"),
            frustum_clip_planes_for: frustum_clip_planes_for[index],
        }
    });

    let setup = DaylightShadowProjectionSetup {
        near_shadow_floor,
        shadow_origin_texel_centre: fit.pixel_center,
        snapped_shadow_centre: snapped_to,
        probe_size: tap_size,
    };

    // The receivers work relative to the eye, so the plane that tells the
    // cascades apart is moved there too.
    let [slope_x, slope_y, slope_z, offset] =
        setup.split_fraction(camera.at_point, camera.fwd);
    let eye = Vec3::from_array(camera.at_point);
    let split_fraction = [
        slope_x,
        slope_y,
        slope_z,
        offset + eye.x * slope_x + eye.y * slope_y + eye.z * slope_z,
    ];
    let receiver_clip = parts[0].world_to_clip * Mat4::from_translation(eye);

    DaylightShadowForcedFrame {
        partitions: parts,
        texel_offset_correction: daylight_shadow_map_pixel_adjust(profile),
        lookup: daylight_shadow_lookup_matrix(receiver_clip, split_fraction, profile),
        receiver: setup.receiver_consts(forced_fallback_metrics(), false, 1.0),
        split_fraction,
        axes,
        eye_forward: camera.fwd,
        eye_right: camera.right_v,
        fov_half_tan: [camera.fov_half_tan_x, camera.fov_half_tan_y],
        near_shadow_floor,
        shadow_origin_texel_centre: fit.pixel_center,
        proj_offset_clip,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An eye 10 up, looking along +x, under a sun overhead and a bit to the
    /// side.
    fn camera() -> DaylightShadowCamera {
        DaylightShadowCamera {
            at_point: [100.0, -40.0, 10.0],
            fwd: [1.0, 0.0, 0.0],
            right_v: [0.0, -1.0, 0.0],
            up_v: [0.0, 0.0, 1.0],
            fov_half_tan_x: 1.0,
            fov_half_tan_y: 0.75,
            front_clip_depth: 4.0,
        }
    }

    fn frame() -> DaylightShadowForcedFrame {
        forced_backup_frame(
            [0.3, 0.2, 0.9],
            camera(),
            [0.0, 0.0, 100.0],
            [2000.0, 2000.0, 200.0],
        )
    }

    #[test]
    fn the_corner_rays_go_round_the_view_direction() {
        let rays = sun_shadow_corner_rays(camera()).map(Vec3::from_array);
        for ray in rays {
            assert!((ray.length() - 1.0).abs() < 1e-5);
            assert!(ray.x > 0.0, "all point ahead");
        }
        // (right, up) is -y and +z for this camera.
        assert!(rays[0].y < 0.0 && rays[0].z > 0.0);
        assert!(rays[1].y > 0.0 && rays[1].z > 0.0);
        assert!(rays[2].y > 0.0 && rays[2].z < 0.0);
        assert!(rays[3].y < 0.0 && rays[3].z < 0.0);
        // Adjacent corners share an edge: they differ in one sign only.
        assert!((rays[0].z - rays[1].z).abs() < 1e-6);
        assert!((rays[1].y - rays[2].y).abs() < 1e-6);
    }

    #[test]
    fn a_camera_with_no_field_of_view_looks_straight_ahead() {
        let mut flat = camera();
        flat.fov_half_tan_x = 0.0;
        flat.fov_half_tan_y = 0.0;
        for ray in sun_shadow_corner_rays(flat) {
            assert_eq!(ray, [1.0, 0.0, 0.0]);
        }
        // Nothing to look at at all: forward is kept, even if it is nothing.
        let mut nothing = flat;
        nothing.fwd = [0.0; 3];
        for ray in sun_shadow_corner_rays(nothing) {
            assert_eq!(ray, [0.0; 3]);
        }
    }

    #[test]
    fn the_forced_atlas_is_two_big_tiles_in_a_column() {
        let metrics = forced_fallback_metrics();
        assert_eq!(metrics.tile_pixel_span, 1024);
        assert_eq!(metrics.tiles_per_atlas_page, 2);
        assert_eq!(metrics.low_coord, 0);
    }

    #[test]
    fn each_cascade_has_its_own_tile() {
        let frame = frame();
        let [near, far] = frame.partitions;
        assert_eq!((near.viewport.x, near.viewport.y), (0, 0));
        assert_eq!((far.viewport.x, far.viewport.y), (0, 1024));
        for partition in [near, far] {
            assert_eq!(
                (partition.viewport.width, partition.viewport.height),
                (1024, 1024)
            );
            assert_eq!(
                partition.world_to_clip,
                partition.projection * partition.view
            );
        }
    }

    #[test]
    fn the_far_cascade_is_four_times_as_coarse() {
        let [near, far] = frame().partitions;
        let ratio = near.projection.x_axis.x / far.projection.x_axis.x;
        assert!((ratio - 4.0).abs() < 1e-4, "{ratio}");
        // Both look along the same depth range.
        assert_eq!(near.projection.z_axis.z, far.projection.z_axis.z);
        assert_eq!(near.depth_bias, far.depth_bias);
    }

    #[test]
    fn the_eye_lands_where_the_fit_put_it() {
        let frame = frame();
        let eye = Vec3::new(100.0, -40.0, 10.0);
        let clip = frame.partitions[0].world_to_clip.project_point3(eye);
        // The snapped eye is within a texel of the true one, so it lands within
        // a texel of the tile's fitted centre.
        let texel = 2.0 / 1024.0;
        assert!(
            (clip.x - frame.proj_offset_clip[0]).abs() <= texel * 1.5,
            "{clip}"
        );
        assert!(
            (clip.y - frame.proj_offset_clip[1]).abs() <= texel * 1.5,
            "{clip}"
        );
        assert!((0.0..=1.0).contains(&clip.z), "{clip}");
    }

    #[test]
    fn the_camera_is_passed_through() {
        let frame = frame();
        assert_eq!(frame.eye_forward, [1.0, 0.0, 0.0]);
        assert_eq!(frame.eye_right, [0.0, -1.0, 0.0]);
        assert_eq!(frame.fov_half_tan, [1.0, 0.75]);
        assert_eq!(frame.axes, daylight_axes_from_dir(Vec3::new(0.3, 0.2, 0.9)));
        assert_eq!(
            frame.texel_offset_correction,
            daylight_shadow_map_pixel_adjust(DAYLIGHT_SHADOW_FORCED_PROFILE)
        );
    }

    #[test]
    fn the_near_cascade_ends_ahead_of_the_eye() {
        let frame = frame();
        assert!(frame.near_shadow_floor > 0.0);
        // The partition plane is zero at the eye (it is relative to it) and one
        // that far ahead.
        let fraction = frame.split_fraction;
        assert!((fraction[3]).abs() < 1e-3, "{fraction:?}");
        let ahead = fraction[0] * frame.near_shadow_floor;
        assert!((ahead - 1.0).abs() < 1e-4, "{ahead}");
    }

    #[test]
    fn the_receiver_switches_inside_the_atlas() {
        let receiver = frame().receiver;
        assert!((0.0..1.0).contains(&receiver.switch_split[0]));
        assert!((0.0..2.0).contains(&receiver.switch_split[1]));
        assert_eq!(receiver.switch_split[2..], [1.0, 0.25]);
        assert_eq!(receiver.shadow_map_scale, [16.0, 32.0, -8.0, -24.0]);
    }

    #[test]
    fn both_cascades_get_a_bound() {
        let [near, far] = frame().partitions;
        assert!(!near.frustum_clip_planes_for.as_slice().is_empty());
        assert!(!far.frustum_clip_planes_for.as_slice().is_empty());
    }

    #[test]
    fn a_moved_eye_snaps_to_the_same_texel_grid() {
        let still = frame();
        let mut nudged = camera();
        nudged.at_point[1] += 0.01;
        let nudged = forced_backup_frame(
            [0.3, 0.2, 0.9],
            nudged,
            [0.0, 0.0, 100.0],
            [2000.0, 2000.0, 200.0],
        );
        // A hundredth of a unit is far under a texel (0.25), so the cascade
        // does not move.
        assert_eq!(nudged.partitions[0].view, still.partitions[0].view);
        assert_eq!(nudged.partitions[1].view, still.partitions[1].view);
    }

    #[test]
    #[should_panic(expected = "flat along this axis")]
    fn a_camera_with_nothing_to_look_at_cannot_be_fitted() {
        let mut blind = camera();
        blind.fwd = [0.0; 3];
        blind.fov_half_tan_x = 0.0;
        blind.fov_half_tan_y = 0.0;
        let _ = forced_backup_frame([0.3, 0.2, 0.9], blind, [0.0; 3], [100.0; 3]);
    }
}
