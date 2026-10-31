//! Storage formats for the elements of a vertex declaration.

/// The memory layout of one element. Codes 0x40 to 0x42 are packings of this
/// engine that sit past the D3D9 numbering; the vertex shader we emit starts
/// with helpers that unpack them.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum VertexElementFormat {
    /// Code 1: two 32-bit floats.
    TwoFloats,
    /// Code 15: two 16-bit floats.
    Float16x2,
    /// Code 2: three 32-bit floats.
    ThreeFloats,
    /// Code 3: four 32-bit floats.
    QuadFloat,
    /// Code 4: a colour, four bytes.
    PackedColorBytes,
    /// Code 5: four bytes read as integers.
    FourBytesRaw,
    /// Code 8: four bytes scaled to 0..1.
    FourBytesNormalised,
    /// Code 0x40: a unit vector as signed 10:10:10 in a u32 (surface directions,
    /// tangents).
    T6UnitVec,
    /// Code 0x41: u and v as half floats sharing a u32, u in the low 16 bits.
    T6TexCoord,
    /// Code 0x42: four u32s carrying a world vertex's texture coordinate and its
    /// lightmap coordinate.
    T6WorldTexLmap,
    /// Any code with no name above, kept as read.
    Unknown(u8),
}

impl VertexElementFormat {
    /// Names a raw declaration type code.
    pub const fn from_raw(code: u8) -> Self {
        match code {
            1 => Self::TwoFloats,
            2 => Self::ThreeFloats,
            3 => Self::QuadFloat,
            4 => Self::PackedColorBytes,
            5 => Self::FourBytesRaw,
            8 => Self::FourBytesNormalised,
            15 => Self::Float16x2,
            0x40 => Self::T6UnitVec,
            0x41 => Self::T6TexCoord,
            0x42 => Self::T6WorldTexLmap,
            unnamed => Self::Unknown(unnamed),
        }
    }

    /// How many bytes one element takes, or `None` when the format is unknown.
    pub const fn byte_len(self) -> Option<u8> {
        let width: u8 = match self {
            Self::TwoFloats => 8,
            Self::ThreeFloats => 12,
            Self::QuadFloat | Self::T6WorldTexLmap => 16,
            Self::Float16x2
            | Self::PackedColorBytes
            | Self::FourBytesRaw
            | Self::FourBytesNormalised
            | Self::T6UnitVec
            | Self::T6TexCoord => 4,
            _ => return None,
        };
        Some(width)
    }
}

#[cfg(test)]
mod tests {
    use super::VertexElementFormat;

    #[test]
    fn every_named_code_maps_and_sizes() {
        let table = [
            (1, VertexElementFormat::TwoFloats, 8),
            (2, VertexElementFormat::ThreeFloats, 12),
            (3, VertexElementFormat::QuadFloat, 16),
            (4, VertexElementFormat::PackedColorBytes, 4),
            (5, VertexElementFormat::FourBytesRaw, 4),
            (8, VertexElementFormat::FourBytesNormalised, 4),
            (15, VertexElementFormat::Float16x2, 4),
            (0x40, VertexElementFormat::T6UnitVec, 4),
            (0x41, VertexElementFormat::T6TexCoord, 4),
            (0x42, VertexElementFormat::T6WorldTexLmap, 16),
        ];
        for (code, format, size) in table {
            assert_eq!(VertexElementFormat::from_raw(code), format);
            assert_eq!(format.byte_len(), Some(size));
        }
    }

    #[test]
    fn other_codes_round_trip_without_a_size() {
        for code in [0, 6, 7, 9, 14, 16, 0x3f, 0x43, 0xff] {
            let format = VertexElementFormat::from_raw(code);
            assert_eq!(format, VertexElementFormat::Unknown(code));
            assert!(format.byte_len().is_none());
        }
    }
}
