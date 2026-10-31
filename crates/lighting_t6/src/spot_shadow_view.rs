//! The matrix a surface uses to find itself in a spot shadow's tile.
//!
//! A spot light renders its shadow through a perspective view down its own
//! cone. The lookup matrix is that view-projection with the light origin
//! folded in and the result squeezed into the light's tile of the atlas.

use crate::cone_shadow::{SPOT_SHADOW_TILE_SCALE, ConeSlotPlan};

/// The cone half-angle cosine is held inside this range so the projection
/// stays finite for a flat or a razor-thin cone.
const COS_MIN: f32 = 0.001;
const COS_MAX: f32 = f32::from_bits(0x3f7f_be77);

/// The near plane sits this far beyond the near bias.
const Z_NEAR_BASE: f32 = 1.0;

/// Clip space runs -1..1; texture space 0..1.
const CLIP_TO_TEXTURE: f32 = 0.5;

fn fov_half_tan(half_angle_cosine: f32) -> f32 {
    let cos = half_angle_cosine.clamp(COS_MIN, COS_MAX);
    libm::sqrtf(1.0 - cos * cos) / cos
}

/// Forward, up and right of the light's view. The view looks against the
/// light's direction vector.
fn view_axes(aim: [f32; 3]) -> [[f32; 3]; 3] {
    let forward = aim.map(|c| -c);
    let right = game_math::perpendicular_vector(forward);
    // up = right x forward, one axis at a time.
    let up = std::array::from_fn(|i| {
        let (j, k) = ((i + 1) % 3, (i + 2) % 3);
        right[j] * forward[k] - right[k] * forward[j]
    });
    [forward, up, right]
}

/// Row vector times matrix.
fn transform_row(m: &[f32; 16], v: [f32; 4]) -> [f32; 4] {
    std::array::from_fn(|c| {
        let wz = m[12 + c] * v[3] + m[8 + c] * v[2];
        // The sums keep the original order: column 0 adds x then y, the
        // others y then x (float addition is not associative).
        if c == 0 {
            wz + v[0] * m[c] + m[4 + c] * v[1]
        } else {
            wz + m[4 + c] * v[1] + m[c] * v[0]
        }
    })
}

/// Puts a view-projection (without translation) and the light origin into
/// the light's tile: x and y go from clip to texture space, y flipped, and
/// the tile's share of the atlas scales and offsets the result.
fn tile_matrix(clip_from_world: &[f32; 16], spot: [f32; 3], plan: ConeSlotPlan) -> [f32; 16] {
    let [offset_x, offset_y] = plan.tile_offset();
    let width_ratio = SPOT_SHADOW_TILE_SCALE * CLIP_TO_TEXTURE;
    let x_shift = SPOT_SHADOW_TILE_SCALE * CLIP_TO_TEXTURE + offset_x;
    let height_ratio = -SPOT_SHADOW_TILE_SCALE * CLIP_TO_TEXTURE;
    let y_shift = SPOT_SHADOW_TILE_SCALE * CLIP_TO_TEXTURE + offset_y;

    // The first three rows are the view-projection's; the fourth carries the
    // light spot through it.
    let [ox, oy, oz] = spot;
    let moved = transform_row(clip_from_world, [-ox, -oy, -oz, 1.0]);
    let basis = |r: usize| -> [f32; 4] {
        match r {
            3 => moved,
            _ => std::array::from_fn(|c| clip_from_world[r * 4 + c]),
        }
    };
    std::array::from_fn(|cell| {
        let row = basis(cell / 4);
        match cell % 4 {
            0 => width_ratio * row[0] + x_shift * row[3],
            1 => height_ratio * row[1] + y_shift * row[3],
            c => row[c],
        }
    })
}

