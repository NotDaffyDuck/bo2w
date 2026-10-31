//! Building the vertices and indices of a trail segment.
//!
//! A trail is drawn as a strip of segments. Each segment has a position,
//! an orientation (stored as two direction bytes triples), a size, a spin
//! and a colour; every vertex of the trail's cross-section is placed from
//! those.

use asset_t6::{T6_PACKED_VERTEX, f32_to_half, pack_unit_vec};

use crate::runtime::trail_template::StreakCorner;

/// A stored basis direction is a signed byte of a 127th.
const BASIS_STEP: f32 = 0.007_874_015_718_698_502;

/// A basis direction is stored as this many steps per unit.
const BASIS_SCALE: f64 = 127.0;

/// A trail vertex's tangent. BO2 unknown: the tangent the game gives a
/// trail vertex has no fact yet (facts/QUESTIONS.md); this is -x packed as a
/// unit vector.
pub const TRAIL_TANGENT_WORD: u32 = 0x0000_0201;

/// The binormal sign written into every code-mesh vertex.
const BINORMAL_SIGN: f32 = -1.0;

/// Which segment of a trail's texture a segment starts on: after
/// `scroll_period_millis` of drawing it moves on, or, when it does not scroll,
/// every `recurrence_distance` units of the trail's length.
pub fn trail_compute_u(
    span: f32,
    recurrence_distance: i32,
    scroll_period_millis: i32,
    draw_ms: i32,
) -> i32 {
    match (scroll_period_millis, recurrence_distance) {
        (0, 0) => 0,
        (0, dist) => libm::floorf(span / dist as f32) as i32,
        (scroll, _) => draw_ms / scroll,
    }
}

/// The two directions (left, up) a segment stores as signed bytes.
pub fn trail_uncompress_basis(basis_chars: &[i8; 6]) -> [[f32; 3]; 2] {
    let w = basis_chars.map(|c| f32::from(c) * BASIS_STEP);
    [[w[0], w[1], w[2]], [w[3], w[4], w[5]]]
}

/// A unit-length component as a signed byte.
fn compress_component(v: f32) -> i8 {
    let steps = (f64::from(v) * BASIS_SCALE) as i32;
    i8::try_from(steps).unwrap_or(if steps < 0 { i8::MIN } else { i8::MAX })
}

/// The basis a segment stores for an orientation: its second and third
/// axes.
pub fn compress_basis_from_axis(axis: [[f32; 3]; 3]) -> [i8; 6] {
    let [_, left, up] = axis;
    let mut stored = [0i8; 6];
    for (slot, component) in stored.iter_mut().zip(left.into_iter().chain(up)) {
        *slot = compress_component(component);
    }
    stored
}

/// The three index pairs joining the vertex pair `(a, b)` of one cross-section
/// to the same pair of the next, which starts `vertex_tally` vertices on from
/// `base0`.
pub fn trail_emit_index_quad(a: u16, b: u16, base0: u16, vertex_tally: u16) -> [[u16; 2]; 3] {
    let pair_at = |base: u16| [a.wrapping_add(base), b.wrapping_add(base)];
    let (this, next) = (pair_at(base0), pair_at(base0.wrapping_add(vertex_tally)));
    [this, next, [next[0], this[1]]]
}

/// The two triangles of the quad [`trail_emit_index_quad`] makes.
pub fn trail_index_quad_tris(pairs: [[u16; 2]; 3]) -> [[u16; 3]; 2] {
    let [this, next, _] = pairs;
    [[this[0], this[1], next[0]], [this[1], next[1], next[0]]]
}

/// Where and how a trail segment is drawn.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StreakPiecePaintStatus {
    pub scene_point: [f32; 3],
    /// The segment's left and up directions.
    pub basis: [[f32; 3]; 2],
    pub rotation: f32,
    pub size: [f32; 2],
    pub across_texel: f32,
    pub tint: [u8; 4],
}

/// One finished trail vertex.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StreakLaidCorner {
    pub spot: [f32; 3],
    pub tint: [u8; 4],
    pub u: f32,
    pub v: f32,
    pub packed_surface_uv: u32,
    pub facing_word: u32,
    pub surface_dir_word: u32,
}

/// A texture coordinate as two half floats, u in the low 16 bits.
pub fn trail_pack_texcoord(u: f32, val: f32) -> u32 {
    u32::from(f32_to_half(u)) | (u32::from(f32_to_half(val)) << 16)
}

/// A unit vector packed signed 10:10:10.
pub fn trail_pack_normal(n: [f32; 3]) -> u32 {
    pack_unit_vec(n)
}

