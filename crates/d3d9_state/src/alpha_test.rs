//! The alpha test a pass applies before it writes a pixel.

use crate::cmp::CompareFunc;

/// The reference is a byte; the shader compares it against alpha in 0 to 1.
pub const ALPHA_REF_SCALE: f32 = 255.0;

/// Keep a pixel when `alpha <func> reference`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct AlphaTest {
    pub func: CompareFunc,
    pub reference: u8,
}

impl AlphaTest {
    pub const fn from_raw(func: u32, reference: u8) -> Self {
        Self {
            func: CompareFunc::from_raw(func),
            reference,
        }
    }

    /// The reference as the 0 to 1 value the shader compares alpha with.
    pub fn normalised_reference(self) -> f32 {
        f32::from(self.reference) / ALPHA_REF_SCALE
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_raw_names_the_function() {
        let test = AlphaTest::from_raw(5, 0);
        assert_eq!(test.func, CompareFunc::Greater);
        assert_eq!(test.reference, 0);
    }

    #[test]
    fn reference_scales_to_unit_range() {
        assert_eq!(AlphaTest::from_raw(2, 255).normalised_reference(), 1.0);
        assert_eq!(AlphaTest::from_raw(2, 0).normalised_reference(), 0.0);
        let half = AlphaTest::from_raw(7, 128).normalised_reference();
        assert!((half - 128.0 / 255.0).abs() < 1e-6);
    }
}
