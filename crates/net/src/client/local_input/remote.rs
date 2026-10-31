//! Steering a missile the player is riding.

use super::{PlayerControlSnapshot, intent_pack::mouse_distance};

/// The (pitch, yaw) steering bytes for a ridden missile: from the mouse while it moves, else
/// from the right stick (pushed up noses it up, pushed right turns it right). Each is a signed
/// byte in -127..=127 stored as `u8`.
pub fn remote_control_axes(
    mouse_cfg: &PlayerControlSnapshot,
    pointer_delta_horizontal: f32,
    pointer_delta_vertical: f32,
    pad_look: [f32; 2],
) -> [u8; 2] {
    let axis = |v: f32| (v.clamp(-1.0, 1.0) * 127.0).round() as i8 as u8;
    if pointer_delta_horizontal == 0.0 && pointer_delta_vertical == 0.0 {
        return [axis(-pad_look[1]), axis(-pad_look[0])];
    }
    let (mx, my) = input_t6::scale_pointer_motion(
        pointer_delta_horizontal,
        pointer_delta_vertical,
        mouse_distance(pointer_delta_horizontal, pointer_delta_vertical),
        mouse_cfg.tick_span_ms.max(1) as f32,
        mouse_cfg.sensitivity,
        mouse_cfg.pointer_acceleration,
        mouse_cfg.view_angle_multiplier,
    );
    [axis(mouse_cfg.pitch_per_count * my), axis(-mouse_cfg.yaw_sensitivity * mx)]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn signed(byte: u8) -> i8 {
        byte as i8
    }

    #[test]
    fn the_right_stick_steers_when_the_mouse_is_still() {
        let input = PlayerControlSnapshot::default();
        let up = remote_control_axes(&input, 0.0, 0.0, [0.0, 1.0]);
        assert_eq!((signed(up[0]), signed(up[1])), (-127, 0));
        let right = remote_control_axes(&input, 0.0, 0.0, [1.0, 0.0]);
        assert_eq!((signed(right[0]), signed(right[1])), (0, -127));
        let diagonal = remote_control_axes(&input, 0.0, 0.0, [-0.5, -0.5]);
        assert_eq!((signed(diagonal[0]), signed(diagonal[1])), (64, 64));
        assert_eq!(remote_control_axes(&input, 0.0, 0.0, [0.0, 0.0]), [0, 0]);
    }

    #[test]
    fn the_stick_is_held_to_full_deflection() {
        let input = PlayerControlSnapshot::default();
        let wild = remote_control_axes(&input, 0.0, 0.0, [-9.0, 9.0]);
        assert_eq!((signed(wild[0]), signed(wild[1])), (-127, 127));
    }

    #[test]
    fn the_mouse_wins_while_it_moves() {
        let input = PlayerControlSnapshot::default();
        let mouse = remote_control_axes(&input, 40.0, 0.0, [0.0, 1.0]);
        assert_eq!(mouse[0], 0);
        assert_ne!(mouse[1], 0);
        // Moving right turns right, which is a negative yaw byte.
        assert!(signed(mouse[1]) < 0);
    }
}
