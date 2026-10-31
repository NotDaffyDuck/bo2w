//! bo2mp: live bodies skinned on the graphics card. Extract hands over each
//! body part's bone joint_transforms and, per drawn surface, which of its shape's
//! vertices to skin with them; the shapes (bind pose and bone weights) are
//! sent once and kept here. A compute pass writes the skinned vertices into
//! the moving models' vertex buffer after the ones the CPU wrote, before
//! anything draws this frame (the sun shadow and the main pass both read
//! them). Skinning and copying ~150k vertices a frame on the CPU cost a
//! drawn window about 5 ms of game thread and 10 ms of render thread.

use super::*;
use std::collections::HashSet;
use bevy::render::renderer::PendingCommandBuffers;
use bevy::render::render_resource::{
    BindGroupLayout, BindGroupLayoutEntry, BindingType, BufferBindingType, BufferDescriptor,
    CommandEncoderDescriptor, ComputePassDescriptor, ComputePipeline, PipelineLayoutDescriptor,
    RawComputePipelineDescriptor, ShaderModuleDescriptor, ShaderSource,
};

/// The shapes kept on the card at most (48 + 32 bytes each); past it they
/// start over with this frame's.
const KEPT_VERTICES: u32 = 1 << 20;
/// Frames with nothing to skin before the shapes are let go (a menu).
const IDLE_FRAMES: u32 = 600;
const WORKGROUP: u32 = 64;
const VERTEX_BYTES: u64 = std::mem::size_of::<PropVertex>() as u64;
const SKIN_BYTES: u64 = 32;

#[derive(Resource)]
pub(super) struct SkinPipeline {
    layout: BindGroupLayout,
    pipeline: ComputePipeline,
}

