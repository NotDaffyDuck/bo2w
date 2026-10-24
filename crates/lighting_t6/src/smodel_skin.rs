//! Skinning static models into the static model cache.
//!
//! A model's vertices come from the asset as 32-byte rows. A cached vertex is
//! also 32 bytes, but laid out for the cache's vertex format:
//!
//! | bytes | cached vertex                           | from the source row |
//! |-------|-----------------------------------------|---------------------|
//! | 0..12 | position, turned and moved by the model | 0..12               |
//! | 12..16 | colour                                 | 16..20              |
//! | 16..20 | texture coordinate                     | 20..24              |
//! | 20..24 | normal, turned by the model            | 24..28              |
//! | 24..28 | tangent, turned by the model           | 28..32              |
//! | 28..32 | lighting                               | made, see below     |

use asset_t6::{pack_unit_vec, unpack_unit_vec};

use crate::smc_layout::SMC_VERT_STRIDE;

const ROW: usize = SMC_VERT_STRIDE as usize;

/// One vertex, source or cached.
type Row = [u8; ROW];

/// Why a skinned surface could not be written.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SmcSkinError {
    ShortDest,
}

/// Where a cached vertex gets its lighting bytes from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SmcCachedVertLighting {
    /// The same four bytes for every vertex.
    Packed([u8; 4]),
    /// Bytes that point at the model's atlas entry.
    FromHandle {
        lighting_handle: u16,
        inv_height_x256: u32,
    },
}

/// One surface of a model to skin: its source rows and where its vertices go.
pub struct SmcSkinSurface<'a> {
    pub packed: &'a [Row],
    /// The surface's first vertex, counted from the start of the model's block.
    pub vert_offset: u16,
    pub lighting: SmcCachedVertLighting,
}

/// The upper left 3x3 of a skin matrix, row by row: what turns normals and
/// tangents.
pub fn setup_transform_unit_vec(m: &[f32; 16]) -> [f32; 9] {
    [m[0], m[1], m[2], m[4], m[5], m[6], m[8], m[9], m[10]]
}

/// A skin matrix for a model placed at `origin`, turned by `axis` and scaled.
pub fn skin_cached_static_model_cmd_matrix(
    origin: [f32; 3],
    axis: [[f32; 3]; 3],
    scale: f32,
) -> [f32; 16] {
    let mut m = [0.0; 16];
    for (row, vector) in axis.iter().enumerate() {
        for (column, component) in vector.iter().enumerate() {
            m[row * 4 + column] = component * scale;
        }
    }
    m[12..15].copy_from_slice(&origin);
    m
}

fn float_at(row: &Row, at: usize) -> f32 {
    f32::from_le_bytes([row[at], row[at + 1], row[at + 2], row[at + 3]])
}

fn word_at(row: &Row, at: usize) -> u32 {
    u32::from_le_bytes([row[at], row[at + 1], row[at + 2], row[at + 3]])
}

/// Turns a packed unit vector by the 3x3 and packs the result.
fn turn_packed_unit_vec(axis: &[f32; 9], packed: u32) -> u32 {
    let [x, y, z] = unpack_unit_vec(packed);
    let component = |i: usize| axis[i] * x + axis[3 + i] * y + axis[6 + i] * z;
    pack_unit_vec([component(0), component(1), component(2)])
}

/// The four lighting bytes of a cached vertex. `sign_word` is the source row's
/// word at byte 12, whose sign bit is the tangent's handedness.
fn lighting_bytes(lighting: SmcCachedVertLighting, sign_word: u32) -> [u8; 4] {
    match lighting {
        SmcCachedVertLighting::Packed(bytes) => bytes,
        SmcCachedVertLighting::FromHandle {
            lighting_handle,
            inv_height_x256,
        } => {
            let entry = u32::from(lighting_handle.wrapping_sub(1));
            [
                entry.wrapping_mul(4).wrapping_add(2) as u8,
                inv_height_x256
                    .wrapping_mul(2)
                    .wrapping_add((entry >> 4) & 0xfc) as u8,
                0x80,
                if sign_word >> 31 != 0 { 0 } else { 2 },
            ]
        }
    }
}

fn skin_vert(
    out: &mut Row,
    src: &Row,
    m: &[f32; 16],
    norm_axis: &[f32; 9],
    lighting: SmcCachedVertLighting,
) {
    let (x, y, z) = (float_at(src, 0), float_at(src, 4), float_at(src, 8));
    for (axis, component) in [
        m[0] * x + m[4] * y + m[8] * z + m[12],
        m[1] * x + m[5] * y + m[9] * z + m[13],
        m[2] * x + m[6] * y + m[10] * z + m[14],
    ]
    .into_iter()
    .enumerate()
    {
        out[axis * 4..axis * 4 + 4].copy_from_slice(&component.to_le_bytes());
    }
    out[12..20].copy_from_slice(&src[16..24]);
    out[20..24].copy_from_slice(&turn_packed_unit_vec(norm_axis, word_at(src, 24)).to_le_bytes());
    out[24..28].copy_from_slice(&turn_packed_unit_vec(norm_axis, word_at(src, 28)).to_le_bytes());
    out[28..].copy_from_slice(&lighting_bytes(lighting, word_at(src, 12)));
}

