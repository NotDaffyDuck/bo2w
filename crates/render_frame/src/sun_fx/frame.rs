//! The per-frame result: where the sun is on screen and what the effects aim for.

/// Inputs and goals for one frame. The `*_goal` and `*_fade_*` fields tell the
/// renderer where each full-screen effect is heading and how fast.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SunEffectsFrame {
    /// Generation of the world the definition came from, if one is installed.
    pub world_generation: Option<u64>,
    pub camera_key: u64,
    /// The camera jumped since last frame, so fades should not interpolate.
    pub camera_cut: bool,
    pub dt_ms: i32,
    pub view_dot: f32,
    /// The sun's clip-space position.
    pub clip: [f32; 4],
    pub viewport_w: f32,
    pub viewport_h: f32,
    pub sprite_size: f32,
    pub flare_size: f32,
    pub flare_alpha: f32,
    pub flare_fade_in_ms: i32,
    pub flare_fade_out_ms: i32,
    pub blind_goal: f32,
    pub blind_fade_in_ms: i32,
    pub blind_fade_out_ms: i32,
    pub glare_goal: f32,
    pub glare_fade_in_ms: i32,
    pub glare_fade_out_ms: i32,
    pub behind_camera: bool,
}

/// Where `dot` falls between `min_dot` (0) and `max_dot` (1). A non-finite dot
/// or one at or below the minimum gives 0; a degenerate range, or a dot at or
/// above the maximum, gives 1.
#[must_use]
pub fn angular_lerp(dot: f32, min_dot: f32, max_dot: f32) -> f32 {
    match () {
        _ if !dot.is_finite() || dot <= min_dot => 0.0,
        _ if max_dot <= min_dot || dot >= max_dot => 1.0,
        _ => ((dot - min_dot) / (max_dot - min_dot)).clamp(0.0, 1.0),
    }
}
