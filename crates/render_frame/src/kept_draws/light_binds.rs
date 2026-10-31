//! Per-light shader inputs that do not fit the generic light pack.

/// The attenuation image a light samples and the sampler slot it uses.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LightFalloffBind {
    pub image: Option<u32>,
    pub sampler: u8,
}

/// Falloff constants for lights in the older shading model. Each field is
/// `None` until the light supplies it.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct LegacyFalloffParams {
    pub diffuse: Option<[f32; 4]>,
    pub specular: Option<[f32; 4]>,
    pub attenuation: Option<[f32; 4]>,
    pub falloff: Option<[f32; 4]>,
    pub pair_ab_vs_b: Option<[f32; 4]>,
    pub yaw_degrees_opt: Option<f32>,
    pub projector_mask_first: Option<[f32; 4]>,
    pub projector_mask_second: Option<[f32; 4]>,
    pub projector_mask_third: Option<[f32; 4]>,
}
