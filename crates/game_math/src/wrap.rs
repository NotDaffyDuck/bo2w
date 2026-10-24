//! Angles that go round: wrapping into one turn, the short way between two angles, easing toward a target, and
//! rounding nearly whole angles. All in single precision with BO2's operation order, so results match its bits.

const DEGREES_PER_TURN: f32 = 360.0;
const TURN_PER_DEGREE: f32 = 1.0 / 360.0;

/// A gap this small counts as already there.
const SETTLED: f32 = 0.001;

/// Two angles closer than this to a whole number (squared distance) snap to it.
const SNAP_WINDOW_SQ: f32 = f32::from_bits(0x3586_37be);

/// `angle` wrapped into [0, 360).
pub fn angle_normalize_360(angle: f32) -> f32 {
    let turns = angle * TURN_PER_DEGREE;
    let degrees = (turns - libm::floorf(turns)) * DEGREES_PER_TURN;
    // Rounding can land exactly on 360; that is 0.
    let over = degrees - DEGREES_PER_TURN;
    if over >= 0.0 { over } else { degrees }
}

/// `from - to` taken the short way round, in [-180, 180).
pub fn angle_subtract(from: f32, to: f32) -> f32 {
    let turns = (from - to) * TURN_PER_DEGREE;
    (turns - libm::floorf(turns + 0.5)) * DEGREES_PER_TURN
}

/// One easing step from `current` toward `target`: the remaining gap shrinks by `rate * frame` of itself. Stays put
/// when already within [`SETTLED`], or when the step would overshoot.
pub fn track(current: f32, target: f32, rate: f32, frame: f32) -> f32 {
    let gap = current - target;
    let step = gap * rate * frame;
    if gap.abs() <= SETTLED || gap.abs() < step.abs() {
        current
    } else {
        target + step
    }
}

/// [`track`] for an angle: `current` is first brought within half a turn of `target`, and the result wrapped.
pub fn track_angle(current: f32, target: f32, rate: f32, frame: f32) -> f32 {
    let mut near = current;
    while near - target > 180.0 {
        near -= DEGREES_PER_TURN;
    }
    while near - target < -180.0 {
        near += DEGREES_PER_TURN;
    }
    angle_subtract(track(near, target, rate, frame), 0.0)
}

/// Each angle that is within a hair of a whole degree becomes that whole degree.
pub fn snap_angles(angles: [f32; 3]) -> [f32; 3] {
    angles.map(|angle| {
        // Adding zero first makes -0 round to +0, as BO2's rounding does.
        let whole = libm::roundf(angle + 0.0);
        let off = whole - angle;
        if off * off < SNAP_WINDOW_SQ { whole } else { angle }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wrapping_keeps_one_turn() {
        assert_eq!(angle_normalize_360(-90.0), 270.0);
        assert_eq!(angle_normalize_360(720.0), 0.0);
        assert!((angle_subtract(350.0, 10.0) + 20.0).abs() < 1e-4);
        assert!((angle_subtract(10.0, 350.0) - 20.0).abs() < 1e-4);
    }

    #[test]
    fn easing_stops_when_close_or_overshooting() {
        assert_eq!(track(10.0, 10.0005, 1.0, 1.0), 10.0);
        assert_eq!(track(10.0, 0.0, 0.5, 1.0), 5.0);
        assert_eq!(track(10.0, 0.0, 3.0, 1.0), 10.0);
        assert_eq!(track_angle(350.0, 10.0, 0.5, 1.0), 0.0);
    }

    #[test]
    fn near_whole_angles_snap() {
        assert_eq!(snap_angles([89.9999, 12.3, -0.0]), [90.0, 12.3, 0.0]);
    }
}