/// The lookup matrix of one light's shadow in its atlas tile, or nothing when
/// the light's view cannot be built (its near and far planes coincide, or the
/// view-projection cannot be inverted).
pub(crate) fn spot_shadow_lookup(
    at: [f32; 3],
    dir: [f32; 3],
    half_angle_cosine: f32,
    z_far: f32,
    close_range_offset: f32,
    plan: ConeSlotPlan,
) -> Option<[f32; 16]> {
    let tan = fov_half_tan(half_angle_cosine);
    let frustum =
        game_math::setup_finite_projection_matrix(tan, tan, close_range_offset + Z_NEAR_BASE, z_far)?;
    let view = game_math::matrix_for_viewer(view_axes(dir));
    let (clip_from_world, _undo) =
        game_math::compose_view_projection(&view, &frustum, at)?;
    Some(tile_matrix(&clip_from_world, at, plan))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cone_shadow::plan_cone_slots;

    /// Ones down the diagonal: every fifth entry.
    const IDENTITY: [f32; 16] = {
        let mut m = [0.0; 16];
        let mut i = 0;
        while i < 16 {
            m[i] = 1.0;
            i += 5;
        }
        m
    };

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-3
    }

    fn dot(u: [f32; 3], w: [f32; 3]) -> f32 {
        u[0] * w[0] + u[1] * w[1] + u[2] * w[2]
    }

    /// Where a world point lands in atlas texture space after the divide.
    fn project(m: &[f32; 16], p: [f32; 3]) -> [f32; 2] {
        let v = transform_row(m, [p[0], p[1], p[2], 1.0]);
        [v[0] / v[3], v[1] / v[3]]
    }

    #[test]
    fn a_half_angle_of_sixty_degrees_has_a_tangent_of_root_three() {
        assert!(close(fov_half_tan(0.5), 1.732_050_8));
    }

    #[test]
    fn a_flat_cone_gets_a_very_wide_but_finite_view() {
        let tan = fov_half_tan(0.0);
        assert!(tan.is_finite());
        assert!((tan - 1000.0).abs() < 0.01);
        assert_eq!(fov_half_tan(-3.0), tan);
    }

    #[test]
    fn a_needle_cone_gets_a_very_narrow_but_nonzero_view() {
        let tan = fov_half_tan(1.0);
        assert!(tan > 0.0 && tan < 0.1);
        assert_eq!(fov_half_tan(7.0), tan);
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
        let plan = plan_cone_slots(0, 512);
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
            let m = tile_matrix(&IDENTITY, [0.0; 3], plan_cone_slots(slot, 512));
            assert_eq!([m[12], m[13]], centre);
        }
    }

    #[test]
    fn the_light_origin_moves_the_view_with_it() {
        let plan = plan_cone_slots(0, 512);
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
            let plan = plan_cone_slots(slot, 512);
            let m = spot_shadow_lookup(origin, dir, 0.7, 200.0, 0.0, plan).unwrap();
            let on_axis = [origin[0], origin[1], origin[2] + 50.0];
            let at = project(&m, on_axis);
            assert!(close(at[0], centre[0]), "slot {slot}: {at:?}");
            assert!(close(at[1], centre[1]), "slot {slot}: {at:?}");
        }
    }

    #[test]
    fn a_point_beside_the_axis_leaves_the_tile_centre() {
        let plan = plan_cone_slots(0, 512);
        let m = spot_shadow_lookup([0.0; 3], [0.0, 0.0, -1.0], 0.7, 200.0, 0.0, plan).unwrap();
        let off = project(&m, [30.0, 0.0, 50.0]);
        let on = project(&m, [0.0, 0.0, 50.0]);
        assert!((off[0] - on[0]).abs() > 0.05 || (off[1] - on[1]).abs() > 0.05);
    }

    #[test]
    fn a_point_stays_inside_the_tile_while_inside_the_cone() {
        let plan = plan_cone_slots(0, 512);
        // cos 0.7 is a half-angle of about 45.6 degrees, tan about 1.02.
        let m = spot_shadow_lookup([0.0; 3], [0.0, 0.0, -1.0], 0.7, 200.0, 0.0, plan).unwrap();
        let inside = project(&m, [40.0, 40.0, 100.0]);
        assert!((0.0..0.5).contains(&inside[0]));
        assert!((0.0..0.5).contains(&inside[1]));
    }

    #[test]
    fn a_far_plane_on_the_near_plane_gives_no_matrix() {
        let plan = plan_cone_slots(0, 512);
        assert!(spot_shadow_lookup([0.0; 3], [0.0, 0.0, -1.0], 0.7, 1.0, 0.0, plan).is_none());
    }

    #[test]
    fn the_near_bias_pushes_the_near_plane_out() {
        let plan = plan_cone_slots(0, 512);
        let flat = spot_shadow_lookup([0.0; 3], [0.0, 0.0, -1.0], 0.7, 200.0, 0.0, plan).unwrap();
        let biased = spot_shadow_lookup([0.0; 3], [0.0, 0.0, -1.0], 0.7, 200.0, 5.0, plan).unwrap();
        assert_ne!(flat, biased);
        // Only the depth column changes with the near plane.
        assert_eq!(flat[0], biased[0]);
        assert_eq!(flat[5], biased[5]);
    }
}
