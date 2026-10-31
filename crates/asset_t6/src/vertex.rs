//! BO2's vertex rows and their reader. A model vertex (`GfxPackedVertex`) is 32
//! bytes and a world vertex (`GfxWorldVertex`) 36: position (3 floats), binormal
//! sign (float), colour (R, G, B, A bytes), texture coordinate (two halves, u in
//! the low half), then the normal and the tangent as signed 10:10:10 words; a
//! world vertex adds its lightmap coordinate (two u16 over 65535).

use std::cmp::Ordering;

pub const T6_PACKED_VERTEX: usize = 32;

pub const T6_WORLD_VERTEX: usize = 36;

/// BO2's packed unit vector: three signed 10-bit components (two's complement)
/// over 511, x in the low bits. The top two bits are not read.
#[must_use]
pub fn decode_unit_normal(bits: u32) -> [f32; 3] {
    let widen = |field: u32| {
        let raw = (field & 0x3ff) as i32;
        ((raw << 22) >> 22) as f32 / 511.0
    };
    [widen(bits), widen(bits >> 10), widen(bits >> 20)]
}

/// Packs a unit vector the way BO2's skinning stores one: each component clamped to
/// -1..=1, scaled by its channel's step (511, 511 << 10, 511 << 20), truncated
/// to an integer and masked to its ten bits. Truncating before the mask makes the
/// x channel round toward zero and the y and z channels round down. The top two
/// bits are 0.
#[must_use]
pub fn pack_unit_vec(axes: [f32; 3]) -> u32 {
    let channel = |axis: f32, step: f32, mask: u32| {
        let scaled = axis.clamp(-1.0, 1.0) * step;
        (scaled as i32 as u32) & mask
    };
    let [x, y, z] = axes;
    channel(x, 511.0, 0x3ff) | channel(y, 523_264.0, 0xf_fc00) | channel(z, 535_822_336.0, 0x3ff0_0000)
}

/// An IEEE half as an f32.
#[must_use]
pub fn short_float_to_full(bits: u16) -> f32 {
    let sign_bit = u32::from(bits >> 15) << 31;
    let biased = u32::from((bits >> 10) & 0x1f);
    let frac = u32::from(bits) & 0x3ff;
    let pattern = if biased == 0x1f {
        sign_bit | 0x7f80_0000 | (frac << 13)
    } else if biased != 0 {
        sign_bit | ((biased + 112) << 23) | (frac << 13)
    } else if frac == 0 {
        sign_bit
    } else {
        // Subnormal: normalise so the leading one lands on the hidden bit.
        let lead = frac.leading_zeros();
        let steps = lead - 21;
        sign_bit | ((134 - lead) << 23) | (((frac << steps) & 0x3ff) << 13)
    };
    f32::from_bits(pattern)
}

/// An f32 as an IEEE half, rounded to the nearest (ties to even); too large
/// goes to infinity, too small to a subnormal or zero.
// TODO(fact): how BO2 rounds when it packs a texture coordinate it computed
// (mark and code-mesh vertices); QUESTIONS.md 10-21 Clean1.
#[must_use]
pub fn f32_to_half(value: f32) -> u16 {
    let raw = value.to_bits();
    let sign_half = ((raw >> 16) & 0x8000) as u16;
    let biased = ((raw >> 23) & 0xff) as i32;
    let frac = raw & 0x007f_ffff;
    if biased == 0xff {
        return sign_half | 0x7c00 | if frac == 0 { 0 } else { 0x0200 };
    }
    let rebased = biased - 112;
    if rebased >= 0x1f {
        return sign_half | 0x7c00;
    }
    // Shift the 24-bit significand (with its hidden bit) down to the half's 10 bits,
    // further for a subnormal, then round to nearest even.
    let (high_bits, drop, significand) = if rebased > 0 {
        ((rebased as u32) << 10, 13u32, frac)
    } else if rebased < -10 {
        return sign_half;
    } else {
        let hidden = if biased == 0 { 0 } else { 0x0080_0000 };
        (0u32, (14 - rebased) as u32, frac | hidden)
    };
    let kept = significand >> drop;
    let remainder = significand & ((1u32 << drop) - 1);
    let half_way = 1u32 << (drop - 1);
    let bump = match remainder.cmp(&half_way) {
        Ordering::Greater => 1,
        Ordering::Less => 0,
        Ordering::Equal => kept & 1,
    };
    // A carry out of the mantissa steps the exponent, which is the right result.
    sign_half | (high_bits + kept + bump) as u16
}

