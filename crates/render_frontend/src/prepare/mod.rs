//! Frame preparation: the world spawn, the camera, visibility culling, model
//! lighting and the gfx scene, scheduled in `ClientSet::Present`.

use bevy::prelude::*;
use frame::{HasWorld, ModelLightingSeated, SessionSwapApplied, WorkerCmdSet};
use net::ClientSet;

pub mod gfx_scene;
pub mod scene;
pub mod sun;
pub mod worker_cmds;

use crate::adapters::anim::fpv_present::{spawn_pending_fpv, tick_fpv_viewmodel};
use crate::adapters::anim::scene_submission::AnimSceneSubmit;
use crate::adapters::anim::script_model::{ScriptModelDrawSet, ScriptModelSkinSet};
use crate::adapters::anim::view_kick::sync_camera_from_presented;
use crate::assemble::drawsurf::{
    self as drawsurf, ColourDrawMethod, DrawSurfList, FpvDrawPlan, MaterialFrameInputs,
    MaterialGeneration, RenderFrameProducts, StaticDrawLane,
};
use crate::prepare::scene::camera::{fly_camera, look_camera_override};
use crate::prepare::scene::cull::{
    DpvsFrameStats, apply_dpvs_cull, log_dpvs_stats_once, log_script_model_gaps_once,
};
use crate::prepare::scene::model_lighting_cache::{
    begin_dyn_model_lighting_frame, enqueue_fpv_model_lighting, update_dirty_model_lighting,
    update_fx_dyn_lighting, update_glass_dyn_lighting,
};
use crate::prepare::scene::smodel_geom_cache::{
    FrontendWorkerCmds, LodRampDvar, LodRampSkinnedDvar, PretessDvar, SmcEnableDvar,
    cache_visible_smodel_surfaces, skin_cached_static_model_cmd,
};
use crate::prepare::scene::smodel_lighting::update_smodel_lighting;
use crate::prepare::scene::spawn::{
    WorldSpawnJob, arm_world_spawn_on_install, despawn_fly_cameras_on_teardown,
    despawn_world_entities_on_teardown, register_world_gpu_ready, reset_world_spawn_on_teardown,
    shutdown_world_on_teardown, spawn_world, spawn_world_finish,
};
use crate::prepare::scene::view_parms::stamp_prepared_scene_view;
use crate::prepare::scene::world::WorldScene;

use scene::gfx_scene::{
    GfxSceneAdd, GfxSceneClear, apply_anim_dobj_scene_submissions, clear_host_gfx_scene,
    occupy_script_brush_scene, snapshot_spot_shadow_occupancy,
};

pub struct RenderPreparePlugin;

impl Plugin for RenderPreparePlugin {
    fn build(&self, app: &mut App) {
        drawsurf::dof::register(app);
        drawsurf::film_vision_view::register(app);
        init_resources(app);
        schedule_gfx_scene(app);
        schedule_frame_prep(app);
        schedule_model_lighting(app);
        schedule_camera_and_cull(app);
        schedule_teardown(app);
        register_world_gpu_ready(app);
        scene::cell_frustum_cmds::register_cell_frustum_cmds(app);
    }
}

