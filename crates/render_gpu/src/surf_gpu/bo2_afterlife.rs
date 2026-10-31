//! bo2zm: Black Ops II's script screen filters, after the bloom on a Black
//! Ops II map. Mob's Afterlife turns on its overlay `zm_afterlife_filter`,
//! which BO2 draws with `generic_filter_afterlife_alcatraz` (fxc
//! disassembly of its technique): two copies of a smoke mask
//! (`zm_afterlife_alcatraz_vignette_cockup`) turning and shrinking in turn,
//! times a tiled noise, lit blue towards the screen's edge, added to the
//! frame; the middle of the frame pushed out a little (its wave warp). The
//! numbers are the material's own constants.

use bevy::core_pipeline::{Core3d, Core3dSystems};
use bevy::prelude::*;
use bevy::render::RenderStartup;
use bevy::render::render_asset::RenderAssets;
use bevy::render::render_resource::binding_types::{
    sampler, texture_2d, uniform_buffer_sized,
};
use bevy::render::render_resource::{
    AddressMode, BindGroupEntries, BindGroupLayoutDescriptor, BindGroupLayoutEntries, Buffer,
    BufferDescriptor, BufferUsages, CachedRenderPipelineId, ColorTargetState, ColorWrites,
    FilterMode, FragmentState, LoadOp, Operations, PipelineCache, RenderPassColorAttachment,
    RenderPassDescriptor, RenderPipelineDescriptor, Sampler, SamplerBindingType,
    SamplerDescriptor, ShaderStages, StoreOp, TextureFormat, TextureSampleType, VertexState,
};
use bevy::render::renderer::{RenderContext, RenderDevice, RenderQueue, ViewQuery};
use bevy::render::texture::GpuImage;
use bevy::render::view::ViewTarget;
use bevy::shader::Shader;
use std::collections::HashMap;
use std::num::NonZeroU64;

const SHADER_SOURCE_PATH: &str = "embedded://render_gpu/surf_gpu/bo2_afterlife.wgsl";
const PARAMS_BYTES: u64 = 64;

/// The map's filters as images the GPU can take, by material name, with
/// their constants: (soft, boost, mask soft, warp), vignette colour,
/// (vignette radii, mask radii), (scroll speed).
#[derive(Resource, Default)]
struct Bo2FilterImages {
    source: usize,
    screen_filters: Vec<(String, Handle<Image>, Handle<Image>, [[f32; 4]; 4])>,
}

fn upload_screen_filters(
    filters: Option<Res<assets::T6ScreenFilters>>,
    mut gpu_imgs: ResMut<Assets<Image>>,
    mut out: ResMut<Bo2FilterImages>,
) {
    let key = filters.as_ref().map_or(0, |f| f.0.as_ptr() as usize);
    if out.source == key {
        return;
    }
    out.source = key;
    out.screen_filters.clear();
    let Some(filters) = filters else {
        return;
    };
    for f in &filters.0 {
        let (Some(noise), Some(mask)) = (f.image("noise"), f.image("cockup")) else {
            continue;
        };
        let c = |name: &str, default: [f32; 4]| f.constant(name).unwrap_or(default);
        let soft = c("VignetteSoft", [2.0; 4])[0];
        let boost = c("Boost", [3.0; 4])[0];
        let colour = c("VignetteColo", [0.43, 0.5, 0.5, 1.0]);
        let mask_soft = c("MaskSoftness", [2.0; 4])[0];
        let warp = c("WaveWarpScal", [-0.22; 4])[0];
        let vr = c("VignetteRadi", [0.42, 1.0, 0.0, 0.0]);
        let mr = c("MaskRadius", [0.2, 0.8, 0.0, 0.0]);
        let scroll = c("ScrollSpeed", [-0.4; 4])[0];
        let [r, g, b, _] = colour;
        let filter_params = [[soft, boost, mask_soft, warp], [r, g, b, 0.0], [vr[0], vr[1], mr[0], mr[1]], [scroll, 0.0, 0.0, 0.0]];
        diag::info!(World, "bo2zm screen filter {} ready {filter_params:?}", f.material);
        let (noise_handle, mask_handle) = (gpu_imgs.add((**noise).clone()), gpu_imgs.add((**mask).clone()));
        out.screen_filters.push((f.material.clone(), noise_handle, mask_handle, filter_params));
    }
}

