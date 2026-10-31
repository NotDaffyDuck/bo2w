//! Frame lengths in whole milliseconds.

/// The longest frame the key-held timers will count, in ms.
pub const INPUT_SPAN_MILLIS_LIMIT: u32 = 200;

/// The time since startup as a millisecond clock, never below 1.
pub fn frame_time_msec(uptime_secs: f32) -> i32 {
    (uptime_secs * 1000.0).max(1.0) as i32
}

/// A frame's length in ms, rounded, and held between 1 and `INPUT_SPAN_MILLIS_LIMIT`.
pub fn pressed_frame_millis(span_secs: f32) -> u32 {
    let whole_ms = (span_secs.max(0.0) * 1000.0).round() as i32;
    whole_ms.clamp(1, INPUT_SPAN_MILLIS_LIMIT as i32) as u32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_clock_truncates_and_never_reads_zero() {
        assert_eq!(frame_time_msec(0.0), 1);
        assert_eq!(frame_time_msec(-3.0), 1);
        assert_eq!(frame_time_msec(0.0169), 16);
        assert_eq!(frame_time_msec(2.5), 2500);
    }

    #[test]
    fn a_frame_is_rounded_and_held_between_one_and_the_maximum() {
        assert_eq!(pressed_frame_millis(0.0), 1);
        assert_eq!(pressed_frame_millis(-1.0), 1);
        assert_eq!(pressed_frame_millis(0.016), 16);
        assert_eq!(pressed_frame_millis(0.0166), 17);
        assert_eq!(pressed_frame_millis(0.2), INPUT_SPAN_MILLIS_LIMIT);
        assert_eq!(pressed_frame_millis(30.0), INPUT_SPAN_MILLIS_LIMIT);
    }
}
