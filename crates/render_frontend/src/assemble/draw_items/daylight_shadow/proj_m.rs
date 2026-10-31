//! A cascade's projection, how it is fitted to the camera frustum, and the
//! constants the receivers use to read it back.

use bevy::math::{Mat4, Vec3, Vec4};
use render_frame::{DaylightShadowAtlasProfile, DaylightShadowReceiverConstants};

use super::{
    DAYLIGHT_SHADOW_HALF, DAYLIGHT_SHADOW_NEG_QUARTER, DAYLIGHT_SHADOW_QUARTER, DAYLIGHT_SHADOW_SAMPLE_EXTENT_PIXELS,
    SunShadowFrustumRays,
};

/// The depth scale of the cascades' orthographic projection, for a scene that
/// spans `near_cut..far_clip` along the sun, with a texel of slack either end.
pub fn sun_shadow_projection_m22(near_cut: f32, far_plane: f32) -> f32 {
    1.0 / (far_plane - near_cut + 2.0)
}

/// The depth offset that goes with [`sun_shadow_projection_m22`]: a depth of
/// `near_cut - 1` maps to zero.
pub fn sun_shadow_projection_m32(near_cut: f32, far_plane: f32) -> f32 {
    -(near_cut - 1.0) * sun_shadow_projection_m22(near_cut, far_plane)
}

/// A cascade's orthographic projection: a square of `1024 * sample_size` world
/// units, shifted by `snapped_clip`, with the scene's depth range squeezed
/// into it.
pub fn daylight_shadow_ortho_projection(
    clip_shift: [f32; 2],
    texel_world: f32,
    near_cut: f32,
    depth_far: f32,
) -> Mat4 {
    let scale = 2.0 / (texel_world * DAYLIGHT_SHADOW_SAMPLE_EXTENT_PIXELS);
    let [shift_x, shift_y] = clip_shift;
    // Set one entry at a time so the other entries stay a plain +0.
    let mut cols = [Vec4::ZERO; 4];
    cols[0].x = scale;
    cols[1].y = scale;
    cols[2].z = sun_shadow_projection_m22(near_cut, depth_far);
    cols[3] = Vec4::new(shift_x, shift_y, sun_shadow_projection_m32(near_cut, depth_far), 1.0);
    let [x, y, z, w] = cols;
    Mat4::from_cols(x, y, z, w)
}

/// The slope-scaled depth bias a caster pass is drawn with.
pub fn shadow_map_polygon_offset(projection_m22: f32) -> [f32; 4] {
    [
        render_material::SM_POLYGON_OFFSET_BIAS_DEFAULT * DAYLIGHT_SHADOW_QUARTER * projection_m22,
        render_material::SM_POLYGON_OFFSET_SCALE_DEFAULT,
        0.0,
        0.0,
    ]
}

/// A quarter texel and half a texel, in the atlas's coordinates, that the
/// receivers add to land on texel centres.
pub fn daylight_shadow_map_pixel_adjust(profile: DaylightShadowAtlasProfile) -> [f32; 4] {
    let partition = profile.split_size() as f32;
    let atlas_px_width = profile.atlas_px_width() as f32;
    [
        DAYLIGHT_SHADOW_QUARTER / partition,
        DAYLIGHT_SHADOW_HALF / atlas_px_width,
        DAYLIGHT_SHADOW_HALF / partition,
        DAYLIGHT_SHADOW_NEG_QUARTER / atlas_px_width,
    ]
}

