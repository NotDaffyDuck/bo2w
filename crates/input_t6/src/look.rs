//! Turning: the look keys, and the mouse.

use crate::HeldKeyLatchBank;

/// Degrees to short-angle units.
pub const DEGREES_TO_WORD16: f32 = 65536.0 / 360.0;

/// Black Ops II's cl_yawspeed: degrees a second the turn keys turn.
pub const KEY_TURN_RATE: f32 = 140.0;
/// key_tilt_rate: degrees a second the look up and down keys turn.
pub const KEY_TILT_RATE: f32 = 140.0;
/// fast_turn_key_ratio: the speed key multiplies the key turn by this.
pub const FAST_TURN_KEY_RATIO: f32 = 1.5;

#[derive(Clone, Copy, Debug)]
pub struct LookTurnRequest {
    /// Seconds since the last frame.
    pub dt: f32,
    pub present_tick_ms: i32,
    pub tick_span_ms: u32,
    pub key_turn_rate: f32,
    pub key_tilt_rate: f32,
    pub fast_turn_key_ratio: f32,
    /// The game's own turn caps, degrees a second; 0 or less is no cap.
    pub look_turn_rate_cap: f32,
    pub look_tilt_rate_cap: f32,
    pub frozen: bool,
}

/// The game's cap on a turn rate; zero or less sets none.
#[derive(Clone, Copy)]
struct RateCap(f32);

impl RateCap {
    /// `rate`, held to this cap when there is one.
    fn hold(self, rate: f32) -> f32 {
        let RateCap(cap) = self;
        if cap <= 0.0 || rate < cap { rate } else { cap }
    }
}

/// This frame's key turn, `(pitch, yaw)` in degrees: each look key turns by the share of the
/// frame it was held. The turn keys do nothing while strafe is held (they move instead).
pub fn adjust_angles(pressed_key_latch: &mut HeldKeyLatchBank, req: LookTurnRequest) -> (f32, f32) {
    if req.frozen {
        return (0.0, 0.0);
    }
    let (now, frame) = (req.present_tick_ms, req.tick_span_ms);
    let scale = if pressed_key_latch.speed.active {
        req.dt * req.fast_turn_key_ratio
    } else {
        req.dt
    };
    let mut turn_deg = 0.0;
    if !pressed_key_latch.strafe.active {
        let step = RateCap(req.look_turn_rate_cap).hold(req.key_turn_rate) * scale;
        let right = pressed_key_latch.right.fraction(now, frame);
        let left = pressed_key_latch.left.fraction(now, frame);
        turn_deg = turn_deg - right * step + left * step;
    }
    let step = RateCap(req.look_tilt_rate_cap).hold(req.key_tilt_rate) * scale;
    let up = pressed_key_latch.lookup.fraction(now, frame);
    let down = pressed_key_latch.lookdown.fraction(now, frame);
    let tilt_deg = 0.0 - up * step + down * step;
    (tilt_deg, turn_deg)
}

/// The mouse move after sensitivity and acceleration: faster hand speed (counts a
/// millisecond) adds gain, and the zoom's field of view scales it.
pub fn scale_pointer_motion(
    side_counts: f32,
    lift_counts: f32,
    hand_speed: f32,
    tick_span_ms: f32,
    base_gain: f32,
    pointer_acceleration: f32,
    view_angle_multiplier: f32,
) -> (f32, f32) {
    let frame = if tick_span_ms > 0.0 { tick_span_ms } else { 1.0 };
    let gain = (hand_speed / frame * pointer_acceleration + base_gain) * view_angle_multiplier;
    (side_counts * gain, lift_counts * gain)
}

/// The mouse turn, `(pitch, yaw)` in short-angle units: moving right turns right (yaw down).
pub fn pointer_turn_units(side_counts: f32, lift_counts: f32, turn_scale: f32, tilt_scale: f32) -> (i32, i32) {
    (
        (lift_counts * tilt_scale * DEGREES_TO_WORD16) as i32,
        (-side_counts * turn_scale * DEGREES_TO_WORD16) as i32,
    )
}
