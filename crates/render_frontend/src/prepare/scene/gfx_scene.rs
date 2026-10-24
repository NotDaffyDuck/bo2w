//! The host's gfx scene as the frontend fills it: animated models submitted
//! by the anim adapters, and script-mover brush models claimed from the
//! snapshot.

pub use render_scene::{
    GfxSceneAdd, GfxSceneClear, HostGfxScene, SceneEntSkinInput, SceneEntSkinInputs,
    SceneEntSkinModel, SceneEntSkinPending, SceneEntSkinPendingModel, SceneEntSurfaceCache,
    ScriptMoverBmodelClaim, SpotShadowEntityOriginTrack, SpotShadowSceneOccupancy,
    expand_scene_ent_pending, gfx_scene_spot_shadow_dobj_slots, gfx_scene_spot_shadow_model_slots,
    hide_part_bits_from_tags, occupy_add_bmodel, occupy_add_dobj, occupy_add_dobj_fx,
    occupy_script_brushes, scene_quat_from_angles, scene_quat_from_viewmodel_axes,
    store_scene_ent_pending,
};
pub(crate) use render_scene::{clear_host_gfx_scene, snapshot_spot_shadow_occupancy};

use std::sync::Arc;

use bevy::prelude::*;
use entity_t6::script_mover::{SOLID_BMODEL, ScriptMoverDraw, script_mover_draw};
use render_scene::AddDObjPose;

use crate::adapters::anim::scene_submission::AnimDObjSceneSubmission;
use crate::prepare::scene::world::WorldScene;

pub(crate) fn apply_anim_dobj_scene_submissions(
    mut submissions: MessageReader<AnimDObjSceneSubmission>,
    mut scene: ResMut<HostGfxScene>,
    mut skin_inputs: ResMut<SceneEntSkinInputs>,
) {
    for submission in submissions.read() {
        add_submitted_dobj(submission, &mut scene, &mut skin_inputs);
    }
}

fn add_submitted_dobj(
    submission: &AnimDObjSceneSubmission,
    scene: &mut HostGfxScene,
    skin_inputs: &mut SceneEntSkinInputs,
) {
    let pose = AddDObjPose {
        origin: submission.origin,
        lighting_origin: submission.lighting_origin,
        radius: submission.radius,
        entnum: submission.entnum,
        quat: submission.quat,
    };
    occupy_add_dobj_fx(
        &mut scene.scene,
        submission.occupy_model_n as usize,
        submission.has_tree,
        submission.render_fx_flags,
        pose,
    );
    if !submission.store_skin {
        return;
    }
    let models = submission
        .models
        .iter()
        .map(|model| SceneEntSkinPendingModel {
            lod: model.lod,
            bone_count: model.bone_count,
            skel: Arc::clone(&model.skel),
        })
        .collect();
    store_scene_ent_pending(
        &mut scene.scene,
        skin_inputs,
        submission.entnum,
        models,
        submission.hide_part_bits,
    );
}

/// A script mover that is solid as a brush model and has no model object of
/// its own is drawn as its brush; this collects those from the entities.
fn live_script_mover_brushes(
    runtimes: &Query<(&net::CEntity, &net::CEntityRuntime)>,
    scene: &HostGfxScene,
    at_time: i32,
) -> Vec<ScriptMoverBmodelClaim> {
    let mut claims = Vec::new();
    for (cent, runtime) in runtimes {
        let entnum = u32::from(cent.number());
        let state = &runtime.next_state;
        let is_brush_mover =
            state.e_type == entity_t6::et::SCRIPTMOVER && state.solid == SOLID_BMODEL;
        if !is_brush_mover || scene.scene.scene_ent_live(entnum) {
            continue;
        }
        let Ok(model_index) = u32::try_from(state.index) else {
            continue;
        };
        // eFlags2 comes with BO2's EntityState; 0 (no demo hide) until then.
        let draw = script_mover_draw(state.lerp.e_flags, 0, SOLID_BMODEL, false, false);
        claims.push(ScriptMoverBmodelClaim {
            model_index,
            origin: entity_t6::evaluate_trajectory(&runtime.current.pos, at_time),
            angles: entity_t6::evaluate_trajectory(&runtime.current.apos, at_time),
            entnum,
            hidden: matches!(draw, ScriptMoverDraw::Hidden { .. }),
        });
    }
    claims
}

pub(crate) fn occupy_script_brush_scene(
    world: Option<Res<WorldScene>>,
    presented: Option<Res<net::PresentedSnapshot>>,
    runtimes: Query<(&net::CEntity, &net::CEntityRuntime)>,
    mut gfx_scene: ResMut<HostGfxScene>,
) {
    let Some(world) = world else {
        return;
    };
    let brush_models = world
        .cull
        .as_ref()
        .map_or(&[][..], |cull| cull.brush_models.as_slice());
    let at_time = presented
        .as_ref()
        .and_then(|presented| presented.snapshot())
        .map_or(0, |snapshot| net::ServerTime::from_tick(snapshot.tick).ms());
    let live = live_script_mover_brushes(&runtimes, &gfx_scene, at_time);
    occupy_script_brushes(
        &mut gfx_scene.scene,
        brush_models,
        &world.script_brush_models,
        &live,
    );
}
