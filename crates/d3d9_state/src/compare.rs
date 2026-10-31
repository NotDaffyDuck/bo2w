//! The eight comparison codes a depth test or an alpha test can use.

pub const DEPTH_COMPARE_STRICT_BELOW: u32 = 2;
pub const COMPARE_PASS_IF_SAME: u32 = 3;
pub const DEPTH_COMPARE_BELOW_OR_SAME: u32 = 4;
pub const COMPARE_PASS_IF_ABOVE: u32 = 5;
pub const COMPARE_PASS_IF_ABOVE_OR_SAME: u32 = 7;
pub const COMPARE_PASS_ALWAYS: u32 = 8;

/// Older spelling CompareFunc still calls; the rename sweep moves it over.

/// One comparison, named after the raw code (shown on the right) it was read from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum CompareOp {
    Never,        // 1
    Less,         // 2
    Equal,        // 3
    LessEqual,    // 4
    Greater,      // 5
    NotEqual,     // 6
    GreaterOrEqual, // 7
    Always,       // 8
    Unlisted(u32), // any other code, as read
}

/// Indexed by `code - 1`.
const NAMED: [CompareOp; 8] = {
    [CompareOp::Never, CompareOp::Less, CompareOp::Equal, CompareOp::LessEqual, CompareOp::Greater, CompareOp::NotEqual, CompareOp::GreaterOrEqual, CompareOp::Always]
};

impl CompareOp {
    pub const fn from_code(code: u32) -> Self {
        match code {
            1..=8 => NAMED[(code - 1) as usize],
            _ => Self::Unlisted(code),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_code_names_its_comparison() {
        let expected = [
            (1, CompareOp::Never),
            (DEPTH_COMPARE_STRICT_BELOW, CompareOp::Less),
            (COMPARE_PASS_IF_SAME, CompareOp::Equal),
            (DEPTH_COMPARE_BELOW_OR_SAME, CompareOp::LessEqual),
            (COMPARE_PASS_IF_ABOVE, CompareOp::Greater),
            (6, CompareOp::NotEqual),
            (COMPARE_PASS_IF_ABOVE_OR_SAME, CompareOp::GreaterOrEqual),
            (COMPARE_PASS_ALWAYS, CompareOp::Always),
        ];
        for (code, op) in expected {
            assert_eq!(CompareOp::from_code(code), op);
        }
    }

    #[test]
    fn codes_past_the_table_stay_raw() {
        for code in [0, 9, 255, u32::MAX] {
            assert_eq!(CompareOp::from_code(code), CompareOp::Unlisted(code));
        }
    }
}
