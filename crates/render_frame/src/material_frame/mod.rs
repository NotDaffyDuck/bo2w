//! Per-frame inputs for evaluating a material's code constants and samplers.

mod receivers;

use glam::{Mat4, Vec3};
use lighting_t6::GfxLightPack;
use render_material::RuntimeCodeSources;

use crate::{LightAttenuationBind, T5LightFalloffPack};

pub use self::receivers::{OutdoorLookup, SpotShadowReceiver};

/// Everything a frame provides to material execution. Matrices are `None` while
/// the frame has not produced them, and constants that depend on a missing matrix
/// are left unset rather than guessed.
#[derive(Clone, Debug, Default)]
pub struct MaterialExecFrame {
    /// Seconds since the level started, wrapped as the shaders expect.
    pub float_time: f32,
    /// The eye position in world space.
    pub view_origin: Vec3,
    /// Named values (time, fog, colours) that code constants read from.
    pub code_sources: RuntimeCodeSources,
    /// World to clip space for the main camera.
    pub clip_from_world: Option<Mat4>,
    /// World to view space for the main camera.
    pub view_from_world: Option<Mat4>,
    /// View to clip space for the main camera.
    pub clip_from_view: Option<Mat4>,
    /// World to clip space for the viewmodel, which has its own projection.
    pub viewmodel_clip_from_world: Option<Mat4>,
    /// Near plane distance used by the viewmodel projection.
    pub viewmodel_near: Option<f32>,
    /// One over the height of the render target in pixels.
    pub inv_image_height: Option<f32>,
    /// Sky visibility, when the level provides it.
    pub outdoor: Option<OutdoorLookup>,
    /// The lights shading the scene, in the packed form shaders read.
    pub primary_lights: Vec<GfxLightPack>,
    /// How each light's intensity falls off with distance.
    pub attenuation: Vec<LightAttenuationBind>,
    /// Falloff parameters for lights authored for the older shading model.
    pub t5_falloff: Vec<T5LightFalloffPack>,
    /// One slot per spot light; `None` where the light casts no shadow.
    pub spot_receivers: Vec<Option<SpotShadowReceiver>>,
}
