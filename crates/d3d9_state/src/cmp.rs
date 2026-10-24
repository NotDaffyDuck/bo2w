//! Comparison functions, shared by the depth test and the alpha test.

pub const D3DCMP_LESS: u32 = 2;
pub const D3DCMP_EQUAL: u32 = 3;
pub const D3DCMP_LESSEQUAL: u32 = 4;
pub const D3DCMP_GREATER: u32 = 5;
pub const D3DCMP_GREATEREQUAL: u32 = 7;
pub const D3DCMP_ALWAYS: u32 = 8;

/// Raw codes 1 to 8, in order.
const BY_CODE: [CompareFunc; 8] = [
    CompareFunc::Never,
    CompareFunc::Less,
    CompareFunc::Equal,
    CompareFunc::LessEqual,
    CompareFunc::Greater,
    CompareFunc::NotEqual,
    CompareFunc::GreaterEqual,
    CompareFunc::Always,
];

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum CompareFunc {
    Never,
    Less,
    Equal,
    LessEqual,
    Greater,
    NotEqual,
    GreaterEqual,
    Always,
    /// A code outside 1 to 8, kept as read.
    Unknown(u32),
}

impl CompareFunc {
    pub const fn from_raw(raw: u32) -> Self {
        if raw >= 1 && raw <= 8 {
            BY_CODE[raw as usize - 1]
        } else {
            Self::Unknown(raw)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_one_to_eight_are_named_in_order() {
        assert_eq!(CompareFunc::from_raw(1), CompareFunc::Never);
        assert_eq!(CompareFunc::from_raw(D3DCMP_LESS), CompareFunc::Less);
        assert_eq!(CompareFunc::from_raw(D3DCMP_EQUAL), CompareFunc::Equal);
        assert_eq!(CompareFunc::from_raw(D3DCMP_LESSEQUAL), CompareFunc::LessEqual);
        assert_eq!(CompareFunc::from_raw(D3DCMP_GREATER), CompareFunc::Greater);
        assert_eq!(CompareFunc::from_raw(6), CompareFunc::NotEqual);
        assert_eq!(CompareFunc::from_raw(D3DCMP_GREATEREQUAL), CompareFunc::GreaterEqual);
        assert_eq!(CompareFunc::from_raw(D3DCMP_ALWAYS), CompareFunc::Always);
    }

    #[test]
    fn other_codes_are_kept_unknown() {
        for raw in [0, 9, 255, u32::MAX] {
            assert_eq!(CompareFunc::from_raw(raw), CompareFunc::Unknown(raw));
        }
    }
}
