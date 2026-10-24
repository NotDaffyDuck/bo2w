//! Building the vertices and indices of a trail segment.
//!
//! A trail is drawn as a strip of segments. Each segment has a position,
//! an orientation (stored as two direction bytes triples), a size, a spin
//! and a colour; every vertex of the trail's cross-section is placed from
//! those.

use asset_t6::{T6_PACKED_VERTEX, f32_to_half, pack_unit_vec};

use crate::runtime::trail_def::FxTrailVertex;

/// A stored basis direction is a signed byte of a 127th.
const BASIS_STEP: f32 = 0.007_874_015_718_698_502;

/// A basis direction is stored as this many steps per unit.
const BASIS_SCALE: f64 = 127.0;

/// A trail vertex's tangent. BO2 unknown: the tangent the game gives a
/// trail vertex has no fact yet (facts/QUESTIONS.md); this is -x packed as a
/// unit vector.
pub const FX_TRAIL_TANGENT_PACKED: u32 = 0x0000_0201;

/// The binormal sign written into every code-mesh vertex.
const BINORMAL_SIGN: f32 = -1.0;

/// Which segment of a trail's texture a segment starts on: after
/// `scroll_time_msec` of drawing it moves on, or, when it does not scroll,
/// every `repeat_dist` units of the trail's length.
pub fn trail_compute_u(
    span: f32,
    repeat_dist: i32,
    scroll_time_msec: i32,
    draw_time_msec: i32,
) -> i32 {
    if scroll_time_msec != 0 {
        return draw_time_msec / scroll_time_msec;
    }
    if repeat_dist == 0 {
        return 0;
    }
    libm::floorf(span / repeat_dist as f32) as i32
}

/// The two directions (left, up) a segment stores as signed bytes.
pub fn trail_uncompress_basis(basis_chars: &[i8; 6]) -> [[f32; 3]; 2] {
    let at = |i: usize| f32::from(basis_chars[i]) * BASIS_STEP;
    [[at(0), at(1), at(2)], [at(3), at(4), at(5)]]
}

/// A unit-length component as a signed byte.
fn compress_component(v: f32) -> i8 {
    let steps = (f64::from(v) * BASIS_SCALE) as i32;
    steps.clamp(i32::from(i8::MIN), i32::from(i8::MAX)) as i8
}

/// The basis a segment stores for an orientation: its second and third
/// axes.
pub fn compress_basis_from_axis(axis: [[f32; 3]; 3]) -> [i8; 6] {
    let [_, left, up] = axis;
    [
        compress_component(left[0]),
        compress_component(left[1]),
        compress_component(left[2]),
        compress_component(up[0]),
        compress_component(up[1]),
        compress_component(up[2]),
    ]
}

/// The three index pairs joining the vertex pair `(a, b)` of one cross-section
/// to the same pair of the next, which starts `vert_count` vertices on from
/// `base0`.
pub fn trail_emit_index_quad(a: u16, b: u16, base0: u16, vert_count: u16) -> [[u16; 2]; 3] {
    let base1 = base0.wrapping_add(vert_count);
    let a0 = a.wrapping_add(base0);
    let b0 = b.wrapping_add(base0);
    let a1 = a.wrapping_add(base1);
    let b1 = b.wrapping_add(base1);
    [[a0, b0], [a1, b1], [a1, b0]]
}

/// The two triangles of the quad [`trail_emit_index_quad`] makes.
pub fn trail_index_quad_tris(pairs: [[u16; 2]; 3]) -> [[u16; 3]; 2] {
    let [[a0, b0], [a1, b1], _] = pairs;
    [[a0, b0, a1], [b0, b1, a1]]
}

/// Where and how a trail segment is drawn.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FxTrailSegmentDrawState {
    pub pos_world: [f32; 3],
    /// The segment's left and up directions.
    pub basis: [[f32; 3]; 2],
    pub rotation: f32,
    pub size: [f32; 2],
    pub u_coord: f32,
    pub color_rgba: [u8; 4],
}

/// One finished trail vertex.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FxTrailEmittedVert {
    pub xyz: [f32; 3],
    pub color_rgba: [u8; 4],
    pub u: f32,
    pub v: f32,
    pub texcoord_packed: u32,
    pub normal_packed: u32,
    pub tangent_packed: u32,
}

/// A texture coordinate as two half floats, u in the low 16 bits.
pub fn trail_pack_texcoord(u: f32, v: f32) -> u32 {
    u32::from(f32_to_half(u)) | (u32::from(f32_to_half(v)) << 16)
}

