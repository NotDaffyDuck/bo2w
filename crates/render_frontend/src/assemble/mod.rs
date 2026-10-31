//! Turning the prepared scene into the frame's draw lists: the plugin that
//! orders the assembly systems after the scene is culled and animated, and
//! before the frame products are published.

use bevy::ecs::schedule::{MultiThreadedExecutor, ScheduleLabel};
use bevy::ecs::system::ScheduleSystem;
use bevy::prelude::*;
use frame::JobQueueStage;
use net::ClientStep;

use crate::adapters::motion::gun_present::{FpvGeometrySet, GunMountStage};
use crate::adapters::motion::script_prop::ScriptPropDrawStage;
use super::prepare::scene::{cull, visual_stage, model_illum_cache, prop_light_slots, spawn};
use crate::prepare::scene::eye_parms::stamp_staged_stage_eye;

pub mod draw_items;
mod teardown_reset;
pub mod packed_word;

use draw_items::mesh_build;

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

pub struct DrawAssemblePlugin;

impl Plugin for DrawAssemblePlugin {
    fn build(&self, app: &mut App) {
        teardown_reset::register_match_reset_steps(app);
        add_static_sun_and_fx(app);
        add_world_plans(app);
        add_draw_lanes(app);
        add_frame_products(app);
        app.add_systems(
            PostUpdate,
            draw_items::log_env_probe_index_census
                .after(bevy::transform::TransformSystems::Propagate),
        );
    }
}

/// The static sun-shadow casters, baked once per world, and the nested
/// schedule that runs that bake alongside the effect geometry.
fn add_static_sun_and_fx(app: &mut App) {
    app.init_resource::<draw_items::StaticSunCasters>()
        .add_systems(StaticSunAndFx, draw_items::bake_static_sun_shadow_casters)
        .edit_schedule(StaticSunAndFx, |sched| {
            sched.set_executor(MultiThreadedExecutor::new());
        })
        .add_systems(
            Update,
            prepare_static_sun_and_fx
                .in_set(JobQueueStage::FxVerts)
                .after(JobQueueStage::FxRemaining)
                .after(JobQueueStage::SmodelCache)
                .after(draw_items::update_command_context_engine_sources)
                .after(cull::apply_sight_hide)
                .after(prop_light_slots::update_prop_illum),
        );
}

/// The plans for the world itself, built as it spawns, and the lighting
/// handed to models once their lighting is resolved.
fn add_world_plans(app: &mut App) {
    app.add_present_systems(
        (
            mesh_build::world::build_level_draw_gpu_plan
                .after(spawn::spawn_level_world)
                .before(spawn::spawn_level_finish),
            mesh_build::prop_mesh::build_prop_gpu_plan
                .after(spawn::spawn_level_finish)
                .before(JobQueueStage::CellStatic),
            mesh_build::sky::make_sky_model_draw_plan
                .after(spawn::spawn_level_finish)
                .before(rebuild_draw_lanes),
            draw_items::update_command_context_engine_sources.after(stamp_staged_stage_eye),
        ),
    );
    app.add_systems(
        Update,
        mesh_build::figure_mesh::apply_resolved_figure_illum
            .after(model_illum_cache::refresh_dirty_model_lighting)
            .in_set(JobQueueStage::AddSceneEnt),
    );
}

/// The three draw lanes, rebuilt together once everything they draw from is
/// ready.
fn add_draw_lanes(app: &mut App) {
    app.add_systems(
        DrawLaneRebuild,
        (
            draw_items::rebuild_figure_draw_track,
            draw_items::refill_fx_draw_lane,
            draw_items::refill_static_draw_lane,
        ),
    )
    .edit_schedule(DrawLaneRebuild, |sched| {
        sched.set_executor(MultiThreadedExecutor::new());
    });
    app.add_present_systems(
        rebuild_draw_lanes
            .after(GunMountStage)
            .after(FpvGeometrySet)
            .after(JobQueueStage::AddSceneEnt)
            .after(mesh_build::figure_mesh::apply_resolved_effect_model_illum)
            .after(ScriptPropDrawStage)
            .after(JobQueueStage::FxVerts)
            .after(cull::apply_sight_hide)
            .after(prop_light_slots::update_prop_illum)
            .after(JobQueueStage::SmodelCache)
            .after(draw_items::take_drawsurf_list)
            .after(draw_items::update_command_context_engine_sources),
    );
}

/// Frame products: open them, bake the shadow casters, execute the sun,
/// spot and camera products, then publish.
fn add_frame_products(app: &mut App) {
    app.add_present_systems(
        (
            draw_items::open_frame_outputs
                .after(JobQueueStage::FxVerts)
                .after(draw_items::update_command_context_engine_sources),
            draw_items::bake_daylight_shadow_casters
                .after(prepare_static_sun_and_fx)
                .after(draw_items::open_frame_outputs)
                .after(rebuild_draw_lanes)
                .after(JobQueueStage::SmodelCache)
                .after(cull::apply_sight_hide)
                .after(prop_light_slots::update_prop_illum),
            draw_items::bake_cone_shadow_casters
                .after(draw_items::open_frame_outputs)
                .after(rebuild_draw_lanes)
                .after(visual_stage::capture_cone_shadow_fill)
                .after(cull::apply_sight_hide),
            draw_items::execute_daylight_output
                .after(draw_items::bake_daylight_shadow_casters)
                .after(rebuild_draw_lanes),
            draw_items::execute_cone_output
                .after(draw_items::bake_cone_shadow_casters)
                .after(rebuild_draw_lanes),
            draw_items::execute_camera_outputs
                .after(draw_items::bake_daylight_shadow_casters)
                .after(draw_items::bake_cone_shadow_casters)
                .after(rebuild_draw_lanes),
            draw_items::publish_frame_outputs
                .after(draw_items::execute_camera_outputs)
                .after(draw_items::execute_daylight_output)
                .after(draw_items::execute_cone_output),
        ),
    );
}

/// Adds systems to the client's present step of every frame.
trait PresentSystems {
    fn add_present_systems<M>(&mut self, systems: impl IntoScheduleConfigs<ScheduleSystem, M>) -> &mut Self;
}

impl PresentSystems for App {
    fn add_present_systems<M>(&mut self, systems: impl IntoScheduleConfigs<ScheduleSystem, M>) -> &mut Self {
        self.add_systems(Update, systems.in_set(ClientStep::Present))
    }
}