/// The matrix a receiver uses to turn a world position into atlas
/// coordinates, with `split_fraction` (the plane that tells the cascades
/// apart) in the last column.
///
/// Clip space is mapped to texture space: x into `0..1` across a partition,
/// y flipped and divided over the atlas's columns.
pub fn daylight_shadow_lookup_matrix(
    view_projection: Mat4,
    split_fraction: [f32; 4],
    profile: DaylightShadowAtlasProfile,
) -> Mat4 {
    let partition = profile.split_size() as f32;
    let atlas_px_width = profile.atlas_px_width() as f32;
    let atlas_tiles_across = profile.atlas_tiles_across() as f32;
    let width_ratio = DAYLIGHT_SHADOW_HALF;
    let x_shift = DAYLIGHT_SHADOW_HALF + DAYLIGHT_SHADOW_HALF / partition;
    let height_ratio = -DAYLIGHT_SHADOW_HALF / atlas_tiles_across;
    let y_shift = DAYLIGHT_SHADOW_HALF / atlas_tiles_across + DAYLIGHT_SHADOW_HALF / atlas_px_width;

    // Each output column is a row of the projection, which is what makes the
    // result take a world position (as a row vector) to texture space.
    let rows = view_projection.transpose();
    let across = rows.x_axis * width_ratio + rows.w_axis * x_shift;
    let down = rows.y_axis * height_ratio + rows.w_axis * y_shift;
    Mat4::from_cols(across, down, rows.z_axis, Vec4::from_array(split_fraction))
}

/// How the atlas is cut up, as the receivers see it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DaylightShadowMapMetrics {
    pub tile_pixel_span: u32,

    pub tiles_per_atlas_page: u32,

    pub low_coord: u32,
}

/// What the cascades' placement settled on, for the receivers.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DaylightShadowProjectionSetup {
    pub near_shadow_floor: f32,

    pub shadow_origin_texel_centre: [f32; 2],

    pub snapped_shadow_centre: [[f32; 2]; 2],

    pub probe_size: [f32; 2],
}

impl DaylightShadowProjectionSetup {
    /// The plane `forward . (p - eye_origin) / near_shadow_floor`: one at
    /// the far edge of the near cascade, which is where the receivers switch
    /// to the far one.
    pub fn split_fraction(self, eye: [f32; 3], ahead: [f32; 3]) -> [f32; 4] {
        let slope = Vec3::from_array(ahead) * (1.0 / self.near_shadow_floor);
        let [ex, ey, ez] = eye;
        let w = -(ex * slope.x + ey * slope.y + ez * slope.z);
        [slope.x, slope.y, slope.z, w]
    }

    /// The constants a receiver reads to pick a cascade and size its filter.
    pub fn receiver_consts(
        self,
        metrics: DaylightShadowMapMetrics,
        offset_on: bool,
        shadow_map_scale: f32,
    ) -> DaylightShadowReceiverConstants {
        let tile_pixel_span = metrics.tile_pixel_span as f32;
        let tiles_per_atlas_page = metrics.tiles_per_atlas_page as f32;
        let low_coord = metrics.low_coord as f32;
        let far_sample = self.probe_size[1];
        // How far the near cascade's origin sits from the far one's, in far texels.
        let shift_x = (self.snapped_shadow_centre[0][0] - self.snapped_shadow_centre[1][0]) / far_sample;
        let shift_y = (self.snapped_shadow_centre[0][1] - self.snapped_shadow_centre[1][1]) / far_sample;

        // Three quarters of the way across the near cascade, from its eye.
        let switch_x =
            (0.75 * (low_coord + self.shadow_origin_texel_centre[0]) + shift_x) / tile_pixel_span;
        let near_y = (0.75 * (low_coord + self.shadow_origin_texel_centre[1]) + shift_y) / tile_pixel_span;
        let switch_y = (1.75 - near_y) / tiles_per_atlas_page;

        DaylightShadowReceiverConstants {
            switch_split: [
                switch_x,
                switch_y,
                if offset_on { 0.0 } else { 1.0 },
                0.25,
            ],
            shadow_map_scale: [
                16.0 / shadow_map_scale,
                (16.0 * tiles_per_atlas_page) / shadow_map_scale,
                -8.0 / shadow_map_scale,
                -24.0 / shadow_map_scale,
            ],
        }
    }
}

/// Where the near cascade's eye sits in its tile and how many tile texels a
/// world unit takes up, per axis, so the camera frustum just fits.
#[derive(Clone, Copy, Debug)]
pub(super) struct DaylightShadowProjectionFit {
    pub pixel_center: [f32; 2],
    pub size_k: [f32; 2],