/// A unit vector packed signed 10:10:10.
pub fn trail_pack_normal(n: [f32; 3]) -> u32 {
    pack_unit_vec(n)
}

/// One row of a code mesh: position, binormal sign, colour, texture
/// coordinate, normal, tangent.
pub fn pack_code_mesh_vertex(
    xyz: [f32; 3],
    color_rgba: [u8; 4],
    texcoord_packed: u32,
    normal_packed: u32,
    tangent_packed: u32,
) -> [u8; T6_PACKED_VERTEX] {
    let mut row = [0u8; T6_PACKED_VERTEX];
    row[0..4].copy_from_slice(&xyz[0].to_le_bytes());
    row[4..8].copy_from_slice(&xyz[1].to_le_bytes());
    row[8..12].copy_from_slice(&xyz[2].to_le_bytes());
    row[12..16].copy_from_slice(&BINORMAL_SIGN.to_le_bytes());
    row[16..20].copy_from_slice(&color_rgba);
    row[20..24].copy_from_slice(&texcoord_packed.to_le_bytes());
    row[24..28].copy_from_slice(&normal_packed.to_le_bytes());
    row[28..32].copy_from_slice(&tangent_packed.to_le_bytes());
    row
}

/// One cross-section vertex of a segment, placed by the segment's state.
fn emit_segment_vert(vert: &FxTrailVertex, state: &FxTrailSegmentDrawState) -> FxTrailEmittedVert {
    let cos = libm::cosf(state.rotation);
    let sin = libm::sinf(state.rotation);
    let [b0, b1] = state.basis;

    let left = [
        cos * b0[0] + sin * b1[0],
        cos * b0[1] + sin * b1[1],
        cos * b0[2] + sin * b1[2],
    ];
    let up = [
        sin * b0[0] + (-cos) * b1[0],
        sin * b0[1] + (-cos) * b1[1],
        sin * b0[2] + (-cos) * b1[2],
    ];

    let across = vert.pos[0] * state.size[0];
    let along = vert.pos[1] * state.size[1];
    let xyz = [
        across * left[0] + state.pos_world[0] + along * up[0],
        across * left[1] + state.pos_world[1] + along * up[1],
        across * left[2] + state.pos_world[2] + along * up[2],
    ];
    let [nx, ny] = vert.normal;
    let normal = [
        ny * up[0] + nx * left[0],
        ny * up[1] + nx * left[1],
        ny * up[2] + nx * left[2],
    ];
    FxTrailEmittedVert {
        xyz,
        color_rgba: state.color_rgba,
        u: state.u_coord,
        v: vert.tex_coord,
        texcoord_packed: trail_pack_texcoord(state.u_coord, vert.tex_coord),
        normal_packed: trail_pack_normal(normal),
        tangent_packed: FX_TRAIL_TANGENT_PACKED,
    }
}

