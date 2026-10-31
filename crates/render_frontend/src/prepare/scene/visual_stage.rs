//! The host's gfx scene as the frontend fills it: animated models submitted
//! by the anim adapters, and script-mover brush models claimed from the
//! snapshot.

pub use render_scene::{
    RenderSceneInsert, RenderSceneWipe, HostVisualStage, EntitySkinningInput,
    EntitySkinningInputSet, EntitySkinModelHandle, EntitySkinAwaitingWork,
    StageActorSkinQueuedModel, EntitySurfaceMemo, ScriptedBrushMoverClaim,
    SpotShadowEntityOriginLog, ConeShadowStageOccupancy, grow_pending_scene_entities,
    spot_shadow_figure_slots, visual_stage_cone_shadow_model_slots,
    hidden_part_mask_for_tags, stage_brush_model, stage_figure, occupy_add_puppet_effect,
    occupy_script_solids, orientation_quat_from_euler, quat_from_viewmodel_basis,
    store_stage_actor_queued,
};
pub(crate) use render_scene::{reset_host_render_scene, capture_cone_shadow_fill};

use std::sync::Arc;

use bevy::prelude::*;
use entity_t6::script_mover::{SOLID_BMODEL, ScriptMoverDraw, script_mover_draw};
use render_scene::DynObjectPlacementArgs;

use crate::adapters::motion::scene_feed::SkeletonSceneHandoff;
use crate::prepare::scene::world::LevelStage;

pub(crate) fn apply_anim_puppet_stage_submissions(
    mut submissions: MessageReader<SkeletonSceneHandoff>,
    mut scene: ResMut<HostVisualStage>,
    mut skinning_feed: ResMut<EntitySkinningInputSet>,
) {
    for handoff in submissions.read() {
        add_submitted_dobj(handoff, &mut scene, &mut skinning_feed);
    }
}

fn add_submitted_dobj(
    handoff: &SkeletonSceneHandoff,
    stage: &mut HostVisualStage,
    skinning_feed: &mut EntitySkinningInputSet,
) {
    let pose = DynObjectPlacementArgs {
        origin: handoff.origin,
        illum_origin: handoff.illum_origin,
        reach: handoff.radius,
        ent_no: handoff.ent_ix,
        spin: handoff.rot_quat,
    };
    occupy_add_puppet_effect(
        &mut stage.scene,
        handoff.occupy_model_tally as usize,
        handoff.carries_anim_graph,
        handoff.render_effect_flags,
        pose,
    );
    if !handoff.keep_skinned_verts {
        return;
    }
    let models = handoff
        .models
        .iter()
        .map(|entry| StageActorSkinQueuedModel {
            lod: entry.lod,
            joint_total: entry.joint_total,
            bone_rig: Arc::clone(&entry.bone_rig),
        })
        .collect();
    store_stage_actor_queued(
        &mut stage.scene,
        skinning_feed,
        handoff.ent_ix,
        models,
        handoff.hidden_mask,
    );
}

/// A script mover that is solid as a brush model and has no model object of
/// its own is drawn as its brush; this collects those from the entities.
fn live_script_mover_brushes(
    pose_states: &Query<(&net::ClientEntityRecord, &net::ClientEntityPoseState)>,
    scene: &HostVisualStage,
    at_time: i32,
) -> Vec<ScriptedBrushMoverClaim> {
    let mut claims = Vec::new();
    for (record, runtime) in pose_states {
        let entnum = u32::from(record.number());
        let state = &runtime.upcoming_condition;
        let is_brush_mover =
            state.e_type == entity_t6::et::SCRIPTMOVER && state.solid == SOLID_BMODEL;
        if !is_brush_mover || scene.scene.entity_present(entnum) {
            continue;
        }
        let Ok(mesh_slot_number) = u32::try_from(state.index) else {
            continue;
        };
        // eFlags2 comes with BO2's EntityState; 0 (no demo hide) until then.
        let draw = script_mover_draw(state.lerp.e_flags, 0, SOLID_BMODEL, false, false);
        claims.push(ScriptedBrushMoverClaim {
            mesh_slot_number,
            origin: entity_t6::evaluate_trajectory(&runtime.current.path, at_time),
            turn: entity_t6::evaluate_trajectory(&runtime.current.spin, at_time),
            ent_no: entnum,
            hidden: matches!(draw, ScriptMoverDraw::Hidden { .. }),
        });
    }
    claims
}

pub(crate) fn occupy_script_solid_stage(
    level: Option<Res<LevelStage>>,
    shown: Option<Res<net::ShownWorldFrame>>,
    pose_states: Query<(&net::ClientEntityRecord, &net::ClientEntityPoseState)>,
    mut visual_stage: ResMut<HostVisualStage>,
) {
    let Some(level) = level else {
        return;
    };
    let solid_models = level
        .culling
        .as_ref()
        .map_or(Default::default(), |cull| cull.solid_models.as_slice());
    let clock_ms = shown
        .as_ref()
        .and_then(|shown| shown.snapshot())
        .map_or(0, |snapshot| net::HostInstant::from_step_number(snapshot.tick).as_millis());
    let live = live_script_mover_brushes(&pose_states, &visual_stage, clock_ms);
    occupy_script_solids(
        &mut visual_stage.scene,
        solid_models,
        &level.script_solid_models,
        &live,
    );
}
