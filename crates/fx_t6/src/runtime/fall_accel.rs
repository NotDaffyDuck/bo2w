//! The downward pull on an element.

use crate::runtime::birth_spot::sample_float_range;
use crate::runtime::dice::DICE_LANE_FALL;
use crate::runtime::dice::random_table_f32;

/// Units per second squared that an authored gravity of 1.0 stands for.
const GRAVITY_UNITS: f32 = 800.0;

/// The acceleration for an authored gravity value.
pub(super) fn elem_gravity_accel_z(authored: f32) -> f32 {
    authored * GRAVITY_UNITS
}

/// The acceleration for an element whose authored gravity is `base` plus a
/// random share of `spread`.
pub fn elem_gravity_accel_z_sampled(base: f32, spread: f32, salt: u32) -> f32 {
    let authored = sample_float_range(base, spread, random_table_f32(salt, DICE_LANE_FALL));
    elem_gravity_accel_z(authored)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn authored_gravity_is_scaled_to_units() {
        assert_eq!(elem_gravity_accel_z(1.0), 800.0);
        assert_eq!(elem_gravity_accel_z(-0.5), -400.0);
    }

    #[test]
    fn no_amplitude_means_no_randomness() {
        assert_eq!(elem_gravity_accel_z_sampled(2.0, 0.0, 77), 1600.0);
    }

    #[test]
    fn the_sample_stays_inside_the_authored_range() {
        for seed in 0..64 {
            let got = elem_gravity_accel_z_sampled(1.0, 2.0, seed);
            assert!((800.0..=2400.0).contains(&got), "{got}");
        }
    }
}
