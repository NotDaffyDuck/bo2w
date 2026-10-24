//! Per-light shader inputs that do not fit the generic light pack.

/// The attenuation image a light samples and the sampler slot it uses.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LightAttenuationBind {
    pub image: Option<u32>,
    pub sampler: u8,
}

/// Falloff constants for lights in the older shading model. Each field is
/// `None` until the light supplies it.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct T5LightFalloffPack {
    pub diffuse: Option<[f32; 4]>,
    pub specular: Option<[f32; 4]>,
    pub attenuation: Option<[f32; 4]>,
    pub falloff: Option<[f32; 4]>,
    pub a_ab_b: Option<[f32; 4]>,
    pub angle_z: Option<f32>,
    pub cookie0: Option<[f32; 4]>,
    pub cookie1: Option<[f32; 4]>,
    pub cookie2: Option<[f32; 4]>,
}