pub(super) fn init(mut commands: Commands, gpu_dev: Res<RenderDevice>) {
    let storage = |binding, read_only| BindGroupLayoutEntry {
        binding,
        visibility: ShaderStages::COMPUTE,
        ty: BindingType::Buffer {
            ty: BufferBindingType::Storage { read_only },
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    };
    let layout = gpu_dev.create_bind_group_layout(
        "bo2mp_skin",
        &[
            storage(0, true),
            storage(1, true),
            storage(2, true),
            storage(3, true),
            storage(4, false),
        ],
    );
    let module = gpu_dev.create_and_validate_shader_module(ShaderModuleDescriptor {
        label: Some("bo2mp_skin"),
        source: ShaderSource::Wgsl(include_str!("skin.wgsl").into()),
    });
    let pipeline_layout = gpu_dev.create_pipeline_layout(&PipelineLayoutDescriptor {
        label: Some("bo2mp_skin"),
        bind_group_layouts: &[Some(&*layout)],
        immediate_size: 0,
    });
    let descriptor = RawComputePipelineDescriptor { label: Some("bo2mp_skin"), layout: Some(&pipeline_layout), module: &module, entry_point: Some("skin_main"), compilation_options: Default::default(), cache: None };
    let pipeline = gpu_dev.create_compute_pipeline(&descriptor);
    commands.insert_resource(SkinPipeline { layout, pipeline });
}

/// The shapes on the card: where each starts (in vertices), kept with what
/// names it; and this frame's bones and runs.
#[derive(Resource, Default)]
pub(super) struct SkinCache {
    shapes: HashMap<usize, (u32, Arc<dyn std::any::Any + Send + Sync>)>,
    used: u32,
    room: u32,
    shape: Option<Buffer>,
    skin: Option<Buffer>,
    bones: Option<Buffer>,
    runs: Option<Buffer>,
    verts: Vec<PropVertex>,
    weights: Vec<[u32; 8]>,
    row_data: Vec<[u32; 8]>,
    idle: u32,
}

impl SkinCache {
    /// A frame with nothing skinned; after a while the shapes go.
    pub(super) fn idle(&mut self) {
        self.idle = self.idle.saturating_add(1);
        if self.idle == IDLE_FRAMES {
            // Drop every shape and buffer but keep counting, so this happens once.
            let idle_flag = self.idle;
            *self = Self { idle: idle_flag, ..Default::default() };
        }
    }
}

/// `need` bytes of `usage` in `slot`, made (bigger) when it is short.
fn room(render_device: &RenderDevice, bay: &mut Option<Buffer>, name: &str, kind: BufferUsages, need: u64) -> Buffer {
    if let Some(have) = bay.as_ref().filter(|have| have.size() >= need) {
        return have.clone();
    }
    // Half again what is needed, at least 64 KiB, in whole words.
    let size = (need + need / 2).max(64 << 10).next_multiple_of(4);
    let made_buf = render_device.create_buffer(&BufferDescriptor { label: Some(name), size, usage: BufferUsages::COPY_DST | kind, mapped_at_creation: false });
    bay.insert(made_buf).clone()
}

/// This frame's card-skinned vertices written into `dst` from vertex
/// `first` on; false when they could not be (nothing is drawn from them).
/// The pass goes first in the frame's one submit (`pending`), ahead of the
/// draws that read it; a submit of its own cost the render thread ~0.5 ms.
#[allow(clippy::too_many_arguments)]
pub(super) fn skin_into(
    gpu_dev: &RenderDevice,
    gpu_queue: &RenderQueue,
    pending: &mut PendingCommandBuffers,
    pipeline: &SkinPipeline,
    pipes: &mut SkinCache,
    skinned: &crate::surf_gpu::scene_geo::T6Skinned,
    dst: &Buffer,
    first: u32,
) -> bool {
    let _span = bevy::log::info_span!(target: "bo2mp", "t6 skin").entered();
    pipes.idle = 0;
    let key = |s: &crate::surf_gpu::scene_geo::T6SkinShape| {
        Arc::as_ptr(&s.source) as *const () as usize
    };
    let missing = |pipes: &SkinCache| -> u32 {
        let mut seen_set = HashSet::new();
        skinned
            .shapes
            .iter()
            .filter(|s| !pipes.shapes.contains_key(&key(s)) && seen_set.insert(key(s)))
            .map(|s| s.vertices as u32)
            .sum()
    };
    let mut new = missing(pipes);
    if pipes.used + new > KEPT_VERTICES {
        pipes.shapes.clear();
        pipes.used = 0;
        new = missing(pipes);
    }
    let mut enc = gpu_dev.create_command_encoder(&CommandEncoderDescriptor {
        label: Some("bo2mp_skin"),
    });
    let need = pipes.used + new;
    if need > pipes.room || pipes.shape.is_none() {
        let room = (need + need / 2).max(1 << 16);
        let usage = BufferUsages::STORAGE | BufferUsages::COPY_DST | BufferUsages::COPY_SRC;
        let make_buf = |name, size: u64| gpu_dev.create_buffer(&BufferDescriptor { label: Some(name), size, usage, mapped_at_creation: false });
        let shape = make_buf("bo2mp_skin_shapes", u64::from(room) * VERTEX_BYTES);
        let skin = make_buf("bo2mp_skin_weights", u64::from(room) * SKIN_BYTES);
        if pipes.used > 0
            && let (Some(old_shape), Some(old_skin)) = (&pipes.shape, &pipes.skin)
        {
            let used = u64::from(pipes.used);
            enc.copy_buffer_to_buffer(old_shape, 0, &shape, 0, used * VERTEX_BYTES);
            enc.copy_buffer_to_buffer(old_skin, 0, &skin, 0, used * SKIN_BYTES);
        }
        pipes.shape = Some(shape);
        pipes.skin = Some(skin);
        pipes.room = room;
    }
    // (The copies above go to the card whatever happens below.)
    let ok = write_and_dispatch(gpu_dev, gpu_queue, pipeline, pipes, skinned, dst, first, &mut enc);
    // (Not `push_encoder`: those go after the frame's other buffers.)
    pending.push([enc.finish()]);
    ok
}

/// The shapes the card lacks, this frame's bones and runs, and the pass
/// that skins them into `dst`.
#[allow(clippy::too_many_arguments)]
fn write_and_dispatch(
    gpu_dev: &RenderDevice,
    gpu_queue: &RenderQueue,
    pipeline: &SkinPipeline,
    pipes: &mut SkinCache,
    skinned: &crate::surf_gpu::scene_geo::T6Skinned,
    dst: &Buffer,
    first: u32,
    enc: &mut bevy::render::render_resource::CommandEncoder,
) -> bool {
    let key = |s: &crate::surf_gpu::scene_geo::T6SkinShape| {
        Arc::as_ptr(&s.source) as *const () as usize
    };
    let (Some(shape_buffer), Some(skin_buffer)) = (pipes.shape.clone(), pipes.skin.clone()) else {
        return false;
    };
    // The shapes the card does not have yet, after the others.
    for s in &skinned.shapes {
        let k = key(s);
        if pipes.shapes.contains_key(&k) {
            continue;
        }
        pipes.verts.clear();
        pipes.weights.clear();
        (s.fill)(&mut pipes.verts, &mut pipes.weights);
        let n = s.vertices;
        if pipes.verts.len() != n
            || pipes.weights.len() != n
            || pipes.used + n as u32 > pipes.room
        {
            return false;
        }
        let at = u64::from(pipes.used);
        let (shape_bytes, skin_bytes): (&[u8], &[u8]) = (bytemuck::cast_slice(&pipes.verts), bytemuck::cast_slice(&pipes.weights));
        gpu_queue.write_buffer(&shape_buffer, at * VERTEX_BYTES, shape_bytes);
        gpu_queue.write_buffer(&skin_buffer, at * SKIN_BYTES, skin_bytes);
        pipes.shapes.insert(k, (pipes.used, Arc::clone(&s.source)));
        pipes.used += n as u32;
    }
    // The runs: a header, then per run where it lands, where it reads
    // from, how many, and its bones.
    pipes.row_data.clear();
    pipes.row_data.push([0; 8]);
    let mut total: u32 = 0;
    for r in &skinned.runs {
        let Some(&(at, _)) = skinned
            .shapes
            .get(r[0] as usize)
            .and_then(|s| pipes.shapes.get(&key(s)))
        else {
            return false;
        };
        if r[2] == 0 {
            continue;
        }
        pipes.row_data.push([total, at + r[1], r[2], r[3], r[4], 0, 0, 0]);
        total += r[2];
    }
    let runs = pipes.row_data.len() as u32 - 1;
    if total == 0 || total as usize != skinned.vertices || total.div_ceil(WORKGROUP) > 65535 {
        return false;
    }
    pipes.row_data[0] = [runs, total, 0, first, 0, 0, 0, 0];
    let bones_buffer = room(
        gpu_dev,
        &mut pipes.bones,
        "bo2mp_skin_bones",
        BufferUsages::STORAGE,
        (skinned.bones.len().max(1) * 64) as u64,
    );
    gpu_queue.write_buffer(&bones_buffer, 0, bytemuck::cast_slice(&skinned.bones));
    let runs_buffer = room(
        gpu_dev,
        &mut pipes.runs,
        "bo2mp_skin_runs",
        BufferUsages::STORAGE,
        (pipes.row_data.len() * 32) as u64,
    );
    gpu_queue.write_buffer(&runs_buffer, 0, bytemuck::cast_slice(&pipes.row_data));
    // Bindings 0..=4: shapes, weights, bones, runs, and the output.
    let bound = [&shape_buffer, &skin_buffer, &bones_buffer, &runs_buffer, dst].map(|buffer| buffer.as_entire_binding());
    let [shapes, weights, bones, runs, out] = bound;
    let entries = BindGroupEntries::sequential((shapes, weights, bones, runs, out));
    let bind_group = gpu_dev.create_bind_group("bo2mp_skin", &pipeline.layout, &entries);
    {
        let mut rp = enc.begin_compute_pass(&ComputePassDescriptor {
            label: Some("bo2mp_skin"),
            timestamp_writes: None,
        });
        rp.set_pipeline(&pipeline.pipeline);
        rp.set_bind_group(0, &*bind_group, &[]);
        rp.dispatch_workgroups(total.div_ceil(WORKGROUP), 1, 1);
    }
    true
}
