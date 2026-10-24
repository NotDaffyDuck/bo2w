use bevy::platform::collections::HashMap;
use bevy::prelude::*;
use lighting_t6::{
    SPOT_SHADOW_CFG_INDEX_ENTS, SPOT_SHADOW_ENT_MARK_LEN, SPOT_SHADOW_SCENE_DOBJ_VIS_STRIDE,
    SPOT_SHADOW_SCENE_MODEL_VIS_STRIDE, SPOT_SHADOW_SCORE_LUMA, SPOT_SHADOW_SM_LIGHT_CAP,
    SpotShadowCasterIn, SpotShadowEmittedSlot, SpotShadowSceneSlot, SpotShadowableLight,
    spot_shadow_choose,
    spot_shadow_dyn_brush_tess_admits, spot_shadow_dyn_brush_vis_bit_index,
    spot_shadow_emit_frontend, spot_shadow_fill_occupancy, spot_shadow_fill_scene_dobj_vis,
    spot_shadow_fill_scene_model_vis, spot_shadow_filter_smodel_ids,
    spot_shadow_link_dyn_brush_vis, spot_shadow_link_entity, spot_shadow_packed_lists_ready,
    spot_shadow_primary_vis_bit, spot_shadow_primary_vis_word_count, spot_shadow_tess_bsp_surfs,
    spot_shadow_tess_scene_dobj_indices, spot_shadow_tess_scene_model_indices,
    spot_shadow_walk_casters, spot_shadow_xmodel_is_caster,
};

use dpvs_t6::smodel_cull_dist_hides;

use super::SpotShadowCasterPlan;
use super::lights::{candidates, choose_dvars, lights_in_use};
use super::lookup::RetainedLookup;
use super::super::retained::{RetainedDrawItem, pack_draw};
use super::super::tess::smodel::{LodRampArgs, SmodelGpuPlan, smodel_camera_lod};
use super::super::tess::xmodel::XModelDrawPlan;
use crate::pack_spot_shadow_frontend;
use crate::prepare::scene::gfx_scene::SpotShadowSceneOccupancy;
use crate::prepare::scene::view_parms::PreparedSceneView;
use crate::prepare::scene::world::WorldScene;

const NO_CASTER: SpotShadowCasterIn = SpotShadowCasterIn {
    info: 0,
    vis_lit: false,
    bounds_ok: false,
    box_mid: [0.0; 3],
    box_half: [0.0; 3],
};

/// Everything one frame's slots are cut from.
struct Frame<'a> {
    world: &'a WorldScene,
    scene: Option<&'a render_frontend::GfxScene>,
    view: &'a PreparedSceneView,
    animated: Option<&'a XModelDrawPlan>,
    static_models: Option<&'a SmodelGpuPlan>,
    retained: RetainedLookup<'a>,
    lights: &'a [SpotShadowableLight],
    lod_ramp: LodRampArgs,
    /// Which lights each scene entity is lit by.
    entity_vis: Vec<u32>,
    /// The same for the dynamic brushes.
    brush_vis: Vec<u32>,
    /// The animated model draws of each scene entity, in draw order.
    by_entity: HashMap<u32, Vec<RetainedDrawItem>>,
}

