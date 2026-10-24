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
use hud::{HudTessBatch, HudTessGpuFrame, HudTessTechnique};
use hud_t6::set_2d::{cmd_buf_set_2d_projection, set_2d_clip_coeffs};

use super::portal_view::PortalCamera;

const SHADER_PATH: &str = "embedded://render_gpu/drawsurf/hud_quads.wgsl";
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
    normal: u32,
}

fn corner_layout() -> VertexBufferLayout {
    let attribute = |format, offset: usize, shader_location| VertexAttribute {
        format,
        offset: offset as u64,
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
    indices: Vec<u16>,
    batches: Vec<HudTessBatch>,
    surface: [f32; 2],
    visible: bool,
    saved_screen_sequence: u64,
}

fn extract_hud_frame(mut frame: ResMut<HudFrame>, hud: Extract<Res<HudTessGpuFrame>>) {
    frame.corners.clear();
    frame.corners.extend(hud.vertices.iter().map(|v| QuadCorner {
        pos: v.xyzw,
        bgra: v.color_bgra,
        uv: v.uv,
        normal: v.packed_normal,
    }));
    frame.indices.clone_from(&hud.indices);
    frame.batches.clone_from(&hud.batches);
    frame.surface = [hud.surface_w, hud.surface_h];
    frame.visible = hud.visible;
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
fn plan_batches(corner_len: usize, index_len: usize, batches: &[HudTessBatch]) -> Vec<BatchDraw> {
    let inside = |first: u32, count: u32, len: usize| {
        (first as usize)
            .checked_add(count as usize)
            .is_some_and(|end| end <= len)
    };
    batches
        .iter()
        .enumerate()
        .filter_map(|(batch, b)| {
            if b.vertex_count == 0 || b.index_count == 0 || b.index_count % 3 != 0 {
                return None;
            }
            if !inside(b.first_vertex, b.vertex_count, corner_len)
                || !inside(b.first_index, b.index_count, index_len)
            {
                return None;
            }
            Some(BatchDraw {
                batch,
                indices: b.first_index..b.first_index.checked_add(b.index_count)?,
                vertex_base: i32::try_from(b.first_vertex).ok()?,
            })
        })
        .collect()
}

/// Queue writes move whole 4-byte words, so an odd index count gets one zero index
/// of padding.
fn index_bytes(indices: &[u16]) -> Cow<'_, [u8]> {
    let bytes: &[u8] = bytemuck::cast_slice(indices);
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
    label: &'static str,
    usage: BufferUsages,
    floor: u64,
    gpu: Option<Buffer>,
}

impl FrameBuffer {
    fn new(label: &'static str, usage: BufferUsages, floor: u64) -> Self {
        Self { label, usage, floor, gpu: None }
    }

    fn upload(&mut self, device: &RenderDevice, queue: &RenderQueue, bytes: &[u8]) {
        if bytes.is_empty() {
            return;
        }
        let have = self.gpu.as_ref().map_or(0, |gpu| gpu.size());
        if let Some(size) = grown_size(have, bytes.len() as u64, self.floor) {
            self.gpu = Some(device.create_buffer(&BufferDescriptor {
                label: Some(self.label),
                size,
                usage: self.usage | BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }));
        }
        if let Some(gpu) = &self.gpu {
            queue.write_buffer(gpu, 0, bytes);
        }
    }
}

#[derive(Resource)]
struct HudGpu {
    corners: FrameBuffer,
    indices: FrameBuffer,
    draws: Vec<BatchDraw>,
}

impl Default for HudGpu {
    fn default() -> Self {
        Self {
            corners: FrameBuffer::new("hud_quads_corners", BufferUsages::VERTEX, VERTEX_BUFFER_FLOOR),
            indices: FrameBuffer::new("hud_quads_indices", BufferUsages::INDEX, INDEX_BUFFER_FLOOR),
            draws: Vec::new(),
        }
    }
}

/// The frozen picture a saved-screen batch shows, copied from the scene when the
/// HUD starts a new saved screen.
struct SavedScreenTexture {
    texture: Texture,
    view: TextureView,
    sampler: Sampler,
    format: TextureFormat,
    size: Extent3d,
}

impl SavedScreenTexture {
    fn fits(&self, format: TextureFormat, size: Extent3d) -> bool {
        self.format == format && self.size == size
    }
}

#[derive(Resource, Default)]
struct SavedScreen {
    texture: Option<SavedScreenTexture>,
    sequence: u64,
    frozen: bool,
}

#[derive(Resource)]
struct HudPipeline {
    shader: Handle<Shader>,
    layout: BindGroupLayoutDescriptor,
    screen: Buffer,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct HudPipelineKey {
    target: TextureFormat,
    state_bits: Option<[u32; 2]>,
}

impl SpecializedRenderPipeline for HudPipeline {
    type Key = HudPipelineKey;

    fn specialize(&self, key: Self::Key) -> RenderPipelineDescriptor {
        // Material state word 0 holds the blend and the colour-write mask.
        let (blend, write_mask) = match key.state_bits.map(|bits| bits[0]) {
            Some(word0) => (
                super::blend_state::blend_state_from_word0(word0, false),
                super::blend_state::colour_writes_from_word0(word0),
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
                shader: self.shader.clone(),
                shader_defs: Vec::new(),
                entry_point: Some("quad_vertex".into()),
                buffers: vec![corner_layout()],
            },
            fragment: Some(FragmentState {
                shader: self.shader.clone(),
                shader_defs: Vec::new(),
                entry_point: Some("quad_fragment".into()),
                targets: vec![Some(ColorTargetState { format: key.target, blend, write_mask })],
            }),
            primitive: PrimitiveState::default(),
            depth_stencil: None,
            multisample: MultisampleState::default(),
            zero_initialize_workgroup_memory: false,
        }
    }
}

fn init_hud_pipeline(mut commands: Commands, assets: Res<AssetServer>, device: Res<RenderDevice>) {
    let screen = device.create_buffer(&BufferDescriptor {
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
        shader: assets.load(SHADER_PATH),
        layout: BindGroupLayoutDescriptor::new("hud_quads_layout", &entries),
        screen,
    });
}

fn prepare_hud_quads(
    frame: Res<HudFrame>,
    mut gpu: ResMut<HudGpu>,
    pipeline: Res<HudPipeline>,
    device: Res<RenderDevice>,
    queue: Res<RenderQueue>,
) {
    gpu.draws.clear();
    if !frame.visible {
        return;
    }
    gpu.draws = plan_batches(frame.corners.len(), frame.indices.len(), &frame.batches);
    if !gpu.draws.is_empty() {
        gpu.corners.upload(&device, &queue, bytemuck::cast_slice(&frame.corners));
        gpu.indices.upload(&device, &queue, &index_bytes(&frame.indices));
    }
    let [w, h] = frame.surface;
    if let Some(projection) = cmd_buf_set_2d_projection(w as i32, h as i32) {
        queue.write_buffer(&pipeline.screen, 0, bytemuck::bytes_of(&set_2d_clip_coeffs(&projection)));
    }
}

/// Keeps a saved-screen texture matching the main view, and warms the plain HUD
/// pipeline for that view's format so the first HUD frame can draw.
fn prepare_saved_screen(
    views: Query<&ViewTarget, (With<Camera3d>, Without<PortalCamera>)>,
    device: Res<RenderDevice>,
    cache: Res<PipelineCache>,
    pipeline: Res<HudPipeline>,
    mut specialized: ResMut<SpecializedRenderPipelines<HudPipeline>>,
    mut saved: ResMut<SavedScreen>,
) {
    let Some(target) = views.iter().next() else {
        return;
    };
    let format = target.main_texture_format();
    let size = target.main_texture().size();
    specialized.specialize(&cache, &pipeline, HudPipelineKey { target: format, state_bits: None });
    if saved.texture.as_ref().is_some_and(|t| t.fits(format, size)) {
        return;
    }
    let texture = device.create_texture(&TextureDescriptor {
        label: Some("hud_quads_saved_screen"),
        size,
        mip_level_count: 1,
        sample_count: 1,
        dimension: TextureDimension::D2,
        format,
        usage: TextureUsages::TEXTURE_BINDING | TextureUsages::COPY_DST,
        view_formats: &[],
    });
    let view = texture.create_view(&TextureViewDescriptor::default());
    let sampler = device.create_sampler(&SamplerDescriptor::default());
    // A new size throws the frozen picture away; the next saved screen captures again.
    saved.frozen = false;
    saved.texture = Some(SavedScreenTexture { texture, view, sampler, format, size });
}

fn draw_hud_quads(
    view: ViewQuery<&ViewTarget, (With<ExtractedView>, Without<PortalCamera>)>,
    frame: Res<HudFrame>,
    gpu: Res<HudGpu>,
    pipeline: Res<HudPipeline>,
    mut specialized: ResMut<SpecializedRenderPipelines<HudPipeline>>,
    cache: Res<PipelineCache>,
    device: Res<RenderDevice>,
    images: Res<RenderAssets<GpuImage>>,
    mut saved: ResMut<SavedScreen>,
    mut context: RenderContext,
) {
    let target = view.into_inner();
    let format = target.main_texture_format();
    let size = target.main_texture().size();

    // A new saved screen freezes the scene as it stands, before the HUD draws on it.
    if frame.saved_screen_sequence != saved.sequence {
        saved.frozen = false;
        if let Some(copy) = saved.texture.as_ref().filter(|t| t.fits(format, size)) {
            context.command_encoder().copy_texture_to_texture(
                target.main_texture().as_image_copy(),
                copy.texture.as_image_copy(),
                size,
            );
            saved.sequence = frame.saved_screen_sequence;
            saved.frozen = true;
        }
    }
    if !frame.visible {
        return;
    }

    let (Some(corners), Some(indices)) = (gpu.corners.gpu.as_ref(), gpu.indices.gpu.as_ref())
    else {
        return;
    };

    let saved_copy = saved.texture.as_ref().filter(|t| saved.frozen && t.fits(format, size));
    let layout = cache.get_bind_group_layout(&pipeline.layout);
    let mut ready = Vec::with_capacity(gpu.draws.len());
    for draw in &gpu.draws {
        let Some(batch) = frame.batches.get(draw.batch) else {
            continue;
        };
        // Saved-screen batches carry a stand-in image; it must be loaded too.
        let Some(image) = images.get(&batch.image) else {
            continue;
        };
        let (texture_view, texture_sampler) = match batch.technique {
            HudTessTechnique::Modulate => (&image.texture_view, &image.sampler),
            HudTessTechnique::SavedScreen => match saved_copy {
                Some(copy) => (&copy.view, &copy.sampler),
                None => continue,
            },
        };
        let key = HudPipelineKey { target: format, state_bits: batch.state_bits };
        let id = specialized.specialize(&cache, &pipeline, key);
        if cache.get_render_pipeline(id).is_none() {
            continue;
        }
        let bind = device.create_bind_group(
            "hud_quads",
            &layout,
            &BindGroupEntries::sequential((
                pipeline.screen.as_entire_buffer_binding(),
                texture_view,
                texture_sampler,
            )),
        );
        ready.push((id, bind, draw));
    }
    if ready.is_empty() {
        return;
    }

    let attachments = [Some(target.get_unsampled_color_attachment())];
    let mut pass = context.begin_tracked_render_pass(RenderPassDescriptor {
        label: Some("hud_quads"),
        color_attachments: &attachments,
        depth_stencil_attachment: None,
        timestamp_writes: None,
        occlusion_query_set: None,
        multiview_mask: None,
    });
    pass.set_vertex_buffer(0, corners.slice(..));
    pass.set_index_buffer(indices.slice(..), IndexFormat::Uint16);
    for (id, bind, draw) in &ready {
        let Some(gpu_pipeline) = cache.get_render_pipeline(*id) else {
            continue;
        };
        pass.set_render_pipeline(gpu_pipeline);
        pass.set_bind_group(0, bind, &[]);
        pass.draw_indexed(draw.indices.clone(), draw.vertex_base, 0..1);
    }
    drop(pass);
}

pub(super) fn register(app: &mut App) {
    bevy::asset::embedded_asset!(app, "hud_quads.wgsl");
    let Some(render_app) = app.get_sub_app_mut(RenderApp) else {
        return;
    };
    render_app
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
                .after(super::postfx_frame::PostFxSet),
        );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn batch(first_vertex: u32, vertex_count: u32, first_index: u32, index_count: u32) -> HudTessBatch {
        HudTessBatch {
            image: Handle::default(),
            technique: HudTessTechnique::Modulate,
            state_bits: None,
            first_index,
            index_count,
            first_vertex,
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
