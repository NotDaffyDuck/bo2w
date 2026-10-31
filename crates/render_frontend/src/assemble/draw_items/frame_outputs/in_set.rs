//! What one frame's product lists are built from, collected up front.
//!
//! `open_frame_outputs` runs first each frame. It resets the per-frame spot
//! shadow state, snapshots the scene inputs the later systems share (world and
//! catalog generation, fog method, the map's lights) and tops the light list
//! up with the effects system's dynamic lights.

use bevy::prelude::*;

use super::super::cmd_context::{DynamicLightBind, LightFalloffBind, LegacyFalloffParams};
use super::super::material_live::MaterialEpochId;
use super::super::{DrawMethodDepthHaze, MapKeyLights, ConeShadowCasterPlan, ConeShadowMapLights};
use crate::assemble::draw_items::frame_outputs::{MaterialFrameFeed, MaterialEpoch};
use crate::prepare::scene::model_illum_atlas::LevelModelIllumAtlas;
use crate::prepare::scene::eye_parms::StagedStageEye;

#[derive(Resource, Debug, Default)]
pub struct FrameBuildFeed {
    pub frame_seq_no: u64,
    pub level_generation: frame::LevelGeneration,
    pub ledger_generation: MaterialEpochId,
    pub recip_image_height: Option<f32>,
    pub depth_haze: bool,
    /// The map's lights first, then this frame's dynamic lights.
    pub key_lights: Vec<lighting_t6::LampRecord>,
    /// How many of `key_lights` come from the map.
    pub map_light_tally: usize,
    pub attenuation: Vec<LightFalloffBind>,
    pub classic_falloff: Vec<LegacyFalloffParams>,
}

impl FrameBuildFeed {
    /// Starts a frame: new id, map lights copied in, dynamic lights not yet.
    fn begin(
        &mut self,
        level_generation: frame::LevelGeneration,
        ledger_generation: MaterialEpochId,
        recip_image_height: Option<f32>,
        depth_haze: bool,
        map: Option<&MapKeyLights>,
    ) {
        self.frame_seq_no = self.frame_seq_no.wrapping_add(1);
        self.level_generation = level_generation;
        self.ledger_generation = ledger_generation;
        self.recip_image_height = recip_image_height;
        self.depth_haze = depth_haze;
        self.key_lights.clear();
        self.attenuation.clear();
        self.classic_falloff.clear();
        if let Some(map) = map {
            self.key_lights.extend_from_slice(&map.lights);
            self.attenuation.extend_from_slice(&map.attenuation);
            self.classic_falloff.extend_from_slice(&map.classic_falloff);
        }
        self.map_light_tally = self.key_lights.len();
    }

    /// Adds the lights the visible effects ask for, each lit with the map's
    /// shared dynamic-light atlas settings.
    fn add_dynamic_lights(
        &mut self,
        fx: &render_fx::EffectsDynamicLightSet,
        dynamic: &DynamicLightBind,
        view: Option<&StagedStageEye>,
    ) {
        let eye = view.map_or([0.0; 3], |view| view.eye.to_array());
        let frustum = view
            .filter(|view| view.primed && !view.view_volume_planes.is_empty())
            .map(|view| view.view_volume_planes.as_slice());
        let in_use = false;
        let wanted: Vec<_> = fx
            .scene
            .iter()
            .map(|&lamp| lighting_t6::BriefLamp { light: lamp, claimed: in_use })
            .collect();
        let zero3 = [0.0; 3];
        let mut packed = [lighting_t6::pack_point_lamp(zero3, 1.0, zero3);
            lighting_t6::TRANSIENT_LAMP_GPU_CEILING];
        let count = lighting_t6::forward_scene_dynamic_lights_to_renderer(
            &wanted,
            eye,
            lighting_t6::TRANSIENT_LAMP_INITIAL_CAP,
            true,
            frustum,
            &mut packed,
        );
        for mut light in packed.into_iter().take(count) {
            light.falloff_map_width = dynamic.falloff_map_width;
            light.lightmap_first = dynamic.lightmap_lookup_first;
            self.key_lights.push(light);
            self.attenuation.push(dynamic.attenuation);
            self.classic_falloff.push(LegacyFalloffParams::default());
        }
    }
}

pub(crate) fn open_frame_outputs(
    generation: Res<MaterialEpoch>,
    level_generation: Option<Res<frame::LevelGeneration>>,
    lighting: Option<Res<LevelModelIllumAtlas>>,
    depth_haze: Option<Res<DrawMethodDepthHaze>>,
    key_lights: Option<Res<MapKeyLights>>,
    effect_dlights: Option<Res<render_fx::EffectsDynamicLightSet>>,
    prepared: Option<Res<StagedStageEye>>,
    mut inputs: ResMut<FrameBuildFeed>,
    mut spot_casters: ResMut<ConeShadowCasterPlan>,
    mut spot_lamps: ResMut<ConeShadowMapLights>,
    mut mat_frame: ResMut<MaterialFrameFeed>,
) {
    spot_casters.wipe_frame();
    spot_lamps.0.clear();
    mat_frame.cone_receivers.clear();

    let recip_image_height = lighting.as_ref().and_then(|atlas| {
        lighting_t6::light_atlas_reciprocal_height(atlas.dims.rows_tall)
    });
    inputs.begin(
        level_generation.map(|g| *g).unwrap_or_default(),
        generation.catalog.revision_stamp,
        recip_image_height,
        depth_haze.is_some_and(|flag| flag.0),
        key_lights.as_deref(),
    );

    let dynamic = key_lights.as_ref().and_then(|map| map.dynamic);
    if let (Some(fx), Some(dynamic)) = (effect_dlights.as_ref(), dynamic.as_ref()) {
        inputs.add_dynamic_lights(fx, dynamic, prepared.as_deref());
    }
}