/// This frame's filter in the render world: how far in, its pictures, its
/// constants and the time.
#[derive(Resource, Default)]
struct Bo2FilterFrame {
    strength: f32, noise_img: Option<AssetId<Image>>, mask: Option<AssetId<Image>>,
    params: [[f32; 4]; 4], time: f32,
}

fn extract_filter(
    (wanted, gpu_imgs, time): (
        bevy::render::Extract<Option<Res<frame::ScriptScreenFilter>>>,
        bevy::render::Extract<Res<Bo2FilterImages>>,
        bevy::render::Extract<Res<Time>>,
    ),
    mut out: ResMut<Bo2FilterFrame>,
) {
    *out = Bo2FilterFrame::default();
    let Some(wanted) = wanted.as_ref() else {
        return;
    };
    let Some(material) = wanted.material.as_deref() else {
        return;
    };
    if wanted.amount <= 0.0 {
        return;
    }
    let Some(filter) = gpu_imgs.screen_filters.iter().find(|f| f.0 == material) else {
        return;
    };
    let (_, noise, mask, params) = filter;
    out.strength = wanted.amount.clamp(0.0, 1.0);
    out.noise_img = Some(noise.id());
    out.mask = Some(mask.id());
    out.params = *params;
    // Kept small so the turning stays smooth over a long game.
    out.time = (time.elapsed_secs_f64() % 10_000.0) as f32;
}

#[derive(Resource)]
struct Bo2Afterlife {
    bgl_desc: BindGroupLayoutDescriptor, program: Handle<Shader>, samplers: Option<(Sampler, Sampler)>,
    params: Option<Buffer>,
    pipelines: HashMap<TextureFormat, CachedRenderPipelineId>,
}

fn init(mut commands: Commands, asset_srv: Res<AssetServer>) {
    let tex = || texture_2d(TextureSampleType::Float { filterable: true });
    commands.insert_resource(Bo2Afterlife {
        bgl_desc: BindGroupLayoutDescriptor::new(
            "bo2zm_afterlife",
            &BindGroupLayoutEntries::with_indices(ShaderStages::FRAGMENT, (
                    (0, tex()),
                    (1, sampler(SamplerBindingType::Filtering)),
                    (2, uniform_buffer_sized(false, NonZeroU64::new(PARAMS_BYTES))),
                    (3, tex()),
                    (4, tex()),
                    (5, sampler(SamplerBindingType::Filtering)),
            )),
        ),
        program: asset_srv.load(SHADER_SOURCE_PATH),
        samplers: None,
        params: None,
        pipelines: HashMap::new(),
    });
}