    /// The least any frustum corner ray points along the view's forward.
    pub min_forward_dot: f32,
}

/// One axis of the fit: the frustum spans `min..max` (eye included), a tile is
/// `pixels` wide, and the frustum has to stay a texel off each edge.
///
/// The eye is put on a texel centre, so a cascade that follows it never
/// shimmers; when the natural scale would not leave both edges on texel
/// centres too, the scale is shrunk until the side that reaches further does.
fn fit_axis(min: f32, max: f32, pixels: f32) -> (f32, f32) {
    let width = max - min;
    assert!(width > 0.0, "sun-shadow frustum is flat along this axis");
    let natural = (pixels - 2.0) / width;
    let leading = (0.5 - natural * min).ceil() + 0.5;
    let trailing = (pixels - 0.5 - natural * max).floor() - 0.5;
    if trailing == leading {
        (leading, natural)
    } else if max <= -min {
        let centre = leading - 1.0;
        (centre, (1.0 - centre) / min)
    } else {
        let centre = trailing + 1.0;
        (centre, (pixels - 1.0 - centre) / max)
    }
}

pub(super) fn sun_shadow_projection_fit(
    rays: &SunShadowFrustumRays,
    eye_forward: [f32; 3],
    tile_pixel_span: f32,
) -> DaylightShadowProjectionFit {
    let forward = Vec3::from_array(eye_forward);
    let nearest_start = f32::INFINITY;
    let least_dot = rays
        .world_rays
        .iter()
        .map(|&ray| Vec3::from_array(ray).dot(forward))
        .fold(nearest_start, f32::min);
    let x = fit_axis(rays.lo_corner[0], rays.hi_corner[0], tile_pixel_span);
    let y = fit_axis(rays.lo_corner[1], rays.hi_corner[1], tile_pixel_span);
    DaylightShadowProjectionFit {
        pixel_center: [x.0, y.0],
        size_k: [x.1, y.1],
        min_forward_dot: least_dot,
    }
}

#[cfg(test)]
mod tests {
    use super::super::sun_shadow_frustum_rays;
    use super::*;

    const AXES: [[f32; 3]; 3] = [Vec3::X.to_array(), Vec3::Y.to_array(), Vec3::Z.to_array()];

    #[test]
    fn depth_is_squeezed_with_a_texel_of_slack_each_end() {
        assert_eq!(sun_shadow_projection_m22(10.0, 108.0), 0.01);
        // 9 maps to zero, 109 to one.
        let (m22, m32) = (
            sun_shadow_projection_m22(10.0, 108.0),
            sun_shadow_projection_m32(10.0, 108.0),
        );
        assert!((9.0 * m22 + m32).abs() < 1e-6);
        assert!((109.0 * m22 + m32 - 1.0).abs() < 1e-6);
    }

    #[test]
    fn the_ortho_projection_spans_a_thousand_texels() {
        let projection = daylight_shadow_ortho_projection([0.25, -0.5], 0.25, 10.0, 108.0);
        assert_eq!(projection.x_axis, Vec4::new(2.0 / 256.0, 0.0, 0.0, 0.0));
        assert_eq!(projection.y_axis, Vec4::new(0.0, 2.0 / 256.0, 0.0, 0.0));
        assert_eq!(projection.z_axis.z, sun_shadow_projection_m22(10.0, 108.0));
        assert_eq!(
            projection.w_axis,
            Vec4::new(0.25, -0.5, sun_shadow_projection_m32(10.0, 108.0), 1.0)
        );
        // 128 world units out lands on the edge of clip space.
        assert_eq!(
            projection.project_point3(Vec3::new(128.0, 0.0, 10.0)).x,
            1.0 + 0.25
        );
    }

    #[test]
    fn the_polygon_offset_follows_the_projection_depth_scale() {
        let offset = shadow_map_polygon_offset(0.5);
        assert_eq!(
            offset,
            [
                render_material::SM_POLYGON_OFFSET_BIAS_DEFAULT * 0.25 * 0.5,
                render_material::SM_POLYGON_OFFSET_SCALE_DEFAULT,
                0.0,
                0.0
            ]
        );
    }