/// Skins each surface into `dest` at its own vertex offset. Surfaces are done
/// in order; the first that does not fit stops the rest, and the ones before
/// it stay written.
pub fn skin_cached_static_model_cmd(
    dest: &mut [u8],
    m: &[f32; 16],
    norm_axis: &[f32; 9],
    surfaces: &[SmcSkinSurface<'_>],
) -> Result<(), SmcSkinError> {
    for surface in surfaces {
        let first = usize::from(surface.vert_offset) * ROW;
        let bytes = surface.packed.len() * ROW;
        let span = dest
            .get_mut(first..first + bytes)
            .ok_or(SmcSkinError::ShortDest)?;
        for (out, src) in span.chunks_exact_mut(ROW).zip(surface.packed) {
            let out: &mut Row = out.try_into().expect("chunks are one row long");
            skin_vert(out, src, m, norm_axis, surface.lighting);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A quarter turn about z that sends x to y, then a move by (1, 2, 3).
    const QUARTER_TURN: [f32; 16] = [
        0.0, 1.0, 0.0, 0.0, -1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 1.0, 2.0, 3.0, 0.0,
    ];

    fn close(got: [f32; 3], want: [f32; 3]) {
        for (g, w) in got.iter().zip(want) {
            assert!((g - w).abs() < 0.01, "{got:?} is not {want:?}");
        }
    }

    fn row(position: [f32; 3], sign_word: u32) -> Row {
        let mut row = [0u8; ROW];
        for (i, v) in position.iter().enumerate() {
            row[i * 4..i * 4 + 4].copy_from_slice(&v.to_le_bytes());
        }
        row[12..16].copy_from_slice(&sign_word.to_le_bytes());
        row[16..24].copy_from_slice(&[1, 2, 3, 4, 5, 6, 7, 8]);
        row[24..28].copy_from_slice(&pack_unit_vec([1.0, 0.0, 0.0]).to_le_bytes());
        row[28..32].copy_from_slice(&pack_unit_vec([0.0, 0.0, -1.0]).to_le_bytes());
        row
    }

    fn skinned(src: &Row, lighting: SmcCachedVertLighting) -> Row {
        let mut out = [0u8; ROW];
        let axis = setup_transform_unit_vec(&QUARTER_TURN);
        skin_vert(&mut out, src, &QUARTER_TURN, &axis, lighting);
        out
    }

    #[test]
    fn the_matrix_scales_the_axes_and_carries_the_origin() {
        let axis = [[1.0, 2.0, 3.0], [4.0, 5.0, 6.0], [7.0, 8.0, 9.0]];
        let m = skin_cached_static_model_cmd_matrix([10.0, 20.0, 30.0], axis, 2.0);
        assert_eq!(
            m,
            [
                2.0, 4.0, 6.0, 0.0, 8.0, 10.0, 12.0, 0.0, 14.0, 16.0, 18.0, 0.0, 10.0, 20.0, 30.0,
                0.0
            ]
        );
    }

    #[test]
    fn the_normal_matrix_is_the_upper_left_3x3() {
        let m: [f32; 16] = core::array::from_fn(|i| i as f32);
        assert_eq!(
            setup_transform_unit_vec(&m),
            [0.0, 1.0, 2.0, 4.0, 5.0, 6.0, 8.0, 9.0, 10.0]
        );
    }

    #[test]
    fn a_packed_unit_vector_is_turned_by_the_axes() {
        let axis = setup_transform_unit_vec(&QUARTER_TURN);
        let turned = turn_packed_unit_vec(&axis, pack_unit_vec([1.0, 0.0, 0.0]));
        close(unpack_unit_vec(turned), [0.0, 1.0, 0.0]);
        let up = turn_packed_unit_vec(&axis, pack_unit_vec([0.0, 0.0, 1.0]));
        close(unpack_unit_vec(up), [0.0, 0.0, 1.0]);
    }

    #[test]
    fn a_vertex_is_turned_moved_and_laid_out_for_the_cache() {
        let src = row([4.0, 0.0, 0.0], 0);
        let out = skinned(&src, SmcCachedVertLighting::Packed([9, 8, 7, 6]));
        assert_eq!(
            [float_at(&out, 0), float_at(&out, 4), float_at(&out, 8)],
            [1.0, 6.0, 3.0]
        );
        assert_eq!(out[12..20], src[16..24]);
        close(unpack_unit_vec(word_at(&out, 20)), [0.0, 1.0, 0.0]);
        close(unpack_unit_vec(word_at(&out, 24)), [0.0, 0.0, -1.0]);
        assert_eq!(out[28..], [9, 8, 7, 6]);
    }

    #[test]
    fn lighting_from_a_handle_points_at_the_entry() {
        let lighting = SmcCachedVertLighting::FromHandle {
            lighting_handle: 0x45,
            inv_height_x256: 0x40,
        };
        // Entry 0x44: byte 0 is entry * 4 + 2 (cut to a byte), byte 1 is the
        // inverse height doubled plus the entry's bits 6 and up, shifted down.
        assert_eq!(lighting_bytes(lighting, 0), [0x12, 0x84, 0x80, 2]);
        assert_eq!(lighting_bytes(lighting, 0x8000_0000), [0x12, 0x84, 0x80, 0]);
        assert_eq!(lighting_bytes(lighting, 0xbf80_0000)[3], 0);
        assert_eq!(lighting_bytes(lighting, 0x3f80_0000)[3], 2);
    }

    #[test]
    fn handle_arithmetic_wraps_instead_of_failing() {
        let lighting = SmcCachedVertLighting::FromHandle {
            lighting_handle: 0,
            inv_height_x256: u32::MAX,
        };
        assert_eq!(lighting_bytes(lighting, 0), [0xfe, 0xfa, 0x80, 2]);
    }

    #[test]
    fn a_vertex_reads_the_sign_from_the_source_row() {
        let lighting = SmcCachedVertLighting::FromHandle {
            lighting_handle: 1,
            inv_height_x256: 0,
        };
        let negative = skinned(&row([0.0; 3], 0xbf80_0000), lighting);
        let positive = skinned(&row([0.0; 3], 0x3f80_0000), lighting);
        assert_eq!(negative[31], 0);
        assert_eq!(positive[31], 2);
    }

    #[test]
    fn each_surface_lands_at_its_vertex_offset() {
        let first = [row([1.0, 0.0, 0.0], 0), row([2.0, 0.0, 0.0], 0)];
        let second = [row([3.0, 0.0, 0.0], 0)];
        let surfaces = [
            SmcSkinSurface {
                packed: &first,
                vert_offset: 0,
                lighting: SmcCachedVertLighting::Packed([1; 4]),
            },
            SmcSkinSurface {
                packed: &second,
                vert_offset: 4,
                lighting: SmcCachedVertLighting::Packed([2; 4]),
            },
        ];
        let mut dest = vec![0xeeu8; 6 * ROW];
        let axis = setup_transform_unit_vec(&QUARTER_TURN);
        skin_cached_static_model_cmd(&mut dest, &QUARTER_TURN, &axis, &surfaces).unwrap();
        let y_at =
            |vert: usize| float_at(dest[vert * ROW..(vert + 1) * ROW].try_into().unwrap(), 4);
        assert_eq!([y_at(0), y_at(1), y_at(4)], [3.0, 4.0, 5.0]);
        assert!(dest[2 * ROW..4 * ROW].iter().all(|&b| b == 0xee));
        assert_eq!(dest[5 * ROW..], [0xee; ROW]);
        assert_eq!(dest[ROW + 28..2 * ROW], [1; 4]);
        assert_eq!(dest[4 * ROW + 28..5 * ROW], [2; 4]);
    }

    #[test]
    fn a_surface_that_does_not_fit_stops_the_rest() {
        let rows = [row([1.0, 0.0, 0.0], 0)];
        let surfaces = [
            SmcSkinSurface {
                packed: &rows,
                vert_offset: 0,
                lighting: SmcCachedVertLighting::Packed([1; 4]),
            },
            SmcSkinSurface {
                packed: &rows,
                vert_offset: 2,
                lighting: SmcCachedVertLighting::Packed([1; 4]),
            },
            SmcSkinSurface {
                packed: &rows,
                vert_offset: 1,
                lighting: SmcCachedVertLighting::Packed([1; 4]),
            },
        ];
        let mut dest = vec![0u8; 2 * ROW];
        let axis = setup_transform_unit_vec(&QUARTER_TURN);
        let result = skin_cached_static_model_cmd(&mut dest, &QUARTER_TURN, &axis, &surfaces);
        assert_eq!(result, Err(SmcSkinError::ShortDest));
        assert_ne!(dest[..ROW], [0; ROW]);
        assert_eq!(dest[ROW..], [0; ROW]);
    }

    #[test]
    fn a_far_vertex_offset_is_a_short_destination_not_a_wrap() {
        let rows = [row([1.0, 0.0, 0.0], 0)];
        let surfaces = [SmcSkinSurface {
            packed: &rows,
            vert_offset: u16::MAX,
            lighting: SmcCachedVertLighting::Packed([0; 4]),
        }];
        let axis = setup_transform_unit_vec(&QUARTER_TURN);
        assert_eq!(
            skin_cached_static_model_cmd(&mut [0; 64], &QUARTER_TURN, &axis, &surfaces),
            Err(SmcSkinError::ShortDest)
        );
    }
}
