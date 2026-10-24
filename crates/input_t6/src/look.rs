//! Turning: the look keys, and the mouse.

use crate::KbuttonSet;

/// Degrees to short-angle units.
pub const ANGLE2SHORT: f32 = 65536.0 / 360.0;

/// Black Ops II's cl_yawspeed: degrees a second the turn keys turn.
pub const CL_YAWSPEED_DEFAULT: f32 = 140.0;
/// cl_pitchspeed: degrees a second the look up and down keys turn.
pub const CL_PITCHSPEED_DEFAULT: f32 = 140.0;
/// cl_anglespeedkey: the speed key multiplies the key turn by this.
pub const CL_ANGLESPEEDKEY_DEFAULT: f32 = 1.5;

#[derive(Clone, Copy, Debug)]
pub struct AdjustAnglesInput {
    /// Seconds since the last frame.
    pub dt: f32,
    pub now_msec: i32,
    pub frame_msec: u32,
    pub cl_yawspeed: f32,
    pub cl_pitchspeed: f32,
    pub cl_anglespeedkey: f32,
    /// The game's own turn caps, degrees a second; 0 or less is no cap.
    pub cgame_max_yaw_speed: f32,
    pub cgame_max_pitch_speed: f32,
    pub frozen: bool,
}

/// A turn rate, held to the game's cap when it sets one.
fn capped(rate: f32, max: f32) -> f32 {
    if max <= 0.0 || rate < max { rate } else { max }
}

/// This frame's key turn, `(pitch, yaw)` in degrees: each look key turns by the share of the
/// frame it was held. The turn keys do nothing while strafe is held (they move instead).
pub fn adjust_angles(kb: &mut KbuttonSet, input: AdjustAnglesInput) -> (f32, f32) {
    if input.frozen {
        return (0.0, 0.0);
    }
    let (now, frame) = (input.now_msec, input.frame_msec);
    let scale = if kb.speed.active {
        input.dt * input.cl_anglespeedkey
    } else {
        input.dt
    };
    let mut yaw = 0.0;
    if !kb.strafe.active {
        let step = capped(input.cl_yawspeed, input.cgame_max_yaw_speed) * scale;
        let right = kb.right.fraction(now, frame);
        let left = kb.left.fraction(now, frame);
        yaw = yaw - right * step + left * step;
    }
    let step = capped(input.cl_pitchspeed, input.cgame_max_pitch_speed) * scale;
    let up = kb.lookup.fraction(now, frame);
    let down = kb.lookdown.fraction(now, frame);
    let pitch = 0.0 - up * step + down * step;
    (pitch, yaw)
}

/// The mouse move after sensitivity and acceleration: faster hand speed (counts a
/// millisecond) adds gain, and the zoom's field of view scales it.
pub fn apply_mouse_sensitivity(
    mx: f32,
    my: f32,
    mouse_counts: f32,
    frame_msec: f32,
    sensitivity: f32,
    mouse_accel: f32,
    fov_scale: f32,
) -> (f32, f32) {
    let frame = if frame_msec > 0.0 { frame_msec } else { 1.0 };
    let gain = (mouse_counts / frame * mouse_accel + sensitivity) * fov_scale;
    (mx * gain, my * gain)
}

/// The mouse turn, `(pitch, yaw)` in short-angle units: moving right turns right (yaw down).
pub fn mouse_move_angles(mx: f32, my: f32, m_yaw: f32, m_pitch: f32) -> (i32, i32) {
    (
        (my * m_pitch * ANGLE2SHORT) as i32,
        (-mx * m_yaw * ANGLE2SHORT) as i32,
    )
}
