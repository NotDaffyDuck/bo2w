use bevy::platform::collections::HashMap;
use bevy::prelude::*;
use lighting_t6::SPOT_SHADOW_CONFIG_ENTITY_LIMIT;
use lighting_t6::SPOT_SHADOW_ENTITY_MARK_CAPACITY;
use lighting_t6::SPOT_SHADOW_ANIMATED_VISIBILITY_SPAN;
use lighting_t6::SPOT_SHADOW_STATIC_VISIBILITY_SPAN;
use lighting_t6::LAMP_LUMA_MIX;
use lighting_t6::CONE_TILE_COUNT;
use lighting_t6::ConeLightOccluderFeed;
use lighting_t6::SpotShadowSubmittedSlot;
use render_scene::stage_info_actor_num;
use lighting_t6::ConeViewSlot;
use lighting_t6::ConeCasterOffer;
use lighting_t6::pick_cone_shadows;
use lighting_t6::mover_brush_admitted;
use lighting_t6::debris_lit_bit;
use lighting_t6::submit_spot_shadow_frontend;
use lighting_t6::populate_spot_shadow_occupancy;
use lighting_t6::populate_spot_animated_visibility;
use lighting_t6::populate_spot_static_visibility;
use lighting_t6::cone_shadow_prune_static_ids;
use lighting_t6::cone_shadow_link_debris_solid_vis;
use lighting_t6::cone_shadow_link_entity;
use lighting_t6::cone_lists_ready;
use lighting_t6::lamp_lit_bit;
use lighting_t6::spot_shadow_main_light_visibility_words;
use lighting_t6::cone_world_facets;
use lighting_t6::split_moving_ids;
use lighting_t6::cone_shadow_stitch_stage_model_indices;
use lighting_t6::cone_shadow_walk_casters;
use lighting_t6::model_casts_cone;

use dpvs_t6::prop_beyond_cull_range;

use super::ConeShadowCasterPlan;
use super::lamps::candidates;
use super::lamps::choose_dvars;
use super::lamps::lights_in_use;
use super::find_in::RetainedLookup;
use super::super::kept_lanes::{HeldDrawItem, pack_draw};
use super::super::mesh_build::prop_mesh::{DetailRampArgs, PropGpuPlan, prop_camera_detail};
use super::super::mesh_build::figure_mesh::FigureDrawPlan;
use crate::pack_cone_shadow_frontend;
use crate::prepare::scene::visual_stage::ConeShadowStageOccupancy;
use crate::prepare::scene::eye_parms::StagedStageEye;
use crate::prepare::scene::world::LevelStage;

const NO_EXTENT: [f32; 3] = [0.0; 3];

const NO_CASTER: ConeLightOccluderFeed = ConeLightOccluderFeed {
    ent_bits: 0,
    sight_lit: false,
    box_valid: false,
    mid_point: NO_EXTENT, half_ext: NO_EXTENT,
};


/// Everything one frame's slots are cut from.
struct Frame<'a> {
    world: &'a LevelStage,
    scene: Option<&'a render_frontend::VisualStage>,
    view: &'a StagedStageEye,
    animated: Option<&'a FigureDrawPlan>,
    prop_set: Option<&'a PropGpuPlan>,
    retained: RetainedLookup<'a>,
    lights: &'a [ConeCasterOffer],
    detail_ramp: DetailRampArgs,
    /// Which lights each scene entity is lit by.
    entity_vis: Vec<u32>,
    /// The same for the dynamic brushes.
    brush_vis: Vec<u32>,
    /// The animated model draws of each scene entity, in draw order.
    by_entity: HashMap<u32, Vec<HeldDrawItem>>,
}

