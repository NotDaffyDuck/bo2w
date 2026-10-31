//! How far through its life an element has come to rest.

/// The `at rest` fraction for an element: whole steps (of 1/255 of its life)
/// elapsed since `window_open_ms`, less a quarter step. `per_life_ms` is one
/// over the life span.
pub fn get_at_rest_fraction(clock_ms_now: f32, window_open_ms: f32, per_life_ms: f32) -> f32 {
    libm::floorf((clock_ms_now - window_open_ms) * 255.0 * per_life_ms - 0.25)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn halfway_through_a_life_is_about_half_of_255() {
        assert_eq!(get_at_rest_fraction(500.0, 0.0, 1.0 / 1000.0), 127.0);
    }

    #[test]
    fn the_start_rounds_down_below_zero() {
        assert_eq!(get_at_rest_fraction(100.0, 100.0, 1.0 / 1000.0), -1.0);
    }

    #[test]
    fn the_end_of_the_life_is_255_less_the_quarter_step() {
        assert_eq!(get_at_rest_fraction(1000.0, 0.0, 1.0 / 1000.0), 254.0);
    }
}
