//! BO2's post-light list and mesh (FX_PostLight_*), from
//! facts/fx-beam-laser-t6.md section 3. Item numbers below are that section's.

use crate::runtime::{pack_code_mesh_vertex, perpendicular_vector, vec3_length_sq, vec3_normalize};

pub const AFTERGLOW_PAINT_LABEL: &str = "PostLight";

/// Lights past this many in one frame are dropped (item 1).
pub const AFTERGLOW_QUEUE_LIMIT: usize = 0x60;

pub const AFTERGLOW_CORNER_TOTAL: u32 = 16;
pub const AFTERGLOW_TRIANGLE_SLOTS: u32 = 0x54;
pub const AFTERGLOW_KNOB_TOTAL: u32 = 2;

/// Lights shorter than this squared span are skipped (item 2).
pub const AFTERGLOW_SHORTEST_STEP_SQ: f32 = 1.0e-4;

/// The ring offset is the radius times this (item 4).
pub const AFTERGLOW_SHAPE_SWELL: f32 = 1.414_213_5;

/// 2 pi / 8 between ring vertices (item 4).
pub const FX_POST_LIGHT_ANGLE_STEP: f32 = core::f32::consts::FRAC_PI_4;

/// Drawn with this draw-surf override and no depth hack (item 7).
pub const FX_POST_LIGHT_DRAW_SURF_OVERRIDE: u8 = 0x18;

/// One queued light. On BO2 a laser's light takes the beam's begin colour
/// (facts section 4, item 11), so the caller packs it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AfterglowSprite {
    pub begin: [f32; 3],
    pub end: [f32; 3],
    pub radius: f32,
    pub tint_word: u32,
    pub surface_title: &'static str,
}