impl Frame<'_> {
    fn sun_lights(&self) -> u32 {
        self.world.daylight_key_light_tally
    }

    /// Each scene puppet's entity number.
    fn puppet_entity_numbers(&self) -> Vec<u32> {
        let puppets = self.scene.map_or(&[][..], |scene| &scene.stage_puppets[..]);
        puppets.iter().map(|dobj| stage_info_actor_num(dobj.about)).collect()
    }

    /// For each scene puppet, whether its pose lets a spot light shadow it.
    fn puppet_pose_flags(&self) -> Vec<bool> {
        let puppets = self.scene.map_or(&[][..], |scene| &scene.stage_puppets[..]);
        puppets.iter().map(|dobj| lighting_t6::cone_pose_ready(dobj.visibility_gate_bits)).collect()
    }

    /// Marks each scene entity and dynamic brush with the lights that reach it.
    fn link_visibility(&mut self, words: usize) {
        let sun = self.sun_lights();
        self.entity_vis = vec![0u32; words];
        if let Some(scene) = self.scene {
            let dobjs = scene
                .stage_puppets
                .iter()
                .map(|held| (held.about, held.origin, held.reach));
            let models = scene
                .stage_models
                .iter()
                .map(|prop| (prop.about, prop.origin, prop.reach));
            for (info, origin, radius) in dobjs.chain(models) {
                cone_shadow_link_entity(
                    &mut self.entity_vis,
                    0,
                    stage_info_actor_num(info),
                    origin,
                    radius.unwrap_or(0.0),
                    &self.world.key_light_hide,
                    sun,
                );
            }
        }
        self.brush_vis = vec![0u32; 512];
        for debris in &self.world.debris_solids {
            let (index, origin) = (u32::from(debris.index), debris.at_point);
            cone_shadow_link_debris_solid_vis(&mut self.brush_vis, index, origin, 0.0, &self.world.key_light_hide, sun);
        }
    }

    /// Whether a static model placement casts into a shadow for this view, and
    /// with which flags.
    fn static_model_flags(&self, placement: u16) -> Option<u8> {
        let inst_ix = self
            .world
            .prop_instances
            .get(usize::from(placement))?
            .as_ref()?;
        if !lighting_t6::spot_shadow_static_model_casts(inst_ix.flags) {
            return None;
        }
        let eye = self.view.eye.to_array();
        let offset: [f32; 3] = std::array::from_fn(|axis| inst_ix.origin[axis] - eye[axis]);
        let squared_gap: f32 = offset.iter().map(|d| d * d).sum();
        if prop_beyond_cull_range(inst_ix.hide_dist, squared_gap, self.detail_ramp.prior_size) {
            return None;
        }
        let lod = self.world.prop_meshes.get(inst_ix.mesh).and_then(|mesh| mesh.lod);
        prop_camera_detail(lod, inst_ix.origin, inst_ix.scale, Some(self.view.eye), self.detail_ramp)?;
        Some(inst_ix.flags)
    }

    /// The draws of the dynamic brush models that light reaches.
    fn brush_draws(&self, slot: &SpotShadowSubmittedSlot) -> Vec<HeldDrawItem> {
        let mut found = Vec::new();
        for solid in &self.world.debris_solids {
            if solid.solid_model == 0 {
                continue;
            }
            let Some(bit) = debris_lit_bit(
                u32::from(solid.index),
                u32::from(slot.shadow_light_slot),
                self.sun_lights(),
                self.lights.len() as u32,
            ) else {
                continue;
            };
            let reached = lamp_lit_bit(&self.brush_vis, bit);
            if !mover_brush_admitted(slot.scans_casters, reached) {
                continue;
            }
            let Some(model) = self
                .world
                .culling
                .as_ref()
                .and_then(|cull| cull.solid_models.get(usize::from(solid.solid_model)))
            else {
                continue;
            };
            let first = u32::from(model.start_facet);
            let surfaces: Vec<u16> = (first..first.saturating_add(u32::from(model.facet_tally)))
                .filter_map(|surf| u16::try_from(surf).ok())
                .collect();
            found.extend(self.retained.world_surfaces(&surfaces));
        }
        found
    }

    /// Animated model draws of the scene entities this slot sees.
    fn scene_entity_draws(
        &self,
        scene: &render_frontend::VisualStage,
        shadow_slot: &SpotShadowSubmittedSlot,
        sink: &mut Vec<HeldDrawItem>,
        entnums: &[u32],
        pose_accepted_cases: &[bool],
    ) {
        let camera = scene
            .stage_actor_walked
            .then_some(scene.stage_actor_visible.as_slice());
        let mut picked = [0u32; 512];
        let take = |entnum: u32, sink: &mut Vec<HeldDrawItem>| {
            if entnum < SPOT_SHADOW_ENTITY_MARK_CAPACITY as u32
                && let Some(draws) = self.by_entity.get(&entnum)
            {
                sink.extend_from_slice(draws);
            }
        };

        let mut dobj_shown = [0u8; SPOT_SHADOW_ANIMATED_VISIBILITY_SPAN * CONE_TILE_COUNT];
        populate_spot_animated_visibility(&mut dobj_shown, shadow_slot.slot_no, entnums, camera, Some(pose_accepted_cases));
        let seen = split_moving_ids(
            &dobj_shown,
            shadow_slot.slot_no,
            scene.stage_puppets.len() as u32,
            &mut picked,
        );
        for &at in &picked[..seen] {
            if let Some(dobj) = scene.stage_puppets.get(at as usize) {
                take(stage_info_actor_num(dobj.about), sink);
            }
        }

        let model_actor_nums: Vec<u32> = scene
            .stage_models
            .iter()
            .map(|model| stage_info_actor_num(model.about))
            .collect();
        let mut props_vis = [0u8; SPOT_SHADOW_STATIC_VISIBILITY_SPAN * CONE_TILE_COUNT];
        populate_spot_static_visibility(&mut props_vis, shadow_slot.slot_no, &model_actor_nums, camera, None);
        let seen = cone_shadow_stitch_stage_model_indices(
            &props_vis,
            shadow_slot.slot_no,
            scene.stage_models.len() as u32,
            &mut picked,
        );
        for &at in &picked[..seen] {
            if let Some(model) = scene.stage_models.get(at as usize) {
                take(stage_info_actor_num(model.about), sink);
            }
        }
    }
}

