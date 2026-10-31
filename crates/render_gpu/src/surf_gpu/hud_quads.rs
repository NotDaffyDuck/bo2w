//! The 2D HUD pass: stretch-pics, text and saved-screen quads, drawn over the
//! finished 3D picture (after post effects, never into a portal view).
//!
//! The HUD hands over a whole frame of quads at once, so the pass ships that frame
//! to the GPU as one vertex upload and one index upload. Each batch then draws its
//! own index range; batch indices count from the batch's first corner, which
//! becomes the draw's vertex base.

use std::borrow::Cow;
use std::mem::offset_of;
use std::num::NonZeroU64;
use std::ops::Range;

use bevy::core_pipeline::{Core3d, Core3dSystems};
use bevy::mesh::VertexBufferLayout;
use bevy::prelude::*;
use bevy::render::render_asset::RenderAssets;
use bevy::render::render_resource::binding_types::{sampler, texture_2d, uniform_buffer_sized};
use bevy::render::render_resource::{
    BindGroupEntries, BindGroupLayoutDescriptor, BindGroupLayoutEntries, BlendComponent,
    BlendFactor, BlendOperation, BlendState, Buffer, BufferDescriptor, BufferUsages,
    ColorTargetState, ColorWrites, Extent3d, FragmentState, IndexFormat, MultisampleState,
    PipelineCache, PrimitiveState, RenderPassDescriptor, RenderPipelineDescriptor, Sampler,
    SamplerBindingType, SamplerDescriptor, ShaderStages, SpecializedRenderPipeline,
    SpecializedRenderPipelines, Texture, TextureDescriptor, TextureDimension, TextureFormat,
    TextureSampleType, TextureUsages, TextureView, TextureViewDescriptor, VertexAttribute,
    VertexFormat, VertexState, VertexStepMode,
};
use bevy::render::renderer::{RenderContext, RenderDevice, RenderQueue, ViewQuery};
use bevy::render::texture::GpuImage;
use bevy::render::view::{ExtractedView, ViewTarget};
use bevy::render::{Extract, ExtractSchedule, Render, RenderApp, RenderStartup, RenderSystems};
use bevy::shader::Shader;
use hud::{HudStitchBatch, OverlayMeshBatch, HudStitchMethod};
use hud_t6::set_2d::{cmd_buf_set_2d_projection, set_2d_clip_coeffs};

use super::portal_view::PortalCamera;

const SHADER_SOURCE_PATH: &str = "embedded://render_gpu/surf_gpu/hud_quads.wgsl";
/// The screen uniform: 2D scale and offset, four floats.
const SCREEN_UNIFORM_BYTES: u64 = 16;
/// Opening GPU buffer sizes; a frame that needs more grows them.
const VERTEX_BUFFER_FLOOR: u64 = 1 << 20;
const INDEX_BUFFER_FLOOR: u64 = 2 << 20;

/// Batches without material state blend by source alpha, alpha channel included.
const SOURCE_ALPHA_OVER: BlendComponent = BlendComponent {
    src_factor: BlendFactor::SrcAlpha,
    dst_factor: BlendFactor::OneMinusSrcAlpha,
    operation: BlendOperation::Add,
};

/// One HUD corner as the GPU reads it. `normal` rides along unread by the shader.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
struct QuadCorner {
    pos: [f32; 4],
    bgra: [u8; 4],
    uv: [f32; 2],
    nrm: u32,
}

fn corner_layout() -> VertexBufferLayout {
    let attribute = |format, at_byte: usize, shader_location| VertexAttribute {
        format,
        offset: at_byte as u64,
        shader_location,
    };
    VertexBufferLayout {
        array_stride: size_of::<QuadCorner>() as u64,
        step_mode: VertexStepMode::Vertex,
        attributes: vec![
            attribute(VertexFormat::Float32x4, offset_of!(QuadCorner, pos), 0),
            attribute(VertexFormat::Unorm8x4, offset_of!(QuadCorner, bgra), 1),
            attribute(VertexFormat::Float32x2, offset_of!(QuadCorner, uv), 2),
        ],
    }
}

/// The HUD frame as the render world keeps it.
#[derive(Resource, Default)]
struct HudFrame {
    corners: Vec<QuadCorner>,
    idx_list: Vec<u16>,
    batches: Vec<HudStitchBatch>,
    surf: [f32; 2],
    vis_q: bool,
    saved_screen_sequence: u64,
}

