//! The weights a colour or alpha blend multiplies its two inputs by.

/// Older spelling BlendFactor still calls; the rename sweep moves it over.

/// One blend weight, named after the raw code (shown on the right) it was read from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum BlendWeight {
    Zero,                   // 1
    One,                    // 2
    SrcColor,               // 3
    OneMinusIncomingColor,  // 4
    SrcAlpha,               // 5
    OneMinusIncomingAlpha,  // 6
    DestAlpha,              // 7
    OneMinusTargetAlpha,    // 8
    DestColor,              // 9
    OneMinusTargetColor,    // 10
    SourceAlphaSaturate,    // 11
    Unlisted(u32),          // any other code, as read
}

/// Indexed by `code - 1`.
const NAMED: [BlendWeight; 11] = {
    [
        BlendWeight::Zero,
        BlendWeight::One,
        BlendWeight::SrcColor,
        BlendWeight::OneMinusIncomingColor,
        BlendWeight::SrcAlpha,
        BlendWeight::OneMinusIncomingAlpha,
        BlendWeight::DestAlpha,
        BlendWeight::OneMinusTargetAlpha,
        BlendWeight::DestColor,
        BlendWeight::OneMinusTargetColor,
        BlendWeight::SourceAlphaSaturate,
    ]
};

impl BlendWeight {
    pub const fn from_raw(code: u32) -> Self {
        match code {
            1..=11 => NAMED[(code - 1) as usize],
            other => Self::Unlisted(other),
        }
    }

    /// False only for a code the table has no name for.
    pub const fn is_listed(&self) -> bool {
        match self {
            Self::Unlisted(_) => false,
            _ => true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_pick_their_named_weight() {
        let samples = [
            (1, BlendWeight::Zero),
            (2, BlendWeight::One),
            (5, BlendWeight::SrcAlpha),
            (6, BlendWeight::OneMinusIncomingAlpha),
            (11, BlendWeight::SourceAlphaSaturate),
        ];
        for (code, weight) in samples {
            assert_eq!(BlendWeight::from_raw(code), weight);
        }
    }

    #[test]
    fn only_out_of_table_codes_are_unknown() {
        assert!((1..=11).all(|code| BlendWeight::from_raw(code).is_listed()));
        for code in [0, 12, 0x20] {
            let weight = BlendWeight::from_raw(code);
            assert!(!weight.is_listed());
            assert_eq!(weight, BlendWeight::Unlisted(code));
        }
    }
}
