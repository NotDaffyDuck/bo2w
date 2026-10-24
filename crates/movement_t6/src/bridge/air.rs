//! The scales an airborne move reads.

/// How the air move is tuned.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AirMoveContext {
    /// Spectators move at this multiple of their normal speed.
    pub player_spectate_speed_scale: f32,
    /// A shell-shocked player falls scaled by this, plus the bias below.
    pub shellshock_gravity_scale: f32,
    pub shellshock_gravity_bias: f32,
}
