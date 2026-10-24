//! BO2's post-light list and mesh (FX_PostLight_*), from
//! facts/fx-beam-laser-t6.md section 3. Item numbers below are that section's.

use crate::runtime::{pack_code_mesh_vertex, perpendicular_vector, vec3_length_sq, vec3_normalize};

pub const FX_POST_LIGHT_DRAW_NAME: &str = "PostLight";

/// Lights past this many in one frame are dropped (item 1).
pub const FX_POST_LIGHT_ADD_CAP: usize = 0x60;

pub const FX_POST_LIGHT_VERT_COUNT: u32 = 16;
pub const FX_POST_LIGHT_INDEX_COUNT: u32 = 0x54;
pub const FX_POST_LIGHT_ARG_COUNT: u32 = 2;

/// Lights shorter than this squared span are skipped (item 2).
pub const FX_POST_LIGHT_MIN_DELTA_SQ: f32 = 1.0e-4;

/// The ring offset is the radius times this (item 4).
pub const FX_POST_LIGHT_POLYGON_RADIUS_GROW: f32 = 1.414_213_5;

/// 2 pi / 8 between ring vertices (item 4).
pub const FX_POST_LIGHT_ANGLE_STEP: f32 = core::f32::consts::FRAC_PI_4;

/// Drawn with this draw-surf override and no depth hack (item 7).
pub const FX_POST_LIGHT_DRAW_SURF_OVERRIDE: u8 = 0x18;

/// One queued light. On BO2 a laser's light takes the beam's begin colour
/// (facts section 4, item 11), so the caller packs it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FxPostLight {
    pub begin: [f32; 3],
    pub end: [f32; 3],
    pub radius: f32,
    pub color_packed: u32,
    pub material_name: &'static str,
}

#[inline]
#[must_use]
pub fn post_light_add_allows(live: usize) -> bool {
    live < FX_POST_LIGHT_ADD_CAP
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FxPostLightTess {
    pub verts: [[f32; 3]; 16],
    pub color_packed: u32,
    pub indices: [u16; 84],
    pub args: [[f32; 4]; 2],
}

/// The light's colour rides in the vertex colour slot, u = 0 (item 4).
#[inline]
#[must_use]
pub fn post_light_pack_vert(xyz: [f32; 3], color_packed: u32) -> [u8; 32] {
    let [b, g, r, a] = color_packed.to_le_bytes();
    pack_code_mesh_vertex(xyz, [r, g, b, a], 0, 0, 0)
}

/// Two rings of 8 vertices, one a radius before `begin` and one a radius
/// past `end`, plus the two shader args (items 4-6).
#[must_use]
pub fn post_light_generate_verts(light: &FxPostLight, eye: [f32; 3]) -> Option<FxPostLightTess> {
    let delta = core::array::from_fn::<f32, 3, _>(|i| light.end[i] - light.begin[i]);
    let len_sq = vec3_length_sq(delta);
    if len_sq < FX_POST_LIGHT_MIN_DELTA_SQ {
        return None;
    }
    // Item 5: the xyz parts are the raw vectors; only w carries a reciprocal.
    let args = [
        [
            light.begin[0] - eye[0],
            light.begin[1] - eye[1],
            light.begin[2] - eye[2],
            1.0 / light.radius,
        ],
        [delta[0], delta[1], delta[2], 1.0 / len_sq],
    ];
    let dir = vec3_normalize(delta);
    let ortho0 = perpendicular_vector(dir);
    let ortho1 = [
        dir[1] * ortho0[2] - dir[2] * ortho0[1],
        dir[2] * ortho0[0] - dir[0] * ortho0[2],
        dir[0] * ortho0[1] - dir[1] * ortho0[0],
    ];
    let grow = light.radius * FX_POST_LIGHT_POLYGON_RADIUS_GROW;
    let mut verts = [[0.0f32; 3]; 16];
    for i in 0..8 {
        let ang = i as f32 * FX_POST_LIGHT_ANGLE_STEP;
        let (s, c) = (libm::sinf(ang), libm::cosf(ang));
        for k in 0..3 {
            let off = (s * ortho0[k] + c * ortho1[k]) * grow;
            let push = dir[k] * light.radius;
            verts[i][k] = light.begin[k] + off - push;
            verts[i + 8][k] = light.end[k] + off + push;
        }
    }
    Some(FxPostLightTess {
        verts,
        color_packed: light.color_packed,
        indices: post_light_indices(),
        args,
    })
}

/// 8 side quads, then a fan cap on each ring (item 6). Both caps fan out
/// from the ring's first vertex (0 and 8).
fn post_light_indices() -> [u16; 84] {
    let mut out = [0u16; 84];
    let mut w = 0;
    for around in 0..8u16 {
        let next = (around + 1) & 7;
        out[w..w + 6].copy_from_slice(&[around, next, around + 8, around + 8, next, next + 8]);
        w += 6;
    }
    for around in (0..6u16).step_by(2) {
        out[w..w + 6].copy_from_slice(&[around + 2, around + 1, 0, around + 3, around + 2, 0]);
        w += 6;
    }
    for around in (0..6u16).step_by(2) {
        out[w..w + 6].copy_from_slice(&[8, around + 9, around + 10, 8, around + 10, around + 11]);
        w += 6;
    }
    debug_assert_eq!(w, 84);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn light(end: [f32; 3]) -> FxPostLight {
        FxPostLight {
            begin: [0.0; 3],
            end,
            radius: 2.0,
            color_packed: 0x8040_2010,
            material_name: "gfx_laser_light",
        }
    }

    #[test]
    fn short_span_is_skipped() {
        assert!(post_light_generate_verts(&light([0.005, 0.0, 0.0]), [0.0; 3]).is_none());
    }

    #[test]
    fn rings_sit_a_radius_past_each_end() {
        let tess = post_light_generate_verts(&light([10.0, 0.0, 0.0]), [0.0; 3]).unwrap();
        for i in 0..8 {
            assert!((tess.verts[i][0] + 2.0).abs() < 1e-5);
            assert!((tess.verts[i + 8][0] - 12.0).abs() < 1e-5);
            let r = libm::sqrtf(tess.verts[i][1].powi(2) + tess.verts[i][2].powi(2));
            assert!((r - 2.0 * FX_POST_LIGHT_POLYGON_RADIUS_GROW).abs() < 1e-4);
        }
        assert!((tess.args[0][3] - 0.5).abs() < 1e-6);
        assert!((tess.args[1][3] - 0.01).abs() < 1e-6);
    }

    #[test]
    fn args_carry_the_raw_vectors() {
        let lit = FxPostLight {
            begin: [1.0, 2.0, 3.0],
            ..light([11.0, 2.0, 3.0])
        };
        let tess = post_light_generate_verts(&lit, [0.0, 0.0, 1.0]).unwrap();
        assert_eq!(tess.args[0], [1.0, 2.0, 2.0, 0.5]);
        assert_eq!(&tess.args[1][..3], &[10.0, 0.0, 0.0]);
        assert!((tess.args[1][3] - 0.01).abs() < 1e-6);
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
}