fn init_resources(app: &mut App) {
    app.init_resource::<HasWorld>()
        .init_resource::<WorldScene>()
        .init_resource::<WorldSpawnJob>()
        .init_resource::<DpvsFrameStats>()
        .init_resource::<DrawSurfList>()
        .init_resource::<StaticDrawLane>()
        .init_resource::<drawsurf::XModelDrawLane>()
        .init_resource::<drawsurf::FxDrawLane>()
        .init_resource::<RenderFrameProducts>()
        .init_resource::<ColourDrawMethod>()
        .init_resource::<SmcEnableDvar>()
        .init_resource::<PretessDvar>()
        .init_resource::<LodRampDvar>()
        .init_resource::<LodRampSkinnedDvar>()
        .init_resource::<FrontendWorkerCmds>()
        .init_resource::<drawsurf::MapSunEffects>()
        .init_resource::<drawsurf::SunEffectsFrameInput>()
        .init_resource::<drawsurf::MapPrimaryLightTypes>()
        .init_resource::<drawsurf::MapPrimaryLights>()
        .init_resource::<drawsurf::DrawMethodDfog>()
        .init_resource::<drawsurf::fog::FogDvars>()
        .init_resource::<drawsurf::SunShadowMapPresent>()
        .init_resource::<drawsurf::SpotShadowMapLights>()
        .init_resource::<drawsurf::SunShadowCasterPlan>()
        .init_resource::<drawsurf::SpotShadowCasterPlan>()
        .init_resource::<MaterialGeneration>()
        .init_resource::<MaterialFrameInputs>()
        .init_resource::<drawsurf::FrameAssemblyInputs>()
        .init_resource::<drawsurf::CameraProducts>()
        .init_resource::<drawsurf::DistortionSettings>()
        .init_resource::<drawsurf::SunProduct>()
        .init_resource::<drawsurf::SpotProduct>()
        .init_resource::<drawsurf::fog::ScriptFogPresentation>()
        .init_resource::<FpvDrawPlan>()
        .init_resource::<drawsurf::RemoteBodyDrawPlan>()
        .init_resource::<drawsurf::ScriptModelDrawPlan>()
        .init_resource::<drawsurf::MissileDrawPlan>()
        .init_resource::<drawsurf::ItemDrawPlan>()
        .init_resource::<drawsurf::FxModelDrawPlan>()
        .init_resource::<drawsurf::DynEntDrawPlan>()
        .init_resource::<drawsurf::XModelDrawPlan>()
        .init_resource::<frame::ClassSelectHandoff>()
        .init_resource::<frame::Retiring>();
}

/// The gfx scene is cleared after the world spawns, filled between the cell
/// walk and the model workers, and read by the spot shadows after that.
fn schedule_gfx_scene(app: &mut App) {
    app.configure_sets(
        Update,
        (
            GfxSceneClear.after(spawn_world).before(GfxSceneAdd),
            GfxSceneAdd
                .after(begin_dyn_model_lighting_frame)
                .after(WorkerCmdSet::CellStatic)
                .before(WorkerCmdSet::CellDynModel)
                .before(WorkerCmdSet::SkinModel),
        )
            .in_set(ClientSet::Present),
    )
    .add_systems(
        Update,
        (
            clear_host_gfx_scene.in_set(GfxSceneClear),
            apply_anim_dobj_scene_submissions
                .after(AnimSceneSubmit)
                .in_set(GfxSceneAdd),
            occupy_script_brush_scene
                .after(spawn_world)
                .after(ScriptModelDrawSet)
                .after(apply_anim_dobj_scene_submissions)
                .in_set(GfxSceneAdd),
            snapshot_spot_shadow_occupancy
                .after(GfxSceneAdd)
                .after(WorkerCmdSet::SkinModel)
                .in_set(ClientSet::Present),
        ),
    );
}