impl Frame<'_> {
    fn sun_lights(&self) -> u32 {
        self.world.sun_primary_light_count
    }

    fn scene_entnums(&self) -> (Vec<u32>, Vec<bool>) {
        let Some(scene) = self.scene else {
            return Default::default();
        };
        scene
            .scene_dobjs
            .iter()
            .map(|dobj| {
                (
                    render_frontend::scene_info_entnum(dobj.info),
                    lighting_t6::spot_shadow_175d0_pose_ok(dobj.cull_gate),
                )
            })
            .unzip()
    }

    /// Marks each scene entity and dynamic brush with the lights that reach it.
    fn link_visibility(&mut self, words: usize) {
        let sun = self.sun_lights();
        self.entity_vis = vec![0u32; words];
        if let Some(scene) = self.scene {
            let dobjs = scene
                .scene_dobjs
                .iter()
                .map(|dobj| (dobj.info, dobj.origin, dobj.radius));
            let models = scene
                .scene_models
                .iter()
                .map(|model| (model.info, model.origin, model.radius));
            for (info, origin, radius) in dobjs.chain(models) {
                spot_shadow_link_entity(
                    &mut self.entity_vis,
                    0,
                    render_frontend::scene_info_entnum(info),
                    origin,
                    radius.unwrap_or(0.0),
                    &self.world.primary_light_cull,
                    sun,
                );
            }
        }
        self.brush_vis = vec![0u32; 512];
        for brush in &self.world.dyn_ent_brushes {
            spot_shadow_link_dyn_brush_vis(
                &mut self.brush_vis,
                u32::from(brush.index),
                brush.origin,
                0.0,
                &self.world.primary_light_cull,
                sun,
            );
        }
    }

    /// Whether a static model placement casts into a shadow for this view, and
    /// with which flags.
    fn static_model_flags(&self, placement: u16) -> Option<u8> {
        let instance = self
            .world
            .static_model_instances
            .get(usize::from(placement))?
            .as_ref()?;
        if !lighting_t6::spot_shadow_smodel_casts(instance.flags) {
            return None;
        }
        let eye = self.view.eye.to_array();
        let offset: [f32; 3] = std::array::from_fn(|axis| instance.origin[axis] - eye[axis]);
        let dist_sq: f32 = offset.iter().map(|d| d * d).sum();
        if smodel_cull_dist_hides(instance.cull_dist, dist_sq, self.lod_ramp.scale_last) {
            return None;
        }
        let lod = self.world.static_model_meshes.get(instance.mesh).and_then(|mesh| mesh.lod);
        smodel_camera_lod(lod, instance.origin, instance.scale, Some(self.view.eye), self.lod_ramp)?;
        Some(instance.flags)
    }

    /// The draws of the dynamic brush models that light reaches.
    fn brush_draws(&self, slot: &SpotShadowEmittedSlot) -> Vec<RetainedDrawItem> {
        let mut found = Vec::new();
        for brush in &self.world.dyn_ent_brushes {
            if brush.brush_model == 0 {
                continue;
            }
            let Some(bit) = spot_shadow_dyn_brush_vis_bit_index(
                u32::from(brush.index),
                u32::from(slot.light_index),
                self.sun_lights(),
                self.lights.len() as u32,
            ) else {
                continue;
            };
            let reached = spot_shadow_primary_vis_bit(&self.brush_vis, bit);
            if !spot_shadow_dyn_brush_tess_admits(slot.walks_casters, reached) {
                continue;
            }
            let Some(model) = self
                .world
                .cull
                .as_ref()
                .and_then(|cull| cull.brush_models.get(usize::from(brush.brush_model)))
            else {
                continue;
            };
            let first = u32::from(model.start_surf);
            let surfaces: Vec<u16> = (first..first.saturating_add(u32::from(model.surface_count)))
                .filter_map(|surf| u16::try_from(surf).ok())
                .collect();
            found.extend(self.retained.world_surfaces(&surfaces));
        }
        found
    }

    /// Animated model draws of the scene entities this slot sees.
    fn scene_entity_draws(
        &self,
        scene: &render_frontend::GfxScene,
        slot: &SpotShadowEmittedSlot,
        sink: &mut Vec<RetainedDrawItem>,
        entnums: &[u32],
        pose_ok: &[bool],
    ) {
        let camera = scene
            .scene_ent_walked
            .then_some(scene.scene_ent_visible.as_slice());
        let mut picked = [0u32; 512];
        let take = |entnum: u32, sink: &mut Vec<RetainedDrawItem>| {
            if entnum < SPOT_SHADOW_ENT_MARK_LEN as u32
                && let Some(draws) = self.by_entity.get(&entnum)
            {
                sink.extend_from_slice(draws);
            }
        };

        let mut dobj_vis = [0u8; SPOT_SHADOW_SCENE_DOBJ_VIS_STRIDE * SPOT_SHADOW_SM_LIGHT_CAP];
        spot_shadow_fill_scene_dobj_vis(&mut dobj_vis, slot.slot_index, entnums, camera, Some(pose_ok));
        let seen = spot_shadow_tess_scene_dobj_indices(
            &dobj_vis,
            slot.slot_index,
            scene.scene_dobjs.len() as u32,
            &mut picked,
        );
        for &at in &picked[..seen] {
            if let Some(dobj) = scene.scene_dobjs.get(at as usize) {
                take(render_frontend::scene_info_entnum(dobj.info), sink);
            }
        }

        let model_entnums: Vec<u32> = scene
            .scene_models
            .iter()
            .map(|model| render_frontend::scene_info_entnum(model.info))
            .collect();
        let mut model_vis = [0u8; SPOT_SHADOW_SCENE_MODEL_VIS_STRIDE * SPOT_SHADOW_SM_LIGHT_CAP];
        spot_shadow_fill_scene_model_vis(&mut model_vis, slot.slot_index, &model_entnums, camera, None);
        let seen = spot_shadow_tess_scene_model_indices(
            &model_vis,
            slot.slot_index,
            scene.scene_models.len() as u32,
            &mut picked,
        );
        for &at in &picked[..seen] {
            if let Some(model) = scene.scene_models.get(at as usize) {
                take(render_frontend::scene_info_entnum(model.info), sink);
            }
        }
    }
}