#[inline]
#[must_use]
pub fn post_light_add_allows(live: usize) -> bool {
    live < AFTERGLOW_QUEUE_LIMIT
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AfterglowMeshBatch {
    pub vertex_points: [[f32; 3]; 16],
    pub tint_word: u32,
    pub indices: [u16; 84],
    pub knobs: [[f32; 4]; 2],
}

/// The light's colour rides in the vertex colour slot, u = 0 (item 4).
#[inline]
#[must_use]
pub fn post_light_pack_vert(xyz: [f32; 3], tint_word: u32) -> [u8; 32] {
    // Packed blue, green, red, alpha from the low byte; the vertex wants RGBA.
    let bgra = tint_word.to_le_bytes();
    pack_code_mesh_vertex(xyz, [2, 1, 0, 3].map(|i| bgra[i]), 0, 0, 0)
}

/// Two rings of 8 vertices, one a radius before `begin` and one a radius
/// past `end`, plus the two shader args (items 4-6).
#[must_use]
pub fn post_light_generate_verts(light: &AfterglowSprite, eye: [f32; 3]) -> Option<AfterglowMeshBatch> {
    let minus = |a: [f32; 3], b: [f32; 3]| -> [f32; 3] { core::array::from_fn(|k| a[k] - b[k]) };
    let delta = minus(light.end, light.begin);
    let span_sq = vec3_length_sq(delta);
    let too_short = span_sq < AFTERGLOW_SHORTEST_STEP_SQ;
    if too_short {
        return None;
    }
    // Item 5: the xyz parts are the raw vectors; only w carries a reciprocal.
    let with_w = |v: [f32; 3], w: f32| [v[0], v[1], v[2], w];
    let args = [with_w(minus(light.begin, eye), 1.0 / light.radius), with_w(delta, 1.0 / span_sq)];

    let axis = vec3_normalize(delta);
    let side = perpendicular_vector(axis);
    // up = axis x side, each component from the two axes after it.
    let up: [f32; 3] = core::array::from_fn(|k| {
        let (j, l) = ((k + 1) % 3, (k + 2) % 3);
        axis[j] * side[l] - axis[l] * side[j]
    });
    let ring_radius = light.radius * AFTERGLOW_SHAPE_SWELL;
    let reach = axis.map(|c| c * light.radius);
    // Each ring point's spread around the axis, shared by both rings.
    let spread = core::array::from_fn::<[f32; 3], 8, _>(|i| {
        let (s, c) = libm::sincosf(i as f32 * FX_POST_LIGHT_ANGLE_STEP);
        core::array::from_fn(|k| (s * side[k] + c * up[k]) * ring_radius)
    });
    // Vertices 0..8 ring the begin (pulled back), 8..16 the end (pushed on).
    let vertex_points = core::array::from_fn(|v| {
        let (centre, sign) = if v < 8 { (light.begin, -1.0) } else { (light.end, 1.0) };
        let off = spread[v % 8];
        core::array::from_fn(|k| centre[k] + off[k] + sign * reach[k])
    });
    let indices = post_light_indices();
    Some(AfterglowMeshBatch { vertex_points, indices, knobs: args, tint_word: light.tint_word })
}

/// 8 side quads, then a fan cap on each ring (item 6). Both caps fan out
/// from the ring's first vertex (0 and 8).
fn post_light_indices() -> [u16; 84] {
    let sides = (0u16..8).flat_map(|k| {
        let next = (k + 1) % 8;
        [k, next, k + 8, k + 8, next, next + 8]
    });
    let near_cap = (1u16..7).flat_map(|k| [k + 1, k, 0]);
    let far_cap = (9u16..15).flat_map(|k| [8, k, k + 1]);
    let mut list = sides.chain(near_cap).chain(far_cap);
    // 48 side indices + 18 per cap fill the 84 exactly.
    core::array::from_fn(|_| list.next().unwrap_or_default())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn light(end: [f32; 3]) -> AfterglowSprite {
        AfterglowSprite {
            begin: [0.0; 3],
            end,
            radius: 2.0,
            tint_word: 0x8040_2010,
            surface_title: "gfx_laser_light",
        }
    }

    #[test]
    fn short_span_is_skipped() {
        assert!(post_light_generate_verts(&light([0.005, 0.0, 0.0]), [0.0; 3]).is_none());
    }

    #[test]
    fn rings_sit_a_radius_past_each_end() {
        let tess = post_light_generate_verts(&light([10.0, 0.0, 0.0]), [0.0; 3]).unwrap();
        let (near, far) = tess.vertex_points.split_at(8);
        for (n, f) in near.iter().zip(far) {
            assert!((n[0] + 2.0).abs() < 1e-5);
            assert!((f[0] - 12.0).abs() < 1e-5);
            let r = libm::sqrtf(n[1].powi(2) + n[2].powi(2));
            assert!((r - 2.0 * AFTERGLOW_SHAPE_SWELL).abs() < 1e-4);
        }
        assert!((tess.knobs[0][3] - 0.5).abs() < 1e-6);
        assert!((tess.knobs[1][3] - 0.01).abs() < 1e-6);
    }

    #[test]
    fn args_carry_the_raw_vectors() {
        let lit = AfterglowSprite {
            begin: [1.0, 2.0, 3.0],
            ..light([11.0, 2.0, 3.0])
        };
        let tess = post_light_generate_verts(&lit, [0.0, 0.0, 1.0]).unwrap();
        assert_eq!(tess.knobs[0], [1.0, 2.0, 2.0, 0.5]);
        assert_eq!(&tess.knobs[1][..3], &[10.0, 0.0, 0.0]);
        assert!((tess.knobs[1][3] - 0.01).abs() < 1e-6);
    }

    #[test]
    fn indices_stay_on_the_16_verts() {
        let idx = post_light_indices();
        assert!(idx.iter().all(|&i| i < 16));
        assert_eq!(&idx[48..54], &[2, 1, 0, 3, 2, 0]);
        // Every near-cap triangle touches the fan centre and none is degenerate.
        for tri in idx[48..66].chunks(3) {
            assert!(tri.contains(&0));
            assert!(tri[0] != tri[1] && tri[1] != tri[2] && tri[0] != tri[2]);
        }
    }

    #[test]
    fn colour_is_unswizzled_into_the_vertex() {
        let row = post_light_pack_vert([1.0, 2.0, 3.0], 0x8040_2010);
        assert_eq!(&row[16..20], &[0x40, 0x20, 0x10, 0x80]);
    }

    #[test]
    fn light_cap_is_96() {
        assert!(post_light_add_allows(95));
        assert!(!post_light_add_allows(96));
    }

    #[test]
    fn a_slanted_light_has_its_two_rings_where_they_were_measured() {
        let lit = AfterglowSprite {
            begin: [1.0, -2.0, 0.5],
            end: [4.0, 3.0, -1.5],
            ..light([0.0; 3])
        };
        let tess = post_light_generate_verts(&lit, [0.3, 0.2, 0.1]).unwrap();
        let want: [[f32; 3]; 16] = [[2.4520278, -5.077428, 1.1488856], [2.0755062, -4.094791, 3.0406961], [0.49880505, -2.8353248, 3.8243098], [-1.3544654, -2.036808, 3.0406961], [-2.3986845, -2.1670005, 1.1488855], [-2.022163, -3.1496377, -0.7429251], [-0.4454621, -4.4091034, -1.5265384], [1.4078089, -5.2076206, -0.74292415], [7.3986845, 3.1670005, -2.1488857], [7.0221634, 4.1496377, -0.25707513], [5.445462, 5.409104, 0.52653843], [3.5921917, 6.2076206, -0.25707513], [2.5479722, 6.077428, -2.148886], [2.9244938, 5.0947905, -4.0406966], [4.501195, 3.8353248, -4.82431], [6.354466, 3.0368078, -4.0406957]];
        for (got, want) in tess.vertex_points.iter().zip(&want) {
            assert!(got.iter().zip(want).all(|(g, w)| (g - w).abs() < 1e-4), "{got:?} vs {want:?}");
        }
    }

    #[test]
    fn the_index_list_is_the_measured_one() {
        let want: [u16; 84] = [0, 1, 8, 8, 1, 9, 1, 2, 9, 9, 2, 10, 2, 3, 10, 10, 3, 11, 3, 4, 11, 11, 4, 12, 4, 5, 12, 12, 5, 13, 5, 6, 13, 13, 6, 14, 6, 7, 14, 14, 7, 15, 7, 0, 15, 15, 0, 8, 2, 1, 0, 3, 2, 0, 4, 3, 0, 5, 4, 0, 6, 5, 0, 7, 6, 0, 8, 9, 10, 8, 10, 11, 8, 11, 12, 8, 12, 13, 8, 13, 14, 8, 14, 15];
        assert_eq!(post_light_indices(), want);
    }
}