/// Places a segment's cross-section vertices into `out`; returns how many
/// were written, the shorter of the two lengths.
pub fn trail_emit_segment_verts(
    verts: &[FxTrailVertex],
    state: &FxTrailSegmentDrawState,
    out: &mut [FxTrailEmittedVert],
) -> usize {
    let count = verts.len().min(out.len());
    for (slot, vert) in out.iter_mut().zip(verts) {
        *slot = emit_segment_vert(vert, state);
    }
    count
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::quat::unit_quat_to_axis;

    fn vert(pos: [f32; 2], normal: [f32; 2], tex_coord: f32) -> FxTrailVertex {
        FxTrailVertex {
            pos,
            normal,
            tex_coord,
        }
    }

    fn state() -> FxTrailSegmentDrawState {
        FxTrailSegmentDrawState {
            pos_world: [10.0, 20.0, 30.0],
            basis: [[1.0, 0.0, 0.0], [0.0, 0.0, 1.0]],
            rotation: 0.0,
            size: [1.0, 10.0],
            u_coord: 0.5,
            color_rgba: [1, 2, 3, 4],
        }
    }

    #[test]
    fn a_scrolling_trail_moves_on_with_the_clock() {
        assert_eq!(trail_compute_u(999.0, 10, 250, 1000), 4);
        assert_eq!(trail_compute_u(999.0, 10, 250, 249), 0);
    }

    #[test]
    fn a_still_trail_repeats_by_distance() {
        assert_eq!(trail_compute_u(25.0, 10, 0, 1000), 2);
        assert_eq!(trail_compute_u(-1.0, 10, 0, 1000), -1);
        assert_eq!(trail_compute_u(25.0, 0, 0, 1000), 0);
    }

    #[test]
    fn basis_bytes_round_trip_close_to_the_original() {
        let bytes = compress_basis_from_axis([[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, -1.0]]);
        assert_eq!(bytes, [0, 127, 0, 0, 0, -127]);
        let [left, up] = trail_uncompress_basis(&bytes);
        assert!((left[1] - 1.0).abs() < 1e-5);
        assert!((up[2] + 1.0).abs() < 1e-5);
    }

    #[test]
    fn basis_components_are_cut_toward_zero_and_clamped() {
        assert_eq!(compress_component(0.5), 63);
        assert_eq!(compress_component(-0.5), -63);
        assert_eq!(compress_component(5.0), 127);
        assert_eq!(compress_component(-5.0), -128);
    }

    #[test]
    fn a_rotation_stores_its_second_and_third_axes() {
        let identity = unit_quat_to_axis([0.0, 0.0, 0.0, 1.0]);
        assert_eq!(compress_basis_from_axis(identity), [0, 127, 0, 0, 0, 127]);
    }

    #[test]
    fn an_index_quad_joins_this_cross_section_to_the_next() {
        let pairs = trail_emit_index_quad(0, 1, 10, 4);
        assert_eq!(pairs, [[10, 11], [14, 15], [14, 11]]);
        assert_eq!(trail_index_quad_tris(pairs), [[10, 11, 14], [11, 15, 14]]);
    }

    #[test]
    fn a_vertex_is_placed_from_the_segment_s_directions_and_size() {
        let got = emit_segment_vert(&vert([2.0, 3.0], [0.0, 1.0], 0.25), &state());
        // Left is the first direction; up is the second, flipped.
        assert_eq!(got.xyz, [12.0, 20.0, 0.0]);
        assert_eq!(got.color_rgba, [1, 2, 3, 4]);
        assert_eq!((got.u, got.v), (0.5, 0.25));
        assert_eq!(got.texcoord_packed, trail_pack_texcoord(0.5, 0.25));
        assert_eq!(got.normal_packed, pack_unit_vec([0.0, 0.0, -1.0]));
        assert_eq!(got.tangent_packed, FX_TRAIL_TANGENT_PACKED);
    }

    #[test]
    fn a_quarter_turn_swaps_the_directions() {
        let mut turned = state();
        turned.rotation = core::f32::consts::FRAC_PI_2;
        let got = emit_segment_vert(&vert([2.0, 0.0], [0.0, 0.0], 0.0), &turned);
        // Left is now the second direction.
        assert!((got.xyz[0] - 10.0).abs() < 1e-4);
        assert!((got.xyz[2] - 32.0).abs() < 1e-4);
    }

    #[test]
    fn texture_coordinates_pack_as_two_halves() {
        assert_eq!(trail_pack_texcoord(1.0, 0.0), 0x0000_3c00);
        assert_eq!(trail_pack_texcoord(0.0, 1.0), 0x3c00_0000);
    }

    #[test]
    fn a_code_mesh_row_lays_its_fields_out_in_order() {
        let row = pack_code_mesh_vertex(
            [1.0, 2.0, 3.0],
            [9, 8, 7, 6],
            0x0102_0304,
            0x0a0b_0c0d,
            0x1112_1314,
        );
        assert_eq!(row[0..4], 1.0f32.to_le_bytes());
        assert_eq!(row[8..12], 3.0f32.to_le_bytes());
        assert_eq!(row[12..16], (-1.0f32).to_le_bytes());
        assert_eq!(row[16..20], [9, 8, 7, 6]);
        assert_eq!(row[20..24], 0x0102_0304u32.to_le_bytes());
        assert_eq!(row[24..28], 0x0a0b_0c0du32.to_le_bytes());
        assert_eq!(row[28..32], 0x1112_1314u32.to_le_bytes());
    }

    #[test]
    fn only_as_many_verts_as_fit_are_emitted() {
        let verts = [
            vert([0.0, 0.0], [0.0, 0.0], 0.0),
            vert([1.0, 0.0], [0.0, 0.0], 0.0),
            vert([2.0, 0.0], [0.0, 0.0], 0.0),
        ];
        let blank = emit_segment_vert(&verts[0], &state());
        let mut out = [blank; 2];
        assert_eq!(trail_emit_segment_verts(&verts, &state(), &mut out), 2);
        assert_eq!(out[1].xyz[0], 11.0);
        let mut roomy = [blank; 5];
        assert_eq!(trail_emit_segment_verts(&verts, &state(), &mut roomy), 3);
    }
}
