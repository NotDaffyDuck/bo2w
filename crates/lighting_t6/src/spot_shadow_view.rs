//! The matrix a surface uses to find itself in a spot shadow's tile.
//!
//! A spot light renders its shadow through a perspective view down its own
//! cone. The lookup matrix is that view-projection with the light origin
//! folded in and the result squeezed into the light's tile of the atlas.

use crate::spot_shadow::{SPOT_SHADOW_TILE_SCALE, SpotShadowSlotPlan};

/// The cone half-angle cosine is held inside this range so the projection
/// stays finite for a flat or a razor-thin cone.
const COS_MIN: f32 = 0.001;
const COS_MAX: f32 = f32::from_bits(0x3f7f_be77);

/// The near plane sits this far beyond the near bias.
const Z_NEAR_BASE: f32 = 1.0;

/// Clip space runs -1..1; texture space 0..1.
const CLIP_TO_TEXTURE: f32 = 0.5;

fn tan_half_fov(cos_half_fov: f32) -> f32 {
    let cos = cos_half_fov.clamp(COS_MIN, COS_MAX);
    libm::sqrtf(1.0 - cos * cos) / cos
}

/// Forward, up and right of the light's view. The view looks against the
/// light's direction vector.
fn view_axes(dir: [f32; 3]) -> [[f32; 3]; 3] {
    let forward = [-dir[0], -dir[1], -dir[2]];
    let right = game_math::perpendicular_vector(forward);
    let up = [
        right[1] * forward[2] - right[2] * forward[1],
        right[2] * forward[0] - right[0] * forward[2],
        right[0] * forward[1] - right[1] * forward[0],
    ];
    [forward, up, right]
}

/// Row vector times matrix.
fn transform_row(m: &[f32; 16], v: [f32; 4]) -> [f32; 4] {
    [
        m[12] * v[3] + m[8] * v[2] + v[0] * m[0] + m[4] * v[1],
        m[13] * v[3] + m[9] * v[2] + m[5] * v[1] + m[1] * v[0],
        m[14] * v[3] + m[10] * v[2] + m[6] * v[1] + m[2] * v[0],
        m[15] * v[3] + m[11] * v[2] + m[7] * v[1] + m[3] * v[0],
    ]
}

/// Puts a view-projection (without translation) and the light origin into
/// the light's tile: x and y go from clip to texture space, y flipped, and
/// the tile's share of the atlas scales and offsets the result.
fn tile_matrix(
    view_projection: &[f32; 16],
    origin: [f32; 3],
    plan: SpotShadowSlotPlan,
) -> [f32; 16] {
    let [offset_x, offset_y] = plan.tile_offset();
    let x_scale = SPOT_SHADOW_TILE_SCALE * CLIP_TO_TEXTURE;
    let x_shift = SPOT_SHADOW_TILE_SCALE * CLIP_TO_TEXTURE + offset_x;
    let y_scale = -SPOT_SHADOW_TILE_SCALE * CLIP_TO_TEXTURE;
    let y_shift = SPOT_SHADOW_TILE_SCALE * CLIP_TO_TEXTURE + offset_y;

    let mut rows = [[0.0f32; 4]; 4];
    for (r, row) in rows.iter_mut().enumerate().take(3) {
        row.copy_from_slice(&view_projection[r * 4..r * 4 + 4]);
    }
    rows[3] = transform_row(view_projection, [-origin[0], -origin[1], -origin[2], 1.0]);

    let mut out = [0.0f32; 16];
    for (r, row) in rows.iter().enumerate() {
        out[r * 4] = x_scale * row[0] + x_shift * row[3];
        out[r * 4 + 1] = y_scale * row[1] + y_shift * row[3];
        out[r * 4 + 2] = row[2];
        out[r * 4 + 3] = row[3];
    }
    out
}

