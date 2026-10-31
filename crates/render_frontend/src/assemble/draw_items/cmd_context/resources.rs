//! Map and frame resources that the code constants are derived from.

use bevy::prelude::*;

use super::super::material_live::LiveImageId;
use crate::assemble::draw_items::cmd_context::{LightFalloffBind, LegacyFalloffParams};

/// Why a frame's code constants could not be produced.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CommandContextRejection {
    MissingEyeProjection,
    MissingFrameHaze,
    KeyLightNotDir,
    ViewportRenderTargetIsEmpty,
}

/// The part of the render target the scene is drawn into.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VisualViewport {
    pub width: i32,
    pub height: i32,
    pub x: i32,
    pub y: i32,
}

/// Exposure requested by the map's sun parse.
#[derive(Clone, Copy, Debug, Resource)]
pub struct MapT5DaylightParseExposure {
    pub exposure: f32,
    /// The vision's saturation the final image takes (Nuketown's 0.853516 for a map without a vision).
    pub saturation: f32,
}

/// Sun effects of the map, when it defines any.
#[derive(Clone, Copy, Debug, Default, Resource)]
pub struct MapSunEffects {
    pub def: Option<render_frame::SunEffectsDef>,
}

/// Outdoor lookup: its image, plus the sixteen words that place the world in
/// it.
#[derive(Clone, Copy, Debug, Resource)]
pub struct MapOpenair {
    pub find_in: [u32; 16],
    pub image: Option<LiveImageId>,
}

/// Binding data for a dynamic light: its attenuation image and the start of
/// its lightmap lookup.
#[derive(Clone, Copy, Debug)]
pub struct DynamicLightBind {
    pub attenuation: LightFalloffBind,
    pub lightmap_lookup_first: i32,
    pub falloff_map_width: Option<u16>,
}

/// Primary lights of the map, each with its attenuation and falloff data, and
/// the binding for dynamic lights.
#[derive(Clone, Debug, Default, Resource)]
pub struct MapKeyLights {
    pub lights: Vec<lighting_t6::LampRecord>,
    pub attenuation: Vec<LightFalloffBind>,
    pub classic_falloff: Vec<LegacyFalloffParams>,
    pub dynamic: Option<DynamicLightBind>,
}