/// One row of a code mesh: position, binormal sign, colour, texture
/// coordinate, normal, tangent.
pub fn pack_code_mesh_vertex(
    xyz: [f32; 3],
    tint: [u8; 4],
    packed_surface_uv: u32,
    facing_word: u32,
    surface_dir_word: u32,
) -> [u8; T6_PACKED_VERTEX] {
    let mut row = [0_u8; T6_PACKED_VERTEX];
    // Eight little-endian words; the colour bytes ride through unchanged.
    let words = [
        xyz[0].to_bits(),
        xyz[1].to_bits(),
        xyz[2].to_bits(),
        BINORMAL_SIGN.to_bits(),
        u32::from_le_bytes(tint),
        packed_surface_uv,
        facing_word,
        surface_dir_word,
    ];
    for (cell, word) in row.chunks_exact_mut(4).zip(words) {
        cell.copy_from_slice(&word.to_le_bytes());
    }
    row
}

/// One cross-section vertex of a segment, placed by the segment's state.
fn emit_segment_vert(vert: &StreakCorner, paint: &StreakPiecePaintStatus) -> StreakLaidCorner {
    let (sin, cos) = (libm::sinf(paint.rotation), libm::cosf(paint.rotation));
    let [first, second] = paint.basis;
    // The turn mixes the stored pair: left leans on the first direction,
    // up on the second, flipped.
    let left = blend(first, cos, second, sin);
    let up = blend(first, sin, second, -cos);

    let across = vert.spot[0] * paint.size[0];
    let along = vert.spot[1] * paint.size[1];
    let xyz = core::array::from_fn(|axis| {
        across * left[axis] + paint.scene_point[axis] + along * up[axis]
    });
    let [facing_x, facing_y] = vert.normal;
    StreakLaidCorner {
        spot: xyz,
        tint: paint.tint,
        u: paint.across_texel,
        v: vert.sheet_v,
        packed_surface_uv: trail_pack_texcoord(paint.across_texel, vert.sheet_v),
        facing_word: pack_unit_vec(blend(up, facing_y, left, facing_x)),
        surface_dir_word: TRAIL_TANGENT_WORD,
    }
}

/// `a` and `b` scaled by their weights and added.
fn blend(a: [f32; 3], wa: f32, b: [f32; 3], wb: f32) -> [f32; 3] {
    core::array::from_fn(|i| a[i] * wa + b[i] * wb)
}

/// Places a segment's cross-section vertices into `out`; returns how many
/// were written, the shorter of the two lengths.
pub fn trail_emit_segment_verts(
    vertex_points: &[StreakCorner],
    state: &StreakPiecePaintStatus,
    out: &mut [StreakLaidCorner],
) -> usize {
    let mut placed = 0;
    for (slot, vert) in out.iter_mut().zip(vertex_points) {
        *slot = emit_segment_vert(vert, state);
        placed += 1;
    }
    placed
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::quat::unit_quat_to_axis;
    use super::StreakCorner;

    fn vert(pos: [f32; 2], face_dir: [f32; 2], tex_coord: f32) -> StreakCorner {
        StreakCorner {
            spot: pos,
            normal: face_dir,
            sheet_v: tex_coord,
        }
    }

    fn state() -> StreakPiecePaintStatus {
        StreakPiecePaintStatus {
            scene_point: [10.0, 20.0, 30.0],
            basis: [[1.0, 0.0, 0.0], [0.0, 0.0, 1.0]],
            rotation: 0.0_f32,
            size: [1.0, 10.0],
            across_texel: 0.5,
            tint: [1, 2, 3, 4],
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
        assert_eq!(got.spot, [12.0, 20.0, 0.0]);
        assert_eq!(got.tint, [1, 2, 3, 4]);
        assert_eq!((got.u, got.v), (0.5, 0.25));
        assert_eq!(got.packed_surface_uv, trail_pack_texcoord(0.5, 0.25));
        assert_eq!(got.facing_word, pack_unit_vec([0.0, 0.0, -1.0]));
        assert_eq!(got.surface_dir_word, TRAIL_TANGENT_WORD);
    }

    #[test]
    fn a_quarter_turn_swaps_the_directions() {
        let mut turned = state();
        turned.rotation = core::f32::consts::FRAC_PI_2;
        let got = emit_segment_vert(&vert([2.0, 0.0], [0.0, 0.0], 0.0), &turned);
        // Left is now the second direction.
        assert!((got.spot[0] - 10.0).abs() < 1e-4);
        assert!((got.spot[2] - 32.0).abs() < 1e-4);
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
        let vertex_points = [
            vert([0.0, 0.0], [0.0, 0.0], 0.0),
            vert([1.0, 0.0], [0.0, 0.0], 0.0),
            vert([2.0, 0.0], [0.0, 0.0], 0.0),
        ];
        let blank = emit_segment_vert(&vertex_points[0], &state());
        let mut out = [blank; 2];
        assert_eq!(trail_emit_segment_verts(&vertex_points, &state(), &mut out), 2);
        assert_eq!(out[1].spot[0], 11.0);
        let mut roomy = [blank; 5];
        assert_eq!(trail_emit_segment_verts(&vertex_points, &state(), &mut roomy), 3);
    }
}
