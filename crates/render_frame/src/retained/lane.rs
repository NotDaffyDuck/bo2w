//! Which of the camera-facing lists a map surface belongs to.

/// The list a map surface was sorted into for the camera.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BspCameraLane {
    /// Lit, fully opaque surfaces.
    LitOpaque,
    /// Lit surfaces that blend with what is behind them.
    LitTrans,
    /// Decals laid over other surfaces.
    Decal,
    /// Surfaces that emit their own light.
    Emissive,
}