    #[test]
    fn the_pixel_adjust_is_a_quarter_and_a_half_texel() {
        assert_eq!(
            daylight_shadow_map_pixel_adjust(DaylightShadowAtlasProfile::Large),
            [0.25 / 1024.0, 0.5 / 2048.0, 0.5 / 1024.0, -0.25 / 2048.0]
        );
        assert_eq!(
            daylight_shadow_map_pixel_adjust(DaylightShadowAtlasProfile::Small),
            [0.25 / 512.0, 0.5 / 2048.0, 0.5 / 512.0, -0.25 / 2048.0]
        );
    }

    #[test]
    fn the_lookup_matrix_maps_clip_space_into_the_atlas() {
        let fraction = [0.1, 0.2, 0.3, 0.4];
        let lookup =
            daylight_shadow_lookup_matrix(Mat4::IDENTITY, fraction, DaylightShadowAtlasProfile::Large);
        assert_eq!(lookup.x_axis, Vec4::new(0.5, 0.0, 0.0, 0.5 + 0.5 / 1024.0));
        assert_eq!(
            lookup.y_axis,
            Vec4::new(0.0, -0.25, 0.0, 0.25 + 0.5 / 2048.0)
        );
        assert_eq!(lookup.z_axis, Vec4::new(0.0, 0.0, 1.0, 0.0));
        assert_eq!(lookup.w_axis, Vec4::from_array(fraction));
    }

    #[test]
    fn the_lookup_matrix_takes_its_rows_from_the_projection() {
        // A projection that only shifts: x by 3, y by 5, depth by 7.
        let shift = Mat4::from_translation(Vec3::new(3.0, 5.0, 7.0));
        let lookup = daylight_shadow_lookup_matrix(shift, [0.0; 4], DaylightShadowAtlasProfile::Large);
        // The shift is the w row of the projection, so it is what the texture
        // coordinates are offset by.
        assert_eq!(lookup.x_axis.x, 0.5);
        assert_eq!(lookup.x_axis.w, 1.5 + (0.5 + 0.5 / 1024.0));
        assert_eq!(lookup.z_axis, Vec4::new(0.0, 0.0, 1.0, 7.0));
    }

    fn setup() -> DaylightShadowProjectionSetup {
        DaylightShadowProjectionSetup {
            near_shadow_floor: 4.0,
            shadow_origin_texel_centre: [512.0, 512.0],
            snapped_shadow_centre: [[0.0; 2]; 2],
            probe_size: [0.25, 1.0],
        }
    }

    #[test]
    fn the_partition_fraction_is_one_at_the_end_of_the_near_cascade() {
        let fraction = setup().split_fraction([8.0, 0.0, 0.0], [1.0, 0.0, 0.0]);
        assert_eq!(fraction, [0.25, 0.0, 0.0, -2.0]);
        // The point 4 units in front of the eye.
        let at = |x: f32| fraction[0] * x + fraction[3];
        assert_eq!(at(12.0), 1.0);
        assert_eq!(at(8.0), 0.0);
    }

    #[test]
    fn the_receivers_switch_three_quarters_of_the_way_to_the_far_edge() {
        let metrics = DaylightShadowMapMetrics {
            tile_pixel_span: 1024,
            tiles_per_atlas_page: 2,
            low_coord: 0,
        };
        let constants = setup().receiver_consts(metrics, false, 1.0);
        assert_eq!(constants.switch_split, [0.375, 0.6875, 1.0, 0.25]);
        assert_eq!(constants.shadow_map_scale, [16.0, 32.0, -8.0, -24.0]);

        let offset = setup().receiver_consts(metrics, true, 2.0);
        assert_eq!(offset.switch_split[2], 0.0);
        assert_eq!(offset.shadow_map_scale, [8.0, 16.0, -4.0, -12.0]);
    }