fn extract_hud_frame(mut frame: ResMut<HudFrame>, hud: Extract<Res<OverlayMeshBatch>>) {
    frame.corners.clear();
    frame.corners.extend(hud.vertices.iter().map(|v| QuadCorner {
        pos: v.clip_pos,
        bgra: v.blue_green_red_alpha,
        uv: v.tex_st,
        nrm: v.squeezed_facing,
    }));
    frame.idx_list.clone_from(&hud.indices);
    frame.batches.clone_from(&hud.batches);
    frame.surf = [hud.canvas_w, hud.canvas_h];
    frame.vis_q = hud.visible;
    frame.saved_screen_sequence = hud.saved_screen_sequence;
}

/// One batch cleared to draw.
#[derive(Clone, Debug, PartialEq, Eq)]
struct BatchDraw {
    batch: usize,
    indices: Range<u32>,
    vertex_base: i32,
}

/// The batches that can draw, in order. A batch is left out when it has no corners
/// or no indices, ends on a partial triangle, or reaches past the frame's data.
fn plan_batches(corner_len: usize, index_len: usize, batches: &[HudStitchBatch]) -> Vec<BatchDraw> {
    let inside = |first: u32, ix_count: u32, len: usize| {
        (first as usize)
            .checked_add(ix_count as usize)
            .is_some_and(|end| end <= len)
    };
    batches
        .iter()
        .enumerate()
        .filter_map(|(batch, b)| {
            if b.vertex_count == 0 || b.index_count == 0 || b.index_count % 3 != 0 {
                return None;
            }
            if !inside(b.vertex_base, b.vertex_count, corner_len)
                || !inside(b.first_index, b.index_count, index_len)
            {
                return None;
            }
            Some(BatchDraw {
                batch,
                indices: b.first_index..b.first_index.checked_add(b.index_count)?,
                vertex_base: i32::try_from(b.vertex_base).ok()?,
            })
        })
        .collect()
}

/// Queue writes move whole 4-byte words, so an odd index count gets one zero index
/// of padding.
fn index_bytes(tri_idx: &[u16]) -> Cow<'_, [u8]> {
    let bytes: &[u8] = bytemuck::cast_slice(tri_idx);
    if bytes.len() % 4 == 0 {
        Cow::Borrowed(bytes)
    } else {
        let mut padded = bytes.to_vec();
        padded.extend_from_slice(&[0, 0]);
        Cow::Owned(padded)
    }
}

/// The size a buffer of `have` bytes must grow to so `need` bytes fit, or None when
/// they already do.
fn grown_size(have: u64, need: u64, floor: u64) -> Option<u64> {
    (need > have).then(|| need.next_power_of_two().max(floor))
}

/// A GPU buffer rewritten from its start every frame.
struct FrameBuffer {
    buf_name: &'static str,
    buf_flags: BufferUsages,
    floor: u64,
    gpu: Option<Buffer>,
}

impl FrameBuffer {
    fn new(buf_name: &'static str, buf_flags: BufferUsages, floor: u64) -> Self {
        Self { buf_name, buf_flags, floor, gpu: None }
    }

    fn upload_bytes(&mut self, gpu_dev: &RenderDevice, gpu_queue: &RenderQueue, data: &[u8]) {
        if data.is_empty() {
            return;
        }
        let have = self.gpu.as_ref().map_or(0, |gpu| gpu.size());
        if let Some(size) = grown_size(have, data.len() as u64, self.floor) {
            self.gpu = Some(gpu_dev.create_buffer(&BufferDescriptor {
                label: Some(self.buf_name),
                size,
                usage: self.buf_flags | BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }));
        }
        if let Some(gpu) = &self.gpu {
            gpu_queue.write_buffer(gpu, 0, data);
        }
    }
}

#[derive(Resource)]
struct HudGpu {
    corner_buf: FrameBuffer,
    indices: FrameBuffer,
    draw_list: Vec<BatchDraw>,
}

impl Default for HudGpu {
    fn default() -> Self {
        Self {
            corner_buf: FrameBuffer::new("hud_quads_corners", BufferUsages::VERTEX, VERTEX_BUFFER_FLOOR),
            indices: FrameBuffer::new("hud_quads_indices", BufferUsages::INDEX, INDEX_BUFFER_FLOOR),
            draw_list: Vec::new(),
        }
    }
}

