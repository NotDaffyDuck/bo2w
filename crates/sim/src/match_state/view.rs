//! What a script does to a player's picture: vision sets, depth of field, blur.

/// A vision set faded in over a time.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VisionChange {
    pub name: String,
    pub duration_ms: i32,
    pub set_ms: i32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ScriptDepthOfField {
    pub near_start: f32,
    pub near_end: f32,
    pub far_start: f32,
    pub far_end: f32,
    pub near_blur: f32,
    pub far_blur: f32,
}

impl ScriptDepthOfField {
    /// Whether the scripts set a focus range; an all-zero near/far range means "leave the scene's own".
    pub fn overrides_scene(&self) -> bool {
        [self.near_start, self.near_end, self.far_start, self.far_end].iter().any(|&edge| edge != 0.0)
    }
}

/// A blur that eases from one strength to another.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ScriptBlur {
    pub from: f32,
    pub to: f32,
    pub set_ms: i32,
    pub duration_ms: i32,
}

impl ScriptBlur {
    /// The strength at `now_ms`.
    pub fn sample(self, now_ms: i32) -> f32 {
        let progress = if self.duration_ms <= 0 {
            1.0
        } else {
            (now_ms.wrapping_sub(self.set_ms) as f32 / self.duration_ms as f32).clamp(0.0, 1.0)
        };
        self.from + (self.to - self.from) * progress
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ViewEffects {
    pub naked_vision: Option<VisionChange>,
    pub thermal_vision: Option<VisionChange>,
    pub missile_vision: Option<VisionChange>,
    pub night_vision: Option<crate::VisionChange>,
    pub pain_vision: Option<crate::VisionChange>,
    pub depth_of_field: ScriptDepthOfField,
    pub blur: Option<ScriptBlur>,
}

/// Picture dvars a script sets are sent to the client straight, without the console's gates.
pub fn is_postfx_dvar(name: &str) -> bool {
    const EXACT: [&str; 8] =
        ["r_blur", "r_hue", "r_brightness", "r_contrast", "r_saturation", "r_gamma", "r_exposure", "nightvision"];
    const FAMILIES: [&str; 3] = ["r_film", "r_glow", "r_dof"];
    let name = name.to_ascii_lowercase();
    FAMILIES.iter().any(|family| name.starts_with(family)) || EXACT.contains(&name.as_str())
}
