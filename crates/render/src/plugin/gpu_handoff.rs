use bevy::prelude::*;
use bevy::render::MainWorld;
use bevy::render::render_resource::PipelineCache;
use render_frontend::prepare::scene::world_gpu::{GpuLoadProgress, GpuSubmitDemand};
use render_gpu::GpuSubmitReady;

/// Render world to main world: how many pipelines exist and how many are
/// still compiling, for the loading screen to show.
pub(super) fn extract_gpu_load_progress(
    mut main_world: ResMut<MainWorld>,
    pipelines: Res<PipelineCache>,
) {
    let Some(mut progress) = main_world.get_resource_mut::<GpuLoadProgress>() else {
        return;
    };
    progress.waiting_n = pipelines.waiting_pipelines().count() as u32;
    progress.pipeline_n = pipelines.pipelines().count() as u32;
}

/// Copies what the frontend wants drawn into the form the GPU side reads.
/// With no demand the GPU side is told it has none.
pub(super) fn stamp_gpu_submit_ready(
    demand: Option<Res<GpuSubmitDemand>>,
    mut ready: ResMut<GpuSubmitReady>,
) {
    *ready = match demand {
        None => GpuSubmitReady::default(),
        Some(demand) => GpuSubmitReady {
            world_generation: demand.world_generation,
            warm_pipelines: demand.warm_pipelines,
            overlay_gpu_wait: demand.overlay_gpu_wait,
            pipeline_world_materials: demand.pipeline_world_materials.clone(),
            pipeline_smodel_materials: demand.pipeline_smodel_materials.clone(),
            pipeline_demand_revision: demand.pipeline_demand_revision,
        },
    };
}