/// The frozen picture a saved-screen batch shows, copied from the scene when the
/// HUD starts a new saved screen.
struct SavedScreenTexture {
    texture: Texture,
    cam_view: TextureView,
    filter_samp: Sampler,
    format: TextureFormat,
    size: Extent3d,
}

impl SavedScreenTexture {
    fn can_hold(&self, format: TextureFormat, size: Extent3d) -> bool {
        self.format == format && self.size == size
    }
}

#[derive(Resource, Default)]
struct SavedScreen {
    texture: Option<SavedScreenTexture>,
    seq_no: u64,
    is_frozen: bool,
}

#[derive(Resource)]
struct HudPipeline {
    program: Handle<Shader>,
    layout: BindGroupLayoutDescriptor,
    screen: Buffer,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct HudPipelineKey {
    out_tgt: TextureFormat,
    state_words: Option<[u32; 2]>,
}

impl SpecializedRenderPipeline for HudPipeline {
    type Key = HudPipelineKey;

    fn specialize(&self, key: Self::Key) -> RenderPipelineDescriptor {
        // Material state word 0 holds the blend and the colour-write mask.
        let (blend, write_mask) = match key.state_words.map(|bits| bits[0]) {
            Some(dword0) => (
                super::blend_state::blend_state_from_word0(dword0, false),
                super::blend_state::colour_writes_from_word0(dword0),
            ),
            None => (
                Some(BlendState { color: SOURCE_ALPHA_OVER, alpha: SOURCE_ALPHA_OVER }),
                ColorWrites::ALL,
            ),
        };
        RenderPipelineDescriptor {
            label: Some("hud_quads".into()),
            layout: vec![self.layout.clone()],
            immediate_size: 0,
            vertex: VertexState {
                shader: self.program.clone(),
                shader_defs: Vec::new(),
                entry_point: Some("quad_vertex".into()),
                buffers: vec![corner_layout()],
            },
            fragment: Some(FragmentState {
                shader: self.program.clone(),
                shader_defs: Vec::new(),
                entry_point: Some("quad_fragment".into()),
                targets: vec![Some(ColorTargetState { format: key.out_tgt, blend, write_mask })],
            }),
            primitive: PrimitiveState::default(),
            depth_stencil: None,
            multisample: MultisampleState::default(),
            zero_initialize_workgroup_memory: false,
        }
    }
}

fn init_hud_pipeline(mut commands: Commands, assets: Res<AssetServer>, gpu_dev: Res<RenderDevice>) {
    let screen = gpu_dev.create_buffer(&BufferDescriptor {
        label: Some("hud_quads_screen"),
        size: SCREEN_UNIFORM_BYTES,
        usage: BufferUsages::UNIFORM | BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let entries = BindGroupLayoutEntries::sequential(
        ShaderStages::VERTEX_FRAGMENT,
        (
            uniform_buffer_sized(false, NonZeroU64::new(SCREEN_UNIFORM_BYTES)),
            texture_2d(TextureSampleType::Float { filterable: true }),
            sampler(SamplerBindingType::Filtering),
        ),
    );
    commands.insert_resource(HudPipeline {
        program: assets.load(SHADER_SOURCE_PATH),
        layout: BindGroupLayoutDescriptor::new("hud_quads_layout", &entries),
        screen,
    });
}

fn prepare_hud_quads(
    frame: Res<HudFrame>,
    mut gpu: ResMut<HudGpu>,
    pipeline: Res<HudPipeline>,
    gpu_dev: Res<RenderDevice>,
    gpu_queue: Res<RenderQueue>,
) {
    gpu.draw_list.clear();
    if !frame.vis_q {
        return;
    }
    gpu.draw_list = plan_batches(frame.corners.len(), frame.idx_list.len(), &frame.batches);
    if !gpu.draw_list.is_empty() {
        gpu.corner_buf.upload_bytes(&gpu_dev, &gpu_queue, bytemuck::cast_slice(&frame.corners));
        gpu.indices.upload_bytes(&gpu_dev, &gpu_queue, &index_bytes(&frame.idx_list));
    }
    let [w, h] = frame.surf;
    if let Some(projection) = cmd_buf_set_2d_projection(w as i32, h as i32) {
        gpu_queue.write_buffer(&pipeline.screen, 0, bytemuck::bytes_of(&set_2d_clip_coeffs(&projection)));
    }
}

/// Keeps a saved-screen texture matching the main view, and warms the plain HUD
/// pipeline for that view's format so the first HUD frame can draw.
fn prepare_saved_screen(
    view_rows: Query<&ViewTarget, (With<Camera3d>, Without<PortalCamera>)>,
    gpu_dev: Res<RenderDevice>,
    pipes: Res<PipelineCache>,
    pipeline: Res<HudPipeline>,
    mut specialized: ResMut<SpecializedRenderPipelines<HudPipeline>>,
    mut saved: ResMut<SavedScreen>,
) {
    let Some(cam_target) = view_rows.iter().next() else {
        return;
    };
    let format = cam_target.main_texture_format();
    let size = cam_target.main_texture().size();
    specialized.specialize(&pipes, &pipeline, HudPipelineKey { out_tgt: format, state_words: None });
    if saved.texture.as_ref().is_some_and(|t| t.can_hold(format, size)) {
        return;
    }
    let tex = gpu_dev.create_texture(&TextureDescriptor {
        label: Some("hud_quads_saved_screen"),
        size,
        mip_level_count: 1,
        sample_count: 1,
        dimension: TextureDimension::D2,
        format,
        usage: TextureUsages::TEXTURE_BINDING | TextureUsages::COPY_DST,
        view_formats: &[],
    });
    let tex_view = tex.create_view(&TextureViewDescriptor::default());
    let sampler = gpu_dev.create_sampler(&SamplerDescriptor::default());
    // A new size throws the frozen picture away; the next saved screen captures again.
    saved.is_frozen = false;
    saved.texture = Some(SavedScreenTexture { texture: tex, cam_view: tex_view, filter_samp: sampler, format, size });
}

fn draw_hud_quads(
    view: ViewQuery<&ViewTarget, (With<ExtractedView>, Without<PortalCamera>)>,
    frame: Res<HudFrame>,
    gpu: Res<HudGpu>,
    pipeline: Res<HudPipeline>,
    mut specialized: ResMut<SpecializedRenderPipelines<HudPipeline>>,
    pipes: Res<PipelineCache>,
    gpu_dev: Res<RenderDevice>,
    gpu_imgs: Res<RenderAssets<GpuImage>>,
    mut saved: ResMut<SavedScreen>,
    mut context: RenderContext,
) {
    let cam_target = view.into_inner();
    let format = cam_target.main_texture_format();
    let size = cam_target.main_texture().size();

    // A new saved screen freezes the scene as it stands, before the HUD draws on it.
    if frame.saved_screen_sequence != saved.seq_no {
        saved.is_frozen = false;
        if let Some(copy) = saved.texture.as_ref().filter(|t| t.can_hold(format, size)) {
            context.command_encoder().copy_texture_to_texture(
                cam_target.main_texture().as_image_copy(),
                copy.texture.as_image_copy(),
                size,
            );
            saved.seq_no = frame.saved_screen_sequence;
            saved.is_frozen = true;
        }
    }
    if !frame.vis_q {
        return;
    }

    let (Some(corners), Some(indices)) = (gpu.corner_buf.gpu.as_ref(), gpu.indices.gpu.as_ref())
    else {
        return;
    };

    let saved_copy = saved.texture.as_ref().filter(|t| saved.is_frozen && t.can_hold(format, size));
    let layout = pipes.get_bind_group_layout(&pipeline.layout);
    let mut ready = Vec::with_capacity(gpu.draw_list.len());
    for draw in &gpu.draw_list {
        let Some(frame_batch) = frame.batches.get(draw.batch) else {
            continue;
        };
        // Saved-screen batches carry a stand-in image; it must be loaded too.
        let Some(image) = gpu_imgs.get(&frame_batch.image) else {
            continue;
        };
        let (texture_view, texture_sampler) = match frame_batch.technique {
            HudStitchMethod::Modulate => (&image.texture_view, &image.sampler),
            HudStitchMethod::SavedScreen => match saved_copy {
                Some(copy) => (&copy.cam_view, &copy.filter_samp),
                None => continue,
            },
        };
        let key = HudPipelineKey { out_tgt: format, state_words: frame_batch.blend_state_words };
        let id = specialized.specialize(&pipes, &pipeline, key);
        if pipes.get_render_pipeline(id).is_none() {
            continue;
        }
        let bind_grp = gpu_dev.create_bind_group(
            "hud_quads",
            &layout,
            &BindGroupEntries::sequential((
                pipeline.screen.as_entire_buffer_binding(),
                texture_view,
                texture_sampler,
            )),
        );
        ready.push((id, bind_grp, draw));
    }
    if ready.is_empty() {
        return;
    }

    let color_atts = [Some(cam_target.get_unsampled_color_attachment())];
    let mut rp = context.begin_tracked_render_pass(RenderPassDescriptor {
        label: Some("hud_quads"),
        color_attachments: &color_atts,
        depth_stencil_attachment: None,
        timestamp_writes: None,
        occlusion_query_set: None,
        multiview_mask: None,
    });
    rp.set_vertex_buffer(0, corners.slice(..));
    rp.set_index_buffer(indices.slice(..), IndexFormat::Uint16);
    for (id, bind_grp, draw) in &ready {
        let Some(gpu_pipeline) = pipes.get_render_pipeline(*id) else {
            continue;
        };
        rp.set_render_pipeline(gpu_pipeline);
        rp.set_bind_group(0, bind_grp, &[]);
        rp.draw_indexed(draw.indices.clone(), draw.vertex_base, 0..1);
    }
    drop(rp);
}

pub(super) fn register(host: &mut App) {
    bevy::asset::embedded_asset!(host, "hud_quads.wgsl");
    let Some(render_sub) = host.get_sub_app_mut(RenderApp) else {
        return;
    };
    render_sub
        .init_resource::<HudFrame>()
        .init_resource::<HudGpu>()
        .init_resource::<SavedScreen>()
        .init_resource::<SpecializedRenderPipelines<HudPipeline>>()
        .add_systems(RenderStartup, init_hud_pipeline)
        .add_systems(ExtractSchedule, extract_hud_frame)
        .add_systems(
            Render,
            (prepare_hud_quads, prepare_saved_screen).in_set(RenderSystems::PrepareResources),
        )
        .add_systems(
            Core3d,
            draw_hud_quads
                .in_set(Core3dSystems::PostProcess)
                .after(super::postfx_frame::PostEffectSet),
        );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn batch(vertex_start: u32, vertex_count: u32, first_ix: u32, ix_count: u32) -> HudStitchBatch {
        HudStitchBatch {
            image: Handle::default(),
            technique: HudStitchMethod::Modulate,
            blend_state_words: None,
            first_index: first_ix,
            index_count: ix_count,
            vertex_base: vertex_start,
            vertex_count,
        }
    }

    #[test]
    fn corner_is_32_bytes_with_the_shader_offsets() {
        assert_eq!(size_of::<QuadCorner>(), 32);
        let offsets: Vec<u64> = corner_layout().attributes.iter().map(|a| a.offset).collect();
        assert_eq!(offsets, [0, 16, 20]);
        assert_eq!(corner_layout().array_stride, 32);
    }

    #[test]
    fn each_batch_draws_its_range_from_its_first_corner() {
        let draws = plan_batches(8, 12, &[batch(0, 4, 0, 6), batch(4, 4, 6, 6)]);
        assert_eq!(
            draws,
            [
                BatchDraw { batch: 0, indices: 0..6, vertex_base: 0 },
                BatchDraw { batch: 1, indices: 6..12, vertex_base: 4 },
            ]
        );
    }

    #[test]
    fn broken_batches_are_left_out() {
        let batches = [
            batch(0, 0, 0, 6),
            batch(0, 4, 0, 0),
            batch(0, 4, 0, 4),
            batch(6, 4, 0, 6),
            batch(0, 4, 9, 6),
            batch(u32::MAX, 4, 0, 6),
            batch(4, 4, 6, 6),
        ];
        let draws = plan_batches(8, 12, &batches);
        assert_eq!(draws.iter().map(|d| d.batch).collect::<Vec<_>>(), [6]);
    }

    #[test]
    fn index_upload_is_whole_words() {
        assert_eq!(index_bytes(&[1, 2]).len(), 4);
        let odd = index_bytes(&[1, 2, 3]);
        assert_eq!(odd.len(), 8);
        assert_eq!(odd[6..], [0, 0]);
    }

    #[test]
    fn buffers_grow_to_a_power_of_two_never_below_the_floor() {
        let mib = 1 << 20;
        assert_eq!(grown_size(0, 100, mib), Some(mib));
        assert_eq!(grown_size(mib, mib, mib), None);
        assert_eq!(grown_size(mib, mib + 1, mib), Some(2 * mib));
    }
}
