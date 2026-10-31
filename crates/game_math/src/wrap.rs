//! Angles that go round: wrapping into one turn, the short way between two angles, easing toward a target, and
//! rounding nearly whole angles. All in single precision with BO2's operation order, so results match its bits.

const DEGREES_PER_TURN: f32 = 360.0;
const TURN_PER_DEGREE: f32 = 1.0 / 360.0;

/// A gap this small counts as already there.
const SETTLED: f32 = 0.001;

/// Two angles closer than this to a whole number (squared distance) snap to it.
const SNAP_WINDOW_SQ: f32 = f32::from_bits(0x3586_37be);

/// `angle` wrapped into [0, 360).
pub fn wrap_degrees(raw: f32) -> f32 {
    let turns = raw * TURN_PER_DEGREE;
    let degrees = (turns - libm::floorf(turns)) * DEGREES_PER_TURN;
    // Rounding can land exactly on 360; that is 0.
    let over = degrees - DEGREES_PER_TURN;
    if over >= 0.0 { over } else { degrees }
}

/// `from - to` taken the short way round, in [-180, 180).
pub fn degree_gap(from: f32, to: f32) -> f32 {
    let turns = (from - to) * TURN_PER_DEGREE;
    (turns - libm::floorf(turns + 0.5)) * DEGREES_PER_TURN
}

/// One easing step from `start_val` toward `goal`: the remaining gap shrinks by `rate * ticks` of itself. Stays put
/// when already within [`SETTLED`], or when the step would overshoot.
pub fn ease_toward(start_val: f32, goal: f32, rate: f32, ticks: f32) -> f32 {
    let gap = start_val - goal;
    let step = gap * rate * ticks;
    if gap.abs() <= SETTLED || gap.abs() < step.abs() {
        start_val
    } else {
        step + goal
    }
}

/// [`track`] for an angle: `start_val` is first brought within half a turn of `goal`, and the result wrapped.
pub fn ease_angle_toward(start_val: f32, goal: f32, rate: f32, ticks: f32) -> f32 {
    let mut near = start_val;
    while near - goal > 180.0 {
        near -= DEGREES_PER_TURN;
    }
    while near - goal < -180.0 {
        near += DEGREES_PER_TURN;
    }
    degree_gap(ease_toward(near, goal, rate, ticks), 0.0)
}

/// Each angle that is within a hair of a whole degree becomes that whole degree.
pub fn round_degrees(triple: [f32; 3]) -> [f32; 3] {
    triple.map(|angle| {
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
        assert_eq!(wrap_degrees(-90.0), 270.0);
        assert_eq!(wrap_degrees(720.0), 0.0);
        assert!((degree_gap(350.0, 10.0) + 20.0).abs() < 1e-4);
        assert!((degree_gap(10.0, 350.0) - 20.0).abs() < 1e-4);
    }

    #[test]
    fn easing_stops_when_close_or_overshooting() {
        assert_eq!(ease_toward(10.0, 10.0005, 1.0, 1.0), 10.0);
        assert_eq!(ease_toward(10.0, 0.0, 0.5, 1.0), 5.0);
        assert_eq!(ease_toward(10.0, 0.0, 3.0, 1.0), 10.0);
        assert_eq!(ease_angle_toward(350.0, 10.0, 0.5, 1.0), 0.0);
    }

    #[test]
    fn near_whole_angles_snap() {
        assert_eq!(round_degrees([89.9999, 12.3, -0.0]), [90.0, 12.3, 0.0]);
    }
}