/// Fills `plan` with the slots of the lights chosen to cast a shadow this
/// frame. Stops quietly, leaving it empty, whenever nothing can be shadowed.
pub fn fill_spot_shadow_caster_plan(
    plan: &mut SpotShadowCasterPlan,
    occupancy: &SpotShadowSceneOccupancy,
    scene: Option<&render_frontend::GfxScene>,
    world: &WorldScene,
    view: Option<&PreparedSceneView>,
    xmodel: Option<&XModelDrawPlan>,
    retained: &[RetainedDrawItem],
    world_run_surfs: &[u16],
    world_ranges: &[(u32, u32)],
    world_vertex_count: u32,
    smodel_ranges: &[(u32, u32)],
    smodel: Option<&SmodelGpuPlan>,
    lod_ramp: LodRampArgs,
    scene_time: u32,
    sm_enable: Option<bool>,
    sm_sun_enable: Option<bool>,
) {
    plan.clear_frame();
    let lights = candidates(world);
    if lights.is_empty() || !lighting_t6::generate_runs_choose(sm_enable, None) {
        return;
    }
    let Some(view) = view.filter(|view| view.ready) else {
        return;
    };
    let animated_draws = xmodel.map_or(&[][..], |plan| plan.draws.as_slice());
    let sun = world.sun_primary_light_count;
    let mut used = lights_in_use(world, animated_draws);
    let sun_enabled = sm_sun_enable.unwrap_or(lighting_t6::SM_SUN_ENABLE_DEFAULT);
    if lighting_t6::generate_clears_used_bits_through_sun(sun_enabled, sun) {
        lighting_t6::clear_used_bits_1_through_sun(&mut used, sun);
    }
    let light_count = lights.len() as u32;
    let Some(chosen) = spot_shadow_choose(
        &mut plan.history,
        used,
        &lights,
        view.eye.to_array(),
        view.forward.to_array(),
        scene_time,
        sun,
        light_count,
        None,
        choose_dvars(),
        SPOT_SHADOW_SCORE_LUMA,
    ) else {
        return;
    };
    let emitted = spot_shadow_emit_frontend(&chosen, &lights, light_count, 0.0);
    // Choosing updates the history even when no light ends up with a slot;
    // the visibility work below only has users in emitted slots.
    let slots: Vec<SpotShadowEmittedSlot> = emitted
        .slots
        .iter()
        .take(SPOT_SHADOW_SM_LIGHT_CAP)
        .flatten()
        .copied()
        .collect();
    if slots.is_empty() {
        return;
    }
    let words = spot_shadow_primary_vis_word_count(1, SPOT_SHADOW_CFG_INDEX_ENTS, sun, light_count);
    if words == 0 {
        return;
    }

    let retained = RetainedLookup::index(retained);
    let mut by_entity = HashMap::<u32, Vec<RetainedDrawItem>>::new();
    if retained.has_animated() {
        for draw in animated_draws {
            if let (Some(entnum), Some(item)) = (draw.scene_entnum, retained.animated(draw)) {
                by_entity.entry(entnum).or_default().push(item);
            }
        }
    }
    let mut frame = Frame {
        world,
        scene,
        view,
        animated: xmodel,
        static_models: smodel,
        retained,
        lights: &lights,
        lod_ramp,
        entity_vis: Vec::new(),
        brush_vis: Vec::new(),
        by_entity,
    };
    frame.link_visibility(words);
    let (entnums, pose_ok) = frame.scene_entnums();
    let mut dobj_casters = vec![NO_CASTER; occupancy.dobjs.len()];
    let mut model_casters = vec![NO_CASTER; occupancy.models.len()];

    for slot in &slots {
        let Some(light) = lights.get(usize::from(slot.light_index)) else {
            continue;
        };
        let occupied = |slots: &[SpotShadowSceneSlot], casters: &mut [SpotShadowCasterIn]| {
            spot_shadow_fill_occupancy(
                slots,
                Some(frame.entity_vis.as_slice()),
                0,
                u32::from(slot.light_index),
                sun,
                light_count,
                casters,
            )
            .is_ok()
        };
        if !occupied(&occupancy.dobjs, &mut dobj_casters)
            || !occupied(&occupancy.models, &mut model_casters)
        {
            continue;
        }
        let mut marks = vec![0u8; SPOT_SHADOW_ENT_MARK_LEN];
        spot_shadow_walk_casters(
            slot.walks_casters,
            light,
            &dobj_casters,
            &model_casters,
            u32::MAX,
            &mut marks,
        );

        let geometry = world.shadow_geometry.get(usize::from(slot.light_index));
        let mut surface_ids = [0u16; 512];
        let surface_n = spot_shadow_tess_bsp_surfs(
            slot.walks_casters,
            geometry.map_or(&[][..], |geometry| geometry.surfaces.as_slice()),
            &mut surface_ids,
        );
        let mut draws = frame.retained.world_surfaces(&surface_ids[..surface_n]);

        let mut placement_ids = [0u16; 512];
        let placement_n = spot_shadow_filter_smodel_ids(
            slot.walks_casters,
            geometry.map_or(&[][..], |geometry| geometry.smodels.as_slice()),
            |placement| frame.static_model_flags(placement),
            &mut placement_ids,
        );
        draws.extend(frame.retained.static_models_at_lod(
            &placement_ids[..placement_n],
            frame.static_models,
            view.eye,
            lod_ramp,
        ));
        draws.extend(frame.brush_draws(slot));
        if let (Some(scene), Some(_)) = (scene, frame.animated) {
            frame.scene_entity_draws(scene, slot, &mut draws, &entnums, &pose_ok);
        }
        if let Some(animated) = frame.animated {
            for draw in &animated.draws {
                if spot_shadow_xmodel_is_caster(&marks, draw.scene_entnum)
                    && let Some(item) = frame.retained.animated(draw)
                {
                    draws.push(item);
                }
            }
        }

        let animated_ranges = frame.animated.map_or(&[][..], |plan| plan.range_rows());
        let flat: Vec<_> = draws.iter().map(pack_draw).collect();
        let packed = pack_spot_shadow_frontend(
            &flat,
            world_run_surfs,
            world_ranges,
            world_vertex_count,
            smodel_ranges,
            animated_ranges,
        );
        if spot_shadow_packed_lists_ready(packed.work_entry_n()) {
            plan.record_slot(draws, *slot, packed);
        }
    }
}
