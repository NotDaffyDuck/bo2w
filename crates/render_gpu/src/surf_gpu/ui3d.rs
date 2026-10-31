//! bo2mp: Black Ops II's UI3D texture, the 512 x 512 picture some sights'
//! glass shows (the Target Finder's screen: code texture UI3D 0x21, read
//! through window 5, the whole 0..1 rect). The main world fills [`Ui3dFrame`]
//! with the screen's quads in texture pixels; here they are drawn into the
//! texture before the view, and the colour pass binds its read view where a
//! material samples UI3D (`install_camera_frame_targets`).
//!
//! The texture is sRGB with a plain-bytes read view (as `portal_view`): a
//! quad drawn through the sRGB view from an sRGB atlas keeps the atlas's
//! bytes, and the glass's shader reads those bytes as its other textures.
use std::num::NonZeroU64;

use bevy::mesh::VertexBufferLayout;
use bevy::prelude::*;
use bevy::render::render_asset::RenderAssets;
use bevy::render::render_resource::binding_types::{sampler, texture_2d, uniform_buffer_sized};
use bevy::render::render_resource::{
    BindGroupEntries, BindGroupLayoutDescriptor, BindGroupLayoutEntries, BlendComponent,
    BlendFactor, BlendOperation, BlendState, Buffer, BufferInitDescriptor, BufferUsages,
    CachedRenderPipelineId, ColorTargetState, ColorWrites, CommandEncoderDescriptor, Extent3d,
    FragmentState, FrontFace, LoadOp, Operations, Origin3d, PipelineCache, PrimitiveState,
    RenderPassColorAttachment, RenderPassDescriptor, RenderPipelineDescriptor, Sampler,
    SamplerBindingType, SamplerDescriptor, ShaderStages, StoreOp, TexelCopyBufferLayout,
    TexelCopyTextureInfo, Texture, TextureAspect, TextureDescriptor, TextureDimension,
    TextureFormat, TextureSampleType, TextureUsages, TextureView, TextureViewDescriptor,
    VertexAttribute, VertexFormat, VertexState, VertexStepMode,
};
use bevy::render::renderer::{PendingCommandBuffers, RenderDevice, RenderQueue};
use bevy::render::texture::GpuImage;
use bevy::render::{Extract, ExtractSchedule, Render, RenderApp, RenderStartup, RenderSystems};
use bevy::shader::Shader;
use hud_t6::set_2d::{cmd_buf_set_2d_projection, set_2d_clip_coeffs};

/// The texture's side in pixels (BO2's UI3D window space).
pub const UI3D_SIZE: u32 = 512;

/// The HUD quad shader (registered by `hud_quads`): pixel position through the
/// 2D projection, texture times vertex colour.
const SHADER_SOURCE_PATH: &str = "embedded://render_gpu/surf_gpu/hud_quads.wgsl";
const PARAM_BLOCK_BYTES: u64 = 16;
const DRAWN_FORMAT: TextureFormat = TextureFormat::Rgba8UnormSrgb;
const READ_FORMAT: TextureFormat = TextureFormat::Rgba8Unorm;

/// One quad of the picture: corners in texture pixels and their atlas UVs.
/// `textured` off: a black quad (the screen's cleared strip).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Ui3dQuad {
    pub corner_xy: [[f32; 2]; 4],
    pub uv: [[f32; 2]; 4],
    pub textured: bool,
}

/// This frame's picture: its quads in draw order and the atlas the textured
/// ones read. No quads: the texture stays cleared.
#[derive(Resource, Clone, Debug, Default, PartialEq)]
pub struct Ui3dFrame {
    pub quads: Vec<Ui3dQuad>,
    pub atlas: Option<Handle<Image>>,
}

/// The texture: the view the pass draws into and the view materials read.
#[derive(Resource)]
pub struct Ui3dTargets {
    _tex_hold: Texture,
    drawn: TextureView,
    read: TextureView,
}

impl Ui3dTargets {
}

#[derive(Resource, Default)]
struct ExtractedUi3d(Ui3dFrame);

#[derive(Resource)]
struct Ui3dGpu {
    program: Handle<Shader>,
    layout: BindGroupLayoutDescriptor,
    filter_params: Buffer,
    filter_samp: Sampler,
    white: TextureView,
    /// By the format drawn into: sRGB for an sRGB atlas, plain otherwise.
    pipelines: Vec<(TextureFormat, CachedRenderPipelineId)>,
    /// The frame now in the texture; `None` until the first draw.
    drawn: Option<Ui3dFrame>,
}

#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
#[repr(C)]
struct Ui3dVertex {
    xyzw: [f32; 4],
    tint: [u8; 4],
    uv: [f32; 2],
    _pad: [u32; 1],
}

