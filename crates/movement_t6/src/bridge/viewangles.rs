//! Conversions between degrees and the 16-bit angles a command carries, and
//! the limits a view is held to.

/// Degrees per 16-bit angle step (360 / 65536).
pub const SHORT2ANGLE: f32 = 0.005_493_164_062_5;

/// 16-bit angle steps per degree.
pub const ANGLE2SHORT: f32 = 65536.0 / 360.0;

/// How far the view may pitch, and whether the clamp is lifted this frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ViewAngleClamp {
    /// The furthest the view may look up, in degrees.
    pub pitch_up: f32,
    /// The furthest the view may look down, in degrees.
    pub pitch_down: f32,
    /// The player-state bit that lifts the pitch clamp.
    pub unclamped_pitch_bit: bool,
}
