//! Element data formats a declaration can name.

/// How one declaration element is stored. The `T6*` formats are engine-private
/// packings with raw codes outside the D3D9 range; the vertex stage unpacks
/// them with the helper functions the emitter prepends.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum DeclType {
    Float2,
    Float16x2,
    Float3,
    Float4,
    D3dColor,
    UByte4,
    UByte4N,
    /// Signed 10:10:10 unit vector in one u32 (normals, tangents). Code 0x40.
    T6UnitVec,
    /// Two half floats in one u32, u in the low half. Code 0x41.
    T6TexCoord,
    /// Four u32s spanning a world vertex texture coordinate and its lightmap
    /// coordinate. Code 0x42.
    T6WorldTexLmap,
    /// A code this crate has no name for.
    Unknown(u8),
}

impl DeclType {
    pub const fn from_raw(raw: u8) -> Self {
        match raw {
            1 => Self::Float2,
            2 => Self::Float3,
            3 => Self::Float4,
            4 => Self::D3dColor,
            5 => Self::UByte4,
            8 => Self::UByte4N,
            15 => Self::Float16x2,
            0x40 => Self::T6UnitVec,
            0x41 => Self::T6TexCoord,
            0x42 => Self::T6WorldTexLmap,
            other => Self::Unknown(other),
        }
    }

    /// Bytes one element occupies; `None` for a type this crate cannot size.
    pub const fn byte_len(self) -> Option<u8> {
        match self {
            Self::Float16x2
            | Self::D3dColor
            | Self::UByte4
            | Self::UByte4N
            | Self::T6UnitVec
            | Self::T6TexCoord => Some(4),
            Self::Float2 => Some(8),
            Self::Float3 => Some(12),
            Self::Float4 | Self::T6WorldTexLmap => Some(16),
            Self::Unknown(_) => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_codes_have_names_and_sizes() {
        let cases = [
            (1, DeclType::Float2, 8),
            (2, DeclType::Float3, 12),
            (3, DeclType::Float4, 16),
            (4, DeclType::D3dColor, 4),
            (5, DeclType::UByte4, 4),
            (8, DeclType::UByte4N, 4),
            (15, DeclType::Float16x2, 4),
            (0x40, DeclType::T6UnitVec, 4),
            (0x41, DeclType::T6TexCoord, 4),
            (0x42, DeclType::T6WorldTexLmap, 16),
        ];
        for (raw, kind, bytes) in cases {
            assert_eq!(DeclType::from_raw(raw), kind);
            assert_eq!(kind.byte_len(), Some(bytes));
        }
    }

    #[test]
    fn unnamed_codes_keep_their_value_and_have_no_size() {
        for raw in [0, 6, 7, 9, 14, 16, 0x3f, 0x43, 0xff] {
            let kind = DeclType::from_raw(raw);
            assert_eq!(kind, DeclType::Unknown(raw));
            assert_eq!(kind.byte_len(), None);
        }
    }
}