    #[test]
    fn a_near_origin_shifted_from_the_far_one_moves_the_switch() {
        let metrics = DaylightShadowMapMetrics {
            tile_pixel_span: 1024,
            tiles_per_atlas_page: 2,
            low_coord: 0,
        };
        let mut shifted = setup();
        shifted.snapped_shadow_centre[0] = [1.0, -1.0];
        let constants = shifted.receiver_consts(metrics, false, 1.0);
        // One far texel is one world unit here.
        assert_eq!(constants.switch_split[0], 385.0 / 1024.0);
        assert_eq!(constants.switch_split[1], (1.75 - 383.0 / 1024.0) / 2.0);
    }

    fn rays_spanning(min: f32, max: f32) -> SunShadowFrustumRays {
        sun_shadow_frustum_rays(
            [
                [min, min, 1.0],
                [max, min, 1.0],
                [max, max, 1.0],
                [min, max, 1.0],
            ],
            AXES,
        )
    }

    #[test]
    fn a_frustum_whose_edges_fall_on_texel_centres_keeps_its_natural_scale() {
        // Spanning -1..3 in a 4 texel tile: centre 1.5 and half a texel a unit.
        let fit = sun_shadow_projection_fit(&rays_spanning(-1.0, 3.0), [0.0, 0.0, 1.0], 4.0);
        assert_eq!(fit.pixel_center, [1.5, 1.5]);
        assert_eq!(fit.size_k, [0.5, 0.5]);
        assert_eq!(fit.min_forward_dot, 1.0);
    }

    #[test]
    fn a_frustum_that_does_not_is_shrunk_to_fit() {
        let fit = sun_shadow_projection_fit(&rays_spanning(-1.0, 1.0), [0.0, 0.0, 1.0], 1024.0);
        assert_eq!(fit.pixel_center, [511.5, 511.5]);
        assert_eq!(fit.size_k, [510.5, 510.5]);
    }

    #[test]
    fn a_frustum_reaching_further_ahead_is_anchored_at_its_far_edge() {
        // The eye is close to the low edge, so the high edge decides the scale
        // and ends exactly a texel from the tile's edge.
        let (centre, scale) = fit_axis(-0.3, 5.0, 100.0);
        assert_eq!(centre, 7.5);
        assert!((scale - 18.3).abs() < 1e-4, "{scale}");
        assert!((centre + scale * 5.0 - 99.0).abs() < 1e-3);
        assert!(centre + scale * -0.3 >= 1.0);
    }

    #[test]
    fn every_fit_leaves_a_texel_at_each_edge() {
        for (min, max) in [
            (-3.0, 0.5),
            (-0.1, 7.0),
            (-20.0, 20.0),
            (-1.0, 0.0),
            (0.0, 5.0),
        ] {
            for pixels in [64.0, 512.0, 1024.0] {
                let (centre, scale) = fit_axis(min, max, pixels);
                assert!(centre + scale * min >= 1.0 - 1e-3, "{min} {max} {pixels}");
                assert!(
                    centre + scale * max <= pixels - 1.0 + 1e-3,
                    "{min} {max} {pixels}"
                );
                // The eye always sits on a texel centre.
                assert_eq!(centre.fract().abs(), 0.5, "{min} {max} {pixels}");
            }
        }
    }

    #[test]
    fn the_least_forward_corner_is_kept() {
        let rays = sun_shadow_frustum_rays(
            [
                [1.0, 0.0, 1.0f32],
                [0.0, 1.0, 2.0],
                [-1.0, 0.0, 3.0],
                [0.0, -1.0, 4.0],
            ],
            AXES,
        );
        let fit = sun_shadow_projection_fit(&rays, [0.0, 0.0, 1.0], 64.0);
        assert_eq!(fit.min_forward_dot, 1.0);
    }

    #[test]
    #[should_panic(expected = "flat along this axis")]
    fn a_frustum_with_no_extent_cannot_be_fitted() {
        let _ = sun_shadow_projection_fit(&rays_spanning(0.0, 0.0), [0.0, 0.0, 1.0], 64.0);
    }
}
