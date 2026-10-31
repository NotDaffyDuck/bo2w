//! What the depth-of-field and glow passes read each frame: the vision's focus ranges, the
//! blur and bloom settings, and the script's colour grade.

use bevy::prelude::*;

/// Materials the glow chain draws with.
pub const BLOOM_SETUP_MATERIAL: &str = "glow_consistent_setup_color2";
pub const BLOOM_APPLY_MATERIAL: &str = "glow_apply_bloom";

/// A focus range only counts once it is wider than this many inches.
const MIN_FOCUS_SPAN: f32 = 1.0;

fn has_span(first_ix: f32, end: f32) -> bool {
    end > first_ix + MIN_FOCUS_SPAN
}

/// The focus ranges of one frame: the viewmodel, and what is too near or too far.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct FocusBlur {
    pub hand_model_start: f32,
    pub hand_model_end: f32,
    pub near_from: f32,
    pub near_to: f32,
    pub far_from: f32,
    pub far_to: f32,
    pub near_soft: f32,
    pub far_soft: f32,
}

impl FocusBlur {
    /// Whether any range blurs anything. A far range with no far blur does nothing.
    pub fn active(self) -> bool {
        has_span(self.hand_model_start, self.hand_model_end)
            || has_span(self.near_from, self.near_to)
            || (has_span(self.far_from, self.far_to) && self.far_soft > 0.0)
    }
}

/// Colour grade that leaves the picture alone.
const NEUTRAL_GRADE: [f32; 4] = [0.0, 1.0, 0.0, 1.0];

/// Everything the post passes need for the frame being drawn.
#[derive(Resource, Clone, Copy, Debug)]
pub struct BokehFrame {
    pub dof_vals: FocusBlur,
    pub bias: f32,
    pub stage_near: f32,
    pub hand_model_near: f32,
    pub glow: BloomFrame,
    pub blur: f32,
    /// Hue in radians, gamma, exposure in stops, saturation.
    pub grading: [f32; 4],
}

impl Default for BokehFrame {
    fn default() -> Self {
        Self {
            dof_vals: FocusBlur::default(),
            bias: 0.0,
            stage_near: 0.0,
            hand_model_near: 0.0,
            glow: BloomFrame::default(),
            blur: 0.0,
            grading: NEUTRAL_GRADE,
        }
    }
}

/// The glow (bloom) settings of one frame, and the two console switches that override them.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BloomFrame {
    pub enable: bool,
    pub spread: f32,
    pub strength: f32,
    pub glow_on: bool,
    pub fullbright_on: bool,
}

impl BloomFrame {
    fn with_settings(enable: bool, spread: f32, strength: f32) -> Self {
        Self {
            enable,
            spread,
            strength,
            glow_on: lighting_t6::BLOOM_INITIAL_ON,
            fullbright_on: lighting_t6::UNSHADED_MODE_INITIAL,
        }
    }

    /// Takes the glow settings from the map's film vision; without one, glow is off.
    pub fn from_vision(vision: Option<asset_world::ScreenGradeLook>) -> Self {
        let look = vision.unwrap_or_default();
        Self::with_settings(look.bloom_enable, look.bloom_radius, look.bloom_intensity)
    }

    /// Whether the glow chain should run this frame.
    pub fn in_use(self) -> bool {
        let Self { enable, spread, strength, glow_on: r_glow, fullbright_on: r_fullbright } = self;
        lighting_t6::using_glow(enable, strength, spread, r_glow, r_fullbright)
    }
}

impl Default for BloomFrame {
    fn default() -> Self {
        Self::with_settings(false, 0.0, 0.0)
    }
}