/// Fills `plan` with the slots of the lights chosen to cast a shadow this
/// frame. Stops quietly, leaving it empty, whenever nothing can be shadowed.
pub fn fill_cone_shadow_caster_plan(
    draw_plan: &mut ConeShadowCasterPlan,
    occupancy: &ConeShadowStageOccupancy,
    scene: Option<&render_frontend::VisualStage>,
    world: &LevelStage,
    view: Option<&StagedStageEye>,
    xmodel: Option<&FigureDrawPlan>,
    retained: &[HeldDrawItem],
    level_run_facets: &[u16],
    level_ranges: &[(u32, u32)],
    level_vertex_count: u32,
    prop_ranges: &[(u32, u32)],
    smodel: Option<&PropGpuPlan>,
    detail_ramp: DetailRampArgs,
    stage_time: u32,
    sm_enable: Option<bool>, sm_daylight_enable: Option<bool>,
) {
    draw_plan.wipe_frame();
    let lights = candidates(world);
    if lights.is_empty() || !lighting_t6::cone_pick_enabled(sm_enable, None) {
        return;
    }
    let Some(view_pos) = view.filter(|view| view.primed) else {
        return;
    };
    let animated_draws = xmodel.map_or(&[][..], |plan| plan.draws.as_slice());
    let sun = world.daylight_key_light_tally;
    let mut used = lights_in_use(world, animated_draws);
    let sun_enabled = sm_daylight_enable.unwrap_or(lighting_t6::SUN_SHADOW_ENABLED_BY_DEFAULT);
    if lighting_t6::sun_off_frees_lights(sun_enabled, sun) {
        lighting_t6::unmark_lights_to_sun(&mut used, sun);
    }
    let light_count = lights.len() as u32;
    let Some(chosen) = pick_cone_shadows(
        &mut draw_plan.history, used, &lights,
        view_pos.eye.to_array(), view_pos.forward.to_array(),
        stage_time,
        sun,
        light_count,
        None,
        choose_dvars(),
        LAMP_LUMA_MIX,
    ) else {
        return;
    };
    let emitted = submit_spot_shadow_frontend(&chosen, &lights, light_count, 0.0);
    // Choosing updates the history even when no light ends up with a slot;
    // the visibility work below only has users in emitted slots.
    let slots: Vec<SpotShadowSubmittedSlot> = emitted
        .slots
        .iter()
        .take(CONE_TILE_COUNT)
        .flatten()
        .copied()
        .collect();
    if slots.first().is_none() {
        return;
    }
    let words = spot_shadow_main_light_visibility_words(1, SPOT_SHADOW_CONFIG_ENTITY_LIMIT, sun, light_count);
    if words == 0 {
        return;
    }

    let retained = RetainedLookup::index(retained);
    let mut by_entity = HashMap::<u32, Vec<HeldDrawItem>>::new();
    if retained.has_animated() {
        for draw in animated_draws {
            if let (Some(entnum), Some(item)) = (draw.stage_actor_num, retained.animated(draw)) {
                by_entity.entry(entnum).or_default().push(item);
            }
        }
    }
    let mut frame = Frame {
        world,
        scene,
        view: view_pos,
        animated: xmodel,
        prop_set: smodel,
        retained,
        lights: &lights,
        detail_ramp,
        entity_vis: Vec::new(),
        brush_vis: Vec::new(),
        by_entity,
    };
    frame.link_visibility(words);
    let entnums = frame.puppet_entity_numbers();
    let pose_accepted_cases = frame.puppet_pose_flags();
    let mut dobj_casters = vec![NO_CASTER; occupancy.dobjs.len()];
    let mut model_casters = vec![NO_CASTER; occupancy.models.len()];

    for slot in &slots {
        let Some(light) = lights.get(usize::from(slot.shadow_light_slot)) else {
            continue;
        };
        let occupied = |slots: &[ConeViewSlot], casters: &mut [ConeLightOccluderFeed]| {
            populate_spot_shadow_occupancy(
                slots,
                Some(frame.entity_vis.as_slice()),
                0,
                u32::from(slot.shadow_light_slot),
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
        let mut marks = vec![0u8; SPOT_SHADOW_ENTITY_MARK_CAPACITY];
        cone_shadow_walk_casters(
            slot.scans_casters,
            light,
            &dobj_casters,
            &model_casters,
            u32::MAX,
            &mut marks,
        );

        let geometry = world.shadow_geometry.get(usize::from(slot.shadow_light_slot));
        let mut surface_ids = [0u16; 512];
        let surface_n = cone_world_facets(
            slot.scans_casters,
            geometry.map_or(&[][..], |geometry| geometry.surface_total.as_slice()),
            &mut surface_ids,
        );
        let mut draws = frame.retained.world_surfaces(&surface_ids[..surface_n]);

        let mut placement_ids = [0u16; 512];
        let placement_n = cone_shadow_prune_static_ids(
            slot.scans_casters,
            geometry.map_or(&[][..], |geometry| geometry.fixed_props.as_slice()),
            |placement| frame.static_model_flags(placement),
            &mut placement_ids,
        );
        draws.extend(frame.retained.static_models_at_lod(
            &placement_ids[..placement_n],
            frame.prop_set,
            view_pos.eye,
            detail_ramp,
        ));
        draws.extend(frame.brush_draws(slot));
        if let (Some(present), true) = (scene, frame.animated.is_some()) {
            frame.scene_entity_draws(present, slot, &mut draws, &entnums, &pose_accepted_cases);
        }
        if let Some(animated) = frame.animated {
            for draw in &animated.draws {
                if model_casts_cone(&marks, draw.stage_actor_num)
                    && let Some(item) = frame.retained.animated(draw)
                {
                    draws.push(item);
                }
            }
        }

        let animated_ranges = frame.animated.map_or(&[][..], |plan| plan.span_rows());
        let flat: Vec<_> = draws.iter().map(pack_draw).collect();
        let packed = pack_cone_shadow_frontend(
            &flat,
            level_run_facets,
            level_ranges,
            level_vertex_count,
            prop_ranges,
            animated_ranges,
        );
        if cone_lists_ready(packed.work_entry_tally()) {
            draw_plan.record_slot(draws, *slot, packed);
        }
    }
}