fn rd_u32(bytes: &[u8], offset: usize) -> u32 {
    bytes[offset..offset + 4].try_into().map(u32::from_le_bytes).unwrap()
}

fn rd_f32(bytes: &[u8], offset: usize) -> f32 {
    f32::from_bits(rd_u32(bytes, offset))
}

/// One BO2 model vertex, fields as stored.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct T6PackedVertex {
    pub position: [f32; 3],
    pub binormal_sign: f32,
    /// R, G, B, A.
    pub color: [u8; 4],
    /// Two halves, u in the low 16 bits.
    pub tex_coord: u32,
    pub normal: u32,
    pub tangent: u32,
}

impl T6PackedVertex {
    /// Reads the first [`T6_PACKED_VERTEX`] bytes of `row`; `None` when it is shorter.
    #[must_use]
    pub fn read(row: &[u8]) -> Option<Self> {
        let row = row.get(..T6_PACKED_VERTEX)?;
        Some(Self {
            position: [rd_f32(row, 0), rd_f32(row, 4), rd_f32(row, 8)],
            binormal_sign: rd_f32(row, 12),
            color: [row[16], row[17], row[18], row[19]],
            tex_coord: rd_u32(row, 20),
            normal: rd_u32(row, 24),
            tangent: rd_u32(row, 28),
        })
    }

    #[must_use]
    pub fn to_bytes(&self) -> [u8; T6_PACKED_VERTEX] {
        let mut row = [0_u8; T6_PACKED_VERTEX];
        for (axis, coord) in self.position.iter().enumerate() {
            row[axis * 4..axis * 4 + 4].copy_from_slice(&coord.to_le_bytes());
        }
        row[12..16].copy_from_slice(&self.binormal_sign.to_le_bytes());
        row[16..20].copy_from_slice(&self.color);
        row[20..24].copy_from_slice(&self.tex_coord.to_le_bytes());
        row[24..28].copy_from_slice(&self.normal.to_le_bytes());
        row[28..32].copy_from_slice(&self.tangent.to_le_bytes());
        row
    }

    #[must_use]
    pub fn uv(&self) -> [f32; 2] {
        [
            short_float_to_full(self.tex_coord as u16),
            short_float_to_full((self.tex_coord >> 16) as u16),
        ]
    }

    #[must_use]
    pub fn unpacked_normal(&self) -> [f32; 3] {
        decode_unit_normal(self.normal)
    }

    #[must_use]
    pub fn unpacked_tangent(&self) -> [f32; 3] {
        decode_unit_normal(self.tangent)
    }

    /// The colour as R, G, B, A over 255.
    #[must_use]
    pub fn color_unorm(&self) -> [f32; 4] {
        self.color.map(|c| f32::from(c) / 255.0)
    }
}

/// One BO2 world vertex: a model vertex's fields plus the lightmap coordinate.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct T6WorldVertex {
    pub base: T6PackedVertex,
    /// Two u16, u in the low 16 bits.
    pub baked_light_uv: u32,
}

impl T6WorldVertex {
    /// Reads the first [`T6_WORLD_VERTEX`] bytes of `row`; `None` when it is shorter.
    #[must_use]
    pub fn read(row: &[u8]) -> Option<Self> {
        let row = row.get(..T6_WORLD_VERTEX)?;
        Some(Self {
            base: T6PackedVertex::read(row)?,
            baked_light_uv: rd_u32(row, 32),
        })
    }

    #[must_use]
    pub fn to_bytes(&self) -> [u8; T6_WORLD_VERTEX] {
        let mut row = [0_u8; T6_WORLD_VERTEX];
        row[..T6_PACKED_VERTEX].copy_from_slice(&self.base.to_bytes());
        row[32..36].copy_from_slice(&self.baked_light_uv.to_le_bytes());
        row
    }

