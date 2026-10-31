//! The per-pixel alpha cutoff a pass checks before writing colour.

use crate::compare::CompareOp;

/// The cutoff is stored as a byte; shaders see alpha as 0 to 1, so they scale by this.
pub const ALPHA_CUTOFF_UNIT: f32 = 255.0;

/// Older spelling AlphaTest still calls; the rename sweep moves it over.

/// A pixel survives when its alpha, compared with `func`, beats the `reference` byte.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct AlphaGate {
    pub func: CompareOp,
    pub cutoff: u8, // cutoff byte, 0..=255
}

impl AlphaGate {
    /// Builds the gate from a raw comparison code and the cutoff byte.
    pub const fn from_raw(code: u32, cutoff: u8) -> Self {
        let func = CompareOp::from_code(code);
        Self { func, cutoff }
    }

    /// The cutoff mapped into the 0 to 1 range the shader works in.
    pub fn unit_scale_reference(self) -> f32 {
        let cutoff = f32::from(self.cutoff);
        cutoff / ALPHA_CUTOFF_UNIT
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn raw_code_becomes_the_named_comparison() {
        let gate = AlphaGate::from_raw(5, 0);
        assert_eq!((gate.func, gate.cutoff), (CompareOp::Greater, 0));
    }

    #[test]
    fn cutoff_maps_into_zero_to_one() {
        let unit = |code, cutoff| AlphaGate::from_raw(code, cutoff).unit_scale_reference();
        assert_eq!(unit(2, 255), 1.0);
        assert_eq!(unit(2, 0), 0.0);
        assert!((unit(7, 128) - 128.0 / 255.0).abs() < 1e-6);
    }
}
