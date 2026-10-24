//! When the script-model systems run.

use bevy::prelude::*;

use super::focus::{FocusSelect, RenderFocus};
use super::mover::apply_script_mover_centity_pose;
use super::plan::commit_script_model_draw_plan;
use super::pose::pose_script_models;
use super::presented::apply_presented_script_model_dobjs;
use super::product::ScriptModelPoseProduct;
use super::scene_ents::occupy_script_model_scene_ents;
use super::spawn::sync_spawned_scene_models;
use super::state::ScriptModelDobjs;
use crate::anim::scene_submission::AnimSceneSubmit;
use crate::ScriptModelDrawPlan;

/// The systems that place script models and tell the scene of them.
#[derive(SystemSet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ScriptModelDrawSet;

/// The systems that pose script models and plan their draws.
#[derive(SystemSet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ScriptModelSkinSet;

pub fn register_script_model_systems(app: &mut App) {
    let select = FocusSelect::from_env()
        .unwrap_or_else(|error| panic!("focused render observation refused: {error}"));
    app.insert_resource(select)
        .init_resource::<RenderFocus>()
        .init_resource::<ScriptModelDrawPlan>()
        .init_resource::<ScriptModelDobjs>()
        .init_resource::<ScriptModelPoseProduct>()
        .add_systems(
            Update,
            (
                sync_spawned_scene_models,
                apply_presented_script_model_dobjs,
                apply_script_mover_centity_pose,
                occupy_script_model_scene_ents,
            )
                .chain()
                .in_set(frame::RenderSet::Anim)
                .before(frame::WorkerCmdSet::CellDynModel)
                .in_set(ScriptModelDrawSet)
                .in_set(render_scene::GfxSceneAdd)
                .in_set(AnimSceneSubmit),
        )
        .add_systems(
            Update,
            (pose_script_models, commit_script_model_draw_plan)
                .chain()
                .after(occupy_script_model_scene_ents)
                .after(frame::WorkerCmdSet::CellSceneEnt)
                .after(frame::WorkerCmdSet::DpvsEnt)
                .in_set(ScriptModelSkinSet)
                .in_set(frame::WorkerCmdSet::SkinModel),
        );
}
