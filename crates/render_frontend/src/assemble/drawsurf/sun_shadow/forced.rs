//! The sun's two cascades as the engine forces them: both in one atlas, the
//! near one four times as fine as the far one, fitted to the camera.

use bevy::math::{Mat4, Vec3};
use render_frame::{SUN_SHADOW_FORCED_PROFILE, SunShadowForcedFrame, SunShadowPartition};

use super::axes::{
    scene_extents_along_dir, snap_sun_proj_origin, sun_axes_from_dir,
    sun_shadow_packed_axes_from_light_dir, sun_shadow_view_matrix, view_org_in_sun_proj,
};
use super::projection::{
    SunShadowMapMetrics, SunShadowProjectionSetup, shadowmap_polygon_offset,
    sun_shadow_lookup_matrix, sun_shadow_ortho_projection, sun_shadow_projection_fit,
    sun_shadow_projection_m22, sun_shadowmap_pixel_adjust,
};
use super::{
    SM_SUN_PARTITION_RATIO, SM_SUN_SAMPLE_SIZE_NEAR_DEFAULT, SunShadowClipInput,
    sun_shadow_clip_planes, sun_shadow_frustum_rays,
};

/// The camera a cascade set is fitted to.
#[derive(Clone, Copy, Debug)]
pub struct SunShadowCamera {
    pub origin: [f32; 3],
    pub forward: [f32; 3],
    pub right: [f32; 3],
    pub up: [f32; 3],
    pub tan_half_fov_x: f32,
    pub tan_half_fov_y: f32,

    /// Distance from the eye to the near clip plane.
    pub z_near: f32,
}

