//! Life spans and the ages of trail elements.

/// A whole number drawn from `base ..= base + amplitude`, picked by the 16-bit
/// random `roll16`.
pub fn sample_life_span_msec(base: i32, spread: i32, roll16: u16) -> i32 {
    let choices = spread.wrapping_add(1);
    base.wrapping_add(i32::from(roll16).wrapping_mul(choices) >> 16)
}

/// Whether a trail element born at `window_open_ms` is still alive at `clock_ms_now`.
pub fn trail_elem_keep(clock_ms_now: i32, window_open_ms: i32, lifespan_ms: i32) -> bool {
    clock_ms_now < window_open_ms.wrapping_add(lifespan_ms)
}

/// The age, as a fraction of the life, of a trail element at the start and at
/// the end of an update. The start is never earlier than the birth. A life of
/// zero gives zero ages.
pub fn trail_elem_norm_ages(
    prior_ms: i32,
    clock_ms_now: i32,
    window_open_ms: i32,
    lifespan_ms: i32,
) -> (f32, f32) {
    let from = prior_ms.max(window_open_ms);
    let per_msec = if lifespan_ms == 0 {
        0.0
    } else {
        1.0 / lifespan_ms as f32
    };
    (
        from.wrapping_sub(window_open_ms) as f32 * per_msec,
        clock_ms_now.wrapping_sub(window_open_ms) as f32 * per_msec,
    )
}

/// A trail element's vertical base velocity as the 16-bit value it is stored
/// in, cut to that range.
pub fn trail_elem_base_vel_z_pack(vel_z: f32) -> i16 {
    (vel_z as i32).clamp(i32::from(i16::MIN), i32::from(i16::MAX)) as i16
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_span_runs_from_base_to_base_plus_amplitude() {
        assert_eq!(sample_life_span_msec(100, 50, 0), 100);
        assert_eq!(sample_life_span_msec(100, 50, 0xffff), 150);
        assert_eq!(sample_life_span_msec(100, 0, 0xffff), 100);
    }

    #[test]
    fn a_middle_random_lands_in_the_middle() {
        assert_eq!(sample_life_span_msec(0, 99, 0x8000), 50);
    }

    #[test]
    fn a_trail_element_lives_until_its_life_is_up() {
        assert!(trail_elem_keep(99, 0, 100));
        assert!(!trail_elem_keep(100, 0, 100));
        assert!(trail_elem_keep(-5, 0, 100));
    }

    #[test]
    fn ages_are_fractions_of_the_life() {
        assert_eq!(trail_elem_norm_ages(50, 75, 0, 100), (0.5, 0.75));
    }

    #[test]
    fn the_start_age_is_never_before_birth() {
        assert_eq!(trail_elem_norm_ages(-30, 50, 0, 100), (0.0, 0.5));
    }

    #[test]
    fn a_zero_life_has_zero_ages() {
        assert_eq!(trail_elem_norm_ages(0, 10, 0, 0), (0.0, 0.0));
    }

    #[test]
    fn the_vertical_velocity_pack_cuts_to_sixteen_bits() {
        assert_eq!(trail_elem_base_vel_z_pack(12.9), 12);
        assert_eq!(trail_elem_base_vel_z_pack(-12.9), -12);
        assert_eq!(trail_elem_base_vel_z_pack(1.0e9), 0x7fff);
        assert_eq!(trail_elem_base_vel_z_pack(-1.0e9), -0x8000);
    }
}
