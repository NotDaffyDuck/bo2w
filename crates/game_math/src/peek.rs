//! Leaning round a corner: the eased lean amount and the sideways shift of the eye it gives.

use crate::angle_vectors;

/// Eases a lean amount in [-1, 1]: `(2 - |x|) * x`, steep at the start and flat at full lean.
#[inline]
pub fn get_lean_fraction(lean: f32) -> f32 {
    (2.0 - lean.abs()) * lean
}

/// `position` shifted sideways for a lean of `lean` (raw, eased here) at most `reach` units, the sideways direction
/// tilted by `roll` scaled by the eased lean.
pub fn add_lean_to_position(position: [f32; 3], yaw: f32, lean: f32, roll: f32, reach: f32) -> [f32; 3] {
    if lean == 0.0 {
        return position;
    }
    let eased = get_lean_fraction(lean);
    let (_, side, _) = angle_vectors([0.0, yaw, roll * eased]);
    let shift = eased * reach;
    core::array::from_fn(|i| position[i] + shift * side[i])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_lean_no_shift() {
        assert_eq!(add_lean_to_position([1.0, 2.0, 3.0], 45.0, 0.0, 10.0, 16.0), [1.0, 2.0, 3.0]);
        assert_eq!(get_lean_fraction(1.0), 1.0);
        assert_eq!(get_lean_fraction(-0.5), -0.75);
    }

    #[test]
    fn a_full_lean_moves_the_full_reach_to_the_right() {
        let moved = add_lean_to_position([0.0; 3], 0.0, 1.0, 0.0, 16.0);
        assert!((moved[1] + 16.0).abs() < 1e-5 && moved[0].abs() < 1e-5, "{moved:?}");
    }
}
