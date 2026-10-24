//! Turning the prepared scene into the frame's draw lists: the plugin that
//! orders the assembly systems after the scene is culled and animated, and
//! before the frame products are published.

use bevy::ecs::schedule::{MultiThreadedExecutor, ScheduleLabel};
use bevy::prelude::*;
use frame::WorkerCmdSet;
use net::ClientSet;

use crate::adapters::anim::fpv_present::{FpvGeometrySet, FpvPlacementSet};
use crate::adapters::anim::script_model::ScriptModelDrawSet;
use crate::prepare::scene::{cull, gfx_scene, model_lighting_cache, smodel_lighting, spawn};
use crate::prepare::scene::view_parms::stamp_prepared_scene_view;

pub mod drawsurf;
mod match_reset;
pub mod pack;

use drawsurf::tess;

/// Bakes the static sun-shadow casters; runs nested so it can use the
/// multi-threaded executor.
#[derive(ScheduleLabel, Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) struct StaticSunAndFx;

/// Rebuilds the models', effects' and static geometry's draw lanes; runs
/// nested on the multi-threaded executor.
#[derive(ScheduleLabel, Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) struct DrawLaneRebuild;

fn prepare_static_sun_and_fx(world: &mut World) {
    let _span = perf::Span::HostStaticSunFxMs.enter();
    world.run_schedule(StaticSunAndFx);
}

fn rebuild_draw_lanes(world: &mut World) {
    world.run_schedule(DrawLaneRebuild);
}

pub struct RenderAssemblePlugin;

impl Plugin for RenderAssemblePlugin {
    fn build(&self, app: &mut App) {
        match_reset::register_match_reset_systems(app);
        add_static_sun_and_fx(app);
        add_world_plans(app);
        add_draw_lanes(app);
        add_frame_products(app);
        app.add_systems(
            PostUpdate,
            drawsurf::log_probe_index_census
                .after(bevy::transform::TransformSystems::Propagate),
        );
    }
}

/// The static sun-shadow casters, baked once per world, and the nested
/// schedule that runs that bake alongside the effect geometry.
fn add_static_sun_and_fx(app: &mut App) {
    app.init_resource::<drawsurf::StaticSunCasters>()
        .add_systems(StaticSunAndFx, drawsurf::bake_static_sun_shadow_casters)
        .edit_schedule(StaticSunAndFx, |schedule| {
            schedule.set_executor(MultiThreadedExecutor::new());
        })
        .add_systems(
            Update,
            prepare_static_sun_and_fx
                .in_set(WorkerCmdSet::FxVerts)
                .after(WorkerCmdSet::FxRemaining)
                .after(WorkerCmdSet::SmodelCache)
                .after(drawsurf::update_command_context_code_sources)
                .after(cull::apply_dpvs_cull)
                .after(smodel_lighting::update_smodel_lighting),
        );
}

/// The plans for the world itself, built as it spawns, and the lighting
/// handed to models once their lighting is resolved.
fn add_world_plans(app: &mut App) {
    app.add_systems(
        Update,
        (
            tess::world::build_world_draw_gpu_plan
                .after(spawn::spawn_world)
                .before(spawn::spawn_world_finish),
            tess::smodel::build_smodel_gpu_plan
                .after(spawn::spawn_world_finish)
                .before(WorkerCmdSet::CellStatic),
            tess::sky::build_sky_model_draw_plan
                .after(spawn::spawn_world_finish)
                .before(rebuild_draw_lanes),
            drawsurf::update_command_context_code_sources.after(stamp_prepared_scene_view),
        )
            .in_set(ClientSet::Present),
    )
    .add_systems(
        Update,
        tess::xmodel::apply_resolved_xmodel_lighting
            .after(model_lighting_cache::update_dirty_model_lighting)
            .in_set(WorkerCmdSet::AddSceneEnt),
    );
}

/// The three draw lanes, rebuilt together once everything they draw from is
/// ready.
fn add_draw_lanes(app: &mut App) {
    app.add_systems(
        DrawLaneRebuild,
        (
            drawsurf::rebuild_xmodel_draw_lane,
            drawsurf::rebuild_fx_draw_lane,
            drawsurf::rebuild_static_draw_lane,
        ),
    )
    .edit_schedule(DrawLaneRebuild, |schedule| {
        schedule.set_executor(MultiThreadedExecutor::new());
    })
    .add_systems(
        Update,
        rebuild_draw_lanes
            .after(FpvPlacementSet)
            .after(FpvGeometrySet)
            .after(WorkerCmdSet::AddSceneEnt)
            .after(tess::xmodel::apply_resolved_fx_model_lighting)
            .after(ScriptModelDrawSet)
            .after(WorkerCmdSet::FxVerts)
            .after(cull::apply_dpvs_cull)
            .after(smodel_lighting::update_smodel_lighting)
            .after(WorkerCmdSet::SmodelCache)
            .after(drawsurf::ingest_drawsurf_list)
            .after(drawsurf::update_command_context_code_sources)
            .in_set(ClientSet::Present),
    );
}

/// Frame products: open them, bake the shadow casters, execute the sun,
/// spot and camera products, then publish.
fn add_frame_products(app: &mut App) {
    app.add_systems(
        Update,
        (
            drawsurf::open_frame_products
                .after(WorkerCmdSet::FxVerts)
                .after(drawsurf::update_command_context_code_sources),
            drawsurf::bake_sun_shadow_casters
                .after(prepare_static_sun_and_fx)
                .after(drawsurf::open_frame_products)
                .after(rebuild_draw_lanes)
                .after(WorkerCmdSet::SmodelCache)
                .after(cull::apply_dpvs_cull)
                .after(smodel_lighting::update_smodel_lighting),
            drawsurf::bake_spot_shadow_casters
                .after(drawsurf::open_frame_products)
                .after(rebuild_draw_lanes)
                .after(gfx_scene::snapshot_spot_shadow_occupancy)
                .after(cull::apply_dpvs_cull),
            drawsurf::execute_sun_product
                .after(drawsurf::bake_sun_shadow_casters)
                .after(rebuild_draw_lanes),
            drawsurf::execute_spot_product
                .after(drawsurf::bake_spot_shadow_casters)
                .after(rebuild_draw_lanes),
            drawsurf::execute_camera_products
                .after(drawsurf::bake_sun_shadow_casters)
                .after(drawsurf::bake_spot_shadow_casters)
                .after(rebuild_draw_lanes),
            drawsurf::publish_frame_products
                .after(drawsurf::execute_camera_products)
                .after(drawsurf::execute_sun_product)
                .after(drawsurf::execute_spot_product),
        )
            .in_set(ClientSet::Present),
    );
}
