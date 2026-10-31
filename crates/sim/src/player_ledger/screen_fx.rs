//! What a script does to a player's picture: vision sets, depth of field, blur.

/// A vision set faded in over a time.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VisionChange {
    pub name: String,
    pub lasts_ms: i32,
    pub set_ms: i32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ScriptDepthOfField {
    pub near_from: f32,
    pub near_to: f32,
    pub far_from: f32,
    pub far_to: f32,
    pub near_smear: f32,
    pub far_smear: f32,
}

impl ScriptDepthOfField {
    /// Whether the scripts set a focus range; an all-zero near/far range means "leave the scene's own".
    pub fn overrides_scene(&self) -> bool {
        [self.near_from, self.near_to, self.far_from, self.far_to].iter().any(|&edge| edge != 0.0)
    }
}

/// A blur that eases from one strength to another.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ScriptBlur {
    pub from: f32,
    pub to: f32,
    pub set_ms: i32,
    pub lasts_ms: i32,
}

impl ScriptBlur {
    /// The strength at `clock_millis`.
    pub fn sample(self, clock_millis: i32) -> f32 {
        let progress = if self.lasts_ms <= 0 {
            1.0
        } else {
            (clock_millis.wrapping_sub(self.set_ms) as f32 / self.lasts_ms as f32).clamp(0.0, 1.0)
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
