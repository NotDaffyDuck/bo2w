//! What one frame's product lists are built from, collected up front.
//!
//! `open_frame_products` runs first each frame. It resets the per-frame spot
//! shadow state, snapshots the scene inputs the later systems share (world and
//! catalog generation, fog method, the map's lights) and tops the light list
//! up with the effects system's dynamic lights.

use bevy::prelude::*;

use super::super::command_context::{DynamicLightBind, LightAttenuationBind, T5LightFalloffPack};
use super::super::material_runtime::MaterialGenerationId;
use super::super::{DrawMethodDfog, MapPrimaryLights, SpotShadowCasterPlan, SpotShadowMapLights};
use super::{MaterialFrameInputs, MaterialGeneration};
use crate::prepare::scene::model_lighting_atlas::WorldModelLightingAtlas;
use crate::prepare::scene::view_parms::PreparedSceneView;

#[derive(Resource, Debug, Default)]
pub struct FrameAssemblyInputs {
    pub frame_id: u64,
    pub world_generation: frame::WorldGeneration,
    pub catalog_generation: MaterialGenerationId,
    pub inv_image_height: Option<f32>,
    pub dfog: bool,
    /// The map's lights first, then this frame's dynamic lights.
    pub primary_lights: Vec<lighting_t6::GfxLightPack>,
    /// How many of `primary_lights` come from the map.
    pub map_light_n: usize,
    pub attenuation: Vec<LightAttenuationBind>,
    pub t5_falloff: Vec<T5LightFalloffPack>,
}

impl FrameAssemblyInputs {
    /// Starts a frame: new id, map lights copied in, dynamic lights not yet.
    fn begin(
        &mut self,
        world_generation: frame::WorldGeneration,
        catalog_generation: MaterialGenerationId,
        inv_image_height: Option<f32>,
        dfog: bool,
        map: Option<&MapPrimaryLights>,
    ) {
        self.frame_id = self.frame_id.wrapping_add(1);
        self.world_generation = world_generation;
        self.catalog_generation = catalog_generation;
        self.inv_image_height = inv_image_height;
        self.dfog = dfog;
        self.primary_lights.clear();
        self.attenuation.clear();
        self.t5_falloff.clear();
        if let Some(map) = map {
            self.primary_lights.extend_from_slice(&map.lights);
            self.attenuation.extend_from_slice(&map.attenuation);
            self.t5_falloff.extend_from_slice(&map.t5_falloff);
        }
        self.map_light_n = self.primary_lights.len();
    }

    /// Adds the lights the visible effects ask for, each lit with the map's
    /// shared dynamic-light atlas settings.
    fn add_dynamic_lights(
        &mut self,
        fx: &render_fx::HostFxDlights,
        dynamic: &DynamicLightBind,
        view: Option<&PreparedSceneView>,
    ) {
        let eye = view.map_or([0.0; 3], |view| view.eye.to_array());
        let frustum = view
            .filter(|view| view.ready && !view.frustum_planes.is_empty())
            .map(|view| view.frustum_planes.as_slice());
        let wanted: Vec<_> = fx
            .scene
            .iter()
            .map(|&light| lighting_t6::SceneDlight { light, used: false })
            .collect();
        let mut packed = [lighting_t6::omni_light_pack([0.0; 3], 1.0, [0.0; 3]);
            lighting_t6::R_DLIGHT_BACKEND_MAX];
        let count = lighting_t6::append_scene_dlights_to_backend(
            &wanted,
            eye,
            lighting_t6::R_DLIGHT_LIMIT_DEFAULT,
            true,
            frustum,
            &mut packed,
        );
        for mut light in packed.into_iter().take(count) {
            light.falloff_image_width = dynamic.falloff_image_width;
            light.lmap_lookup_start = dynamic.lmap_lookup_start;
            self.primary_lights.push(light);
            self.attenuation.push(dynamic.attenuation);
            self.t5_falloff.push(T5LightFalloffPack::default());
        }
    }
}

pub(crate) fn open_frame_products(
    generation: Res<MaterialGeneration>,
    world_generation: Option<Res<frame::WorldGeneration>>,
    lighting: Option<Res<WorldModelLightingAtlas>>,
    dfog: Option<Res<DrawMethodDfog>>,
    primary_lights: Option<Res<MapPrimaryLights>>,
    fx_dlights: Option<Res<render_fx::HostFxDlights>>,
    prepared: Option<Res<PreparedSceneView>>,
    mut inputs: ResMut<FrameAssemblyInputs>,
    mut spot_casters: ResMut<SpotShadowCasterPlan>,
    mut spot_lights: ResMut<SpotShadowMapLights>,
    mut mat_frame: ResMut<MaterialFrameInputs>,
) {
    spot_casters.clear_frame();
    spot_lights.0.clear();
    mat_frame.spot_receivers.clear();

    let inv_image_height = lighting.as_ref().and_then(|atlas| {
        lighting_t6::model_lighting_inv_image_height(atlas.dims.image_height)
    });
    inputs.begin(
        world_generation.map(|g| *g).unwrap_or_default(),
        generation.catalog.generation_id,
        inv_image_height,
        dfog.is_some_and(|flag| flag.0),
        primary_lights.as_deref(),
    );

    let dynamic = primary_lights.as_ref().and_then(|map| map.dynamic);
    if let (Some(fx), Some(dynamic)) = (fx_dlights.as_ref(), dynamic.as_ref()) {
        inputs.add_dynamic_lights(fx, dynamic, prepared.as_deref());
    }
}
