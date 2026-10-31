//! Sun glare: a one-texel history texture tracks how much sky the sun sees, and the sprite,
//! flare and screen-grade passes read it back. `draw` runs the passes, `pipelines` builds them,
//! `targets` makes the small textures and `uniforms` packs the per-frame numbers.

mod draw;
mod pipelines;
mod targets;
mod uniforms;

use bevy::core_pipeline::tonemapping::tonemapping;
use bevy::core_pipeline::{Core3d, Core3dSystems};
use bevy::prelude::*;
use bevy::render::RenderStartup;
use bevy::render::render_resource::binding_types::{
    sampler, texture_2d, texture_depth_2d, uniform_buffer_sized,
};
use bevy::render::render_resource::{
    BindGroupLayoutDescriptor, BindGroupLayoutEntries, Buffer, BufferDescriptor, BufferUsages,
    CachedRenderPipelineId, FilterMode, Sampler, SamplerBindingType, SamplerDescriptor,
    ShaderStages, SpecializedRenderPipelines, TextureSampleType,
};
use bevy::render::renderer::{RenderDevice, RenderQueue};
use bevy::render::{Render, RenderApp, RenderSystems};
use bevy::shader::Shader;
use std::num::NonZeroU64;

use super::postfx_frame::PostEffectSet;
use targets::Texel;
use uniforms::UNIFORM_BYTES;

const SHADER_SOURCE_PATH: &str = "embedded://render_gpu/surf_gpu/sun_effects/sun.wgsl";

/// Render-world state shared by all four sun passes.
#[derive(Resource)]
struct SunGpu {
    program: Handle<Shader>,
    layout: BindGroupLayoutDescriptor,
    uniforms: Buffer,
    filter_samp: Sampler,
    white: Texel,
    /// Ping-pong pair: one is read as last frame's state while the other is written.
    history: [Texel; 2],
    /// Index of the history texture written this frame.
    write: usize,
    pipelines: [Option<CachedRenderPipelineId>; 4],
}

fn bind_layout() -> BindGroupLayoutDescriptor {
    let filterable = TextureSampleType::Float { filterable: true };
    BindGroupLayoutDescriptor::new(
        "bo2_sun_effects",
        &BindGroupLayoutEntries::sequential(
            ShaderStages::VERTEX_FRAGMENT,
            (
                uniform_buffer_sized(false, NonZeroU64::new(UNIFORM_BYTES)),
                texture_depth_2d(),
                texture_2d(filterable),
                texture_2d(filterable),
                sampler(SamplerBindingType::Filtering),
                texture_2d(filterable),
            ),
        ),
    )
}

fn init_sun_gpu(
    mut commands: Commands,
    asset_srv: Res<AssetServer>,
    gpu_dev: Res<RenderDevice>,
    gpu_queue: Res<RenderQueue>,
) {
    let uniforms = gpu_dev.create_buffer(&BufferDescriptor {
        label: Some("bo2_sun_params"),
        size: UNIFORM_BYTES,
        usage: BufferUsages::UNIFORM | BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let sampler = gpu_dev.create_sampler(&SamplerDescriptor {
        label: Some("bo2_sun_sampler"),
        mag_filter: FilterMode::Linear,
        min_filter: FilterMode::Linear,
        ..default()
    });
    commands.insert_resource(SunGpu {
        program: asset_srv.load(SHADER_SOURCE_PATH),
        layout: bind_layout(),
        uniforms,
        filter_samp: sampler,
        white: targets::white_texel(&gpu_dev, &gpu_queue),
        history: [
            targets::history_texel(&gpu_dev, "bo2_sun_history_a"),
            targets::history_texel(&gpu_dev, "bo2_sun_history_b"),
        ],
        write: 0,
        pipelines: [None; 4],
    });
}

pub(super) fn register(host: &mut App) {
    bevy::asset::embedded_asset!(host, "sun.wgsl");
    let Some(render_sub) = host.get_sub_app_mut(RenderApp) else {
        return;
    };
    render_sub
        .init_resource::<SpecializedRenderPipelines<SunGpu>>()
        .add_systems(RenderStartup, init_sun_gpu)
        .add_systems(
            Render,
            pipelines::queue_pipelines.in_set(RenderSystems::Prepare),
        )
        .add_systems(
            Core3d,
            draw::draw_sun
                .in_set(Core3dSystems::PostProcess)
                .after(tonemapping)
                .before(PostEffectSet),
        );
}
