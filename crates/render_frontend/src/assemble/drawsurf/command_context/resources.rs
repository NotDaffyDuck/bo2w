//! The per-map and per-frame values the code constants are built from.

use bevy::prelude::*;

use super::super::material_runtime::RuntimeImageId;
use super::{LightAttenuationBind, T5LightFalloffPack};

/// The exposure the map's sun parse asked for.
#[derive(Clone, Copy, Debug, Resource)]
pub struct MapT5SunParseExposure {
    pub exposure: f32,
}

/// The outdoor-lookup image and the sixteen words that place the world in it.
#[derive(Clone, Copy, Debug, Resource)]
pub struct MapOutdoor {
    pub image: Option<RuntimeImageId>,
    pub lookup: [u32; 16],
}

/// The sun-effects definition the map carries, if any.
#[derive(Clone, Copy, Debug, Default, Resource)]
pub struct MapSunEffects {
    pub def: Option<render_frame::SunEffectsDef>,
}

/// The map's primary lights with the attenuation and falloff data each needs,
/// plus the light bound for dynamic lighting.
#[derive(Clone, Debug, Default, Resource)]
pub struct MapPrimaryLights {
    pub lights: Vec<lighting_t6::GfxLightPack>,
    pub attenuation: Vec<LightAttenuationBind>,
    pub t5_falloff: Vec<T5LightFalloffPack>,
    pub dynamic: Option<DynamicLightBind>,
}

/// What a dynamic light needs bound: its attenuation image and where its
/// lightmap lookup starts.
#[derive(Clone, Copy, Debug)]
pub struct DynamicLightBind {
    pub attenuation: LightAttenuationBind,
    pub falloff_image_width: Option<u16>,
    pub lmap_lookup_start: i32,
}

/// The rectangle of the render target the scene is drawn into.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GfxViewport {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

/// Why a frame's code constants could not be produced.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CommandContextRefusal {
    MissingViewProjection,
    MissingFrameFog,
    PrimaryLightNotDir,
    ViewportHasZeroRenderTarget,
}
