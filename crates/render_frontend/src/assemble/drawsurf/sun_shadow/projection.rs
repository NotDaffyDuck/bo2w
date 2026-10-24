//! A cascade's projection, how it is fitted to the camera frustum, and the
//! constants the receivers use to read it back.

use bevy::math::{Mat4, Vec3, Vec4};
use render_frame::{SunShadowAtlasProfile, SunShadowReceiverConstants};

use super::{
    SUN_SHADOW_HALF, SUN_SHADOW_NEG_QUARTER, SUN_SHADOW_QUARTER, SUN_SHADOW_SAMPLE_EXTENT_PIXELS,
    SunShadowFrustumRays,
};

/// The depth scale of the cascades' orthographic projection, for a scene that
/// spans `near_clip..far_clip` along the sun, with a texel of slack either end.
pub fn sun_shadow_projection_m22(near_clip: f32, far_clip: f32) -> f32 {
    1.0 / (far_clip - near_clip + 2.0)
}

/// The depth offset that goes with [`sun_shadow_projection_m22`]: a depth of
/// `near_clip - 1` maps to zero.
pub fn sun_shadow_projection_m32(near_clip: f32, far_clip: f32) -> f32 {
    -(near_clip - 1.0) * sun_shadow_projection_m22(near_clip, far_clip)
}

/// A cascade's orthographic projection: a square of `1024 * sample_size` world
/// units, shifted by `snapped_clip`, with the scene's depth range squeezed
/// into it.
pub fn sun_shadow_ortho_projection(
    snapped_clip: [f32; 2],
    sample_size: f32,
    near_clip: f32,
    far_clip: f32,
) -> Mat4 {
    let side = sample_size * SUN_SHADOW_SAMPLE_EXTENT_PIXELS;
    let scale = 2.0 / side;
    Mat4::from_cols(
        Vec4::new(scale, 0.0, 0.0, 0.0),
        Vec4::new(0.0, scale, 0.0, 0.0),
        Vec4::new(
            0.0,
            0.0,
            sun_shadow_projection_m22(near_clip, far_clip),
            0.0,
        ),
        Vec4::new(
            snapped_clip[0],
            snapped_clip[1],
            sun_shadow_projection_m32(near_clip, far_clip),
            1.0,
        ),
    )
}

/// The slope-scaled depth bias a caster pass is drawn with.
pub fn shadowmap_polygon_offset(projection_m22: f32) -> [f32; 4] {
    [
        render_material::SM_POLYGON_OFFSET_BIAS_DEFAULT * SUN_SHADOW_QUARTER * projection_m22,
        render_material::SM_POLYGON_OFFSET_SCALE_DEFAULT,
        0.0,
        0.0,
    ]
}

/// A quarter texel and half a texel, in the atlas's coordinates, that the
/// receivers add to land on texel centres.
pub fn sun_shadowmap_pixel_adjust(profile: SunShadowAtlasProfile) -> [f32; 4] {
    let partition = profile.partition_size() as f32;
    let atlas_width = profile.atlas_width() as f32;
    [
        SUN_SHADOW_QUARTER / partition,
        SUN_SHADOW_HALF / atlas_width,
        SUN_SHADOW_HALF / partition,
        SUN_SHADOW_NEG_QUARTER / atlas_width,
    ]
}

/// The matrix a receiver uses to turn a world position into atlas
/// coordinates, with `partition_fraction` (the plane that tells the cascades
/// apart) in the last column.
///
/// Clip space is mapped to texture space: x into `0..1` across a partition,
/// y flipped and divided over the atlas's columns.
pub fn sun_shadow_lookup_matrix(
    view_projection: Mat4,
    partition_fraction: [f32; 4],
    profile: SunShadowAtlasProfile,
) -> Mat4 {
    let partition = profile.partition_size() as f32;
    let atlas_width = profile.atlas_width() as f32;
    let atlas_columns = profile.atlas_columns() as f32;
    let x_scale = SUN_SHADOW_HALF;
    let x_shift = SUN_SHADOW_HALF + SUN_SHADOW_HALF / partition;
    let y_scale = -SUN_SHADOW_HALF / atlas_columns;
    let y_shift = SUN_SHADOW_HALF / atlas_columns + SUN_SHADOW_HALF / atlas_width;

    // Each output column is a row of the projection, which is what makes the
    // result take a world position (as a row vector) to texture space.
    let rows = view_projection.transpose();
    Mat4::from_cols(
        rows.x_axis * x_scale + rows.w_axis * x_shift,
        rows.y_axis * y_scale + rows.w_axis * y_shift,
        rows.z_axis,
        Vec4::from_array(partition_fraction),
    )
}

