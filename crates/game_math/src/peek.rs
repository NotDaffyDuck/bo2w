//! Leaning round a corner: the eased lean amount and the sideways shift of the eye it gives.

use crate::euler_basis;

/// Eases a lean amount in [-1, 1]: `(2 - |x|) * x`, steep at the start and flat at full lean.
#[inline]
pub fn lean_amount(lean: f32) -> f32 {
    (2.0 - lean.abs()) * lean
}

/// `origin` shifted sideways for a lean of `lean` (raw, eased here) at most `reach` units, the sideways direction
/// tilted by `roll` scaled by the eased lean.
pub fn lean_offset(origin: [f32; 3], yaw: f32, lean: f32, roll: f32, reach: f32) -> [f32; 3] {
    let is_leaning = lean != 0.0;
    if is_leaning {
        let eased = lean_amount(lean);
        let side = euler_basis([0.0, yaw, roll * eased]).1;
        let shift = eased * reach;
        core::array::from_fn(|i| origin[i] + shift * side[i])
    } else {
        origin
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_lean_no_shift() {
        assert_eq!(lean_offset([1.0, 2.0, 3.0], 45.0, 0.0, 10.0, 16.0), [1.0, 2.0, 3.0]);
        assert_eq!(lean_amount(1.0), 1.0);
        assert_eq!(lean_amount(-0.5), -0.75);
    }

    #[test]
    fn a_full_lean_moves_the_full_reach_to_the_right() {
        let moved = lean_offset([0.0; 3], 0.0, 1.0, 0.0, 16.0);
        assert!((moved[1] + 16.0).abs() < 1e-5 && moved[0].abs() < 1e-5, "{moved:?}");
    }
}
