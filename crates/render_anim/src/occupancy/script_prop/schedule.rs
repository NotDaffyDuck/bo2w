//! When the script-model systems run.

use bevy::prelude::*;

use super::focus::{FocusSelect, DrawFocus};
use super::mover::apply_scripted_mover_centity_pose;
use super::plan::commit_scripted_model_draw_plan;
use super::pose::pose_script_models;
use super::presented::apply_shown_script_model_dobjs;
use super::product::ScriptPropPoseOutput;
use super::scene_ents::occupy_scripted_model_scene_ents;
use super::spawn::sync_spawned_scene_models;
use super::state::ScriptPropSkeletons;
use crate::motion::scene_feed::SkeletonSceneStage;
use crate::occupancy::zm_ragdoll::Ragdolls;
use crate::ScriptedModelPaintPlan;

/// The systems that place script models and tell the scene of them.
#[derive(SystemSet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ScriptPropDrawStage;

/// The systems that pose script models and plan their draws.
#[derive(SystemSet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ScriptPropSkinStage;

pub fn wire_script_prop_systems(application: &mut App) {
    let select = FocusSelect::from_env()
        .unwrap_or_else(|refusal| panic!("focused render observation refused: {refusal}"));
    application.insert_resource(select);
    application.init_resource::<DrawFocus>();
    application.init_resource::<ScriptedModelPaintPlan>();
    application.init_resource::<ScriptPropSkeletons>();
    application.init_resource::<Ragdolls>();
    application.init_resource::<ScriptPropPoseOutput>();

    let place_and_submit = (
        sync_spawned_scene_models,
        apply_shown_script_model_dobjs,
        apply_scripted_mover_centity_pose,
        occupy_scripted_model_scene_ents,
    );
    application.add_systems(
        Update,
        place_and_submit
            .chain()
            .before(frame::JobQueueStage::CellDynModel)
            .in_set(frame::DrawPassStage::Anim)
            .in_set(ScriptPropDrawStage)
            .in_set(render_scene::RenderSceneInsert)
            .in_set(SkeletonSceneStage),
    );

    let skin_and_plan = (pose_script_models, commit_scripted_model_draw_plan);
    application.add_systems(
        Update,
        skin_and_plan
            .chain()
            .after(occupy_scripted_model_scene_ents)
            .after(frame::JobQueueStage::DpvsEnt)
            .after(frame::JobQueueStage::CellSceneEnt)
            .in_set(ScriptPropSkinStage)
            .in_set(frame::JobQueueStage::SkinModel),
    );
}