/// The camera frustum's four corner rays, unit length, round the view
/// direction in the order (right, up), (left, up), (left, down), (right, down).
fn sun_shadow_corner_rays(camera: SunShadowCamera) -> [[f32; 3]; 4] {
    let forward = Vec3::from_array(camera.forward);
    let right = Vec3::from_array(camera.right) * camera.tan_half_fov_x;
    let up = Vec3::from_array(camera.up) * camera.tan_half_fov_y;
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
pub const fn forced_fallback_metrics() -> SunShadowMapMetrics {
    SunShadowMapMetrics {
        pixels_per_tile: SUN_SHADOW_FORCED_PROFILE.partition_size(),
        tiles_per_texture: SUN_SHADOW_FORCED_PROFILE.atlas_columns(),
        min_coord: 0,
    }
}

/// Everything a frame needs to draw the sun's shadow: both cascades fitted to
/// `camera`, a scene box (`world_mid` and `world_half`) to cover, and the
/// constants the receivers read it back with.
///
/// `sun_direction` points at the sun.
pub fn forced_fallback_frame(
    sun_direction: [f32; 3],
    camera: SunShadowCamera,
    world_mid: [f32; 3],
    world_half: [f32; 3],
) -> SunShadowForcedFrame {
    let profile = SUN_SHADOW_FORCED_PROFILE;
    let tile = profile.partition_size();
    let to_light = Vec3::from_array(sun_direction);

    let axes = sun_axes_from_dir(sun_direction);
    let packed_axes = sun_shadow_packed_axes_from_light_dir((-to_light).to_array());
    let eye_on_plane = view_org_in_sun_proj(camera.origin, axes);

    let sample_near = SM_SUN_SAMPLE_SIZE_NEAR_DEFAULT;
    let sample_far = sample_near * SM_SUN_PARTITION_RATIO;
    let sample_size = [sample_near, sample_far];

    let rays = sun_shadow_frustum_rays(sun_shadow_corner_rays(camera), packed_axes);
    let fit = sun_shadow_projection_fit(&rays, camera.forward, tile as f32);
    let snapped = [
        snap_sun_proj_origin(eye_on_plane, sample_near),
        snap_sun_proj_origin(eye_on_plane, sample_far),
    ];
    let projection_offset_clip = fit
        .pixel_center
        .map(|centre| centre * 2.0 / tile as f32 - 1.0);

    // Where the near cascade stops: its coarsest axis, along the view.
    let near_shadow_min_dist = sample_near * fit.scale[0].min(fit.scale[1]) * fit.min_forward_dot;
    assert!(
        near_shadow_min_dist > 0.0,
        "sun-shadow near partition has no forward extent"
    );

    let (near_clip, far_clip) = scene_extents_along_dir(axes[0], world_mid, world_half);
    let depth_scale = sun_shadow_projection_m22(near_clip, far_clip);
    let clip_planes = sun_shadow_clip_planes(
        &SunShadowClipInput {
            packed_axes,
            camera_origin: camera.origin,
            camera_forward: camera.forward,
            camera_near_distance: camera.z_near,
            near_shadow_min_distance: near_shadow_min_dist,
            shadow_origin: eye_on_plane,
            shadow_origin_pixel_center: fit.pixel_center,
            snapped_shadow_origin: snapped,
            sample_size,
            useful_size: tile,
        },
        &rays,
    );
    let partitions: [SunShadowPartition; 2] = std::array::from_fn(|index| {
        let view = sun_shadow_view_matrix(axes, snapped[index]);
        let projection = sun_shadow_ortho_projection(
            projection_offset_clip,
            sample_size[index],
            near_clip,
            far_clip,
        );
        SunShadowPartition {
            clip_from_world: projection * view,
            view,
            projection,
            polygon_offset: shadowmap_polygon_offset(depth_scale),
            viewport: profile
                .partition_viewport(index as u32)
                .expect("forced fallback has two partitions"),
            clip_planes: clip_planes[index],
        }
    });

    let setup = SunShadowProjectionSetup {
        near_shadow_min_dist,
        shadow_org_pixel_center: fit.pixel_center,
        snapped_shadow_org: snapped,
        sample_size,
    };

    // The receivers work relative to the eye, so the plane that tells the
    // cascades apart is moved there too.
    let [slope_x, slope_y, slope_z, offset] =
        setup.partition_fraction(camera.origin, camera.forward);
    let eye = Vec3::from_array(camera.origin);
    let partition_fraction = [
        slope_x,
        slope_y,
        slope_z,
        offset + eye.x * slope_x + eye.y * slope_y + eye.z * slope_z,
    ];
    let receiver_clip = partitions[0].clip_from_world * Mat4::from_translation(eye);

    SunShadowForcedFrame {
        partitions,
        pixel_adjust: sun_shadowmap_pixel_adjust(profile),
        lookup: sun_shadow_lookup_matrix(receiver_clip, partition_fraction, profile),
        receiver: setup.receiver_constants(forced_fallback_metrics(), false, 1.0),
        partition_fraction,
        axes,
        view_forward: camera.forward,
        view_right: camera.right,
        tan_half_fov: [camera.tan_half_fov_x, camera.tan_half_fov_y],
        near_shadow_min_dist,
        shadow_org_pixel_center: fit.pixel_center,
        projection_offset_clip,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An eye 10 up, looking along +x, under a sun overhead and a bit to the
    /// side.
    fn camera() -> SunShadowCamera {
        SunShadowCamera {
            origin: [100.0, -40.0, 10.0],
            forward: [1.0, 0.0, 0.0],
            right: [0.0, -1.0, 0.0],
            up: [0.0, 0.0, 1.0],
            tan_half_fov_x: 1.0,
            tan_half_fov_y: 0.75,
            z_near: 4.0,
        }
    }

    fn frame() -> SunShadowForcedFrame {
        forced_fallback_frame(
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
        flat.tan_half_fov_x = 0.0;
        flat.tan_half_fov_y = 0.0;
        for ray in sun_shadow_corner_rays(flat) {
            assert_eq!(ray, [1.0, 0.0, 0.0]);
        }
        // Nothing to look at at all: forward is kept, even if it is nothing.
        let mut nothing = flat;
        nothing.forward = [0.0; 3];
        for ray in sun_shadow_corner_rays(nothing) {
            assert_eq!(ray, [0.0; 3]);
        }
    }

    #[test]
    fn the_forced_atlas_is_two_big_tiles_in_a_column() {
        let metrics = forced_fallback_metrics();
        assert_eq!(metrics.pixels_per_tile, 1024);
        assert_eq!(metrics.tiles_per_texture, 2);
        assert_eq!(metrics.min_coord, 0);
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
                partition.clip_from_world,
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
        assert_eq!(near.polygon_offset, far.polygon_offset);
    }

    #[test]
    fn the_eye_lands_where_the_fit_put_it() {
        let frame = frame();
        let eye = Vec3::new(100.0, -40.0, 10.0);
        let clip = frame.partitions[0].clip_from_world.project_point3(eye);
        // The snapped eye is within a texel of the true one, so it lands within
        // a texel of the tile's fitted centre.
        let texel = 2.0 / 1024.0;
        assert!(
            (clip.x - frame.projection_offset_clip[0]).abs() <= texel * 1.5,
            "{clip}"
        );
        assert!(
            (clip.y - frame.projection_offset_clip[1]).abs() <= texel * 1.5,
            "{clip}"
        );
        assert!((0.0..=1.0).contains(&clip.z), "{clip}");
    }

    #[test]
    fn the_camera_is_passed_through() {
        let frame = frame();
        assert_eq!(frame.view_forward, [1.0, 0.0, 0.0]);
        assert_eq!(frame.view_right, [0.0, -1.0, 0.0]);
        assert_eq!(frame.tan_half_fov, [1.0, 0.75]);
        assert_eq!(frame.axes, sun_axes_from_dir([0.3, 0.2, 0.9]));
        assert_eq!(
            frame.pixel_adjust,
            sun_shadowmap_pixel_adjust(SUN_SHADOW_FORCED_PROFILE)
        );
    }

    #[test]
    fn the_near_cascade_ends_ahead_of_the_eye() {
        let frame = frame();
        assert!(frame.near_shadow_min_dist > 0.0);
        // The partition plane is zero at the eye (it is relative to it) and one
        // that far ahead.
        let fraction = frame.partition_fraction;
        assert!((fraction[3]).abs() < 1e-3, "{fraction:?}");
        let ahead = fraction[0] * frame.near_shadow_min_dist;
        assert!((ahead - 1.0).abs() < 1e-4, "{ahead}");
    }

    #[test]
    fn the_receiver_switches_inside_the_atlas() {
        let receiver = frame().receiver;
        assert!((0.0..1.0).contains(&receiver.switch_partition[0]));
        assert!((0.0..2.0).contains(&receiver.switch_partition[1]));
        assert_eq!(receiver.switch_partition[2..], [1.0, 0.25]);
        assert_eq!(receiver.shadowmap_scale, [16.0, 32.0, -8.0, -24.0]);
    }

    #[test]
    fn both_cascades_get_a_bound() {
        let [near, far] = frame().partitions;
        assert!(!near.clip_planes.as_slice().is_empty());
        assert!(!far.clip_planes.as_slice().is_empty());
    }

    #[test]
    fn a_moved_eye_snaps_to_the_same_texel_grid() {
        let still = frame();
        let mut nudged = camera();
        nudged.origin[1] += 0.01;
        let nudged = forced_fallback_frame(
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
    #[should_panic(expected = "zero span")]
    fn a_camera_with_nothing_to_look_at_cannot_be_fitted() {
        let mut blind = camera();
        blind.forward = [0.0; 3];
        blind.tan_half_fov_x = 0.0;
        blind.tan_half_fov_y = 0.0;
        let _ = forced_fallback_frame([0.3, 0.2, 0.9], blind, [0.0; 3], [100.0; 3]);
    }
}