/// The lookup matrix of one light's shadow in its atlas tile, or nothing when
/// the light's view cannot be built (its near and far planes coincide, or the
/// view-projection cannot be inverted).
pub(crate) fn spot_shadow_lookup(
    origin: [f32; 3],
    dir: [f32; 3],
    cos_half_fov: f32,
    z_far: f32,
    near_bias: f32,
    plan: SpotShadowSlotPlan,
) -> Option<[f32; 16]> {
    let tan = tan_half_fov(cos_half_fov);
    let projection =
        game_math::setup_finite_projection_matrix(tan, tan, near_bias + Z_NEAR_BASE, z_far)?;
    let view = game_math::matrix_for_viewer(view_axes(dir));
    let (view_projection, _inverse) =
        game_math::compose_view_projection(&view, &projection, origin)?;
    Some(tile_matrix(&view_projection, origin, plan))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::spot_shadow::spot_shadow_slot_plan;

    const IDENTITY: [f32; 16] = [
        1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
    ];

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-3
    }

    fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
        a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
    }

    /// Where a world point lands in atlas texture space after the divide.
    fn project(m: &[f32; 16], p: [f32; 3]) -> [f32; 2] {
        let v = transform_row(m, [p[0], p[1], p[2], 1.0]);
        [v[0] / v[3], v[1] / v[3]]
    }

    #[test]
    fn a_half_angle_of_sixty_degrees_has_a_tangent_of_root_three() {
        assert!(close(tan_half_fov(0.5), 1.732_050_8));
    }

    #[test]
    fn a_flat_cone_gets_a_very_wide_but_finite_view() {
        let tan = tan_half_fov(0.0);
        assert!(tan.is_finite());
        assert!((tan - 1000.0).abs() < 0.01);
        assert_eq!(tan_half_fov(-3.0), tan);
    }

    #[test]
    fn a_needle_cone_gets_a_very_narrow_but_nonzero_view() {
        let tan = tan_half_fov(1.0);
        assert!(tan > 0.0 && tan < 0.1);
        assert_eq!(tan_half_fov(7.0), tan);
    }

    #[test]
    fn the_view_looks_against_the_light_direction() {
        let [forward, _, _] = view_axes([0.0, 0.0, -1.0]);
        assert_eq!(forward, [0.0, 0.0, 1.0]);
    }

    #[test]
    fn the_view_axes_are_perpendicular_and_unit_length() {
        for dir in [[0.0, 0.0, -1.0], [1.0, 0.0, 0.0], [0.6, 0.0, -0.8]] {
            let [forward, up, right] = view_axes(dir);
            assert!(close(dot(forward, right), 0.0));
            assert!(close(dot(forward, up), 0.0));
            assert!(close(dot(up, right), 0.0));
            assert!(close(dot(up, up), 1.0));
            assert!(close(dot(right, right), 1.0));
        }
    }

    #[test]
    fn an_identity_view_squeezes_into_the_first_tile() {
        let plan = spot_shadow_slot_plan(0, 512);
        let m = tile_matrix(&IDENTITY, [0.0; 3], plan);
        assert_eq!(
            m,
            [
                0.25, 0.0, 0.0, 0.0, //
                0.0, -0.25, 0.0, 0.0, //
                0.0, 0.0, 1.0, 0.0, //
                0.25, 0.25, 0.0, 1.0,
            ]
        );
    }

    #[test]
    fn later_tiles_shift_the_centre_by_half_an_atlas() {
        for (slot, centre) in [(1, [0.75, 0.25]), (2, [0.25, 0.75]), (3, [0.75, 0.75])] {
            let m = tile_matrix(&IDENTITY, [0.0; 3], spot_shadow_slot_plan(slot, 512));
            assert_eq!([m[12], m[13]], centre);
        }
    }

    #[test]
    fn the_light_origin_moves_the_view_with_it() {
        let plan = spot_shadow_slot_plan(0, 512);
        let at_origin = tile_matrix(&IDENTITY, [0.0; 3], plan);
        let moved = tile_matrix(&IDENTITY, [4.0, 0.0, 0.0], plan);
        assert!(close(moved[12], at_origin[12] - 4.0 * 0.25));
        assert_eq!(moved[0..12], at_origin[0..12]);
    }

    #[test]
    fn the_cone_axis_lands_on_the_centre_of_every_tile() {
        let origin = [10.0, -20.0, 30.0];
        let dir = [0.0, 0.0, -1.0];
        for (slot, centre) in [
            (0, [0.25, 0.25]),
            (1, [0.75, 0.25]),
            (2, [0.25, 0.75]),
            (3, [0.75, 0.75]),
        ] {
            let plan = spot_shadow_slot_plan(slot, 512);
            let m = spot_shadow_lookup(origin, dir, 0.7, 200.0, 0.0, plan).unwrap();
            let on_axis = [origin[0], origin[1], origin[2] + 50.0];
            let at = project(&m, on_axis);
            assert!(close(at[0], centre[0]), "slot {slot}: {at:?}");
            assert!(close(at[1], centre[1]), "slot {slot}: {at:?}");
        }
    }

    #[test]
    fn a_point_beside_the_axis_leaves_the_tile_centre() {
        let plan = spot_shadow_slot_plan(0, 512);
        let m = spot_shadow_lookup([0.0; 3], [0.0, 0.0, -1.0], 0.7, 200.0, 0.0, plan).unwrap();
        let off = project(&m, [30.0, 0.0, 50.0]);
        let on = project(&m, [0.0, 0.0, 50.0]);
        assert!((off[0] - on[0]).abs() > 0.05 || (off[1] - on[1]).abs() > 0.05);
    }

    #[test]
    fn a_point_stays_inside_the_tile_while_inside_the_cone() {
        let plan = spot_shadow_slot_plan(0, 512);
        // cos 0.7 is a half-angle of about 45.6 degrees, tan about 1.02.
        let m = spot_shadow_lookup([0.0; 3], [0.0, 0.0, -1.0], 0.7, 200.0, 0.0, plan).unwrap();
        let inside = project(&m, [40.0, 40.0, 100.0]);
        assert!((0.0..0.5).contains(&inside[0]));
        assert!((0.0..0.5).contains(&inside[1]));
    }

    #[test]
    fn a_far_plane_on_the_near_plane_gives_no_matrix() {
        let plan = spot_shadow_slot_plan(0, 512);
        assert!(spot_shadow_lookup([0.0; 3], [0.0, 0.0, -1.0], 0.7, 1.0, 0.0, plan).is_none());
    }

    #[test]
    fn the_near_bias_pushes_the_near_plane_out() {
        let plan = spot_shadow_slot_plan(0, 512);
        let flat = spot_shadow_lookup([0.0; 3], [0.0, 0.0, -1.0], 0.7, 200.0, 0.0, plan).unwrap();
        let biased = spot_shadow_lookup([0.0; 3], [0.0, 0.0, -1.0], 0.7, 200.0, 5.0, plan).unwrap();
        assert_ne!(flat, biased);
        // Only the depth column changes with the near plane.
        assert_eq!(flat[0], biased[0]);
        assert_eq!(flat[5], biased[5]);
    }
}
