//! Blend factors for the colour and alpha blend equations.

/// Raw codes 1 to 11, in order.
const BY_CODE: [BlendFactor; 11] = [
    BlendFactor::Zero,
    BlendFactor::One,
    BlendFactor::SrcColor,
    BlendFactor::InvSrcColor,
    BlendFactor::SrcAlpha,
    BlendFactor::InvSrcAlpha,
    BlendFactor::DestAlpha,
    BlendFactor::InvDestAlpha,
    BlendFactor::DestColor,
    BlendFactor::InvDestColor,
    BlendFactor::SrcAlphaSat,
];

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum BlendFactor {
    Zero,
    One,
    SrcColor,
    InvSrcColor,
    SrcAlpha,
    InvSrcAlpha,
    DestAlpha,
    InvDestAlpha,
    DestColor,
    InvDestColor,
    SrcAlphaSat,
    /// A code outside 1 to 11, kept as read.
    Unknown(u32),
}

impl BlendFactor {
    pub const fn from_raw(raw: u32) -> Self {
        if raw >= 1 && raw <= 11 {
            BY_CODE[raw as usize - 1]
        } else {
            Self::Unknown(raw)
        }
    }

    pub const fn is_known(self) -> bool {
        !matches!(self, Self::Unknown(_))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn code_table_matches_the_named_values() {
        assert_eq!(BlendFactor::from_raw(1), BlendFactor::Zero);
        assert_eq!(BlendFactor::from_raw(2), BlendFactor::One);
        assert_eq!(BlendFactor::from_raw(5), BlendFactor::SrcAlpha);
        assert_eq!(BlendFactor::from_raw(6), BlendFactor::InvSrcAlpha);
        assert_eq!(BlendFactor::from_raw(11), BlendFactor::SrcAlphaSat);
    }

    #[test]
    fn only_codes_outside_the_range_are_unknown() {
        assert!((1..=11).all(|raw| BlendFactor::from_raw(raw).is_known()));
        for raw in [0, 12, 0x20] {
            assert!(!BlendFactor::from_raw(raw).is_known());
            assert_eq!(BlendFactor::from_raw(raw), BlendFactor::Unknown(raw));
        }
    }
}
