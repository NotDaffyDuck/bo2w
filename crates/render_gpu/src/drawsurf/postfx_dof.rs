use bevy::prelude::*;

pub const GLOW_SETUP_MATERIAL: &str = "glow_consistent_setup_color2";
pub const GLOW_APPLY_MATERIAL: &str = "glow_apply_bloom";

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct DepthOfField {
    pub view_model_start: f32,
    pub view_model_end: f32,
    pub near_start: f32,
    pub near_end: f32,
    pub far_start: f32,
    pub far_end: f32,
    pub near_blur: f32,
    pub far_blur: f32,
}
impl DepthOfField {
    pub fn active(self) -> bool {
        self.view_model_end > self.view_model_start + 1.0
            || self.near_end > self.near_start + 1.0
            || (self.far_end > self.far_start + 1.0 && self.far_blur > 0.0)
    }
}

#[derive(Resource, Clone, Copy, Debug)]
pub struct DofFrame {
    pub dof: DepthOfField,
    pub bias: f32,
    pub scene_near: f32,
    pub view_model_near: f32,
    pub glow: GlowFrame,
    pub blur: f32,
    /// Hue in radians, gamma, exposure in stops, saturation.
    pub grading: [f32; 4],
}

impl Default for DofFrame {
    fn default() -> Self {
        Self {
            dof: DepthOfField::default(),
            bias: 0.0,
            scene_near: 0.0,
            view_model_near: 0.0,
            glow: GlowFrame::default(),
            blur: 0.0,
            grading: [0.0, 1.0, 0.0, 1.0],
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GlowFrame {
    pub enable: bool,
    pub radius: f32,
    pub intensity: f32,

    pub r_glow: bool,

    pub r_fullbright: bool,
}

impl Default for GlowFrame {
    fn default() -> Self {
        Self {
            enable: false,
            radius: 0.0,
            intensity: 0.0,
            r_glow: lighting_t6::R_GLOW_DEFAULT,
            r_fullbright: lighting_t6::R_FULLBRIGHT_DEFAULT,
        }
    }
}

impl GlowFrame {
    pub fn from_vision(vision: Option<asset_world::FilmVision>) -> Self {
        let vision = vision.unwrap_or_default();
        Self {
            enable: vision.glow_enable,
            radius: vision.glow_radius,
            intensity: vision.glow_bloom_intensity,
            r_glow: lighting_t6::R_GLOW_DEFAULT,
            r_fullbright: lighting_t6::R_FULLBRIGHT_DEFAULT,
        }
    }

    pub fn using(self) -> bool {
        lighting_t6::using_glow(
            self.enable,
            self.intensity,
            self.radius,
            self.r_glow,
            self.r_fullbright,
        )
    }
}
