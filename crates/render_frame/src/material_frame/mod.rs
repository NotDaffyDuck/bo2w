//! Per-frame inputs for evaluating a material's code constants and samplers.

mod receivers;

use glam::{Mat4, Vec3};
use lighting_t6::LampRecord;
use render_material::LiveEngineSources;

use crate::{LightFalloffBind, LegacyFalloffParams};

pub use self::receivers::{OpenAirProbe, ConeShadowSink};

/// Everything a frame provides to material execution. Matrices are `None` while
/// the frame has not produced them, and constants that depend on a missing matrix
/// are left unset rather than guessed.
#[derive(Clone, Debug, Default)]
pub struct MaterialRunFrame {
    /// Seconds since the level started, wrapped as the shaders expect.
    pub time_float: f32,
    /// The eye position in world space.
    pub eye_origin: Vec3,
    /// Named values (time, fog, colours) that code constants read from.
    pub engine_sources: LiveEngineSources,
    /// World to clip space for the main camera.
    pub world_to_clip: Option<Mat4>,
    /// World to view space for the main camera.
    pub world_to_view: Option<Mat4>,
    /// View to clip space for the main camera.
    pub view_to_clip: Option<Mat4>,
    /// World to clip space for the viewmodel, which has its own projection.
    pub hand_model_clip_from_level: Option<Mat4>,
    /// Near plane distance used by the viewmodel projection.
    pub hand_near_plane: Option<f32>,
    /// One over the height of the render target in pixels.
    pub recip_image_height: Option<f32>,
    /// Sky visibility, when the level provides it.
    pub outdoor: Option<OpenAirProbe>,
    /// The lights shading the scene, in the packed form shaders read.
    pub key_lights: Vec<LampRecord>,
    /// How each light's intensity falls off with distance.
    pub attenuation: Vec<LightFalloffBind>,
    /// Falloff parameters for lights authored for the older shading model.
    pub classic_falloff: Vec<LegacyFalloffParams>,
    /// One slot per spot light; `None` where the light casts no shadow.
    pub cone_receivers: Vec<Option<ConeShadowSink>>,
}