/// How the atlas is cut up, as the receivers see it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SunShadowMapMetrics {
    pub pixels_per_tile: u32,

    pub tiles_per_texture: u32,

    pub min_coord: u32,
}

/// What the cascades' placement settled on, for the receivers.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SunShadowProjectionSetup {
    pub near_shadow_min_dist: f32,

    pub shadow_org_pixel_center: [f32; 2],

    pub snapped_shadow_org: [[f32; 2]; 2],

    pub sample_size: [f32; 2],
}

impl SunShadowProjectionSetup {
    /// The plane `forward . (p - view_origin) / near_shadow_min_dist`: one at
    /// the far edge of the near cascade, which is where the receivers switch
    /// to the far one.
    pub fn partition_fraction(
        self,
        view_origin: [f32; 3],
        view_axis_forward: [f32; 3],
    ) -> [f32; 4] {
        let slope = Vec3::from_array(view_axis_forward) * (1.0 / self.near_shadow_min_dist);
        let offset =
            -(view_origin[0] * slope.x + view_origin[1] * slope.y + view_origin[2] * slope.z);
        [slope.x, slope.y, slope.z, offset]
    }

    /// The constants a receiver reads to pick a cascade and size its filter.
    pub fn receiver_constants(
        self,
        metrics: SunShadowMapMetrics,
        use_shadow_offset: bool,
        shadowmap_scale: f32,
    ) -> SunShadowReceiverConstants {
        let pixels_per_tile = metrics.pixels_per_tile as f32;
        let tiles_per_texture = metrics.tiles_per_texture as f32;
        let min_coord = metrics.min_coord as f32;
        let far_sample = self.sample_size[1];
        // How far the near cascade's origin sits from the far one's, in far texels.
        let shift_x = (self.snapped_shadow_org[0][0] - self.snapped_shadow_org[1][0]) / far_sample;
        let shift_y = (self.snapped_shadow_org[0][1] - self.snapped_shadow_org[1][1]) / far_sample;

        // Three quarters of the way across the near cascade, from its eye.
        let switch_x =
            (0.75 * (min_coord + self.shadow_org_pixel_center[0]) + shift_x) / pixels_per_tile;
        let switch_y = (1.75
            - (0.75 * (min_coord + self.shadow_org_pixel_center[1]) + shift_y) / pixels_per_tile)
            / tiles_per_texture;

        SunShadowReceiverConstants {
            switch_partition: [
                switch_x,
                switch_y,
                if use_shadow_offset { 0.0 } else { 1.0 },
                0.25,
            ],
            shadowmap_scale: [
                16.0 / shadowmap_scale,
                (16.0 * tiles_per_texture) / shadowmap_scale,
                -8.0 / shadowmap_scale,
                -24.0 / shadowmap_scale,
            ],
        }
    }
}

/// Where the near cascade's eye sits in its tile and how many tile texels a
/// world unit takes up, per axis, so the camera frustum just fits.
#[derive(Clone, Copy, Debug)]
pub(super) struct SunShadowProjectionFit {
    pub pixel_center: [f32; 2],
    pub scale: [f32; 2],

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
    let span = max - min;
    assert!(span > 0.0, "sun-shadow frustum projection has zero span");
    let natural = (pixels - 2.0) / span;
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
    view_forward: [f32; 3],
    pixels_per_tile: f32,
) -> SunShadowProjectionFit {
    let forward = Vec3::from_array(view_forward);
    let min_forward_dot = rays
        .world_rays
        .iter()
        .map(|&ray| Vec3::from_array(ray).dot(forward))
        .fold(f32::INFINITY, f32::min);
    let x = fit_axis(rays.mins[0], rays.maxs[0], pixels_per_tile);
    let y = fit_axis(rays.mins[1], rays.maxs[1], pixels_per_tile);
    SunShadowProjectionFit {
        pixel_center: [x.0, y.0],
        scale: [x.1, y.1],
        min_forward_dot,
    }
}

#[cfg(test)]
mod tests {
    use super::super::sun_shadow_frustum_rays;
    use super::*;