fn draw_bo2_afterlife(
    view: ViewQuery<&ViewTarget>,
    enabled: Res<super::bo2_bloom::Bo2BloomEnabled>,
    frame: Res<Bo2FilterFrame>, gpu_imgs: Res<RenderAssets<GpuImage>>,
    filter: Option<ResMut<Bo2Afterlife>>,
    pipes: Res<PipelineCache>,
    gpu_dev: Res<RenderDevice>,
    gpu_queue: Res<RenderQueue>, mut pass_ctx: RenderContext,
) {
    if !enabled.0 || frame.strength <= 0.0 || !crate::surf_gpu::geometry_inspect_enabled() {
        return;
    }
    let Some(mut filter) = filter else {
        return;
    };
    let (Some(noise), Some(mask)) = (
        frame.noise_img.and_then(|id| gpu_imgs.get(id)),
        frame.mask.and_then(|id| gpu_imgs.get(id)),
    ) else {
        return;
    };
    let target = view.into_inner();
    let colour_format = target.main_texture_format();
    if !filter.pipelines.contains_key(&colour_format) {
        let d = RenderPipelineDescriptor {
            label: Some("bo2zm_afterlife".into()),
            layout: vec![filter.bgl_desc.clone()],
            immediate_size: 0,
            vertex: VertexState { shader: filter.program.clone(), shader_defs: Vec::new(), entry_point: Some("vertex".into()), buffers: Vec::new() },
            fragment: Some(FragmentState {
                shader: filter.program.clone(), shader_defs: Vec::new(), entry_point: Some("fragment".into()),
                targets: vec![Some(ColorTargetState { format: colour_format, blend: None, write_mask: ColorWrites::ALL })],
            }),
            primitive: Default::default(), depth_stencil: None,
            multisample: Default::default(),
            zero_initialize_workgroup_memory: false,
        };
        let id = pipes.queue_render_pipeline(d);
        filter.pipelines.insert(colour_format, id);
    }
    let Some(pipe) = filter.pipelines.get(&colour_format).and_then(|&id| pipes.get_render_pipeline(id))
    else {
        return;
    };
    let (clamp, repeat) = filter
        .samplers
        .get_or_insert_with(|| {
            let clamp = gpu_dev.create_sampler(&SamplerDescriptor {
                label: Some("bo2zm_afterlife_scene"),
                mag_filter: FilterMode::Linear,
                min_filter: FilterMode::Linear,
                ..Default::default()
            });
            let repeat = gpu_dev.create_sampler(&SamplerDescriptor {
                label: Some("bo2zm_afterlife_tiles"),
                address_mode_u: AddressMode::Repeat,
                address_mode_v: AddressMode::Repeat,
                mag_filter: FilterMode::Linear,
                min_filter: FilterMode::Linear,
                mipmap_filter: bevy::render::render_resource::MipmapFilterMode::Linear,
                ..Default::default()
            });
            (clamp, repeat)
        })
        .clone();
    let params = filter
        .params
        .get_or_insert_with(|| {
            let usage = BufferUsages::UNIFORM | BufferUsages::COPY_DST;
            let buf_desc = BufferDescriptor { label: Some("bo2zm_afterlife_params"), size: PARAMS_BYTES, usage, mapped_at_creation: false };
            gpu_dev.create_buffer(&buf_desc)
        })
        .clone();
    let mut data = frame.params;
    data[1][3] = frame.strength;
    data[3][1] = frame.time;
    gpu_queue.write_buffer(&params, 0, bytemuck::cast_slice(&data));
    let layout = pipes.get_bind_group_layout(&filter.bgl_desc);
    let pingpong = target.post_process_write();
    let bind_grp = gpu_dev.create_bind_group(
        "bo2zm_afterlife",
        &layout,
        &BindGroupEntries::with_indices((
            (0, pingpong.source),
            (1, &clamp),
            (2, params.as_entire_binding()),
            (3, &noise.texture_view),
            (4, &mask.texture_view),
            (5, &repeat),
        )),
    );
    let mut rp = pass_ctx.command_encoder().begin_render_pass(&RenderPassDescriptor {
        label: Some("bo2zm_afterlife"),
        color_attachments: &[Some(RenderPassColorAttachment {
            view: pingpong.destination,
            resolve_target: None, ops: Operations { load: LoadOp::Clear(LinearRgba::BLACK.into()), store: StoreOp::Store },
            depth_slice: None,
        })],
        depth_stencil_attachment: None, timestamp_writes: None,
        occlusion_query_set: None,
        multiview_mask: None,
    });
    rp.set_pipeline(pipe);
    rp.set_bind_group(0, &bind_grp, &[]);
    rp.draw(0..3, 0..1); // one triangle covering the screen
}

pub(super) fn register(host: &mut App) {
    bevy::asset::embedded_asset!(host, "bo2_afterlife.wgsl");
    host.init_resource::<Bo2FilterImages>()
        .add_systems(Update, upload_screen_filters);
    let Some(render_sub) = host.get_sub_app_mut(bevy::render::RenderApp) else {
        return;
    };
    render_sub
        .init_resource::<Bo2FilterFrame>()
        .add_systems(RenderStartup, init)
        .add_systems(bevy::render::ExtractSchedule, extract_filter)
        .add_systems(
            Core3d,
            draw_bo2_afterlife
                .in_set(Core3dSystems::PostProcess).after(super::bo2_bloom::Bo2BloomSet)
                .before(super::postfx_frame::PostEffectSet),
        );
}