/// The world spawn, then everything that stamps and consumes the frame's
/// view: fog, sun, the draw-surface list, static-model lighting and cache.
fn schedule_frame_prep(app: &mut App) {
    app.add_systems(
        Update,
        (
            arm_world_spawn_on_install.before(spawn_world),
            spawn_world,
            spawn_world_finish.after(spawn_world),
            render_anim::prepare_fpv_compositions.after(spawn_world_finish),
            render_anim::prepare_model_materials.after(spawn_world_finish),
            drawsurf::fog::sync_script_fog
                .after(spawn_world_finish)
                .before(stamp_prepared_scene_view),
            fly_camera,
            stamp_prepared_scene_view
                .after(fly_camera)
                .after(sync_camera_from_presented)
                .after(spawn_pending_fpv)
                .after(tick_fpv_viewmodel),
            drawsurf::publish_sun_effects_frame.after(stamp_prepared_scene_view),
            drawsurf::ingest_drawsurf_list
                .after(WorkerCmdSet::CellStatic)
                .after(WorkerCmdSet::CellDynModel)
                .after(WorkerCmdSet::CellDynBrush),
            update_smodel_lighting
                .after(WorkerCmdSet::CellStatic)
                .after(WorkerCmdSet::SkinModel)
                .after(ScriptModelSkinSet),
            cache_visible_smodel_surfaces.after(update_smodel_lighting),
            begin_dyn_model_lighting_frame.after(spawn_world),
            enqueue_fpv_model_lighting
                .after(render_anim::spawn_pending_fpv)
                .after(render_anim::tick_fpv_viewmodel)
                .after(begin_dyn_model_lighting_frame)
                .after(render_anim::ScriptModelDrawSet)
                .before(WorkerCmdSet::CellDynModel),
            log_dpvs_stats_once.after(WorkerCmdSet::CellStatic),
            log_script_model_gaps_once,
        )
            .in_set(ClientSet::Present),
    )
    .add_systems(
        Update,
        skin_cached_static_model_cmd
            .in_set(WorkerCmdSet::SmodelCache)
            .after(cache_visible_smodel_surfaces),
    );
}

/// Dynamic model lighting: dirty models, then glass, then effects, with the
/// effects' lit models resolved last.
fn schedule_model_lighting(app: &mut App) {
    app.add_systems(
        Update,
        update_dirty_model_lighting
            .after(begin_dyn_model_lighting_frame)
            .after(WorkerCmdSet::SkinModel)
            .after(WorkerCmdSet::CellDynModel)
            .after(ScriptModelSkinSet)
            .after(enqueue_fpv_model_lighting)
            .in_set(ModelLightingSeated)
            .before(update_glass_dyn_lighting),
    )
    .add_systems(
        Update,
        (
            update_glass_dyn_lighting
                .after(update_dirty_model_lighting)
                .before(WorkerCmdSet::FxVerts),
            update_fx_dyn_lighting
                .after(WorkerCmdSet::FxVerts)
                .after(update_glass_dyn_lighting),
            drawsurf::tess::xmodel::apply_resolved_fx_model_lighting
                .after(update_fx_dyn_lighting),
        )
            .in_set(ClientSet::Present),
    );
}

fn schedule_camera_and_cull(app: &mut App) {
    app.add_systems(
        Update,
        look_camera_override
            .after(fly_camera)
            .after(sync_camera_from_presented)
            .before(stamp_prepared_scene_view),
    )
    .add_systems(
        Update,
        apply_dpvs_cull
            .after(fly_camera)
            .after(stamp_prepared_scene_view)
            .in_set(WorkerCmdSet::CellStatic),
    );
}

/// On a session swap the world goes in a fixed order: its entities, its
/// spawn state, the world itself, then the fly cameras.
fn schedule_teardown(app: &mut App) {
    app.add_systems(
        Update,
        (
            despawn_world_entities_on_teardown,
            reset_world_spawn_on_teardown,
            shutdown_world_on_teardown,
            despawn_fly_cameras_on_teardown,
        )
            .chain()
            .after(SessionSwapApplied)
            .in_set(ClientSet::Load),
    )
    .add_systems(
        Update,
        publish_dyn_atpoint_lookup
            .after(SessionSwapApplied)
            .after(shutdown_world_on_teardown)
            .in_set(ClientSet::Load),
    );
}

/// Static tables are complete when the install transaction publishes its
/// message, so that is when the dynamic-entity and cell lookups are filled.
fn publish_dyn_atpoint_lookup(
    mut installed: MessageReader<frame::MatchInstalled>,
    scene: Res<WorldScene>,
    mut lookup: ResMut<render_scene::DynAtPointLookup>,
    mut cells: ResMut<render_scene::WorldDpvsCells>,
) {
    if installed.read().count() != 0 {
        scene.publish_dyn_atpoint(&mut lookup);
        scene.publish_dpvs_cells(&mut cells);
    }
}