    const AXES: [[f32; 3]; 3] = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];

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
        let projection = sun_shadow_ortho_projection([0.25, -0.5], 0.25, 10.0, 108.0);
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
        let offset = shadowmap_polygon_offset(0.5);
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
            sun_shadowmap_pixel_adjust(SunShadowAtlasProfile::Large),
            [0.25 / 1024.0, 0.5 / 2048.0, 0.5 / 1024.0, -0.25 / 2048.0]
        );
        assert_eq!(
            sun_shadowmap_pixel_adjust(SunShadowAtlasProfile::Small),
            [0.25 / 512.0, 0.5 / 2048.0, 0.5 / 512.0, -0.25 / 2048.0]
        );
    }

    #[test]
    fn the_lookup_matrix_maps_clip_space_into_the_atlas() {
        let fraction = [0.1, 0.2, 0.3, 0.4];
        let lookup =
            sun_shadow_lookup_matrix(Mat4::IDENTITY, fraction, SunShadowAtlasProfile::Large);
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
        let lookup = sun_shadow_lookup_matrix(shift, [0.0; 4], SunShadowAtlasProfile::Large);
        // The shift is the w row of the projection, so it is what the texture
        // coordinates are offset by.
        assert_eq!(lookup.x_axis.x, 0.5);
        assert_eq!(lookup.x_axis.w, 1.5 + (0.5 + 0.5 / 1024.0));
        assert_eq!(lookup.z_axis, Vec4::new(0.0, 0.0, 1.0, 7.0));
    }

    fn setup() -> SunShadowProjectionSetup {
        SunShadowProjectionSetup {
            near_shadow_min_dist: 4.0,
            shadow_org_pixel_center: [512.0, 512.0],
            snapped_shadow_org: [[0.0; 2]; 2],
            sample_size: [0.25, 1.0],
        }
    }

    #[test]
    fn the_partition_fraction_is_one_at_the_end_of_the_near_cascade() {
        let fraction = setup().partition_fraction([8.0, 0.0, 0.0], [1.0, 0.0, 0.0]);
        assert_eq!(fraction, [0.25, 0.0, 0.0, -2.0]);
        // The point 4 units in front of the eye.
        let at = |x: f32| fraction[0] * x + fraction[3];
        assert_eq!(at(12.0), 1.0);
        assert_eq!(at(8.0), 0.0);
    }

    #[test]
    fn the_receivers_switch_three_quarters_of_the_way_to_the_far_edge() {
        let metrics = SunShadowMapMetrics {
            pixels_per_tile: 1024,
            tiles_per_texture: 2,
            min_coord: 0,
        };
        let constants = setup().receiver_constants(metrics, false, 1.0);
        assert_eq!(constants.switch_partition, [0.375, 0.6875, 1.0, 0.25]);
        assert_eq!(constants.shadowmap_scale, [16.0, 32.0, -8.0, -24.0]);

        let offset = setup().receiver_constants(metrics, true, 2.0);
        assert_eq!(offset.switch_partition[2], 0.0);
        assert_eq!(offset.shadowmap_scale, [8.0, 16.0, -4.0, -12.0]);
    }

    #[test]
    fn a_near_origin_shifted_from_the_far_one_moves_the_switch() {
        let metrics = SunShadowMapMetrics {
            pixels_per_tile: 1024,
            tiles_per_texture: 2,
            min_coord: 0,
        };
        let mut shifted = setup();
        shifted.snapped_shadow_org[0] = [1.0, -1.0];
        let constants = shifted.receiver_constants(metrics, false, 1.0);
        // One far texel is one world unit here.
        assert_eq!(constants.switch_partition[0], 385.0 / 1024.0);
        assert_eq!(constants.switch_partition[1], (1.75 - 383.0 / 1024.0) / 2.0);
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
        assert_eq!(fit.scale, [0.5, 0.5]);
        assert_eq!(fit.min_forward_dot, 1.0);
    }

    #[test]
    fn a_frustum_that_does_not_is_shrunk_to_fit() {
        let fit = sun_shadow_projection_fit(&rays_spanning(-1.0, 1.0), [0.0, 0.0, 1.0], 1024.0);
        assert_eq!(fit.pixel_center, [511.5, 511.5]);
        assert_eq!(fit.scale, [510.5, 510.5]);
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
                [1.0, 0.0, 1.0],
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
    #[should_panic(expected = "zero span")]
    fn a_frustum_with_no_extent_cannot_be_fitted() {
        let _ = sun_shadow_projection_fit(&rays_spanning(0.0, 0.0), [0.0, 0.0, 1.0], 64.0);
    }
}
