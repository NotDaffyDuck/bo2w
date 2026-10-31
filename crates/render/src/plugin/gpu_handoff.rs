use bevy::prelude::*;
use bevy::render::MainWorld;
use bevy::render::render_resource::PipelineCache;
use render_frontend::prepare::scene::level_gpu::{GpuLoadStatus, GpuSubmitNeed};
use render_gpu::SubmitReadyGpu;

/// Render world to main world: how many pipelines exist and how many are
/// still compiling, for the loading screen to show.
pub(super) fn extract_gpu_load_progress(
    mut main_world: ResMut<MainWorld>,
    pipelines: Res<PipelineCache>,
) {
    let Some(mut progress) = main_world.get_resource_mut::<GpuLoadStatus>() else {
        return;
    };
    progress.waiting_tally = pipelines.waiting_pipelines().count() as u32;
    progress.pipeline_tally = pipelines.pipelines().count() as u32;
}

/// Copies what the frontend wants drawn into the form the GPU side reads.
/// With no demand the GPU side is told it has none.
pub(super) fn stamp_gpu_submit_ready(
    demand: Option<Res<GpuSubmitNeed>>,
    mut ready: ResMut<SubmitReadyGpu>,
) {
    *ready = match demand {
        None => SubmitReadyGpu::default(),
        Some(demand) => SubmitReadyGpu {
            level_generation: demand.level_generation,
            prewarmed_pipelines: demand.prewarmed_pipelines,
            topmost_gpu_wait: demand.topmost_gpu_wait,
            pipeline_level_materials: demand.pipeline_level_materials.clone(),
            pipeline_prop_materials: demand.pipeline_prop_materials.clone(),
            pipeline_demand_revision: demand.pipeline_demand_revision,
        },
    };
}
