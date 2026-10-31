//! The view angles.

use input_t6::{
    DEGREES_TO_WORD16, LookTurnRequest, FAST_TURN_KEY_RATIO, KEY_TILT_RATE, PlayerIntentFeed,
    adjust_angles,
};

use super::ViewAimAccumulator;

/// View angles in degrees (pitch, yaw, roll) as 16-bit angle units.
pub fn aim_angles_from_degrees(viewangles: [f32; 3]) -> [i32; 3] {
    viewangles.map(|degrees| (degrees * DEGREES_TO_WORD16) as i32)
}

/// Turn the view by the keys held this frame (the arrow keys, or whatever is bound to look),
/// adding the result to `look.angles`.
#[allow(clippy::too_many_arguments)]
pub fn fold_aim_input(
    dt: f32,
    present_tick_ms: i32,
    tick_span_ms: u32,
    client: &mut PlayerIntentFeed,
    look: &mut ViewAimAccumulator,
    key_yaw_speed: f32,
    held_still: bool,
    look_tilt_rate_cap: f32,
    look_turn_rate_cap: f32,
) {
    let (pitch_degrees, yaw_degrees) = adjust_angles(
        &mut client.pressed_key_latch,
        LookTurnRequest {
            dt,
            present_tick_ms,
            tick_span_ms,
            key_turn_rate: key_yaw_speed,
            key_tilt_rate: KEY_TILT_RATE,
            fast_turn_key_ratio: FAST_TURN_KEY_RATIO,
            look_turn_rate_cap,
            look_tilt_rate_cap,
            frozen: held_still,
        },
    );
    look.angles[0] = look.angles[0].wrapping_add((pitch_degrees * DEGREES_TO_WORD16) as i32);
    look.angles[1] = look.angles[1].wrapping_add((yaw_degrees * DEGREES_TO_WORD16) as i32);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn degrees_become_angle_units() {
        assert_eq!(aim_angles_from_degrees([0.0; 3]), [0; 3]);
        // 65536 units to the circle; the float scale may land a unit short.
        let units = aim_angles_from_degrees([90.0, 180.0, -90.0]);
        for (got, want) in units.into_iter().zip([16384, 32768, -16384]) {
            assert!((got - want).abs() <= 1, "{got} vs {want}");
        }
    }

    #[test]
    fn no_keys_held_leaves_the_view_where_it_was() {
        let mut client = PlayerIntentFeed::default();
        let mut look = ViewAimAccumulator { angles: [100, -200, 300], ..ViewAimAccumulator::default() };
        fold_aim_input(0.016, 1000, 16, &mut client, &mut look, 140.0, false, 0.0, 0.0);
        assert_eq!(look.angles, [100, -200, 300]);
    }
}