pub(super) fn register(host: &mut App) {
    host.init_resource::<Ui3dFrame>();
    let Some(render_sub) = host.get_sub_app_mut(RenderApp) else {
        return;
    };
    render_sub
        .init_resource::<ExtractedUi3d>()
        .add_systems(RenderStartup, init)
        .add_systems(ExtractSchedule, extract_ui3d)
        .add_systems(Render, draw_ui3d.in_set(RenderSystems::PrepareResources));
}

fn init(
    mut commands: Commands,
    asset_srv: Res<AssetServer>,
    gpu_dev: Res<RenderDevice>,
    gpu_queue: Res<RenderQueue>,
) {
    let texture = gpu_dev.create_texture(&TextureDescriptor {
        label: Some("bo2mp_ui3d"),
        size: Extent3d { width: UI3D_SIZE, height: UI3D_SIZE, depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: TextureDimension::D2,
        format: DRAWN_FORMAT,
        usage: TextureUsages::RENDER_ATTACHMENT | TextureUsages::TEXTURE_BINDING,
        view_formats: &[READ_FORMAT],
    });
    let drawn = texture.create_view(&TextureViewDescriptor::default());
    let read = texture.create_view(&TextureViewDescriptor { format: Some(READ_FORMAT), ..default() });
    commands.insert_resource(Ui3dTargets { _tex_hold: texture, drawn, read });

    let white = gpu_dev.create_texture(&TextureDescriptor {
        label: Some("bo2mp_ui3d_white"),
        size: Extent3d { width: 1, height: 1, depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: TextureDimension::D2,
        format: READ_FORMAT,
        usage: TextureUsages::TEXTURE_BINDING | TextureUsages::COPY_DST,
        view_formats: &[],
    });
    gpu_queue.write_texture(
        TexelCopyTextureInfo {
            texture: &white,
            mip_level: 0,
            origin: Origin3d::ZERO,
            aspect: TextureAspect::All,
        },
        &[255; 4],
        TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(4), rows_per_image: Some(1) },
        Extent3d { width: 1, height: 1, depth_or_array_layers: 1 },
    );
    let coeffs = cmd_buf_set_2d_projection(UI3D_SIZE as i32, UI3D_SIZE as i32)
        .map(|projection| set_2d_clip_coeffs(&projection))
        .unwrap_or_default();
    let params = gpu_dev.create_buffer_with_data(&BufferInitDescriptor {
        label: Some("bo2mp_ui3d_params"),
        contents: bytemuck::bytes_of(&coeffs),
        usage: BufferUsages::UNIFORM,
    });
    commands.insert_resource(Ui3dGpu {
        program: asset_srv.load(SHADER_SOURCE_PATH),
        layout: BindGroupLayoutDescriptor::new(
            "bo2mp_ui3d_layout",
            &BindGroupLayoutEntries::sequential(
                ShaderStages::VERTEX_FRAGMENT,
                (
                    uniform_buffer_sized(false, NonZeroU64::new(PARAM_BLOCK_BYTES)),
                    texture_2d(TextureSampleType::Float { filterable: true }),
                    sampler(SamplerBindingType::Filtering),
                ),
            ),
        ),
        filter_params: params,
        filter_samp: gpu_dev.create_sampler(&SamplerDescriptor {
            label: Some("bo2mp_ui3d_sampler"),
            mag_filter: bevy::render::render_resource::FilterMode::Linear,
            min_filter: bevy::render::render_resource::FilterMode::Linear,
            ..default()
        }),
        white: white.create_view(&TextureViewDescriptor::default()),
        pipelines: Vec::new(),
        drawn: None,
    });
}

fn extract_ui3d(mut pulled: ResMut<ExtractedUi3d>, out_frame: Extract<Res<Ui3dFrame>>) {
    if pulled.0 != **out_frame {
        pulled.0 = (**out_frame).clone();
    }
}

fn pipeline(gpu: &Ui3dGpu, format: TextureFormat) -> RenderPipelineDescriptor {
    let mix = BlendComponent {
        src_factor: BlendFactor::SrcAlpha,
        dst_factor: BlendFactor::OneMinusSrcAlpha,
        operation: BlendOperation::Add,
    };
    RenderPipelineDescriptor {
        label: Some("bo2mp_ui3d".into()),
        layout: vec![gpu.layout.clone()],
        immediate_size: 0,
        vertex: VertexState {
            shader: gpu.program.clone(),
            shader_defs: Vec::new(),
            entry_point: Some("quad_vertex".into()),
            buffers: vec![VertexBufferLayout {
                array_stride: core::mem::size_of::<Ui3dVertex>() as u64,
                step_mode: VertexStepMode::Vertex,
                attributes: vec![
                    VertexAttribute { format: VertexFormat::Float32x4, offset: 0, shader_location: 0 },
                    VertexAttribute { format: VertexFormat::Unorm8x4, offset: 16, shader_location: 1 },
                    VertexAttribute { format: VertexFormat::Float32x2, offset: 20, shader_location: 2 },
                ],
            }],
        },
        fragment: Some(FragmentState {
            shader: gpu.program.clone(),
            shader_defs: Vec::new(),
            entry_point: Some("quad_fragment".into()),
            targets: vec![Some(ColorTargetState {
                format,
                blend: Some(BlendState { color: mix, alpha: mix }),
                write_mask: ColorWrites::ALL,
            })],
        }),
        primitive: PrimitiveState { front_face: FrontFace::Ccw, cull_mode: None, ..default() },
        depth_stencil: None,
        multisample: default(),
        zero_initialize_workgroup_memory: false,
    }
}

/// Two triangles per quad, corners 0-1-2 and 0-2-3; colour bytes in the
/// shader's BGRA order.
fn quad_vertices(quads: &[Ui3dQuad], textured: bool) -> Vec<Ui3dVertex> {
    let color = if textured { [255; 4] } else { [0, 0, 0, 255] };
    quads
        .iter()
        .filter(|q| q.textured == textured)
        .flat_map(|q| {
            [0, 1, 2, 0, 2, 3].map(|i| Ui3dVertex {
                xyzw: [q.corner_xy[i][0], q.corner_xy[i][1], 0.0, 1.0],
                tint: color,
                uv: q.uv[i],
                _pad: [0],
            })
        })
        .collect()
}

#[allow(clippy::too_many_arguments)]
fn draw_ui3d(
    extracted: Res<ExtractedUi3d>,
    targets: Option<Res<Ui3dTargets>>,
    gpu: Option<ResMut<Ui3dGpu>>,
    gpu_dev: Res<RenderDevice>,
    pipes: Res<PipelineCache>,
    gpu_imgs: Res<RenderAssets<GpuImage>>,
    mut pending: ResMut<PendingCommandBuffers>,
) {
    let (Some(targets), Some(mut gpu)) = (targets, gpu) else {
        return;
    };
    let frame = &extracted.0;
    if gpu.drawn.as_ref() == Some(frame) {
        return;
    }
    let atlas = frame.atlas.as_ref().and_then(|handle| gpu_imgs.get(handle));
    let wants_atlas = frame.quads.iter().any(|q| q.textured);
    if wants_atlas && atlas.is_none() {
        return;
    }
    // Drawn through the view whose encoding matches the atlas's, so its
    // bytes land unchanged.
    let format = match atlas {
        Some(image) if !image.view_format().is_srgb() => READ_FORMAT,
        _ => DRAWN_FORMAT,
    };
    let id = match gpu.pipelines.iter().find(|(f, _)| *f == format) {
        Some(&(_, id)) => id,
        None => {
            let id = pipes.queue_render_pipeline(pipeline(&gpu, format));
            gpu.pipelines.push((format, id));
            id
        }
    };
    let Some(render_pipeline) = pipes.get_render_pipeline(id) else {
        return;
    };
    let layout = pipes.get_bind_group_layout(&gpu.layout);
    let bind_grp = |view: &TextureView| {
        gpu_dev.create_bind_group(
            "bo2mp_ui3d",
            &layout,
            &BindGroupEntries::sequential((gpu.filter_params.as_entire_buffer_binding(), view, &gpu.filter_samp)),
        )
    };
    // The strip first (BO2 clears it, then draws the lists over it).
    let batches = [(false, Some(&gpu.white)), (true, atlas.map(|a| &a.texture_view))]
        .into_iter()
        .filter_map(|(textured, view)| {
            let view = view?;
            let vertices = quad_vertices(&frame.quads, textured);
            (!vertices.is_empty()).then(|| {
                let buffer = gpu_dev.create_buffer_with_data(&BufferInitDescriptor {
                    label: Some("bo2mp_ui3d_quads"),
                    contents: bytemuck::cast_slice(&vertices),
                    usage: BufferUsages::VERTEX,
                });
                (buffer, vertices.len() as u32, bind_grp(view))
            })
        })
        .collect::<Vec<_>>();
    let draw_view = if format == READ_FORMAT { &targets.read } else { &targets.drawn };
    let mut enc = gpu_dev.create_command_encoder(&CommandEncoderDescriptor { label: Some("bo2mp_ui3d") });
    {
        let mut rp = enc.begin_render_pass(&RenderPassDescriptor {
            label: Some("bo2mp_ui3d"),
            color_attachments: &[Some(RenderPassColorAttachment {
                view: draw_view,
                resolve_target: None,
                // TODO(fact): BO2's UI3D clear colour is not measured; transparent black for now.
                ops: Operations { load: LoadOp::Clear(LinearRgba::NONE.into()), store: StoreOp::Store },
                depth_slice: None,
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        rp.set_pipeline(render_pipeline);
        for (buffer, ix_count, bind_grp) in &batches {
            rp.set_bind_group(0, bind_grp, &[]);
            rp.set_vertex_buffer(0, *buffer.slice(..));
            rp.draw(0..*ix_count, 0..1);
        }
    }
    pending.push([enc.finish()]);
    gpu.drawn = Some(frame.clone());
}
