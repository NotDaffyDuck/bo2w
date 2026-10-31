//! Conversions between degrees and the 16-bit angles a command carries, and
//! the limits a view is held to.

/// Degrees per 16-bit angle step (360 / 65536).
pub const WORD16_TO_DEGREES: f32 = 0.005_493_164_062_5;

/// 16-bit angle steps per degree.
pub const DEGREES_TO_WORD16: f32 = 65536.0 / 360.0;

/// How far the view may pitch, and whether the clamp is lifted this frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LookRotationLimit {
    /// The furthest the view may look up, in degrees.
    pub tilt_upper_limit: f32,
    /// The furthest the view may look down, in degrees.
    pub tilt_lower_limit: f32,
    /// The player-state bit that lifts the pitch clamp.
    pub free_tilt_flag: bool,
}
