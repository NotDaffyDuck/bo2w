//! The scales an airborne move reads.

/// How the air move is tuned.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AirborneStepArgs {
    /// Spectators move at this multiple of their normal speed.
    pub observer_pace_ratio: f32,
    /// A shell-shocked player falls scaled by this, plus the bias below.
    pub concussed_fall_ratio: f32,
    pub concussed_fall_offset: f32,
}