    #[must_use]
    pub fn lightmap_uv(&self) -> [f32; 2] {
        [
            f32::from(self.baked_light_uv as u16) / 65535.0,
            f32::from((self.baked_light_uv >> 16) as u16) / 65535.0,
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unit_vectors_are_signed_ten_bit() {
        assert_eq!(decode_unit_normal(0x1ff | 0x201 << 10), [1.0, -1.0, 0.0]);
        assert_eq!(decode_unit_normal(0x3ff << 20)[2], -1.0 / 511.0);
        assert_eq!(decode_unit_normal(0xc000_0000), [0.0; 3]);
        for v in [[1.0, 0.0, 0.0], [0.0, -1.0, 0.0], [0.6, -0.8, 0.0], [-0.3, 0.2, 0.9]] {
            let back = decode_unit_normal(pack_unit_vec(v));
            for (a, b) in v.iter().zip(back) {
                assert!((a - b).abs() <= 1.0 / 511.0 + 1e-6, "{v:?} -> {back:?}");
            }
        }
        assert_eq!(pack_unit_vec([2.0, -2.0, 0.0]), 0x1ff | 0x201 << 10);
        // x truncates toward zero; y and z round down.
        let tiny = -0.5 / 511.0;
        assert_eq!(decode_unit_normal(pack_unit_vec([tiny, tiny, tiny])), [0.0, -1.0 / 511.0, -1.0 / 511.0]);
    }

    #[test]
    fn halves_decode() {
        assert_eq!(short_float_to_full(0x3c00), 1.0);
        assert_eq!(short_float_to_full(0xc000), -2.0);
        assert_eq!(short_float_to_full(0x3800), 0.5);
        assert_eq!(short_float_to_full(0x0001), 2f32.powi(-24));
        assert!(short_float_to_full(0x7c00).is_infinite());
    }

    #[test]
    fn halves_encode() {
        for bits in [0x3c00u16, 0xc000, 0x3800, 0x0001, 0x03ff, 0x0400, 0x7bff, 0x8000, 0x3555] {
            assert_eq!(f32_to_half(short_float_to_full(bits)), bits, "{bits:#06x}");
        }
        assert_eq!(f32_to_half(0.0), 0);
        assert_eq!(f32_to_half(1.0e6), 0x7c00);
        assert_eq!(f32_to_half(1.0e-9), 0);
        assert!(short_float_to_full(f32_to_half(f32::NAN)).is_nan());
        // 1 + 2^-11 is halfway between 1 and the next half: ties go to even (1).
        assert_eq!(f32_to_half(1.0 + 2f32.powi(-11)), 0x3c00);
        assert_eq!(f32_to_half(1.0 + 3.0 * 2f32.powi(-11)), 0x3c02);
        // Rounding up out of the mantissa steps the exponent.
        assert_eq!(f32_to_half(2.0 - 2f32.powi(-12)), 0x4000);
    }

    fn sample_world_row() -> [u8; T6_WORLD_VERTEX] {
        let mut row = [0_u8; T6_WORLD_VERTEX];
        row[0..4].copy_from_slice(&1.5f32.to_le_bytes());
        row[4..8].copy_from_slice(&(-2.0f32).to_le_bytes());
        row[8..12].copy_from_slice(&64.0f32.to_le_bytes());
        row[12..16].copy_from_slice(&(-1.0f32).to_le_bytes());
        row[16..20].copy_from_slice(&[10, 20, 30, 255]);
        // u = 0.5 (low half), v = 1.0 (high half).
        row[20..24].copy_from_slice(&(0x3c00_3800u32).to_le_bytes());
        row[24..28].copy_from_slice(&pack_unit_vec([0.0, 0.0, 1.0]).to_le_bytes());
        row[28..32].copy_from_slice(&pack_unit_vec([1.0, 0.0, 0.0]).to_le_bytes());
        row[32..36].copy_from_slice(&(0xffff_0000u32).to_le_bytes());
        row
    }

    #[test]
    fn a_world_row_reads_field_by_field() {
        let row = sample_world_row();
        let v = T6WorldVertex::read(&row).unwrap();
        assert_eq!(v.base.position, [1.5, -2.0, 64.0]);
        assert_eq!(v.base.binormal_sign, -1.0);
        assert_eq!(v.base.color, [10, 20, 30, 255]);
        assert_eq!(v.base.color_unorm()[3], 1.0);
        assert_eq!(v.base.uv(), [0.5, 1.0]);
        assert_eq!(v.base.unpacked_normal(), [0.0, 0.0, 1.0]);
        assert_eq!(v.base.unpacked_tangent(), [1.0, 0.0, 0.0]);
        assert_eq!(v.lightmap_uv(), [0.0, 1.0]);
        assert_eq!(v.to_bytes(), row);
    }

    #[test]
    fn a_model_row_is_the_world_row_without_the_lightmap() {
        let world = sample_world_row();
        let v = T6PackedVertex::read(&world[..T6_PACKED_VERTEX]).unwrap();
        assert_eq!(v, T6WorldVertex::read(&world).unwrap().base);
        assert_eq!(v.to_bytes()[..], world[..T6_PACKED_VERTEX]);
        // A longer slice reads only its first row.
        assert_eq!(T6PackedVertex::read(&world), Some(v));
    }

    /// Real model rows from our loader: mp_nuketown_2020 then mp_carrier, three of
    /// each binormal sign (facts/vertex-format-t6-raw.txt L25-30, L63-68).
    const REAL_MODEL_ROWS: [(f32, &str); 12] = [
        (1.0, "7b153c40080244bee290ad3e0000803fffffffffa53a25387f4c890fa0fa6b2f"),
        (1.0, "47029f3f0d985a41fbb2403f0000803fffffffffc835bf3b5f02701224b9be19"),
        (1.0, "bf3c9c40a4fd00c0f46d833f0000803fffffffff6b399638d8f10c008d581516"),
        (-1.0, "feffebc0feff83c00ef53b3a000080bfffffffff6ab1d53500341c1c00fcd830"),
        (-1.0, "da1081c0f147613e903ab041000080bfffffffffc53b34370cfce73ff0d15f38"),
        (-1.0, "d57c1d3f3259f23f125da540000080bfffffffff013b9c389e9807001a7a0200"),
        (1.0, "00000000bd3786b5030000410000803fffffffff663b00000004080001020000"),
        (1.0, "84c6c740c114fbc10000803f0000803fffffffff873bec3b642c08000b720e00"),
        (1.0, "e7fb1d403a5d263f8edadd400000803fffffffffe23861340102000000fcf73f"),
        (-1.0, "99599ec00000000099599e40000080bfffffffffb030b03000040800ff010000"),
        (-1.0, "99599ec00000000099599e40000080bfffffffffb030b03000040800ff010000"),
        (-1.0, "c760b3423c7f00c152f10940000080bfffffffffd5275f27e84b4b26ee93e912"),
    ];

    fn real_row(hex: &str) -> [u8; T6_PACKED_VERTEX] {
        let mut row = [0_u8; T6_PACKED_VERTEX];
        for (i, b) in row.iter_mut().enumerate() {
            *b = u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16).unwrap();
        }
        row
    }

    #[test]
    fn real_model_rows_read_and_write_back() {
        for (sign, hex) in REAL_MODEL_ROWS {
            let row = real_row(hex);
            let v = T6PackedVertex::read(&row).unwrap();
            assert_eq!(v.to_bytes(), row, "{hex}");
            // Rule 3: the sign is exactly +1 or -1.
            assert_eq!(v.binormal_sign, sign, "{hex}");
            assert!(v.position.iter().all(|p| p.is_finite()), "{hex}");
            assert!(v.uv().iter().all(|t| t.is_finite()), "{hex}");
        }
    }

    #[test]
    fn real_normals_and_tangents_are_signed_ten_bit_units() {
        let len = |v: [f32; 3]| v.iter().map(|c| c * c).sum::<f32>().sqrt();
        for (_, hex) in REAL_MODEL_ROWS {
            let v = T6PackedVertex::read(&real_row(hex)).unwrap();
            for word in [v.normal, v.tangent] {
                // Rule 6: bits 30..31 are 0 and the length over 511 is 1 within 0.002.
                assert_eq!(word >> 30, 0, "{hex}");
                let unit = decode_unit_normal(word);
                assert!((len(unit) - 1.0).abs() <= 0.002, "{hex}: {unit:?}");
                // Rule 7: the packer gives back the stored word.
                assert_eq!(pack_unit_vec(unit), word, "{hex}");
            }
            // Rule 8: the normal and the tangent are perpendicular.
            let (n, t) = (v.unpacked_normal(), v.unpacked_tangent());
            let dot: f32 = n.iter().zip(t).map(|(a, b)| a * b).sum();
            assert!(dot.abs() <= 0.003, "{hex}: {dot}");
        }
    }

    #[test]
    fn short_rows_are_refused() {
        let world = sample_world_row();
        assert_eq!(T6WorldVertex::read(&world[..T6_WORLD_VERTEX - 1]), None);
        assert_eq!(T6PackedVertex::read(&world[..T6_PACKED_VERTEX - 1]), None);
    }
}
