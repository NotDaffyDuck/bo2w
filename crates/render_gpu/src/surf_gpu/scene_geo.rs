use crate::surf_gpu::scene_depth::{SCENE_DEPTH_FORMAT, SceneDepthTexture};
use std::collections::HashMap;
use std::sync::{Arc, OnceLock};
use std::time::Instant;

use bevy::core_pipeline::core_3d::main_opaque_pass_3d;
use bevy::core_pipeline::{Core3d, Core3dSystems};
use bevy::mesh::VertexBufferLayout;
use bevy::prelude::*;
use bevy::render::RenderStartup;
use bevy::render::render_resource::binding_types::{
    sampler, storage_buffer_read_only, texture_2d, texture_2d_array, texture_3d, texture_cube,
    texture_cube_array, texture_depth_2d, uniform_buffer, uniform_buffer_sized,
};
use bevy::render::render_resource::{
    AddressMode, BindGroup, BindGroupEntries, BindGroupLayoutDescriptor, BindGroupLayoutEntries,
    BlendComponent, BlendFactor, BlendOperation, BlendState, Buffer, BufferInitDescriptor, BufferUsages, CachedRenderPipelineId, ColorTargetState,
    ColorWrites, CompareFunction, DepthBiasState, DepthStencilState, Face, FilterMode,
    FragmentState, FrontFace,
    IndexFormat, MipmapFilterMode, MultisampleState, PipelineCache, PrimitiveState,
    RenderPipelineDescriptor, Sampler, SamplerBindingType, SamplerDescriptor, ShaderStages,
    SpecializedRenderPipeline, SpecializedRenderPipelines, StoreOp, TextureFormat,
    TextureSampleType, TextureViewDimension, VertexAttribute, VertexFormat, VertexState,
    VertexStepMode,
};
use bevy::ecs::system::SystemParam;
use bevy::render::render_phase::TrackedRenderPass;
use bevy::render::render_resource::{RenderPassDepthStencilAttachment, RenderPassDescriptor};
use bevy::render::renderer::{RenderContext, RenderDevice, RenderQueue, ViewQuery};
use bevy::render::view::{
    ExtractedView, Msaa, ViewTarget, ViewUniform, ViewUniformOffset, ViewUniforms,
};
use bevy::render::{Render, RenderSystems};
use bevy::shader::{Shader, ShaderDefVal};

use super::gpu_bufs::{LightmapPlane, LiveUploadedImageRegistry};
use super::z_layers::DepthLayer;
use render_frame::{
    FrameOutputKind, HeldDrawItem, HeldDrawKind, PropVertex, LevelVertex,
};
use render_material::MaterialEpochId;

const OVERLAY_SHADER: &str = "embedded://render_gpu/surf_gpu/scene_geo.wgsl";

mod skin;
mod sun_shadow;

/// The overlay draws the level and its models only: effects, marks and glass
/// have their own passes, and the gun is drawn after everything else.
fn draws_in_overlay(kind: &HeldDrawKind, item_key: u64) -> bool {
    let level_or_model = matches!(kind, HeldDrawKind::World { .. } | HeldDrawKind::Smodel { .. } | HeldDrawKind::XModel { .. });
    level_or_model && DepthLayer::of_draw(kind, item_key) != DepthLayer::Gun
}

static OVERLAY_SWITCHED_OFF: OnceLock<bool> = OnceLock::new();

/// The overlay's systems skip their work while this holds.
fn overlay_off() -> bool {
    !geometry_inspect_enabled()
}

pub fn geometry_inspect_enabled() -> bool {
    !*OVERLAY_SWITCHED_OFF.get_or_init(|| std::env::var_os("BO2MP_NO_DIAGNOSTIC").is_some())
}

#[derive(Default)]
struct OverlayScratch {
    draws: Vec<HeldDrawItem>,
    model_rows: Vec<ModelMatrixRow>,
}

/// bo2zm: frame pacing and prop counts over a window, logged every 5 s.
#[derive(Default)]
struct PacingWindow {
    window_start: Option<Instant>,
    last_frame: Option<Instant>,
    frame_n: u32,
    worst_ms: f32,
    props_drawn: u64,
    /// bo2mp: the draws the opaque ones took.
    prop_batches: u64,
    props_culled: u64,
    props_unseen: u64,
    last_census_at: Option<Instant>,
}

#[derive(Resource, Clone, Debug, Default)]
pub struct ExtractedInspectGeometry {
    pub generation: MaterialEpochId,
    pub topmost_gpu_wait: bool,
    pub level_vertex_total: Arc<Vec<LevelVertex>>,
    pub level_indices: Arc<Vec<u32>>,
    pub world_surface_ranges: Arc<Vec<(u32, u32)>>,
    pub static_model_verts_used: Arc<Vec<PropVertex>>,
    pub static_model_slots_used: Arc<Vec<u32>>,
    pub smodel_surface_ranges: Arc<Vec<(u32, u32)>>,
    pub xmodel_vertices: Arc<Vec<PropVertex>>,
    pub figure_indices: Arc<Vec<u32>>,
    pub figure_facet_ranges: Arc<Vec<(u32, u32)>>,
    pub figure_edition: u64,
    pub g0_level_facets: Vec<u16>,
    /// bo2zm: per world surface, its colour map's uploaded image slot
    /// (`u32::MAX` = none); the fallback draw samples it when present.
    pub world_surface_color: Arc<Vec<u32>>,
    /// bo2zm: per world surface, its Black Ops II draw code
    /// (`asset_core::T6Draw::code`; `u32::MAX` = none).
    pub world_surface_draw: Arc<Vec<u32>>,
    /// bo2zm: per world surface, its lightmap page (`u8::MAX` = none).
    pub world_surface_lightmap: Arc<Vec<u8>>,
    /// bo2zm: the T6 sun direction and exposure scale, the sun colour, then
    /// the sky's rotation and brightness.
    pub t6_lighting: Option<[[f32; 4]; 4]>,
    /// bo2mp: the map's `sm_sunSampleSizeNear`; 0 = not set.
    pub t6_sun_sample_size_near: f32,
    /// bo2zm: Black Ops II props: `[surface, colour slot, draw code,
    /// instance]` per first-LOD surface of each placement.
    pub t6_props: Arc<Vec<[u32; 4]>>,
    /// bo2zm: per prop draw, its normal and specular map slots (shine).
    pub t6_prop_shine: Arc<Vec<[u32; 3]>>,
    /// bo2zm: per prop instance, world-from-local columns, light, then
    /// primary light and its visibility.
    pub t6_prop_instances: Arc<Vec<[f32; 28]>>,
    /// bo2zm: the key the prop list came from (material epoch, placement count, exposure bits).
    pub t6_props_key: Option<(MaterialEpochId, usize, u32)>,
    /// bo2zm: the uploaded slot of the Black Ops II sky's cube map.
    pub t6_sky_slot: Option<u32>,
    /// bo2zm: per world surface, its primary light index.
    pub world_surface_primary: Arc<Vec<u8>>,
    /// bo2zm: the map's primary lights (four vec4 each).
    pub t6_lights: Arc<Vec<[f32; 16]>>,
    /// bo2zm: per world surface, its layer 1 and 2 colour map slots.
    pub world_surface_layers: Arc<Vec<[u32; 4]>>,
    /// bo2zm: per world vertex, layer 1 and 2 texcoords (or empty).
    pub world_layer_uvs: Arc<Vec<[f32; 4]>>,
    /// bo2zm: per world surface, its normal and specular map slots
    /// (`u32::MAX` = none) and its reflection probe (the shine).
    pub world_surface_shine: Arc<Vec<[u32; 3]>>,
    /// bo2zm: the reflection probes' cube array slot, and per probe its
    /// `lightingSH`.
    pub t6_probe_slot: Option<u32>,
    pub t6_probes: Arc<Vec<[[f32; 4]; 3]>>,
}

/// bo2zm: one Black Ops II thing that moves, drawn by the fallback pass
/// with the props' pipelines: vertices in its own space, indices, per draw
/// `[first index, index count, colour slot, draw code, instance]`, and the
/// instance rows (world-from-local columns, light, primary light,
/// visibility).
#[derive(Clone, Debug, Default)]
pub struct T6DynamicDraw {
    pub vertices: Vec<PropVertex>,
    pub indices: Vec<u32>,
    /// (first index, count, colour slot, draw code, instance, normal map
    /// slot, specular map slot)
    pub draws: Vec<[u32; 7]>,
    pub instances: Vec<[f32; 28]>,
    /// The vertices and indices written on the render thread instead (the
    /// live players' ~250k vertices a frame: copied there, not while the
    /// game waits on extract).
    pub deferred: Option<T6DeferredFill>,
    /// bo2mp: and vertices skinned on the graphics card, after the others.
    pub skinned: Option<T6Skinned>,
}

/// bo2mp: the moving models skinned on the graphics card, written after
/// the vertices `vertices` or `deferred` adds (the indices already count
/// them there): their shapes (sent once and kept on the card), each model
/// part's bone joint_transforms, and which shape vertices each surface skins.
#[derive(Clone, Debug, Default)]
pub struct T6Skinned {
    pub shapes: Vec<T6SkinShape>,
    /// Skin joint_transforms (column-major), each part's bones in a row.
    pub bones: Vec<[f32; 16]>,
    /// Per drawn surface: (shape, its first vertex in the shape, how many,
    /// the part's first bone, the part's bone count).
    pub runs: Vec<[u32; 5]>,
    /// The vertices the runs add, all told.
    pub vertices: usize,
}

/// One shape the card skins from: kept there by what `source` points at
/// (which it holds on to), filled once with its bind-pose vertices and,
/// per vertex, its four bones and then its four weights' bits.
#[derive(Clone)]
pub struct T6SkinShape {
    pub source: Arc<dyn std::any::Any + Send + Sync>,
    pub vertices: usize,
    pub fill: Arc<dyn Fn(&mut Vec<PropVertex>, &mut Vec<[u32; 8]>) + Send + Sync>,
}

impl std::fmt::Debug for T6SkinShape {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "T6SkinShape({} vertices)", self.vertices)
    }
}

/// Fills a draw's vertex and index lists (emptied first) from shared,
/// unchanging data; `vertices` and `indices` are how many it adds.
#[derive(Clone)]
pub struct T6DeferredFill {
    pub vertices: usize,
    pub indices: usize,
    /// bo2mp: what the indices are made from (0: not known). The same as
    /// last frame's and they are not made or sent again: the list's index
    /// buffer still has them (the live players' ~300k a frame cost ~1 ms).
    pub indices_key: u64,
    /// The vertices, and the indices when there is somewhere to put them.
    pub fill: Arc<dyn Fn(&mut Vec<PropVertex>, Option<&mut Vec<u32>>) + Send + Sync>,
}

impl std::fmt::Debug for T6DeferredFill {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let Self { vertices, indices, .. } = self;
        write!(f, "T6DeferredFill({vertices} vertices, {indices} indices)")
    }
}

/// bo2zm M3: the models that hold their shape (mannequins, perk machines,
/// the box, wall guns; grenades in flight; bo2mp: guns on the ground,
/// corpses at rest): their shapes uploaded once, in the chunks they came
/// in (indices count from the first chunk's first vertex), placed by this
/// frame's instance rows. Re-sending ~150 models' vertices every frame cost
/// ~12 ms. A new `epoch` starts the upload over.
#[derive(Clone, Debug, Default)]
pub struct T6StaticDraw {
    pub epoch: u64,
    pub chunks: Vec<Arc<(Vec<PropVertex>, Vec<u32>)>>,
    pub draws: Vec<[u32; 7]>,
    pub instances: Vec<[f32; 28]>,
}

/// bo2zm: Black Ops II things that move, extracted every frame: the
/// first-person gun and arms. `materials` lists every T6 material's colour
/// slot and draw code, so pipelines and texture bind groups exist for
/// whatever a gun or effect draws with.
#[derive(Resource, Clone, Debug, Default)]
pub struct ExtractedT6Dynamic {
    pub generation: MaterialEpochId,
    pub mat_list: Arc<Vec<(u32, u32)>>,
    pub viewmodel: Option<T6DynamicDraw>,
    /// Bullet holes and other decal marks, in world space.
    pub marks: Option<T6DynamicDraw>,
    /// Projectiles in flight (grenades, rockets), each in its own space.
    pub missiles: Option<T6DynamicDraw>,
    /// bo2zm M3: the script models that hold still.
    pub script_static: Option<T6StaticDraw>,
    /// Lights effects throw (origin, radius; colour), nearest first.
    pub dlights: Vec<[f32; 8]>,
    /// Effect sprites, in world space.
    pub effects: Option<T6DynamicDraw>,
    /// Effect sprites drawn with the first-person gun (muzzle flashes).
    pub viewmodel_effects: Option<T6DynamicDraw>,
    /// bo2mp: the map's global wind vector (`asset_core::t6_map_wind`),
    /// which sways the tree canopies.
    pub wind: [f32; 3],
}

/// The held shapes on the GPU: which gathering, how many chunks of it are
/// written, and how far into each buffer (bytes); the buffers have room to
/// grow, so a new chunk is written after the others.
struct HeldStatics {
    epoch: u64,
    chunks: usize,
    vertex_bytes: u64,
    index_bytes: u64,
    vb: Buffer,
    ib: Buffer,
}

/// One per-frame list's buffers (vertex, index, instance), kept from frame
/// to frame and grown when a frame needs more: new buffers for ~13 MB of
/// players every frame cost more than writing into the old ones.
/// bo2mp: and what the index buffer was last made from (its key and
/// count), holding that fill so the models it names stay where they are.
#[derive(Default)]
struct HeldBuffers(
    [Option<Buffer>; 3],
    Vec<PropVertex>,
    Vec<u32>,
    Option<(u64, usize, Arc<dyn Fn(&mut Vec<PropVertex>, Option<&mut Vec<u32>>) + Send + Sync>)>,
);

impl HeldBuffers {
    /// `bytes` written into buffer `at`, made bigger first when they do not
    /// fit.
    fn write(
        &mut self,
        gpu_dev: &RenderDevice,
        gpu_queue: &RenderQueue,
        buf_tag: &str,
        at: usize,
        usage_flags: BufferUsages,
        bytes: &[u8],
    ) -> Buffer {
        self.write_in(gpu_dev, gpu_queue, buf_tag, at, usage_flags, bytes.len() as u64, bytes)
    }

    /// The same, in a buffer with room for `need` bytes (bo2mp: the
    /// card-skinned vertices go after `bytes`).
    #[allow(clippy::too_many_arguments)]
    fn write_in(
        &mut self,
        gpu_dev: &RenderDevice,
        gpu_queue: &RenderQueue,
        buf_tag: &str,
        at: usize,
        buf_usage: BufferUsages,
        need: u64,
        bytes: &[u8],
    ) -> Buffer {
        let need = need.max(bytes.len() as u64);
        if self.0[at]
            .as_ref()
            .is_none_or(|b| b.size() < need || !b.usage().contains(buf_usage))
        {
            let room = (need + need / 2).max(64 << 10).next_multiple_of(4);
            self.0[at] = Some(make_writable(gpu_dev, buf_tag, buf_usage, room));
        }
        let buffer = self.0[at].clone().expect("made above");
        if !bytes.is_empty() {
            gpu_queue.write_buffer(&buffer, 0, bytes);
        }
        buffer
    }
}

/// bo2zm: the GPU side of `ExtractedT6Dynamic`.
#[derive(Resource, Default)]
struct OverlayMovingLists {
    /// marks, viewmodel, missiles, effects, viewmodel effects, held
    /// shapes' instances.
    kept: [HeldBuffers; 6],
    marks: Option<(Buffer, Buffer, Buffer, Vec<[u32; 7]>)>,
    missiles: Option<(Buffer, Buffer, Buffer, Vec<[u32; 7]>)>,
    script_static: Option<(Buffer, Buffer, Buffer, Vec<[u32; 7]>)>,
    /// The held shapes as uploaded so far.
    script_static_geometry: Option<HeldStatics>,
    viewmodel: Option<(Buffer, Buffer, Buffer, Vec<[u32; 7]>)>,
    effects: Option<(Buffer, Buffer, Buffer, Vec<[u32; 7]>)>,
    viewmodel_effects: Option<(Buffer, Buffer, Buffer, Vec<[u32; 7]>)>,
}

fn mirror_moving_draw(
    gpu_dev: &RenderDevice,
    gpu_queue: &RenderQueue,
    keep_dst: &mut HeldBuffers,
    buf_tag: &str,
    draw: Option<&T6DynamicDraw>,
    card_skin: Option<(
        &skin::SkinPipeline,
        &mut skin::SkinCache,
        &mut bevy::render::renderer::PendingCommandBuffers,
    )>,
) -> Option<(Buffer, Buffer, Buffer, Vec<[u32; 7]>)> {
    let draw = draw?;
    // A deferred list is filled here into this list's own kept room.
    let mut vertices = std::mem::take(&mut keep_dst.1);
    let mut indices = std::mem::take(&mut keep_dst.2);
    // bo2mp: indices made from the same as last frame's are still there.
    let same_indices = draw.deferred.as_ref().and_then(|d| {
        let &(key, n, _) = keep_dst.3.as_ref()?;
        (d.indices_key != 0 && key == d.indices_key && n == d.indices && n > 0)
            .then(|| keep_dst.0[1].clone())
            .flatten()
    });
    keep_dst.3 = None;
    let (v, i) = match &draw.deferred {
        Some(deferred) => {
            vertices.clear();
            indices.clear();
            vertices.reserve(deferred.vertices);
            let _span = bevy::log::info_span!(target: "bo2mp", "t6 fill", name = buf_tag).entered();
            if same_indices.is_some() {
                (deferred.fill)(&mut vertices, None);
            } else {
                indices.reserve(deferred.indices);
                (deferred.fill)(&mut vertices, Some(&mut indices));
            }
            (vertices.as_slice(), indices.as_slice())
        }
        None => (draw.vertices.as_slice(), draw.indices.as_slice()),
    };
    let index_tally = match (&same_indices, &draw.deferred) {
        (Some(_), Some(d)) => d.indices,
        _ => i.len(),
    };
    // bo2mp: the card-skinned vertices go after these, written there.
    let skinned = draw.skinned.as_ref().filter(|s| s.vertices > 0);
    let skinned_n = skinned.map_or(0, |s| s.vertices);
    let buffers = ((!v.is_empty() || skinned_n > 0)
        && index_tally > 0
        && !draw.draws.is_empty()
        && !draw.instances.is_empty())
    .then(|| {
        let _span = bevy::log::info_span!(target: "bo2mp", "t6 write", name = buf_tag).entered();
        let usage = match skinned {
            Some(_) => BufferUsages::VERTEX | BufferUsages::STORAGE,
            None => BufferUsages::VERTEX,
        };
        let need = ((v.len() + skinned_n) * std::mem::size_of::<PropVertex>()) as u64;
        let vb = keep_dst.write_in(gpu_dev, gpu_queue, buf_tag, 0, usage, need, bytemuck::cast_slice(v));
        let ib = match &same_indices {
            Some(ib) => ib.clone(),
            None => keep_dst.write(gpu_dev, gpu_queue, buf_tag, 1, BufferUsages::INDEX, bytemuck::cast_slice(i)),
        };
        (vb, ib)
    });
    let written = v.len() as u32;
    if buffers.is_some()
        && let Some(d) = draw.deferred.as_ref().filter(|d| d.indices_key != 0)
    {
        keep_dst.3 = Some((d.indices_key, index_tally, Arc::clone(&d.fill)));
    }
    let index_tally = index_tally as u64;
    keep_dst.1 = vertices;
    keep_dst.2 = indices;
    let (vb, ib) = buffers?;
    if let Some(skinned) = skinned
        && !card_skin.is_some_and(|(pipeline, pipes, pending)| {
            skin::skin_into(gpu_dev, gpu_queue, pending, pipeline, pipes, skinned, &vb, written)
        })
    {
        // Their vertices are not there: nothing in this list is drawn.
        static SAID: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        if SAID.fetch_add(1, std::sync::atomic::Ordering::Relaxed) < 8 {
            diag::warn!(
                World,
                "{buf_tag}: {} card-skinned vertices ({} shapes, {} runs) could not be skinned; list left out",
                skinned.vertices,
                skinned.shapes.len(),
                skinned.runs.len(),
            );
        }
        return None;
    }
    let inst = keep_dst.write(
        gpu_dev,
        gpu_queue,
        buf_tag,
        2,
        BufferUsages::VERTEX,
        bytemuck::cast_slice(draw.instances.as_slice()),
    );
    Some((vb, ib, inst, draws_within(buf_tag, &draw.draws, index_tally)))
}

/// A list's draws that stay inside its `indices`: one that reaches past
/// them fails the graphics checks, and bevy then closes the game (a fast
/// quality test match, 224 s in). The first few say which list and where.
fn draws_within(caption: &str, draw_list: &[[u32; 7]], indices: u64) -> Vec<[u32; 7]> {
    let over = |d: &[u32; 7]| u64::from(d[0]) + u64::from(d[1]) > indices;
    let Some(first) = draw_list.iter().find(|d| over(d)) else {
        return draw_list.to_vec();
    };
    static SAID: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    if SAID.fetch_add(1, std::sync::atomic::Ordering::Relaxed) < 8 {
        diag::warn!(
            World,
            "{caption}: {} of {} draws reach past its {indices} indices (first: from {} for {}); left out",
            draw_list.iter().filter(|d| over(d)).count(),
            draw_list.len(),
            first[0],
            first[1],
        );
    }
    draw_list.iter().filter(|d| !over(d)).copied().collect()
}

/// Draws issued and draws dropped for one kind of geometry in a frame.
#[derive(Clone, Copy, Default)]
struct Count {
    draws: u32,
    skip_n: u32,
}

impl Count {
    fn draw(&mut self) {
        self.draws = self.draws.saturating_add(1);
    }

    fn skip(&mut self) {
        self.skip_n = self.skip_n.saturating_add(1);
    }

    /// Turn a draw already counted into a skipped one (its index range was empty).
    fn retract(&mut self) {
        self.draws = self.draws.saturating_sub(1);
        self.skip();
    }
}

#[derive(Default)]
struct PassCounts {
    world: Count,
    smodel: Count,
    xmodel: Count,
    fx: Count,
}

/// The buffers, per-surface index ranges and pipeline of one instanced model family.
struct BodyKind<'a> {
    kind: ShadeKind,
    vertex: Option<&'a Buffer>,
    index: Option<&'a Buffer>,
    ranges: &'a [(u32, u32)],
    pipeline: Option<&'a bevy::render::render_resource::RenderPipeline>,
}

/// Binds `family` unless it is already bound and returns the index range plus the instance
/// range for item `slot`; `None` when any buffer, the pipeline or the surface is missing.
fn draw_body_surface<'w>(
    rp: &mut TrackedRenderPass<'w>,
    bound: &mut Option<ShadeKind>,
    family: &BodyKind<'w>,
    instances: Option<&'w Buffer>,
    surf_no: usize,
    slot_ix: usize,
) -> Option<(u32, u32, std::ops::Range<u32>)> {
    let (vtx_buf, index, inst_span, pipeline, &(first_ix, ix_count)) = (
        family.vertex?,
        family.index?,
        instances?,
        family.pipeline?,
        family.ranges.get(surf_no)?,
    );
    if *bound != Some(family.kind) {
        rp.set_render_pipeline(pipeline);
        rp.set_vertex_buffer(0, vtx_buf.slice(..));
        rp.set_vertex_buffer(1, inst_span.slice(..));
        rp.set_index_buffer(index.slice(..), IndexFormat::Uint32);
        *bound = Some(family.kind);
    }
    let first = u32::try_from(slot_ix).unwrap_or(u32::MAX);
    Some((first_ix, ix_count, first..first.saturating_add(1)))
}

/// A retained draw (`first`, `count`) that stays inside its buffer's
/// `indices`; the first few that do not are logged under `what`.
fn retained_within(what: &str, first: u32, ix_count: u32, idx_list: usize) -> bool {
    if u64::from(first) + u64::from(ix_count) <= idx_list as u64 {
        return true;
    }
    static SAID: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    if SAID.fetch_add(1, std::sync::atomic::Ordering::Relaxed) < 8 {
        diag::warn!(World, "{what} draw from {first} for {ix_count} reaches past its {idx_list} indices; left out");
    }
    false
}

#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
struct ModelMatrixRow {
    columns: [[f32; 4]; 4],
}

/// One mesh's GPU buffers, its per-surface (first index, index count) ranges and the sizes
/// it was uploaded at.
#[derive(Default)]
struct MeshGpu {
    vertex: Option<Buffer>,
    index: Option<Buffer>,
    ranges: Arc<Vec<(u32, u32)>>,
    vertex_count: usize,
    index_count: usize,
}

impl MeshGpu {
    fn size_matches(&self, vtx_n: usize, ix_n: usize) -> bool {
        self.vertex_count == vtx_n && self.index_count == ix_n
    }

    /// Drops the buffers and ranges, remembering only the new sizes.
    fn resize_for(&mut self, vtx_n: usize, ix_n: usize) {
        *self = Self {
            vertex_count: vtx_n,
            index_count: ix_n,
            ..Self::default()
        };
    }

    /// Brings the mesh in line with its source: re-uploaded when `stale` or when the source's
    /// sizes differ from what was uploaded, otherwise only the ranges are refreshed.
    fn sync<V: bytemuck::Pod>(
        &mut self,
        gpu_dev: &RenderDevice,
        kind: &str,
        vtx_list: &[V],
        index_list: &[u32],
        ranges: &Arc<Vec<(u32, u32)>>,
        stale: bool,
    ) {
        if stale {
            self.resize_for(vtx_list.len(), index_list.len());
            self.upload(gpu_dev, kind, vtx_list, index_list, ranges);
        } else {
            self.ranges = Arc::clone(ranges);
        }
    }

    /// Uploads the mesh; a mesh missing vertices or indices gets no buffers and no ranges.
    fn upload<V: bytemuck::Pod>(
        &mut self,
        gpu_dev: &RenderDevice,
        kind: &str,
        verts: &[V],
        ix_list: &[u32],
        ranges: &Arc<Vec<(u32, u32)>>,
    ) {
        if verts.is_empty() || ix_list.is_empty() {
            return;
        }
        let named = |suffix: &str| format!("t6_world_{kind}_{suffix}");
        self.vertex = Some(make_filled(gpu_dev, &named("vb"), BufferUsages::VERTEX, bytemuck::cast_slice(verts)));
        self.index = Some(make_filled(gpu_dev, &named("ib"), BufferUsages::INDEX, bytemuck::cast_slice(ix_list)));
        self.ranges = Arc::clone(ranges);
    }
}

#[derive(Resource, Default)]
struct OverlayGpuScene {
    generation: MaterialEpochId,
    world: MeshGpu,
    smodel: MeshGpu,
    xmodel: MeshGpu,
    body_rev: u64,

    loose_world_surfaces: Vec<u16>,
    world_surface_color: Arc<Vec<u32>>,
    world_surface_draw: Arc<Vec<u32>>,
    world_surface_lightmap: Arc<Vec<u8>>,
    t6_lighting: Option<[[f32; 4]; 4]>,
    t6_sun_sample_size_near: f32,
    t6_props: Arc<Vec<[u32; 4]>>,
    t6_prop_shine: Arc<Vec<[u32; 3]>>,
    world_surface_primary: Arc<Vec<u8>>,
    t6_lights: Arc<Vec<[f32; 16]>>,
    world_surface_layers: Arc<Vec<[u32; 4]>>,
    /// bo2zm: per world surface, normal and specular map slots and probe.
    world_surface_shine: Arc<Vec<[u32; 3]>>,
    t6_probe_slot: Option<u32>,
    t6_probes: Arc<Vec<[[f32; 4]; 3]>>,
    /// bo2zm: the world's layer texcoords, parallel to its vertices.
    world_layer_vertex: Option<Buffer>,
    world_layer_uvs: Arc<Vec<[f32; 4]>>,
    /// bo2zm: every textured pipeline the world surfaces and props need.
    t6_styles: Vec<ShadeKind>,
    /// bo2zm: the blended props' draw order (indices into `t6_props`, by
    /// sort key).
    t6_blended_props: Vec<usize>,
    t6_prop_instances: Arc<Vec<[f32; 28]>>,
    t6_prop_instance_buffer: Option<Buffer>,
    /// bo2mp: per prop draw (parallel to `t6_props`), its sphere in the
    /// world (centre, radius; a negative radius = unknown, always drawn),
    /// and what it was built from.
    prop_bounds: Vec<[f32; 4]>,
    t6_prop_spheres_key: [usize; 5],
    t6_sky_slot: Option<u32>,
}

/// bo2zm: one bind group per lightmap page the world surfaces read (page
/// view, sampler, T6 lighting constants), over the uploaded lightmap views.
#[derive(Resource, Default)]
struct OverlayLightGroups {
    generation: MaterialEpochId,
    sampler: Option<Sampler>,
    lighting_buf: Option<Buffer>,
    written: Option<[[f32; 4]; 4]>,
    bind_groups: HashMap<u8, BindGroup>,
    /// The sky: its cube map's slot and bind group (cube, sampler, lighting).
    sky: Option<(u32, BindGroup)>,
    /// The primary lights uniform (`T6_LIGHTS_BYTES`) and what it holds.
    lights: Option<Buffer>,
    lights_written: Option<Arc<Vec<[f32; 16]>>>,
    /// Group 2 for surfaces without a lightmap: lighting and lights only.
    lighting_group: Option<BindGroup>,
    /// The lights effects throw (`T6_DYN_BYTES`), rewritten every frame.
    dyn_lights: Option<Buffer>,
    /// bo2mp: the placed models' own light grid sets as asset_world last
    /// installed them (that install's generation): their 56-direction
    /// colours (a 3D texture, `t6_model_light_texels`) and spherical
    /// harmonics (three vec4 a model).
    model_lights: Option<(u64, bevy::render::render_resource::TextureView, Buffer)>,
}

/// bo2zm: the effect lights uniform: a count, then 16 lights of two vec4.
const T6_DYN_LIGHTS: usize = 16;
/// bo2mp: after the lights, the canopy wind table (`asset_core::t6_wind_table`).
const T6_DYN_WIND: usize = 17;
const T6_DYN_BYTES: u64 = (16 + T6_DYN_LIGHTS * 32 + T6_DYN_WIND * 16) as u64;

/// bo2zm: the primary lights uniform: 256 lights of four vec4 (a surface's
/// light index is a byte; Die Rise has 214), then the two fog rows.
const T6_LIGHT_ROWS: usize = 258;
const T6_LIGHTS_BYTES: u64 = (T6_LIGHT_ROWS * 64) as u64;

/// bo2zm: Black Ops II draw codes (see `asset_core::T6Draw::code`).
const T6_BLEND_MASK: u32 = 0x0f;
const T6_UNLIT: u32 = 0x10;
const T6_SHADOW_ONLY: u32 = 4;
const T6_NONE: u32 = u32::MAX;
/// Draw-code bit: a layered material; its vertex colour is layer weights,
/// not a tint (lightmapped styles only).
const T6_LAYERED: u32 = 0x40;
/// Draw-code bit: alpha tested (opaque styles only).
const T6_ALPHA_TEST: u32 = 0x80;
/// Draw-code bits: cull (8..9) and polygon offset (10..11).
const T6_RASTER_MASK: u32 = 0x0f00;

/// Style bit: the surface reads its lightmap (lit codes only).
const STYLE_LIGHTMAPPED: u32 = 0x20;
/// Style bit: a lit lightmapped opaque surface with a normal or specular
/// map gets BO2's shine (normal map, gloss highlight and a reflection
/// probe: group 1 bindings 8 and 9, plus group 3).
const STYLE_SHINE: u32 = 0x08;
/// The reflection probe table: per probe its `lightingSH`, three vec4.
const T6_MAX_PROBES: usize = 32;
const T6_PROBES_BYTES: u64 = (T6_MAX_PROBES * 48) as u64;
/// Draw-code bits: the unlit colour scale's log2 (12..14), extra layers
/// 1 and 2 (24..25, 26..27).
const T6_UNLIT_SCALE_MASK: u32 = 0x7000;
const T6_LAYER_SHIFT: u32 = 24;

/// The sort key a draw code carries (blended surfaces draw in its order).
fn t6_sort_key(code: u32) -> u32 {
    if code == T6_NONE { 0 } else { (code >> 16) & 0xff }
}

/// A draw code blends (alpha, add or multiply).
fn t6_blended(code: u32) -> bool {
    code != T6_NONE && matches!(code & T6_BLEND_MASK, 1..=3)
}

/// The pipeline style of a world surface: blend (bits 0..1), unlit (4),
/// lightmapped (5), layered (6), alpha tested (7), cull (8..9), polygon
/// offset (10..11). A surface with no BO2 draw is style 0.
fn world_style(code: u32, lightmapped: bool) -> u32 {
    if code == T6_NONE {
        return 0;
    }
    let mut style = code & (3 | T6_UNLIT | T6_RASTER_MASK);
    if code & T6_UNLIT != 0 {
        style |= code & T6_UNLIT_SCALE_MASK;
    }
    if lightmapped && code & T6_UNLIT == 0 {
        style |= STYLE_LIGHTMAPPED | (code & T6_LAYERED);
        // Lit lightmapped styles keep the extra layers in bits 12..15.
        style |= ((code >> T6_LAYER_SHIFT) & 0xf) << 12;
    }
    if code & T6_ALPHA_TEST != 0 && code & 3 == 0 {
        style |= T6_ALPHA_TEST;
    }
    // bo2mp: water and emissive flow mark their lit lightmapped style with
    // bit 2 (free in world styles: the blend keeps bits 0..1), the kind in
    // the layer bits.
    if code & T6_WATER != 0 && style & STYLE_LIGHTMAPPED != 0 {
        style = (style & !STYLE_LAYER_MASK) | STYLE_WATER;
    }
    // bo2mp: emissive flow (lava), likewise.
    if code & T6_FLOW != 0 && style & STYLE_LIGHTMAPPED != 0 && code & T6_UNLIT == 0 {
        style = (style & !STYLE_LAYER_MASK) | STYLE_FLOW;
    }
    // bo2mp: a test layer 1 (`t1c1`): bit 2 with layer 1's kind blend.
    if code & T6_TEST1 != 0 && style_layers(style) && (style >> 12) & 3 == 1 {
        style |= STYLE_WORLD_OWN;
    }
    style
}

/// Draw-code bit of a lit code: layer 1 is a test layer (`t1c1`).
const T6_TEST1: u32 = 0x2000;

/// A style without its extra layers (the base layer alone).
fn strip_layers(style: u32) -> u32 {
    if style_layers(style) {
        style & !(STYLE_LAYER_MASK | STYLE_WORLD_OWN)
    } else {
        style
    }
}

/// Draw-code bit: BO2's emissive flow (`cod7emissiveflow`: molten lava).
const T6_FLOW: u32 = 0x8000_0000;
/// Style bits of a lit lightmapped emissive flow surface.
const STYLE_FLOW: u32 = STYLE_WORLD_OWN | 0x2000;

/// A lit lightmapped emissive flow style.
fn style_flow(style: u32) -> bool {
    style & STYLE_LIGHTMAPPED != 0
        && style & T6_UNLIT == 0
        && style & (STYLE_WORLD_OWN | STYLE_LAYER_MASK) == STYLE_FLOW
}

/// Draw-code bit: BO2's water (`cod7water`, `cod7watershore`).
const T6_WATER: u32 = 0x4000_0000;
/// Style bit of a lit lightmapped world surface drawn by a shader of its
/// own (water: no layer bits; emissive flow: layer bits 0x2000), or of a
/// layered one whose layer 1 is a test layer (layer 1's kind blend).
const STYLE_WORLD_OWN: u32 = 0x04;
/// Style bits of a lit lightmapped water surface.
const STYLE_WATER: u32 = STYLE_WORLD_OWN;

/// A lit lightmapped water style.
fn style_water(style: u32) -> bool {
    style & STYLE_LIGHTMAPPED != 0
        && style & T6_UNLIT == 0
        && style & (STYLE_WORLD_OWN | STYLE_LAYER_MASK) == STYLE_WATER
}

/// Draw-code bit / prop style bit: a BO2 effect technique (props are never
/// lightmapped, so the bit is free in prop styles).
const T6_EFFECT: u32 = 0x20;
/// Draw-code bit / prop style bit: a BO2 particle cloud (bit 15 is free in
/// both).
const T6_CLOUD: u32 = 0x8000;
/// Prop style bit: an effect drawn with soft edges (BO2's zfeather pixel
/// shaders), reading the scene depth (group 3). Props are never layered,
/// so the layered bit is free in prop styles.
const STYLE_SOFT: u32 = 0x40;
/// Draw-code bit: BO2's objective technique (the box's question marks).
const T6_OBJECTIVE: u32 = 0x1000_0000;
/// Prop style bit: an objective surface (bit 2 is free in prop styles: the
/// blend keeps bits 0..1).
const STYLE_OBJECTIVE: u32 = 0x04;
/// Prop style bit: a projected decal (the blood a hit paints on a zombie,
/// lit): its box paints what lies inside it, from the scene depth (group 3).
const STYLE_DECAL: u32 = 0x1_0000;
/// Prop style bit: a ghost surface (Mob's Afterlife ghost hands, lit).
const STYLE_GHOST: u32 = 0x2_0000;
/// Prop style bit: a scrolling cloud sheet (unlit; Die Rise's cloud floor).
/// It leaves out the cloud bit, which an animated screen's style carries.
const STYLE_SHEET: u32 = 0x4_0000;
/// Draw-code bits of a cloud sheet: water and emissive flow together, which
/// no other code carries, on an unlit code with the cloud bit.
const T6_SHEET_MARK: u32 = 0xc000_0000;
/// bo2mp: draw-code bits of a lens flare (`sw4_3d_superflare_*`): an effect
/// with the cloud bit and water but not flow, which no other code carries.
const T6_FLARE_MARK: u32 = T6_EFFECT | T6_CLOUD | T6_WATER;
/// Prop style bit: a lens flare, its 25 ghosts laid out on screen by its
/// vertex shader; group 1 is the layered layout, its grime map in layer
/// 1's place.
const STYLE_FLARE: u32 = 0x8_0000;

/// bo2mp: a lens flare's code.
fn flare_code(code: u32) -> bool {
    code != T6_NONE && code & (T6_FLARE_MARK | T6_FLOW) == T6_FLARE_MARK
}

/// Draw-code bit: BO2's foliage technique (`treecanopy`): the vertex colour
/// is wind data and a grid-light scale, not a tint.
const T6_FOLIAGE: u32 = 0x2000_0000;
/// Prop style bit: a lit foliage surface (bit 12 is the unlit colour scale,
/// unused by lit styles).
const STYLE_FOLIAGE: u32 = 0x1000;

/// bo2mp: an animated screen's code (`sw4_3d_animated_unlit`): unlit, no
/// effect, bit 15 (a cloud's bit, which only effects use).
fn anim_code(code: u32) -> bool {
    code != T6_NONE
        && code & T6_UNLIT != 0
        && code & T6_EFFECT == 0
        && code & T6_CLOUD != 0
        && !sheet_code(code)
}
/// Style bit of an unlit animated screen drawn with its frame grid: its
/// group 1 is the layered layout, its constants in layer 1's place. Set
/// only where that group is bound (`prop_style` and `world_style` leave
/// it out); bit 15 is free in unlit styles that are no effect.
const STYLE_ANIM: u32 = 0x8000;

/// An animated screen style (unlit, no effect, `STYLE_ANIM`).
fn style_anim(style: u32) -> bool {
    style & T6_UNLIT != 0 && style & T6_EFFECT == 0 && style & STYLE_ANIM != 0
}

/// The pipeline style of a prop surface (never lightmapped or layered).
fn prop_style(code: u32) -> u32 {
    if code == T6_NONE {
        return 0;
    }
    if flare_code(code) {
        return (code & (3 | T6_UNLIT | T6_RASTER_MASK)) | T6_EFFECT | STYLE_FLARE;
    }
    let mut style = code & (3 | T6_UNLIT | T6_RASTER_MASK | T6_EFFECT | T6_CLOUD);
    if code & T6_UNLIT != 0 {
        style |= code & T6_UNLIT_SCALE_MASK;
    }
    // An animated screen's bit 15 is added where its group is bound.
    if anim_code(code) {
        style &= !STYLE_ANIM;
    }
    if code & T6_ALPHA_TEST != 0 && code & 3 == 0 {
        style |= T6_ALPHA_TEST;
    }
    if sheet_code(code) {
        return (style & !T6_CLOUD) | STYLE_SHEET;
    }
    if ghost_code(code) {
        if code & T6_UNLIT == 0 {
            style |= STYLE_GHOST;
        }
        return style;
    }
    if decal_code(code) {
        return style | STYLE_DECAL;
    }
    if code & T6_OBJECTIVE != 0 {
        style |= STYLE_OBJECTIVE;
    }
    if code & T6_FOLIAGE != 0 && code & T6_UNLIT == 0 {
        style |= STYLE_FOLIAGE;
    }
    if code & T6_FLOW != 0 && code & T6_UNLIT == 0 {
        style |= STYLE_PROP_FLOW;
    }
    if code & T6_TILE != 0 && code & T6_UNLIT == 0 {
        style |= STYLE_PROP_TILE;
    }
    // bo2mp: a tattered flag tests its own alpha (the mixed edge), not the
    // diffuse's (gloss). A tile blend sets both bits (tile and flag).
    if code & T6_FLAG != 0 && code & T6_UNLIT == 0 {
        style = (style & !(T6_ALPHA_TEST)) | STYLE_PROP_FLAG;
    }
    style
}

/// bo2mp: the solid pass of a blended lit foliage code. Black Ops II draws
/// a tree canopy twice: a depth pass that discards below 128/255 alpha
/// (technique 0), then the lit pass blended (src alpha, inverse src alpha),
/// depth test less-equal, no depth write (state bits of techniques 5/6),
/// alpha saturate(2 * texel alpha). Drawn here as that depth pass with its
/// colour (the texels it keeps come out solid in the lit pass too), then
/// the blended pass: the soft edges blend over what is behind.
fn foliage_solid_style(code: u32) -> Option<u32> {
    (code != T6_NONE
        && code & T6_FOLIAGE != 0
        && !ghost_code(code)
        && code & T6_UNLIT == 0
        && code & T6_BLEND_MASK == 1)
        .then(|| (prop_style(code) & !3) | T6_ALPHA_TEST)
}

/// The prop kind among emissive tile, tattered flag and tile blend.
fn prop_own_kind(style: u32) -> u32 {
    if style & T6_UNLIT != 0 {
        0
    } else {
        style & (STYLE_PROP_TILE | STYLE_PROP_FLAG)
    }
}

/// Draw-code bit of a lit code: BO2's tattered flag (`flag_tatters`).
const T6_FLAG: u32 = 0x4000;
/// Prop style bit: a lit tattered flag (bit 6 is the soft bit of effects
/// only, which are unlit).
const STYLE_PROP_FLAG: u32 = 0x40;

/// Draw-code bit of a lit code: BO2's emissive tile (`cod7_emissive_tile`;
/// bit 12 is the unlit colour scale in unlit codes).
const T6_TILE: u32 = 0x1000;
/// Prop style bit: a lit emissive tile surface (bit 14, likewise free).
const STYLE_PROP_TILE: u32 = 0x4000;

/// Prop style bit: a lit emissive flow surface (lava; bit 13 is the unlit
/// colour scale, unused by lit styles).
const STYLE_PROP_FLOW: u32 = 0x2000;

/// Draw-code bit / prop style bit of a lit code: BO2's view-model glass
/// (`sw4_3d_viewmodel_transparent`, the riot shield's window). It is the
/// cloud bit, which only effects carry, and they are unlit; `prop_style`
/// keeps it with the cloud mask.
const T6_GLASS: u32 = 0x8000;

/// bo2zm: draws one dynamic list (marks, thrown and fired things, effects,
/// the gun) into a pass: the draws `keep` admits, opaque first, then blended by sort key;
/// effects with soft edges when `soft` (the scene depth) is given. Returns
/// the draws made.
#[allow(clippy::too_many_arguments)]
fn draw_dynamic_list<'w>(
    rp: &mut TrackedRenderPass<'w>,
    buffers: Option<&'w (Buffer, Buffer, Buffer, Vec<[u32; 7]>)>,
    lighting_group: Option<&'w BindGroup>,
    depth_layer: DepthLayer,
    viewport: [f32; 4],
    keep: &dyn Fn(u32) -> bool,
    soft: Option<&'w BindGroup>,
    pipes: &'w PipelineCache,
    pipelines: &HashMap<ShadeKind, CachedRenderPipelineId>,
    wtex: &'w OverlayTextureGroups,
) -> u32 {
    let (Some((vb, ibuf, inst, draws)), Some(lighting_group)) = (buffers, lighting_group) else {
        return 0;
    };
    apply_viewport(rp, viewport, depth_layer.viewport_depth());
    rp.set_bind_group(2, lighting_group, &[]);
    rp.set_vertex_buffer(0, vb.slice(..));
    rp.set_vertex_buffer(1, inst.slice(..));
    rp.set_index_buffer(ibuf.slice(..), IndexFormat::Uint32);
    // bo2mp: a tree canopy on a script model (a swaying fxanim tree) also
    // draws its solid pass with the opaque draws (foliage_solid_style),
    // then its blended pass, as the placed trees do.
    let mut order: Vec<(usize, Option<u32>)> = (0..draws.len())
        .filter(|&i| keep(draws[i][3]))
        .flat_map(|i| {
            let solid = foliage_solid_style(draws[i][3]).map(|s| (i, Some(s)));
            solid.into_iter().chain(std::iter::once((i, None)))
        })
        .collect();
    order.sort_by_key(|&(i, solid)| (solid.is_none() && t6_blended(draws[i][3]), t6_sort_key(draws[i][3])));
    // bo2mp: back-to-back draws of one look and placement whose indices
    // follow on are one draw (marks lit alike share a placement).
    let mut merged: Vec<([u32; 7], Option<u32>)> = Vec::with_capacity(order.len());
    for (i, solid) in order {
        let d = draws[i];
        if let Some((last, last_solid)) = merged.last_mut()
            && *last_solid == solid
            && last[2..] == d[2..]
            && last[0].checked_add(last[1]) == Some(d[0])
        {
            last[1] = last[1].saturating_add(d[1]);
            continue;
        }
        merged.push((d, solid));
    }
    let mut bound: Option<ShadeKind> = None;
    let mut bound_slot = None;
    let mut drawn = 0u32;
    for ([first_ix, ix_count, slot, code, instance, normal, specular], solid) in merged {
        if ix_count == 0 || (code != T6_NONE && code & T6_BLEND_MASK >= T6_SHADOW_ONLY) {
            continue;
        }
        let shine_group = (prop_shine_code(code) && (normal != u32::MAX || specular != u32::MAX))
            .then(|| wtex.probes.as_ref())
            .flatten()
            .and_then(|_| {
                wtex
                    .shine_groups
                    .get(&(slot, normal, specular, u32::MAX, [u32::MAX; 3]))
            });
        let soft_here = soft.filter(|_| solid.is_none() && (soft_code(code) || decal_code(code)));
        // A projected decal without the scene depth has nothing to paint
        // (its box itself is never drawn).
        if decal_code(code) && soft_here.is_none() {
            continue;
        }
        // (A decal's style already says it reads the scene depth.)
        let style = solid.unwrap_or_else(|| prop_style(code))
            | if shine_group.is_some() { STYLE_SHINE } else { 0 }
            | if soft_here.is_some() && !decal_code(code) { STYLE_SOFT } else { 0 };
        let key = ShadeKind::PropTextured(style);
        let pipeline = pipelines.get(&key).and_then(|&id| pipes.get_render_pipeline(id));
        // bo2mp: a lens flare draws with its grime map (the sixth word).
        let flare = flare_code(code);
        let flare_group = flare.then(|| wtex.layer_groups.get(&(slot, normal, u32::MAX))).flatten();
        let (Some(pipe), Some(view_bg)) = (
            pipeline,
            if flare { flare_group } else { shine_group.or_else(|| wtex.bind_groups.get(&slot)) },
        ) else {
            // BO2MP_T6_DRAWLOG=1: why a draw was left out.
            static LOG: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
            if *LOG.get_or_init(|| std::env::var_os("BO2MP_T6_DRAWLOG").is_some()) {
                diag::info!(
                    World,
                    "bo2zm draw skipped: slot {slot} code {code:#x} style {style:#x} pipeline {} ready {} texture {}",
                    pipelines.contains_key(&key),
                    pipeline.is_some(),
                    wtex.bind_groups.contains_key(&slot)
                );
            }
            continue;
        };
        if bound != Some(key) {
            rp.set_render_pipeline(pipe);
            if shine_group.is_some()
                && let Some(shine_probes) = wtex.probes.as_ref().map(|p| &p.1)
            {
                rp.set_bind_group(3, shine_probes, &[]);
            }
            if let Some(soft) = soft_here {
                rp.set_bind_group(3, soft, &[]);
            }
            bound = Some(key);
        }
        let slot_key = (slot, shine_group.or(flare_group).map(|_| [normal, specular]));
        if bound_slot != Some(slot_key) {
            rp.set_bind_group(1, view_bg, &[]);
            bound_slot = Some(slot_key);
        }
        rp.draw_indexed(first_ix..first_ix.saturating_add(ix_count), 0, instance..instance + 1);
        drawn += 1;
    }
    drawn
}

/// bo2mp: one draw of opaque props: one surface at `instances`
/// placements, which sit together in this frame's own placement rows.
struct PropBatch<'a> {
    style: u32,
    slot_tag: (u32, Option<[u32; 3]>),
    pipeline: &'a bevy::render::render_resource::RenderPipeline,
    bind_group: &'a BindGroup,
    /// The reflection probes, for a surface with shine.
    probes: Option<&'a BindGroup>,
    first_ix: u32,
    ix_count: u32,
    instances: std::ops::Range<u32>,
}

/// bo2mp: the opaque props this view draws, each surface's placements in
/// one draw (Nuketown drew ~1,450 a frame one placement at a time), sorted
/// by pipeline and texture so they switch less; `rows` gets the placements
/// in the order the draws read them. Returns the draws, then how many
/// props were hidden by distance and how many were out of view.
fn t6_opaque_prop_batches<'a>(
    geometry: &OverlayGpuScene,
    world_textures: &'a OverlayTextureGroups,
    pipes: &'a PipelineCache,
    pipelines: &HashMap<ShadeKind, CachedRenderPipelineId>,
    view: &ExtractedView,
    rows: &mut Vec<[f32; 28]>,
) -> (Vec<PropBatch<'a>>, u32, u32) {
    // The game's own per-placement draw distance (the fast quality
    // setting hides props at 60% of it).
    let eye = view.world_from_view.translation();
    let cull_scale = if asset_core::t6_fast() { 0.6 } else { 1.0 };
    let view_planes =
        view_volume_planes(view.clip_from_view * view.world_from_view.to_matrix().inverse());
    let (mut culled, mut unseen) = (0u32, 0u32);
    let mut each: Vec<PropBatch<'a>> = Vec::new();
    for (i, &[surface, slot, code, instance]) in geometry.t6_props.iter().enumerate() {
        // A blended canopy also draws here, as its depth pass.
        let solid = foliage_solid_style(code);
        if (t6_blended(code) && solid.is_none())
            || (code != T6_NONE && code & T6_BLEND_MASK >= T6_SHADOW_ONLY)
        {
            continue;
        }
        let Some(row) = geometry.t6_prop_instances.get(instance as usize) else {
            continue;
        };
        if row[22] > 0.0 {
            let d = Vec3::new(row[12], row[13], row[14]) - eye;
            let cull = row[22] * cull_scale;
            if d.length_squared() > cull * cull {
                culled += 1;
                continue;
            }
        }
        if let (Some(planes), Some(&[x, y, z, radius])) =
            (view_planes.as_ref(), geometry.prop_bounds.get(i))
            && radius >= 0.0
            && planes
                .iter()
                .any(|plane| plane.truncate().dot(Vec3::new(x, y, z)) + plane.w < -radius)
        {
            unseen += 1;
            continue;
        }
        let Some(&(first_ix, ix_count)) = geometry.smodel.ranges.get(surface as usize) else {
            continue;
        };
        // Shine: a lit opaque prop with a normal or specular map.
        let shine = geometry.t6_prop_shine.get(i).copied().unwrap_or([u32::MAX; 3]);
        let flow = prop_style(code) & (STYLE_PROP_FLOW | STYLE_PROP_TILE) != 0
            || (prop_style(code) & STYLE_PROP_FLAG != 0 && code & T6_UNLIT == 0);
        let probes = world_textures.probes.as_ref().map(|(_, probes)| probes);
        let shine_group = (prop_shine_code(code) && (shine[0] != u32::MAX || shine[1] != u32::MAX))
            .then_some(probes)
            .flatten()
            .and_then(|_| {
                let aux = if flow { shine[2] } else { u32::MAX };
                world_textures.shine_groups.get(&(slot, shine[0], shine[1], aux, [u32::MAX; 3]))
            });
        // bo2mp: an animated screen with its constants.
        let anim_group = anim_code(code)
            .then(|| world_textures.layer_groups.get(&(slot, shine[2], u32::MAX)))
            .flatten();
        let style = solid.unwrap_or_else(|| prop_style(code))
            | if shine_group.is_some() { STYLE_SHINE } else { 0 }
            | if anim_group.is_some() { STYLE_ANIM } else { 0 };
        let pipeline = pipelines
            .get(&ShadeKind::PropTextured(style))
            .and_then(|&id| pipes.get_render_pipeline(id));
        let (Some(pso), Some(bind_group)) = (
            pipeline,
            shine_group
                .or(anim_group)
                .or_else(|| world_textures.bind_groups.get(&slot)),
        ) else {
            continue;
        };
        if ix_count == 0 || !retained_within("prop", first_ix, ix_count, geometry.smodel.index_count) {
            continue;
        }
        each.push(PropBatch {
            style,
            slot_tag: (slot, shine_group.or(anim_group).map(|_| shine)),
            pipeline: pso,
            bind_group,
            probes: shine_group.and(probes),
            first_ix,
            ix_count,
            instances: instance..instance + 1,
        });
    }
    each.sort_unstable_by_key(|b| (b.style, b.slot_tag, b.first_ix, b.ix_count, b.instances.start));
    let mut batches: Vec<PropBatch<'a>> = Vec::new();
    for b in each {
        let at = rows.len() as u32;
        rows.push(geometry.t6_prop_instances[b.instances.start as usize]);
        match batches.last_mut() {
            Some(last)
                if (last.style, last.slot_tag, last.first_ix, last.ix_count)
                    == (b.style, b.slot_tag, b.first_ix, b.ix_count) =>
            {
                last.instances.end = at + 1;
            }
            _ => batches.push(PropBatch {
                instances: at..at + 1,
                ..b
            }),
        }
    }
    (batches, culled, unseen)
}

/// bo2zm: an effect code that draws with soft edges in the world (BO2's
/// zfeather techniques; a material without `featherParms` reads none of
/// it): blended or added, not a particle cloud.
fn soft_code(code: u32) -> bool {
    code != T6_NONE && code & T6_EFFECT != 0 && code & T6_CLOUD == 0 && t6_blended(code)
}

/// bo2zm 27c: a cloud sheet code (Die Rise's `scrolling_zfeather` cloud
/// floor): its texture drifts, and with the scene depth it fades where what
/// is behind it is near.
fn sheet_code(code: u32) -> bool {
    code != T6_NONE && code & T6_CLOUD != 0 && code & T6_SHEET_MARK == T6_SHEET_MARK
}

/// bo2zm: a ghost code (Mob's Afterlife ghost hands): objective and foliage
/// together.
fn ghost_code(code: u32) -> bool {
    code != T6_NONE && code & (T6_OBJECTIVE | T6_FOLIAGE) == (T6_OBJECTIVE | T6_FOLIAGE)
}

/// bo2zm M4 retest 5: a projected decal code: lit, not an effect, with the
/// emissive tile, flag and test layer bits all set (no real surface has
/// them together).
fn decal_code(code: u32) -> bool {
    code != T6_NONE
        && code & T6_UNLIT == 0
        && code & T6_EFFECT == 0
        && code & 0x7000 == 0x7000
        && !ghost_code(code)
}

/// bo2zm: a prop or dynamic code that can draw with shine: lit, opaque or
/// blended (glass), not an effect, a projected decal or a ghost.
fn prop_shine_code(code: u32) -> bool {
    code != T6_NONE
        && code & T6_UNLIT == 0
        && code & 3 <= 1
        && code & T6_EFFECT == 0
        && !decal_code(code)
        && !ghost_code(code)
}

/// Style bits of a lit lightmapped style's extra layers (12..15).
const STYLE_LAYER_MASK: u32 = 0xf000;

/// Whether a style has extra layers: the lit lightmap styles with layer kinds.
fn style_layers(style: u32) -> bool {
    style & STYLE_LIGHTMAPPED != 0
        && style & T6_UNLIT == 0
        && style & STYLE_LAYER_MASK != 0
        && !style_water(style)
        && !style_flow(style)
}

/// Every textured pipeline the codes need: each lit world code with and
/// without its lightmap, each prop code.
fn t6_styles(world: &[u32], props: &[[u32; 4]]) -> Vec<ShadeKind> {
    let mut styles = std::collections::BTreeSet::new();
    for &code in world {
        if code != T6_NONE && code & T6_BLEND_MASK >= T6_SHADOW_ONLY {
            continue;
        }
        styles.insert(ShadeKind::WorldTextured(world_style(code, false)));
        styles.insert(ShadeKind::WorldTextured(world_style(code, true)));
        if anim_code(code) {
            styles.insert(ShadeKind::WorldTextured(world_style(code, false) | STYLE_ANIM));
        }
        // Layered: also the base-layer-only style (layer maps missing).
        styles.insert(ShadeKind::WorldTextured(strip_layers(world_style(
            code, true,
        ))));
        // Opaque lit lightmapped: also with shine.
        let lit = strip_layers(world_style(code, true));
        if lit & STYLE_LIGHTMAPPED != 0 && lit & 3 <= 1 {
            styles.insert(ShadeKind::WorldTextured(lit | STYLE_SHINE));
        }
        // bo2mp: water and emissive flow draw only with their shine maps
        // and probes.
        if style_water(world_style(code, true)) || style_flow(world_style(code, true)) {
            styles.insert(ShadeKind::WorldTextured(
                world_style(code, true) | STYLE_SHINE,
            ));
        }
    }
    for draw in props {
        let code = draw[2];
        if code != T6_NONE && code & T6_BLEND_MASK >= T6_SHADOW_ONLY {
            continue;
        }
        if prop_shine_code(code) {
            styles.insert(ShadeKind::PropTextured(prop_style(code) | STYLE_SHINE));
        }
        styles.insert(ShadeKind::PropTextured(prop_style(code)));
        if sheet_code(code) {
            styles.insert(ShadeKind::PropTextured(prop_style(code) | STYLE_SOFT));
        }
        if anim_code(code) {
            styles.insert(ShadeKind::PropTextured(prop_style(code) | STYLE_ANIM));
        }
        if let Some(solid) = foliage_solid_style(code) {
            if prop_shine_code(code) {
                styles.insert(ShadeKind::PropTextured(solid | STYLE_SHINE));
            }
            styles.insert(ShadeKind::PropTextured(solid));
        }
    }
    styles.into_iter().collect()
}

/// bo2mp: a shine bind group's slots: colour, normal, specular, aux (a
/// flow's remap or a sea's control map), then a sea's shore picture and
/// white-water noise and mask (`u32::MAX` = the stand-in).
type ShineKey = (u32, u32, u32, u32, [u32; 3]);

/// bo2zm: one bind group per colour map slot the world surfaces sample,
/// over the uploaded material image views; rebuilt when the catalog
/// generation changes.
#[derive(Resource, Default)]
struct OverlayTextureGroups {
    generation: MaterialEpochId,
    sampler: Option<Sampler>,
    /// The anisotropy the sampler was made with (his texture filtering).
    aniso: u16,
    bind_groups: HashMap<u32, BindGroup>,
    /// bo2zm: layered surfaces' group 1, by (colour, layer 1, layer 2)
    /// slots; a missing layer reuses the colour map (its weight is still
    /// applied, so the layer kind decides whether one is bound).
    layer_groups: HashMap<(u32, u32, u32), BindGroup>,
    /// bo2zm: shine surfaces' group 1, by (colour, normal, specular)
    /// slots, then two extra pictures (emissive flow's remap; a sea's control
    /// map and shore picture); a missing map binds a flat normal / black specular.
    shine_groups: HashMap<ShineKey, BindGroup>,
    /// bo2zm: 1x1 stand-ins: a flat normal, a black specular map.
    flat_normal: Option<bevy::render::render_resource::TextureView>,
    no_specular: Option<bevy::render::render_resource::TextureView>,
    /// bo2zm: group 3 of shine surfaces: the probes' cube array, its
    /// sampler and their lighting table, for the cube array's slot.
    probes: Option<(u32, BindGroup)>,
    probe_table: Option<Buffer>,
    probe_written: Option<Arc<Vec<[[f32; 4]; 3]>>>,
}

#[derive(Resource)]
struct OverlayPipelines {
    camera_layout: BindGroupLayoutDescriptor,
    texture_layout: BindGroupLayoutDescriptor,
    lightmap_layout: BindGroupLayoutDescriptor,
    /// bo2zm: group 2 without a lightmap (bindings 2, 3).
    lighting_layout: BindGroupLayoutDescriptor,
    /// bo2zm: group 1 of layered surfaces: colour map, sampler, layer 1
    /// and 2 colour maps (bindings 0, 1, 6, 7).
    layered_texture_layout: BindGroupLayoutDescriptor,
    sky_layout: BindGroupLayoutDescriptor,
    /// bo2zm: group 1 of shine surfaces: colour map, sampler, normal and
    /// specular maps (bindings 0, 1, 8, 9).
    shine_texture_layout: BindGroupLayoutDescriptor,
    /// bo2zm: group 3 of shine surfaces: the reflection probes' cube
    /// array, its sampler, their lighting table (bindings 0..2).
    probe_layout: BindGroupLayoutDescriptor,
    /// bo2zm: group 3 of soft effects: the scene depth (binding 0).
    soft_layout: BindGroupLayoutDescriptor,
    program: Handle<Shader>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct OverlayVariant {
    colour_format: TextureFormat,
    sample_n: u32,
    shade: ShadeKind,
}

/// Which kind of geometry a pipeline draws; the variants that carry a `u16` carry the
/// material style bits too.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
enum ShadeKind {
    World,
    /// A world surface with a colour map, keyed by `world_style`.
    WorldTextured(u32),
    /// A Black Ops II prop with a colour map, keyed by `prop_style`.
    PropTextured(u32),
    /// The sky: one full-screen triangle at the far plane, sampling the sky cube map.
    Sky,
    Smodel,
    XModel,
}

impl ShadeKind {
    /// The style bits of a textured kind; the others have none.
    fn style(self) -> Option<u32> {
        match self {
            Self::WorldTextured(style) | Self::PropTextured(style) => Some(style),
            _ => None,
        }
    }

    /// The preprocessor flags both shader stages are built with, in the order the shader
    /// reads them: what a textured kind's style bits switch on.
    fn shader_defs(self) -> Vec<ShaderDefVal> {
        let Some(style) = self.style() else {
            return Vec::new();
        };
        let prop = matches!(self, Self::PropTextured(_));
        let has = |bit: u32| style & bit != 0;
        let blend = style & 3;
        let unlit = has(T6_UNLIT);
        let effect = has(T6_EFFECT);
        let shine = has(STYLE_SHINE);
        // Props that are lit, and props that are neither unlit nor an effect.
        let lit_prop = prop && !unlit;
        let plain_prop = lit_prop && !effect;
        let own_kind = match prop_own_kind(style) {
            STYLE_PROP_TILE => "TILE",
            STYLE_PROP_FLAG => "FLAG",
            0x4040 => "TILE_BLEND",
            _ => "",
        };
        let flags = [
            (unlit, "UNLIT"),
            (unlit && blend == 3, "MULTIPLY"),
            (unlit && blend == 2, "ADD"),
            (unlit && style_anim(style), "ANIM"),
            (has(T6_ALPHA_TEST), "ALPHA_TEST"),
            (has(STYLE_LIGHTMAPPED) && has(T6_LAYERED), "UNTINTED"),
            (shine, "SHINE"),
            (shine && style_water(style), "WATER"),
            (shine && style_flow(style), "FLOW"),
            (prop && has(STYLE_OBJECTIVE), "OBJECTIVE"),
            (lit_prop && has(STYLE_FOLIAGE), "FOLIAGE"),
            (lit_prop && has(STYLE_PROP_FLOW) && shine, "FLOW"),
            (plain_prop && shine && !own_kind.is_empty(), own_kind),
            (plain_prop && has(T6_GLASS), "GLASS"),
            // bo2zm: Mob's Afterlife ghost hands.
            (prop && has(STYLE_GHOST), "GHOST"),
            // bo2mp: a lens flare (Nuketown's sun).
            (prop && has(STYLE_FLARE), "FLARE"),
            (prop && effect, "EFFECT"),
            (prop && effect && blend == 2, "EFFECT_ADD"),
            (prop && effect && blend == 3, "EFFECT_MULTIPLY"),
            (prop && effect && has(T6_CLOUD), "CLOUD"),
            (prop && effect && has(STYLE_SOFT), "SOFT"),
            // bo2zm: a scrolling cloud sheet (Die Rise's cloud floor), soft where asked.
            (prop && has(STYLE_SHEET), "SCROLL"),
            (prop && has(STYLE_SHEET) && has(STYLE_SOFT), "SOFT"),
            // bo2zm: a projected decal (the blood a hit paints on a zombie), lit.
            (prop && has(STYLE_DECAL) && !unlit, "DECAL"),
            (prop && has(STYLE_DECAL) && !unlit, "SOFT"),
        ];
        // The unlit colour scale (`scaleRGB`), a power of two.
        let mut defs = vec![ShaderDefVal::UInt(
            "UNLIT_SCALE".into(),
            1u32 << ((style >> 12) & 7),
        )];
        defs.extend(
            flags
                .into_iter()
                .filter(|&(on, _)| on)
                .map(|(_, name)| ShaderDefVal::from(name)),
        );
        if style_layers(style) {
            defs.push("LAYERS".into());
            for (layer, shift) in [(1, 12), (2, 14)] {
                match (style >> shift) & 3 {
                    1 if layer == 1 && has(STYLE_WORLD_OWN) => defs.push("LAYER1_TEST".into()),
                    1 => defs.push(format!("LAYER{layer}_BLEND").into()),
                    2 => defs.push(format!("LAYER{layer}_MULTIPLY").into()),
                    3 => defs.push(format!("LAYER{layer}_ADD").into()),
                    _ => {}
                }
            }
        }
        defs
    }

    fn vertex_entry(self) -> &'static str {
        match self {
            Self::World | Self::WorldTextured(_) => "vertex",
            Self::PropTextured(_) => "vertex_prop",
            Self::Sky => "vertex_sky",
            Self::Smodel | Self::XModel => "vertex_smodel",
        }
    }

    fn fragment_entry(self) -> &'static str {
        match self {
            Self::Sky => "fragment_sky",
            Self::PropTextured(_) => "fragment_prop",
            Self::WorldTextured(style) if style & STYLE_LIGHTMAPPED != 0 => "fragment_lightmapped",
            Self::WorldTextured(_) => "fragment_textured",
            _ => "fragment",
        }
    }

    fn vertex_buffers(self) -> Vec<VertexBufferLayout> {
        match self {
            Self::WorldTextured(style) if style_layers(style) => {
                vec![level_vertex_rows(), level_layer_rows()]
            }
            Self::World | Self::WorldTextured(_) => vec![level_vertex_rows()],
            Self::PropTextured(_) => vec![model_vertex_rows(), prop_matrix_rows()],
            Self::Sky => Vec::new(),
            Self::Smodel | Self::XModel => vec![model_vertex_rows(), model_matrix_rows()],
        }
    }

    /// bo2zm: the material's own cull (as the engine maps IW GFXS_CULL_*: back = wgpu back
    /// with clockwise front faces).
    fn cull_mode(self) -> Option<Face> {
        // Two bits: 0 and 3 keep both sides, 1 drops the back, 2 the front.
        let by_bits = [None, Some(Face::Back), Some(Face::Front), None];
        by_bits[((self.style()? >> 8) & 3) as usize]
    }

    fn depth_write(self) -> bool {
        match self {
            // bo2mp: water blends here (the game refracts instead) but stays solid in depth,
            // as the game's: the sky, drawn last where nothing is, must not cover an open sea.
            Self::WorldTextured(style) if style_water(style) => true,
            // The blended ones leave depth alone.
            Self::WorldTextured(style) | Self::PropTextured(style) => style & 3 == 0,
            Self::Sky => false,
            _ => true,
        }
    }

    /// bo2zm: decals' polygon offset, the engine's IW defaults turned for reverse Z.
    fn depth_bias(self) -> DepthBiasState {
        let offset = self.style().map_or(0, |style| u32::from((style >> 10) & 3));
        if offset == 0 {
            return DepthBiasState::default();
        }
        let (slope, steps) = render_material::gpu_default_depth_slope_offsets(offset);
        // Reverse Z: nearer is bigger, so the pull goes the other way.
        DepthBiasState { constant: -steps, slope_scale: -slope, clamp: 0.0 }
    }

    /// Clockwise triangles face the camera; the style picks which side is dropped.
    fn raster_rules(self) -> PrimitiveState {
        PrimitiveState { front_face: FrontFace::Cw, cull_mode: self.cull_mode(), ..Default::default() }
    }

    /// The scene depth, reverse Z (nearer passes as greater or equal).
    fn depth_rules(self) -> DepthStencilState {
        let near_wins = Some(CompareFunction::GreaterEqual);
        let write = Some(self.depth_write());
        DepthStencilState { format: SCENE_DEPTH_FORMAT, depth_write_enabled: write, depth_compare: near_wins, stencil: Default::default(), bias: self.depth_bias() }
    }
}

impl OverlayPipelines {
    /// The bind groups, by slot, a kind's pipeline is laid out over.
    fn group_layouts(&self, shade: ShadeKind) -> Vec<BindGroupLayoutDescriptor> {
        use ShadeKind::{PropTextured, Sky, WorldTextured};
        let view = &self.camera_layout;
        let groups = match shade {
            Sky => vec![view, &self.sky_layout],
            // bo2mp: a lens flare: its grime map in layer 1's place.
            PropTextured(style) if style & STYLE_FLARE != 0 => {
                vec![view, &self.layered_texture_layout, &self.lighting_layout]
            }
            // bo2mp: an animated screen: its constants in layer 1's place.
            WorldTextured(style) | PropTextured(style) if style_anim(style) => {
                vec![view, &self.layered_texture_layout, &self.lighting_layout]
            }
            WorldTextured(style) if style_layers(style) => {
                vec![view, &self.layered_texture_layout, &self.lightmap_layout]
            }
            WorldTextured(style) if style & STYLE_SHINE != 0 => {
                vec![view, &self.shine_texture_layout, &self.lightmap_layout, &self.probe_layout]
            }
            WorldTextured(style) if style & STYLE_LIGHTMAPPED != 0 => {
                vec![view, &self.texture_layout, &self.lightmap_layout]
            }
            PropTextured(style)
                if (style & STYLE_SOFT != 0 && (style & T6_EFFECT != 0 || style & STYLE_SHEET != 0))
                    || style & STYLE_DECAL != 0 =>
            {
                vec![view, &self.texture_layout, &self.lighting_layout, &self.soft_layout]
            }
            PropTextured(style) if style & STYLE_SHINE != 0 => {
                vec![view, &self.shine_texture_layout, &self.lighting_layout, &self.probe_layout]
            }
            WorldTextured(_) | PropTextured(_) => {
                vec![view, &self.texture_layout, &self.lighting_layout]
            }
            _ => vec![view],
        };
        groups.into_iter().cloned().collect()
    }
}

impl OverlayPipelines {
    /// The vertex half of a variant: its entry point and the rows it reads.
    fn vertex_half(&self, shade_kind: ShadeKind, def_list: Vec<ShaderDefVal>) -> VertexState {
        let entry = Some(shade_kind.vertex_entry().into());
        VertexState { shader: self.program.clone(), shader_defs: def_list, entry_point: entry, buffers: shade_kind.vertex_buffers() }
    }

    /// The fragment half: one colour target in the camera's format, blended as the style says.
    fn fragment_half(&self, shade: ShadeKind, frag_defs: Vec<ShaderDefVal>, format: TextureFormat) -> FragmentState {
        let blend = shade.style().and_then(t6_blend_state);
        let col_state = ColorTargetState { format, blend, write_mask: ColorWrites::ALL };
        let entry = Some(shade.fragment_entry().into());
        FragmentState { shader: self.program.clone(), shader_defs: frag_defs, entry_point: entry, targets: vec![Some(col_state)] }
    }
}

impl SpecializedRenderPipeline for OverlayPipelines {
    type Key = OverlayVariant;

    fn specialize(&self, ov_variant: OverlayVariant) -> RenderPipelineDescriptor {
        let shade = ov_variant.shade;
        let defs = shade.shader_defs();
        let vertex = self.vertex_half(shade, defs.clone());
        let fragment = self.fragment_half(shade, defs, ov_variant.colour_format);
        let multisample = MultisampleState { count: ov_variant.sample_n, ..Default::default() };
        RenderPipelineDescriptor {
            label: Some("t6_world".into()),
            layout: self.group_layouts(shade),
            immediate_size: 0,
            vertex,
            fragment: Some(fragment),
            primitive: shade.raster_rules(),
            depth_stencil: Some(shade.depth_rules()),
            multisample,
            zero_initialize_workgroup_memory: false,
        }
    }
}

/// The colour blend a Black Ops II draw code asks for: 1 blends by alpha, 2 adds, 3
/// multiplies the colour already there; 0 is opaque and has no blend. Alpha is never
/// touched.
fn t6_blend_state(code: u32) -> Option<BlendState> {
    let (src_factor, dst_factor) = match code & 3 {
        1 => (BlendFactor::SrcAlpha, BlendFactor::OneMinusSrcAlpha),
        2 => (BlendFactor::SrcAlpha, BlendFactor::One),
        3 => (BlendFactor::Dst, BlendFactor::Zero),
        _ => return None,
    };
    let colour = BlendComponent { src_factor, dst_factor, operation: BlendOperation::Add };
    // Alpha keeps what the target already holds.
    let kept = BlendComponent { src_factor: BlendFactor::Zero, dst_factor: BlendFactor::One, operation: BlendOperation::Add };
    Some(BlendState { color: colour, alpha: kept })
}

#[derive(Component)]
struct OverlayCameraBinds {
    camera_group: BindGroup,
    level_pipeline: CachedRenderPipelineId,
    /// bo2zm: the textured world and prop pipelines, by key.
    coded_pipelines: HashMap<ShadeKind, CachedRenderPipelineId>,
    sky_variant: CachedRenderPipelineId,
    model_pipeline: CachedRenderPipelineId,
    body_pipeline: CachedRenderPipelineId,
}

/// Attributes laid out back to back in one vertex buffer: each `(format, byte offset)`
/// takes the next shader location, counting up from `first_location`.
fn sequential_attributes(
    first_location: u32,
    fields: &[(VertexFormat, u64)],
) -> Vec<VertexAttribute> {
    fields
        .iter()
        .zip(first_location..)
        .map(|(&(format, offset), shader_location)| VertexAttribute {
            format,
            offset,
            shader_location,
        })
        .collect()
}

/// The four `Float32x4` columns of a matrix instance, from `first_location` up.
fn matrix_columns(first_location: u32) -> Vec<VertexAttribute> {
    sequential_attributes(
        first_location,
        &[
            (VertexFormat::Float32x4, 0),
            (VertexFormat::Float32x4, 16),
            (VertexFormat::Float32x4, 32),
            (VertexFormat::Float32x4, 48),
        ],
    )
}

/// A buffer read once per vertex, `stride` bytes a row.
fn per_vertex(stride: u64, attributes: Vec<VertexAttribute>) -> VertexBufferLayout {
    VertexBufferLayout { array_stride: stride, step_mode: VertexStepMode::Vertex, attributes }
}

/// A buffer read once per instance, `stride` bytes a row.
fn per_instance(stride: u64, attributes: Vec<VertexAttribute>) -> VertexBufferLayout {
    VertexBufferLayout { array_stride: stride, step_mode: VertexStepMode::Instance, attributes }
}

/// A world vertex: position, normal, colour, tangent, then the colour map and lightmap texcoords.
fn level_vertex_rows() -> VertexBufferLayout {
    use VertexFormat::{Float32x2, Float32x3, Float32x4};
    let fields = [(Float32x3, 0), (Float32x3, 12), (Float32x4, 24), (Float32x4, 40), (Float32x2, 56), (Float32x2, 64)];
    per_vertex(size_of::<LevelVertex>() as u64, sequential_attributes(0, &fields))
}

/// bo2zm: a world vertex's layer 1 and 2 texcoords (location 6), from a
/// second buffer parallel to the world vertices.
fn level_layer_rows() -> VertexBufferLayout {
    per_vertex(16, sequential_attributes(6, &[(VertexFormat::Float32x4, 0)]))
}

/// A model vertex: position, normal, colour, uv_semantic.
fn model_vertex_rows() -> VertexBufferLayout {
    use VertexFormat::{Float32x2, Float32x3, Float32x4};
    let fields = [(Float32x3, 0), (Float32x3, 12), (Float32x4, 24), (Float32x2, 40)];
    per_vertex(size_of::<PropVertex>() as u64, sequential_attributes(0, &fields))
}

fn model_matrix_rows() -> VertexBufferLayout {
    per_instance(size_of::<ModelMatrixRow>() as u64, matrix_columns(6))
}

/// bo2zm: a prop instance: world-from-local columns (locations 6..9), its
/// light (location 10), then its primary light and visibility (11);
/// bo2mp: then a tree canopy's sway constants (12).
fn prop_matrix_rows() -> VertexBufferLayout {
    let mut attributes = matrix_columns(6);
    attributes.extend(sequential_attributes(
        10,
        &[
            (VertexFormat::Float32x4, 64),
            (VertexFormat::Float32x4, 80),
            // bo2mp: a swaying tree canopy's treeCanopySwayParms (12).
            (VertexFormat::Float32x4, 96),
        ],
    ));
    per_instance(112, attributes)
}

/// bo2zm: the lighting constants, `t6_lighting`'s four vec4.
const T6_LIGHTING_BYTES: u64 = 64;
/// Bindings only the pixel half reads.
const FRAGMENT_ONLY: ShaderStages = ShaderStages::FRAGMENT;
/// Bindings both halves read (the camera, the effect lights and wind).
const BOTH_STAGES: ShaderStages = ShaderStages::VERTEX_FRAGMENT;

/// A uniform block of exactly `bytes`.
fn constants(bytes: u64) -> bevy::render::render_resource::BindGroupLayoutEntryBuilder {
    uniform_buffer_sized(false, std::num::NonZeroU64::new(bytes))
}

fn colour_map() -> bevy::render::render_resource::BindGroupLayoutEntryBuilder {
    texture_2d(TextureSampleType::Float { filterable: true })
}

fn linear_sampler() -> bevy::render::render_resource::BindGroupLayoutEntryBuilder {
    sampler(SamplerBindingType::Filtering)
}

/// The sun's shadow cascades and their comparison sampler, as the lit groups take them.
fn shadow_cascades() -> (bevy::render::render_resource::BindGroupLayoutEntryBuilder, bevy::render::render_resource::BindGroupLayoutEntryBuilder) {
    (
        texture_2d_array(TextureSampleType::Depth),
        sampler(SamplerBindingType::Comparison),
    )
}

fn make_overlay_layouts(mut commands: Commands, asset_srv: Res<AssetServer>) {
    commands.insert_resource(OverlayPipelines {
        camera_layout: BindGroupLayoutDescriptor::new(
            "t6_world_view",
            &BindGroupLayoutEntries::single(BOTH_STAGES, uniform_buffer::<ViewUniform>(true)),
        ),
        texture_layout: BindGroupLayoutDescriptor::new(
            "bo2zm_geometry_diagnostic_colour_map",
            &BindGroupLayoutEntries::sequential(
                FRAGMENT_ONLY,
                (colour_map(), linear_sampler()),
            ),
        ),
        sky_layout: BindGroupLayoutDescriptor::new(
            "bo2zm_geometry_diagnostic_sky",
            &BindGroupLayoutEntries::with_indices(
                FRAGMENT_ONLY,
                (
                    (3, texture_cube(TextureSampleType::Float { filterable: true })),
                    (4, linear_sampler()),
                    (5, constants(T6_LIGHTING_BYTES)),
                    (6, constants(T6_LIGHTS_BYTES)),
                ),
            ),
        ),
        lightmap_layout: {
            let (cascade_maps, cascade_sampler) = shadow_cascades();
            BindGroupLayoutDescriptor::new(
                "bo2zm_geometry_diagnostic_lightmap",
                &BindGroupLayoutEntries::sequential(
                    FRAGMENT_ONLY,
                    (
                        colour_map(),
                        linear_sampler(),
                        constants(T6_LIGHTING_BYTES),
                        constants(T6_LIGHTS_BYTES),
                        constants(T6_DYN_BYTES).visibility(BOTH_STAGES),
                        cascade_maps,
                        cascade_sampler,
                        constants(sun_shadow::PARAMS_BYTES),
                    ),
                ),
            )
        },
        layered_texture_layout: BindGroupLayoutDescriptor::new(
            "bo2zm_geometry_diagnostic_layered_colour_maps",
            &BindGroupLayoutEntries::with_indices(
                FRAGMENT_ONLY,
                (
                    (0, colour_map()),
                    (1, linear_sampler()),
                    (6, colour_map()),
                    (7, colour_map()),
                ),
            ),
        ),
        lighting_layout: {
            let (cascade_maps, cascade_sampler) = shadow_cascades();
            BindGroupLayoutDescriptor::new(
                "bo2zm_geometry_diagnostic_lighting",
                &BindGroupLayoutEntries::with_indices(
                    FRAGMENT_ONLY,
                    (
                        (2, constants(T6_LIGHTING_BYTES)),
                        (3, constants(T6_LIGHTS_BYTES)),
                        (4, constants(T6_DYN_BYTES).visibility(BOTH_STAGES)),
                        (5, cascade_maps),
                        (6, cascade_sampler),
                        (7, constants(sun_shadow::PARAMS_BYTES)),
                        // bo2mp: the placed models' own light grid sets.
                        (8, texture_3d(TextureSampleType::Float { filterable: true })),
                        (9, linear_sampler()),
                        (10, storage_buffer_read_only::<[f32; 4]>(false)),
                    ),
                ),
            )
        },
        shine_texture_layout: BindGroupLayoutDescriptor::new(
            "bo2zm_geometry_diagnostic_shine_maps",
            &BindGroupLayoutEntries::with_indices(
                FRAGMENT_ONLY,
                (
                    (0, colour_map()),
                    (1, linear_sampler()),
                    (8, colour_map()),
                    (9, colour_map()),
                    (10, colour_map()),
                    (11, colour_map()),
                    (12, colour_map()),
                    (13, colour_map()),
                ),
            ),
        ),
        soft_layout: BindGroupLayoutDescriptor::new(
            "bo2zm_geometry_diagnostic_soft_depth",
            &BindGroupLayoutEntries::with_indices(FRAGMENT_ONLY, ((0, texture_depth_2d()),)),
        ),
        probe_layout: BindGroupLayoutDescriptor::new(
            "bo2zm_geometry_diagnostic_probes",
            &BindGroupLayoutEntries::with_indices(
                FRAGMENT_ONLY,
                (
                    (0, texture_cube_array(TextureSampleType::Float { filterable: true })),
                    (1, linear_sampler()),
                    (2, constants(T6_PROBES_BYTES)),
                ),
            ),
        ),
        program: asset_srv.load(OVERLAY_SHADER),
    });
}

fn mirror_scene_to_gpu(
    inspect: Res<ExtractedInspectGeometry>,
    mut geometry: ResMut<OverlayGpuScene>,
    gpu_dev: Res<RenderDevice>,
    mut bloom: ResMut<super::bo2_bloom::Bo2BloomEnabled>,
) {
    if overlay_off() {
        return;
    }
    // bo2zm: BO2's bloom and final colour on a Black Ops II map (it has T6
    // lighting). (`BO2MP_T6_NO_BLOOM` turns the bloom off, not the colour.)
    bloom.0 = inspect.t6_lighting.is_some();
    if inspect.topmost_gpu_wait {
        use std::sync::atomic::{AtomicBool, Ordering};
        static WAIT_SAID: AtomicBool = AtomicBool::new(false);
        if !WAIT_SAID.swap(true, Ordering::Relaxed) {
            diag::info!(
                World,
                "overlay upload held back this frame: still waiting on GPU residency"
            );
        }
        return;
    }
    // The world and its static models are rebuilt together; the dynamic models by revision.
    let scene_stale = geometry.generation != inspect.generation
        || !geometry
            .world
            .size_matches(inspect.level_vertex_total.len(), inspect.level_indices.len())
        || !geometry
            .smodel
            .size_matches(inspect.static_model_verts_used.len(), inspect.static_model_slots_used.len());
    geometry.generation = inspect.generation;
    geometry.world.sync(
        &gpu_dev,
        "world",
        &inspect.level_vertex_total,
        &inspect.level_indices,
        &inspect.world_surface_ranges,
        scene_stale,
    );
    geometry.smodel.sync(
        &gpu_dev,
        "smodel",
        &inspect.static_model_verts_used,
        &inspect.static_model_slots_used,
        &inspect.smodel_surface_ranges,
        scene_stale,
    );
    let xmodel_stale = geometry.body_rev != inspect.figure_edition
        || !geometry
            .xmodel
            .size_matches(inspect.xmodel_vertices.len(), inspect.figure_indices.len());
    geometry.body_rev = inspect.figure_edition;
    geometry.xmodel.sync(
        &gpu_dev,
        "xmodel",
        &inspect.xmodel_vertices,
        &inspect.figure_indices,
        &inspect.figure_facet_ranges,
        xmodel_stale,
    );
    geometry.loose_world_surfaces.clone_from(&inspect.g0_level_facets);
    geometry.world_surface_color = Arc::clone(&inspect.world_surface_color);
    if !Arc::ptr_eq(&geometry.world_surface_draw, &inspect.world_surface_draw)
        || !Arc::ptr_eq(&geometry.t6_props, &inspect.t6_props)
    {
        geometry.t6_styles = t6_styles(&inspect.world_surface_draw, &inspect.t6_props);
        let mut blended: Vec<usize> = (0..inspect.t6_props.len())
            .filter(|&i| t6_blended(inspect.t6_props[i][2]))
            .collect();
        blended.sort_by_key(|&i| t6_sort_key(inspect.t6_props[i][2]));
        geometry.t6_blended_props = blended;
    }
    geometry.world_surface_draw = Arc::clone(&inspect.world_surface_draw);
    geometry.world_surface_lightmap = Arc::clone(&inspect.world_surface_lightmap);
    geometry.t6_lighting = inspect.t6_lighting;
    geometry.t6_sun_sample_size_near = inspect.t6_sun_sample_size_near;
    geometry.t6_props = Arc::clone(&inspect.t6_props);
    geometry.t6_prop_shine = Arc::clone(&inspect.t6_prop_shine);
    geometry.world_surface_primary = Arc::clone(&inspect.world_surface_primary);
    geometry.world_surface_layers = Arc::clone(&inspect.world_surface_layers);
    geometry.world_surface_shine = Arc::clone(&inspect.world_surface_shine);
    geometry.t6_probe_slot = inspect.t6_probe_slot;
    geometry.t6_probes = Arc::clone(&inspect.t6_probes);
    if !Arc::ptr_eq(&geometry.world_layer_uvs, &inspect.world_layer_uvs) {
        geometry.world_layer_uvs = Arc::clone(&inspect.world_layer_uvs);
        geometry.world_layer_vertex = (!inspect.world_layer_uvs.is_empty()
            && inspect.world_layer_uvs.len() == inspect.level_vertex_total.len())
        .then(|| vertex_buffer(&gpu_dev, "bo2zm_world_layer_uvs", inspect.world_layer_uvs.as_slice()));
    }
    geometry.t6_lights = Arc::clone(&inspect.t6_lights);
    geometry.t6_sky_slot = inspect.t6_sky_slot;
    if !Arc::ptr_eq(&geometry.t6_prop_instances, &inspect.t6_prop_instances) {
        geometry.t6_prop_instances = Arc::clone(&inspect.t6_prop_instances);
        geometry.t6_prop_instance_buffer = (!inspect.t6_prop_instances.is_empty())
            .then(|| vertex_buffer(&gpu_dev, "bo2zm_prop_instances", inspect.t6_prop_instances.as_slice()));
    }
    let spheres_key = [
        Arc::as_ptr(&inspect.static_model_verts_used) as usize,
        Arc::as_ptr(&inspect.static_model_slots_used) as usize,
        Arc::as_ptr(&inspect.smodel_surface_ranges) as usize,
        Arc::as_ptr(&inspect.t6_props) as usize,
        Arc::as_ptr(&inspect.t6_prop_instances) as usize,
    ];
    if geometry.t6_prop_spheres_key != spheres_key {
        geometry.t6_prop_spheres_key = spheres_key;
        geometry.prop_bounds = prop_bounds(
            &inspect.static_model_verts_used,
            &inspect.static_model_slots_used,
            &inspect.smodel_surface_ranges,
            &inspect.t6_props,
            &inspect.t6_prop_instances,
        );
    }
}

/// bo2mp: each prop draw's sphere in the world, from its surface's own
/// vertices and its placement, so the draws outside the view are skipped.
fn prop_bounds(
    verts: &[PropVertex],
    ix_list: &[u32],
    ix_ranges: &[(u32, u32)],
    props: &[[u32; 4]],
    instances: &[[f32; 28]],
) -> Vec<[f32; 4]> {
    const UNCLASSIFIED: [f32; 4] = [0.0, 0.0, 0.0, -1.0];
    let mut local: Vec<Option<[f32; 4]>> = vec![None; ix_ranges.len()];
    props
        .iter()
        .map(|&[surface, _, _, instance]| {
            let (Some(&(first_ix, ix_count)), Some(row)) =
                (ix_ranges.get(surface as usize), instances.get(instance as usize))
            else {
                return UNCLASSIFIED;
            };
            let sphere = *local[surface as usize].get_or_insert_with(|| {
                let (mut lo, mut hi) = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));
                let end = (first_ix as usize).saturating_add(ix_count as usize).min(ix_list.len());
                for &index in ix_list.get(first_ix as usize..end).unwrap_or(&[]) {
                    if let Some(vertex) = verts.get(index as usize) {
                        let p = Vec3::from(vertex.position);
                        lo = lo.min(p);
                        hi = hi.max(p);
                    }
                }
                if lo.x > hi.x {
                    return UNCLASSIFIED;
                }
                let centre = (lo + hi) * 0.5;
                [centre.x, centre.y, centre.z, (hi - centre).length()]
            });
            if sphere[3] < 0.0 {
                return UNCLASSIFIED;
            }
            let world_from_local = Mat4::from_cols_slice(&row[..16]);
            let centre = world_from_local.transform_point3(Vec3::new(sphere[0], sphere[1], sphere[2]));
            let scale = world_from_local
                .x_axis
                .truncate()
                .length()
                .max(world_from_local.y_axis.truncate().length())
                .max(world_from_local.z_axis.truncate().length());
            let radius = sphere[3] * scale;
            if !centre.is_finite() || !radius.is_finite() {
                return UNCLASSIFIED;
            }
            [centre.x, centre.y, centre.z, radius]
        })
        .collect()
}

/// bo2mp: the view's four sides and the plane through the eye, facing in.
fn view_volume_planes(clip_from_world: Mat4) -> Option<[Vec4; 5]> {
    let (x, y, w) = (clip_from_world.row(0), clip_from_world.row(1), clip_from_world.row(3));
    let mut planes = [w + x, w - x, w + y, w - y, w];
    for plane in &mut planes {
        let len = plane.truncate().length();
        if !len.is_finite() || len <= f32::EPSILON {
            return None;
        }
        *plane /= len;
    }
    Some(planes)
}

/// The held shapes' chunks not on the GPU yet, written after the others;
/// all of them into bigger buffers when they do not fit, or when a new
/// gathering began.
fn mirror_static_to_gpu(
    gpu_dev: &RenderDevice,
    gpu_queue: &RenderQueue,
    have: &mut Option<HeldStatics>,
    s: &T6StaticDraw,
) {
    let vertex_size = std::mem::size_of::<PropVertex>() as u64;
    let bytes = |chunks: &[Arc<(Vec<PropVertex>, Vec<u32>)>]| {
        chunks.iter().fold((0u64, 0u64), |(v, i), c| {
            (v + c.0.len() as u64 * vertex_size, i + c.1.len() as u64 * 4)
        })
    };
    let (vertex_bytes, index_bytes) = bytes(&s.chunks);
    let from = match have.as_ref() {
        Some(g)
            if g.epoch == s.epoch
                && g.chunks <= s.chunks.len()
                && vertex_bytes <= g.vb.size()
                && index_bytes <= g.ib.size() =>
        {
            g.chunks
        }
        _ => {
            // Room for the shapes to double before the next new buffers.
            let buffer = |usage: BufferUsages, need: u64| {
                make_writable(gpu_dev, "bo2zm_script_static", usage, (need * 2).max(1 << 20).next_multiple_of(4))
            };
            *have = Some(HeldStatics {
                epoch: s.epoch,
                chunks: 0,
                vertex_bytes: 0,
                index_bytes: 0,
                vb: buffer(BufferUsages::VERTEX, vertex_bytes),
                ib: buffer(BufferUsages::INDEX, index_bytes),
            });
            0
        }
    };
    let Some(g) = have.as_mut() else { return };
    for chunk in &s.chunks[from..] {
        if !chunk.0.is_empty() {
            gpu_queue.write_buffer(&g.vb, g.vertex_bytes, bytemuck::cast_slice(chunk.0.as_slice()));
        }
        if !chunk.1.is_empty() {
            gpu_queue.write_buffer(&g.ib, g.index_bytes, bytemuck::cast_slice(chunk.1.as_slice()));
        }
        g.vertex_bytes += chunk.0.len() as u64 * vertex_size;
        g.index_bytes += chunk.1.len() as u64 * 4;
    }
    g.chunks = s.chunks.len();
}

/// bo2zm: this frame's moving things into GPU buffers.
fn mirror_moving_to_gpu(
    source: Option<Res<ExtractedT6Dynamic>>,
    mut dynamic: ResMut<OverlayMovingLists>,
    gpu_dev: Res<RenderDevice>,
    gpu_queue: Res<RenderQueue>,
    skin_pipeline: Option<Res<skin::SkinPipeline>>,
    mut skin_cache: ResMut<skin::SkinCache>,
    mut pending: ResMut<bevy::render::renderer::PendingCommandBuffers>,
) {
    let source = source.as_deref();
    let dynamic = &mut *dynamic;
    if source
        .and_then(|s| s.missiles.as_ref())
        .is_none_or(|m| m.skinned.is_none())
    {
        skin_cache.idle();
    }
    let [marks, viewmodel, missiles, effects, viewmodel_effects, held] = &mut dynamic.kept;
    dynamic.marks = mirror_moving_draw(
        &gpu_dev,
        &gpu_queue,
        marks,
        "bo2zm_marks",
        source.and_then(|s| s.marks.as_ref()),
        None,
    );
    dynamic.viewmodel = mirror_moving_draw(
        &gpu_dev,
        &gpu_queue,
        viewmodel,
        "bo2zm_viewmodel",
        source.and_then(|s| s.viewmodel.as_ref()),
        None,
    );
    dynamic.missiles = mirror_moving_draw(
        &gpu_dev,
        &gpu_queue,
        missiles,
        "bo2zm_missiles",
        source.and_then(|s| s.missiles.as_ref()),
        skin_pipeline
            .as_deref()
            .map(|p| (p, &mut *skin_cache, &mut *pending)),
    );
    dynamic.script_static = match source.and_then(|s| s.script_static.as_ref()) {
        Some(s) if !s.draws.is_empty() && !s.instances.is_empty() && !s.chunks.is_empty() => {
            let _span = bevy::log::info_span!(target: "bo2mp", "t6 held upload").entered();
            mirror_static_to_gpu(&gpu_dev, &gpu_queue, &mut dynamic.script_static_geometry, s);
            let inst = held.write(
                &gpu_dev,
                &gpu_queue,
                "bo2zm_script_static",
                2,
                BufferUsages::VERTEX,
                bytemuck::cast_slice(s.instances.as_slice()),
            );
            dynamic.script_static_geometry.as_ref().map(|g| {
                let draws = draws_within("bo2zm_script_static", &s.draws, g.index_bytes / 4);
                (g.vb.clone(), g.ib.clone(), inst, draws)
            })
        }
        _ => None,
    };
    dynamic.effects = mirror_moving_draw(
        &gpu_dev,
        &gpu_queue,
        effects,
        "bo2zm_effects",
        source.and_then(|s| s.effects.as_ref()),
        None,
    );
    dynamic.viewmodel_effects = mirror_moving_draw(
        &gpu_dev,
        &gpu_queue,
        viewmodel_effects,
        "bo2zm_viewmodel_effects",
        source.and_then(|s| s.viewmodel_effects.as_ref()),
        None,
    );
    // bo2mp test aid: `BO2MP_T6_SKIP_LIST=marks,missiles,static,effects`
    // leaves those dynamic lists out (bisecting a stray surface).
    static SKIP: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    let skip = SKIP.get_or_init(|| std::env::var("BO2MP_T6_SKIP_LIST").unwrap_or_default());
    for name in skip.split(',') {
        match name {
            "marks" => dynamic.marks = None,
            "missiles" => dynamic.missiles = None,
            "static" => dynamic.script_static = None,
            "effects" => dynamic.effects = None,
            _ => {}
        }
    }
}

/// bo2zm: bind groups for the lightmap pages the world surfaces read, with
/// the T6 lighting constants; a page whose view has not uploaded yet is
/// tried again next frame.
fn bind_light_groups(
    geo_pipe: Option<Res<OverlayPipelines>>,
    geometry: Res<OverlayGpuScene>,
    registry: Option<Res<LiveUploadedImageRegistry>>,
    pipes: Res<PipelineCache>,
    gpu_dev: Res<RenderDevice>,
    gpu_queue: Res<RenderQueue>,
    mut lightmaps: ResMut<OverlayLightGroups>,
    dynamic_source: Option<Res<ExtractedT6Dynamic>>,
    shadow: Option<Res<sun_shadow::SunShadow>>,
) {
    if overlay_off() {
        return;
    }
    let (Some(geo_pipe), Some(registry), Some(light_res), Some(shadow_res)) =
        (geo_pipe, registry, geometry.t6_lighting, shadow)
    else {
        return;
    };
    // bo2zm: the lights effects throw this frame.
    let dyn_lights = lightmaps
        .dyn_lights
        .get_or_insert_with(|| make_filled(&gpu_dev, "bo2zm_effect_lights", REWRITTEN_UNIFORM, &[0u8; T6_DYN_BYTES as usize]))
        .clone();
    {
        let mut table = vec![0.0f32; T6_DYN_BYTES as usize / 4];
        let lights = dynamic_source.as_ref().map_or(&[][..], |d| d.dlights.as_slice());
        let n = lights.len().min(T6_DYN_LIGHTS);
        table[0] = n as f32;
        // The shaders' game time in seconds (BO2's gameTime): the objective
        // glow pulses by it.
        static START: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
        table[1] = (START.get_or_init(std::time::Instant::now).elapsed().as_secs_f64() % 3600.0) as f32;
        for (i, light) in lights.iter().take(n).enumerate() {
            table[4 + i * 8..4 + i * 8 + 8].copy_from_slice(light);
        }
        // bo2mp: the wind that sways the tree canopies, this moment.
        let wind = dynamic_source.as_ref().map_or([0.0; 3], |d| d.wind);
        let at = 4 + T6_DYN_LIGHTS * 8;
        for (i, row) in asset_core::t6_wind_table(wind, table[1]).iter().enumerate() {
            table[at + i * 4..at + i * 4 + 4].copy_from_slice(row);
        }
        gpu_queue.write_buffer(&dyn_lights, 0, bytemuck::cast_slice(&table));
    }
    if lightmaps.generation != registry.revision_stamp {
        lightmaps.bind_groups.clear();
        lightmaps.sky = None;
        lightmaps.lighting_group = None;
        lightmaps.generation = registry.revision_stamp;
    }
    let constants = lightmaps
        .lighting_buf
        .get_or_insert_with(|| make_filled(&gpu_dev, "bo2zm_world_lighting", REWRITTEN_UNIFORM, bytemuck::cast_slice(&light_res)))
        .clone();
    if lightmaps.written != Some(light_res) {
        gpu_queue.write_buffer(&constants, 0, bytemuck::cast_slice(&light_res));
        lightmaps.written = Some(light_res);
    }
    let lights = lightmaps
        .lights
        .get_or_insert_with(|| make_filled(&gpu_dev, "bo2zm_primary_lights", REWRITTEN_UNIFORM, &[0u8; T6_LIGHTS_BYTES as usize]))
        .clone();
    if lightmaps
        .lights_written
        .as_ref()
        .is_none_or(|have| !Arc::ptr_eq(have, &geometry.t6_lights))
    {
        let mut table = vec![[0.0f32; 16]; T6_LIGHT_ROWS];
        for (row, light) in table.iter_mut().zip(geometry.t6_lights.iter()) {
            *row = *light;
        }
        gpu_queue.write_buffer(&lights, 0, bytemuck::cast_slice(&table));
        lightmaps.lights_written = Some(Arc::clone(&geometry.t6_lights));
    }
    let sampler = lightmaps
        .sampler
        .get_or_insert_with(|| {
            gpu_dev.create_sampler(&SamplerDescriptor {
                label: Some("bo2zm_world_lightmap_sampler"),
                address_mode_u: AddressMode::ClampToEdge,
                address_mode_v: AddressMode::ClampToEdge,
                address_mode_w: AddressMode::ClampToEdge,
                mag_filter: FilterMode::Linear,
                min_filter: FilterMode::Linear,
                ..Default::default()
            })
        })
        .clone();
    // bo2mp: the placed models' own light grid sets, made again whenever a
    // map installs new ones (and the lighting group with them).
    let (light_generation, model_lights) = lighting_t6::t6_model_lights();
    if lightmaps
        .model_lights
        .as_ref()
        .is_some_and(|(have, _, _)| *have != light_generation)
    {
        lightmaps.model_lights = None;
        lightmaps.lighting_group = None;
    }
    let (_, model_view, model_sh) = lightmaps
        .model_lights
        .get_or_insert_with(|| {
            use bevy::render::render_resource::{Extent3d, TextureDataOrder, TextureDescriptor, TextureDimension, TextureUsages};
            let (width, height, depth, texels) = lighting_t6::t6_model_light_texels(&model_lights);
            let texture = gpu_dev.create_texture_with_data(
                &gpu_queue,
                &TextureDescriptor {
                    label: Some("bo2mp_model_light_cubes"),
                    size: Extent3d { width, height, depth_or_array_layers: depth },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: TextureDimension::D3,
                    format: TextureFormat::Rgba8Unorm,
                    usage: TextureUsages::TEXTURE_BINDING | TextureUsages::COPY_DST,
                    view_formats: &[],
                },
                TextureDataOrder::LayerMajor,
                &texels,
            );
            let mut sh: Vec<[f32; 4]> = model_lights.iter().flat_map(|l| l.sh).collect();
            if sh.is_empty() {
                sh = vec![[0.0; 4]; 3];
            }
            let sh = make_filled(&gpu_dev, "bo2mp_model_light_sh", BufferUsages::STORAGE, bytemuck::cast_slice(&sh));
            (light_generation, texture.create_view(&Default::default()), sh)
        })
        .clone();
    if lightmaps.lighting_group.is_none() {
        lightmaps.lighting_group = Some(gpu_dev.create_bind_group(
            "bo2zm_world_lighting_only",
            &pipes.get_bind_group_layout(&geo_pipe.lighting_layout),
            &BindGroupEntries::with_indices((
                (2, constants.as_entire_binding()),
                (3, lights.as_entire_binding()),
                (4, dyn_lights.as_entire_binding()),
                (5, &shadow_res.array),
                (6, &shadow_res.sampler),
                (7, shadow_res.params.as_entire_binding()),
                (8, &model_view),
                (9, &sampler),
                (10, model_sh.as_entire_binding()),
            )),
        ));
    }
    if let Some(slot) = geometry.t6_sky_slot
        && lightmaps.sky.as_ref().is_none_or(|(have, _)| *have != slot)
        && let Some(view) = registry.material_view(slot)
        && view.dimension == TextureViewDimension::Cube
    {
        let sky_sampler = gpu_dev.create_sampler(&SamplerDescriptor {
            label: Some("bo2zm_sky_sampler"),
            mag_filter: FilterMode::Linear,
            min_filter: FilterMode::Linear,
            mipmap_filter: MipmapFilterMode::Linear,
            ..Default::default()
        });
        let sky = gpu_dev.create_bind_group(
            "bo2zm_sky",
            &pipes.get_bind_group_layout(&geo_pipe.sky_layout),
            &BindGroupEntries::with_indices((
                (3, &view.tex_view),
                (4, &sky_sampler),
                (5, constants.as_entire_binding()),
                (6, lights.as_entire_binding()),
            )),
        );
        lightmaps.sky = Some((slot, sky));
    }
    let layout = pipes.get_bind_group_layout(&geo_pipe.lightmap_layout);
    for &page in geometry.world_surface_lightmap.iter() {
        if page == u8::MAX || lightmaps.bind_groups.contains_key(&page) {
            continue;
        }
        let Some(view) = registry.lightmap_view(usize::from(page), LightmapPlane::Backup) else {
            continue;
        };
        let made = gpu_dev.create_bind_group(
            "bo2zm_world_lightmap",
            &layout,
            &BindGroupEntries::sequential((
                view,
                &sampler,
                constants.as_entire_binding(),
                lights.as_entire_binding(),
                dyn_lights.as_entire_binding(),
                &shadow_res.array,
                &shadow_res.sampler,
                shadow_res.params.as_entire_binding(),
            )),
        );
        lightmaps.bind_groups.insert(page, made);
    }
}

/// bo2zm: bind groups for the colour maps the world surfaces sample, from the
/// uploaded material images; a slot whose image has not uploaded yet is
/// tried again next frame.
fn bind_texture_groups(
    geo_pipe: Option<Res<OverlayPipelines>>,
    geometry: Res<OverlayGpuScene>,
    registry: Option<Res<LiveUploadedImageRegistry>>,
    pipes: Res<PipelineCache>,
    gpu_dev: Res<RenderDevice>,
    gpu_queue: Res<RenderQueue>,
    mut textures: ResMut<OverlayTextureGroups>,
    dynamic: Option<Res<ExtractedT6Dynamic>>,
) {
    if overlay_off() {
        return;
    }
    let (Some(geo_pipe), Some(registry)) = (geo_pipe, registry) else {
        return;
    };
    if geometry.world_surface_color.iter().all(|&slot| slot == u32::MAX)
        && geometry.t6_props.is_empty()
    {
        return;
    }
    if textures.generation != registry.revision_stamp {
        textures.bind_groups.clear();
        textures.layer_groups.clear();
        textures.shine_groups.clear();
        textures.probes = None;
        textures.generation = registry.revision_stamp;
    }
    // His texture filtering row changed: a new sampler, new bind groups.
    let aniso = asset_core::t6_aniso().min(if asset_core::t6_fast() { 4 } else { 16 });
    if textures.aniso != aniso {
        textures.bind_groups.clear();
        textures.layer_groups.clear();
        textures.shine_groups.clear();
        textures.probes = None;
        textures.sampler = None;
        textures.aniso = aniso;
    }
    let sampler = textures
        .sampler
        .get_or_insert_with(|| {
            gpu_dev.create_sampler(&SamplerDescriptor {
                label: Some("bo2zm_world_colour_sampler"),
                address_mode_u: AddressMode::Repeat,
                address_mode_v: AddressMode::Repeat,
                address_mode_w: AddressMode::Repeat,
                mag_filter: FilterMode::Linear,
                min_filter: FilterMode::Linear,
                mipmap_filter: MipmapFilterMode::Linear,
                // bo2zm: his texture filtering (low 1x, medium 4x, high
                // 16x), 4x at most at the fast quality setting.
                anisotropy_clamp: aniso,
                ..Default::default()
            })
        })
        .clone();
    let layout = pipes.get_bind_group_layout(&geo_pipe.texture_layout);
    let prop_slots = geometry.t6_props.iter().map(|draw| draw[1]);
    // bo2zm: every T6 material's colour map (guns, arms, effects).
    let dynamic_slots: Vec<u32> = dynamic
        .as_ref()
        .map(|d| d.mat_list.iter().map(|m| m.0).collect())
        .unwrap_or_default();
    for slot in geometry
        .world_surface_color
        .iter()
        .copied()
        .chain(prop_slots)
        .chain(dynamic_slots)
    {
        if slot == u32::MAX || textures.bind_groups.contains_key(&slot) {
            continue;
        }
        let Some(view) = registry.material_view(slot) else {
            continue;
        };
        if view.dimension != TextureViewDimension::D2 {
            continue;
        }
        let made = gpu_dev.create_bind_group(
            "bo2zm_world_colour_map",
            &layout,
            &BindGroupEntries::sequential((&view.tex_view, &sampler)),
        );
        textures.bind_groups.insert(slot, made);
    }
    // Layered surfaces: colour map plus layer 1 and 2 maps.
    let layered_layout = pipes.get_bind_group_layout(&geo_pipe.layered_texture_layout);
    let view_of = |slot: u32| registry.flat_map_view(slot).cloned();
    for (i, &slot) in geometry.world_surface_color.iter().enumerate() {
        let Some(&[l1, l2, _, _]) = geometry.world_surface_layers.get(i) else {
            continue;
        };
        if slot == u32::MAX || (l1 == u32::MAX && l2 == u32::MAX) {
            continue;
        }
        let key = (slot, l1, l2);
        if textures.layer_groups.contains_key(&key) {
            continue;
        }
        let Some(base) = view_of(slot) else { continue };
        let layer1 = if l1 == u32::MAX { Some(base.clone()) } else { view_of(l1) };
        let layer2 = if l2 == u32::MAX { Some(base.clone()) } else { view_of(l2) };
        let (Some(layer1), Some(layer2)) = (layer1, layer2) else {
            continue;
        };
        let made = gpu_dev.create_bind_group(
            "bo2zm_world_layered_colour_maps",
            &layered_layout,
            &BindGroupEntries::with_indices((
                (0, &base),
                (1, &sampler),
                (6, &layer1),
                (7, &layer2),
            )),
        );
        textures.layer_groups.insert(key, made);
    }
    // bo2mp: animated screen props: colour map plus their constants (layer
    // 1's slot, bound as both layers).
    for (draw, sh) in geometry.t6_props.iter().zip(geometry.t6_prop_shine.iter()) {
        let (slot, consts) = (draw[1], sh[2]);
        if !anim_code(draw[2]) || slot == u32::MAX || consts == u32::MAX {
            continue;
        }
        let key = (slot, consts, u32::MAX);
        if textures.layer_groups.contains_key(&key) {
            continue;
        }
        let (Some(base), Some(consts)) = (view_of(slot), view_of(consts)) else {
            continue;
        };
        let made = gpu_dev.create_bind_group(
            "bo2mp_anim_screen_maps",
            &layered_layout,
            &BindGroupEntries::with_indices(((0, &base), (1, &sampler), (6, &consts), (7, &consts))),
        );
        textures.layer_groups.insert(key, made);
    }
    // bo2mp: lens flares: sprite atlas plus grime map (layer 1's slot, bound
    // as both layers).
    let flare_draws = dynamic.iter().flat_map(|d| d.effects.iter()).flat_map(|e| e.draws.iter());
    for d in flare_draws {
        let (slot, grime) = (d[2], d[5]);
        if !flare_code(d[3]) || slot == u32::MAX || grime == u32::MAX {
            continue;
        }
        let key = (slot, grime, u32::MAX);
        if textures.layer_groups.contains_key(&key) {
            continue;
        }
        let (Some(base), Some(grime)) = (view_of(slot), view_of(grime)) else {
            continue;
        };
        let made = gpu_dev.create_bind_group(
            "bo2mp_flare_maps",
            &layered_layout,
            &BindGroupEntries::with_indices(((0, &base), (1, &sampler), (6, &grime), (7, &grime))),
        );
        textures.layer_groups.insert(key, made);
    }
    bind_shine_groups(
        &geo_pipe,
        &geometry,
        dynamic.as_deref(),
        &registry,
        &pipes,
        &gpu_dev,
        &gpu_queue,
        &mut textures,
        &sampler,
    );
}

/// bo2zm: the shine surfaces' group 1 (colour, normal, specular maps) and
/// the probes' group 3.
#[allow(clippy::too_many_arguments)]
fn bind_shine_groups(
    pipeline: &OverlayPipelines,
    geometry: &OverlayGpuScene,
    dynamic: Option<&ExtractedT6Dynamic>,
    registry: &LiveUploadedImageRegistry,
    pipes: &PipelineCache,
    gpu_dev: &RenderDevice,
    gpu_queue: &RenderQueue,
    textures: &mut OverlayTextureGroups,
    sampler: &Sampler,
) {
    let no_shine = |s: &[u32; 2]| s[0] == u32::MAX && s[1] == u32::MAX;
    let dynamic_shine: Vec<ShineKey> = dynamic
        .into_iter()
        .flat_map(|d| [d.viewmodel.as_ref(), d.marks.as_ref()].into_iter().flatten())
        .flat_map(|draw| draw.draws.iter())
        .filter(|d| !no_shine(&[d[5], d[6]]))
        .map(|d| (d[2], d[5], d[6], u32::MAX, [u32::MAX; 3]))
        .collect();
    if geometry.world_surface_shine.iter().all(|s| no_shine(&[s[0], s[1]]))
        && geometry
            .t6_prop_shine
            .iter()
            .all(|s| no_shine(&[s[0], s[1]]))
        && dynamic_shine.is_empty()
    {
        return;
    }
    // 1x1 stand-ins: a flat normal (128/255 reads as 0 after BO2's scale
    // and bias), a black specular map (no highlight, no reflection).
    let one_pixel = |pixel_name: &'static str, format: TextureFormat, texel: &[u8]| {
        use bevy::render::render_resource::{TextureDescriptor, TextureUsages, TextureViewDescriptor};
        use bevy::render::render_resource::{Extent3d, TextureDimension};
        let texture = gpu_dev.create_texture_with_data(
            gpu_queue,
            &TextureDescriptor {
                label: Some(pixel_name),
                size: Extent3d {
                    width: 1,
                    height: 1,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: TextureDimension::D2,
                format,
                usage: TextureUsages::TEXTURE_BINDING | TextureUsages::COPY_DST,
                view_formats: &[],
            },
            bevy::render::render_resource::TextureDataOrder::LayerMajor,
            texel,
        );
        texture.create_view(&TextureViewDescriptor::default())
    };
    if textures.flat_normal.is_none() {
        textures.flat_normal = Some(one_pixel("bo2zm_flat_normal", TextureFormat::Rgba8Unorm, &[128, 128, 255, 255]));
    }
    if textures.no_specular.is_none() {
        textures.no_specular = Some(one_pixel("bo2zm_no_specular", TextureFormat::Rgba8Unorm, &[0, 0, 0, 0]));
    }
    let view_of = |slot: u32| registry.flat_map_view(slot).cloned();
    let layout = pipes.get_bind_group_layout(&pipeline.shine_texture_layout);
    let world_keys = geometry
        .world_surface_shine
        .iter()
        .zip(geometry.world_surface_color.iter())
        .enumerate()
        .map(|(i, (sh, &colour))| {
            // bo2mp: an emissive flow surface's remap and constants; a sea's
            // control map, then its shore picture and white-water noise and mask.
            let style = geometry.world_surface_draw.get(i).map(|&code| world_style(code, true));
            let (flow, water) = (style.is_some_and(style_flow), style.is_some_and(style_water));
            let layers = geometry.world_surface_layers.get(i).copied().unwrap_or([u32::MAX; 4]);
            let aux = if flow || water { layers[0] } else { u32::MAX };
            let sea = if water { [layers[1], layers[2], layers[3]] } else { [u32::MAX; 3] };
            (colour, sh[0], sh[1], aux, sea)
        });
    let prop_keys = geometry
        .t6_prop_shine
        .iter()
        .zip(geometry.t6_props.iter())
        .map(|(sh, draw)| {
            let flow = prop_style(draw[2]) & (STYLE_PROP_FLOW | STYLE_PROP_TILE) != 0
                || (prop_style(draw[2]) & STYLE_PROP_FLAG != 0 && draw[2] & T6_UNLIT == 0);
            (draw[1], sh[0], sh[1], if flow { sh[2] } else { u32::MAX }, [u32::MAX; 3])
        });
    let keys: Vec<ShineKey> =
        world_keys.chain(prop_keys).chain(dynamic_shine).collect();
    for key in keys {
        let (colour, n, s, aux, sea) = key;
        if n == u32::MAX && s == u32::MAX {
            continue;
        }
        if colour == u32::MAX || textures.shine_groups.contains_key(&key) {
            continue;
        }
        let Some(base) = view_of(colour) else { continue };
        let normal = if n == u32::MAX { textures.flat_normal.clone() } else { view_of(n) };
        let specular = if s == u32::MAX { textures.no_specular.clone() } else { view_of(s) };
        let aux = if aux == u32::MAX { textures.no_specular.clone() } else { view_of(aux) };
        let [aux2, noise, mask] =
            sea.map(|slot| if slot == u32::MAX { textures.no_specular.clone() } else { view_of(slot) });
        let (Some(normal), Some(specular), Some(aux), Some(aux2), Some(noise), Some(mask)) =
            (normal, specular, aux, aux2, noise, mask)
        else {
            continue;
        };
        let made = gpu_dev.create_bind_group(
            "bo2zm_world_shine_maps",
            &layout,
            &BindGroupEntries::with_indices((
                (0, &base),
                (1, sampler),
                (8, &normal),
                (9, &specular),
                (10, &aux),
                (11, &aux2),
                (12, &noise),
                (13, &mask),
            )),
        );
        textures.shine_groups.insert(key, made);
    }
    // The probes: their lighting table, then their group.
    let table = textures
        .probe_table
        .get_or_insert_with(|| make_writable(&gpu_dev, "bo2zm_probe_table", BufferUsages::UNIFORM, T6_PROBES_BYTES))
        .clone();
    if textures
        .probe_written
        .as_ref()
        .is_none_or(|have| !Arc::ptr_eq(have, &geometry.t6_probes))
    {
        let mut rows = vec![[0.0f32; 12]; T6_MAX_PROBES];
        for (row, probe) in rows.iter_mut().zip(geometry.t6_probes.iter()) {
            for (k, v) in probe.iter().enumerate() {
                row[k * 4..k * 4 + 4].copy_from_slice(v);
            }
        }
        gpu_queue.write_buffer(&table, 0, bytemuck::cast_slice(&rows));
        textures.probe_written = Some(Arc::clone(&geometry.t6_probes));
    }
    if let Some(slot) = geometry.t6_probe_slot
        && textures.probes.as_ref().is_none_or(|(have, _)| *have != slot)
        && let Some(view) = registry.material_view(slot)
        && view.dimension == TextureViewDimension::CubeArray
    {
        let probe_sampler = gpu_dev.create_sampler(&SamplerDescriptor {
            label: Some("bo2zm_probe_sampler"),
            mag_filter: FilterMode::Linear,
            min_filter: FilterMode::Linear,
            mipmap_filter: MipmapFilterMode::Linear,
            ..Default::default()
        });
        let group = gpu_dev.create_bind_group(
            "bo2zm_probes",
            &pipes.get_bind_group_layout(&pipeline.probe_layout),
            &BindGroupEntries::with_indices((
                (0, &view.tex_view),
                (1, &probe_sampler),
                (2, table.as_entire_binding()),
            )),
        );
        textures.probes = Some((slot, group));
    }
}

/// Every style a view needs a pipeline for: those the world and its props already use, plus
/// a prop style for each T6 material in the dynamic set (what guns, arms and effects draw with).
fn view_styles(
    geometry: &OverlayGpuScene,
    dynamic: Option<&ExtractedT6Dynamic>,
) -> std::collections::BTreeSet<ShadeKind> {
    let mut styles: std::collections::BTreeSet<ShadeKind> =
        geometry.t6_styles.iter().copied().collect();
    let codes = dynamic.into_iter().flat_map(|d| d.mat_list.iter().map(|&(_, code)| code));
    for code in codes {
        let occluder_exclusive = code != T6_NONE && code & T6_BLEND_MASK >= T6_SHADOW_ONLY;
        if occluder_exclusive {
            continue;
        }
        let base = prop_style(code);
        let shiny = prop_shine_code(code);
        styles.insert(ShadeKind::PropTextured(base));
        if shiny {
            styles.insert(ShadeKind::PropTextured(base | STYLE_SHINE));
        }
        if soft_code(code) {
            styles.insert(ShadeKind::PropTextured(base | STYLE_SOFT));
        }
        if let Some(solid) = foliage_solid_style(code) {
            styles.insert(ShadeKind::PropTextured(solid));
            if shiny {
                styles.insert(ShadeKind::PropTextured(solid | STYLE_SHINE));
            }
        }
    }
    styles
}

fn bind_overlay_cameras(
    mut commands: Commands,
    geometry: Res<OverlayGpuScene>,
    dynamic: Option<Res<ExtractedT6Dynamic>>,
    geo_pipe: Option<Res<OverlayPipelines>>,
    mut specialized: ResMut<SpecializedRenderPipelines<OverlayPipelines>>,
    pipes: Res<PipelineCache>,
    gpu_dev: Res<RenderDevice>,
    view_unis: Res<ViewUniforms>,
    view_rows: Query<(Entity, &ExtractedView, &Msaa)>,
) {
    if overlay_off() {
        return;
    }
    let (Some(geo_pipe), Some(view_bind)) = (geo_pipe, view_unis.uniforms.binding()) else {
        return;
    };
    let styles = view_styles(&geometry, dynamic.as_deref());
    let camera_layout = pipes.get_bind_group_layout(&geo_pipe.camera_layout);
    for (ent, cam_view, aa_mode) in &view_rows {
        let mut pipeline_for = |shade: ShadeKind| {
            specialized.specialize(
                &pipes,
                &geo_pipe,
                OverlayVariant {
                    colour_format: cam_view.target_format,
                    sample_n: aa_mode.samples(),
                    shade,
                },
            )
        };
        let coded_pipelines = styles.iter().map(|&shade| (shade, pipeline_for(shade))).collect();
        commands.entity(ent).insert(OverlayCameraBinds {
            camera_group: gpu_dev.create_bind_group(
                "t6_world_view",
                &camera_layout,
                &BindGroupEntries::single(view_bind.clone()),
            ),
            coded_pipelines,
            level_pipeline: pipeline_for(ShadeKind::World),
            model_pipeline: pipeline_for(ShadeKind::Smodel),
            body_pipeline: pipeline_for(ShadeKind::XModel),
            sky_variant: pipeline_for(ShadeKind::Sky),
        });
    }
}

/// One entry of a per-surface table, or `absent` when the table is shorter than the surface id.
fn at<T: Copy>(table: &[T], surf: u16, absent: T) -> T {
    table.get(usize::from(surf)).copied().unwrap_or(absent)
}

/// How a world surface binds when it is drawn textured: the style that selects its pipeline,
/// its texture group (slot 1) and its lighting group (slot 2, with the id that tells
/// consecutive draws apart).
struct WorldSurfaceBinding<'a> {
    style: u32,
    colour_slot: u32,
    layers: [u32; 2],
    shine: [u32; 3],
    layered: Option<&'a BindGroup>,
    shining: Option<&'a BindGroup>,
    texture_group: Option<&'a BindGroup>,
    lighting: Option<(&'a BindGroup, u16)>,
}

impl WorldSurfaceBinding<'_> {
    fn key(&self) -> ShadeKind {
        ShadeKind::WorldTextured(self.style)
    }

    fn texture_key(&self) -> (u32, Option<[u32; 2]>, Option<[u32; 3]>) {
        (
            self.colour_slot,
            self.layered.map(|_| self.layers),
            self.shining.map(|_| [self.shine[0], self.shine[1], self.layers[0]]),
        )
    }

    /// The instance index a draw carries: the surface's primary light, and for a shine
    /// surface its reflection probe above that (bits 6..).
    fn instance(&self, light: u32) -> u32 {
        if self.style & STYLE_SHINE == 0 {
            return light;
        }
        (light & 63) | (self.shine[2].min(T6_MAX_PROBES as u32 - 1) << 6)
    }
}

/// Works out how world surface `surf` binds; `None` when it is not drawn this frame
/// (shadow-only, or water and flow still waiting on their normal maps and probes).
fn world_surface_binding<'a>(
    geometry: &OverlayGpuScene,
    lightmaps: &'a OverlayLightGroups,
    textures: &'a OverlayTextureGroups,
    surf: u16,
) -> Option<WorldSurfaceBinding<'a>> {
    let colour_slot = at(&geometry.world_surface_color, surf, u32::MAX);
    let code = at(&geometry.world_surface_draw, surf, T6_NONE);
    if code != T6_NONE && code & T6_BLEND_MASK >= T6_SHADOW_ONLY {
        return None;
    }
    let page = at(&geometry.world_surface_lightmap, surf, u8::MAX);
    let lit = code != T6_NONE && code & T6_UNLIT == 0;
    let own_page = lit
        .then(|| lightmaps.bind_groups.get(&page))
        .flatten()
        .map(|group| (group, u16::from(page)));
    // Emissive flow (lava) has no lightmap (its shader reads none), nor has a distant sea;
    // they draw in the lightmapped pipeline over the first page.
    let first_page = || {
        let wants_any = lit && code & (T6_FLOW | T6_WATER) != 0;
        wants_any
            .then(|| lightmaps.bind_groups.iter().min_by_key(|(page, _)| **page))
            .flatten()
            .map(|(page, group)| (group, 0x100 | u16::from(*page)))
    };
    let lightmap = own_page.or_else(first_page);
    let lighting =
        lightmap.or_else(|| lightmaps.lighting_group.as_ref().map(|group| (group, u16::MAX)));
    let mut style = world_style(code, lightmap.is_some());
    // Layered: its colour, layer 1 and 2 maps, else the base layer alone.
    let layers = at(&geometry.world_surface_layers, surf, [u32::MAX; 4]);
    let layer_key = (colour_slot, layers[0], layers[1]);
    let layered = (style_layers(style) && geometry.world_layer_vertex.is_some())
        .then(|| textures.layer_groups.get(&layer_key))
        .flatten();
    if style_layers(style) && layered.is_none() {
        style = strip_layers(style);
    }
    // Shine: an opaque lit lightmapped surface with a normal or specular map, once the
    // probes are ready. Emissive flow also binds its colour remap and constants (layer 1's
    // slot) beside its maps; water its control map and shore picture (layers 1 and 2).
    let shine = at(&geometry.world_surface_shine, surf, [u32::MAX; 3]);
    let can_shine = style & STYLE_LIGHTMAPPED != 0
        && !style_layers(style)
        && style & 3 <= 1
        && lightmap.is_some()
        && textures.probes.is_some();
    let water = style_water(style);
    let flow_aux = if style_flow(style) || water { layers[0] } else { u32::MAX };
    let sea = if water { [layers[1], layers[2], layers[3]] } else { [u32::MAX; 3] };
    let shining = can_shine
        .then(|| textures.shine_groups.get(&(colour_slot, shine[0], shine[1], flow_aux, sea)))
        .flatten();
    if shining.is_some() {
        style |= STYLE_SHINE;
    } else if style_water(style) || style_flow(style) {
        return None;
    }
    // An animated screen draws with its constants (layer 1's slot).
    let animated = anim_code(code).then(|| textures.layer_groups.get(&layer_key)).flatten();
    if animated.is_some() {
        style |= STYLE_ANIM;
    }
    let layered = layered.or(animated);
    let texture_group = shining
        .or(layered)
        .or_else(|| textures.bind_groups.get(&colour_slot));
    Some(WorldSurfaceBinding {
        style,
        colour_slot,
        layers: [layers[0], layers[1]],
        shine,
        layered,
        shining,
        texture_group,
        lighting,
    })
}

/// Binds a mesh's vertex buffer (slot 0) and index buffer.
fn bind_mesh<'a>(rp: &mut TrackedRenderPass<'a>, vert_buf: &'a Buffer, index: &'a Buffer) {
    rp.set_vertex_buffer(0, vert_buf.slice(..));
    rp.set_index_buffer(index.slice(..), IndexFormat::Uint32);
}

/// Sets the viewport with `depth` as its near and far range; an empty viewport is left alone.
fn apply_viewport(rp: &mut TrackedRenderPass<'_>, rect: [f32; 4], (near, far): (f32, f32)) {
    let [x, y, w, h] = rect;
    if w > 0.0 && h > 0.0 {
        rp.set_viewport(x, y, w, h, near, far);
    }
}

/// Starts one of the world's passes over the view's colour target with view group 0 bound.
fn begin_pass<'a>(
    context: &'a mut RenderContext,
    caption: &'static str,
    out_tgt: &ViewTarget,
    depth: RenderPassDepthStencilAttachment<'_>,
    view: (&'a OverlayCameraBinds, &ViewUniformOffset),
) -> TrackedRenderPass<'a> {
    let colour = [Some(out_tgt.get_color_attachment())];
    let mut rp = context.begin_tracked_render_pass(RenderPassDescriptor {
        label: Some(caption),
        color_attachments: &colour,
        depth_stencil_attachment: Some(depth),
        timestamp_writes: None,
        occlusion_query_set: None,
        multiview_mask: None,
    });
    rp.set_bind_group(0, &view.0.camera_group, &[view.1.offset]);
    rp
}

/// A uniform the gpu_queue writes again after it is made.
const REWRITTEN_UNIFORM: BufferUsages = BufferUsages::UNIFORM.union(BufferUsages::COPY_DST);

/// A new buffer for `usage`, holding `bytes` from the start.
fn make_filled(gpu_dev: &RenderDevice, tag_text: &str, use_flags: BufferUsages, bytes: &[u8]) -> Buffer {
    let init = BufferInitDescriptor { label: Some(tag_text), contents: bytes, usage: use_flags };
    gpu_dev.create_buffer_with_data(&init)
}

/// A new, blank buffer of `size` bytes for `usage`, which the gpu_queue fills later.
fn make_writable(gpu_dev: &RenderDevice, buf_label: &str, buf_flags: BufferUsages, size: u64) -> Buffer {
    let buf_flags = buf_flags | BufferUsages::COPY_DST;
    let buf_desc = bevy::render::render_resource::BufferDescriptor { label: Some(buf_label), size, usage: buf_flags, mapped_at_creation: false };
    gpu_dev.create_buffer(&buf_desc)
}

/// A vertex buffer holding `rows`.
fn vertex_buffer<T: bytemuck::NoUninit>(gpu_dev: &RenderDevice, name_tag: &str, row_data: &[T]) -> Buffer {
    make_filled(gpu_dev, name_tag, BufferUsages::VERTEX, bytemuck::cast_slice(row_data))
}

/// Everything the world pass reads besides its view.
#[derive(SystemParam)]
struct OverlayDrawParams<'w, 's> {
    dynamic: Res<'w, OverlayMovingLists>,
    colour_frame: Res<'w, super::PublishedRenderFrame>,
    geometry: Res<'w, OverlayGpuScene>,
    pipes: Res<'w, PipelineCache>,
    gpu_dev: Res<'w, RenderDevice>,
    extracted: Option<Res<'w, ExtractedInspectGeometry>>,
    world_textures: Res<'w, OverlayTextureGroups>,
    world_lightmaps: Res<'w, OverlayLightGroups>,
    diagnostic_pipeline: Res<'w, OverlayPipelines>,
    last_census: Local<'s, Option<String>>,
    perf: Local<'s, PacingWindow>,
    spare: Local<'s, OverlayScratch>,
}

/// Where a model draw sits in the world; anything else is not moved.
fn placement(kind: &HeldDrawKind) -> Mat4 {
    if let HeldDrawKind::Smodel { model_to_world: at, .. } | HeldDrawKind::XModel { model_to_world: at, .. } = kind {
        *at
    } else {
        Mat4::IDENTITY
    }
}

fn draw_overlay_scene(
    view: ViewQuery<(&'static ViewTarget, &'static SceneDepthTexture, &'static ExtractedView, &'static ViewUniformOffset, &'static OverlayCameraBinds)>,
    mut frame_ctx: RenderContext,
    params: OverlayDrawParams,
) {
    let OverlayDrawParams {
        dynamic,
        colour_frame,
        geometry,
        pipes,
        gpu_dev,
        extracted,
        world_textures,
        world_lightmaps,
        diagnostic_pipeline,
        mut last_census,
        mut perf,
        mut spare,
    } = params;
    let waiting_on_overlay = extracted.is_some_and(|extracted| extracted.topmost_gpu_wait);
    if waiting_on_overlay || overlay_off() {
        return;
    }
    let products = &colour_frame.frame_products.0;
    let OverlayScratch { draws, model_rows } = &mut *spare;
    draws.clear();
    let ordered = [FrameOutputKind::Colour, FrameOutputKind::Emissive]
        .into_iter()
        .flat_map(|kind| products.product(kind).draw_sequence.iter());
    draws.extend(ordered.filter(|item| draws_in_overlay(&item.kind, item.key)).copied());
    // Surfaces that no ordered draw reaches are queued separately; draw them first-class.
    for &surf in &geometry.loose_world_surfaces {
        let kind = HeldDrawKind::world(surf);
        let item = HeldDrawItem { kind, key: 0, material_key: None, material_order_no: 0, facet_samplers: Default::default(), camera_zone: None };
        if draws_in_overlay(&item.kind, item.key) {
            draws.push(item);
        }
    }
    if draws.is_empty() && geometry.t6_props.is_empty() && geometry.t6_sky_slot.is_none() {
        return;
    }
    let (col_out, depth, cam, eye_shift, view_bg) = view.into_inner();
    let level_pipeline = pipes.get_render_pipeline(view_bg.level_pipeline);
    let t6_pipeline = |shade: ShadeKind| {
        view_bg
            .coded_pipelines
            .get(&shade)
            .and_then(|&id| pipes.get_render_pipeline(id))
    };
    let sky_variant = pipes.get_render_pipeline(view_bg.sky_variant);
    let model_pipeline = pipes.get_render_pipeline(view_bg.model_pipeline);
    let body_pipeline = pipes.get_render_pipeline(view_bg.body_pipeline);
    if level_pipeline.is_none() && model_pipeline.is_none() && body_pipeline.is_none() {
        return;
    }
    model_rows.clear();
    // One matrix row per draw, so a draw's index is its instance; only
    // models carry a placement, the rest sit at the origin.
    model_rows.extend(draws.iter().map(|item| ModelMatrixRow { columns: placement(&item.kind).to_cols_array_2d() }));
    let model_row_buffer = (!model_rows.is_empty())
        .then(|| vertex_buffer(&gpu_dev, "t6_world_smodel_instances", model_rows));
    // bo2mp: the opaque props, a surface's placements per draw, from
    // their own rows this frame.
    let mut prop_rows = Vec::new();
    let (prop_batches, mut props_culled, mut props_unseen) = t6_opaque_prop_batches(
        &geometry,
        &world_textures,
        &pipes,
        &view_bg.coded_pipelines,
        cam,
        &mut prop_rows,
    );
    let prop_rows_buffer =
        (!prop_rows.is_empty()).then(|| vertex_buffer(&gpu_dev, "bo2mp_prop_rows", &prop_rows));
    // bo2zm: the pass's GPU time (`render/bo2zm_world/elapsed_gpu`), which a
    // driver's frame cap on a background window does not hide.
    use bevy::render::diagnostic::RecordDiagnostics;
    let recorder = frame_ctx.diagnostic_recorder();
    let recorder = recorder.as_deref();
    let gpu_span = recorder.time_span(frame_ctx.command_encoder(), "bo2zm_world");
    let view_binds = (view_bg, eye_shift);
    let mut rp = begin_pass(
        &mut frame_ctx,
        "t6_world_pass",
        col_out,
        depth.get_attachment(StoreOp::Store),
        view_binds,
    );
    let mut bound_shade = None;
    let mut bound_texture = None;
    let mut bound_lightmap = None;
    let mut bound_offset = None;
    let surface_draw = |item: &HeldDrawItem| match item.kind {
        HeldDrawKind::World { surf: surf_ix, .. } => geometry
            .world_surface_draw
            .get(usize::from(surf_ix))
            .copied()
            .unwrap_or(T6_NONE),
        _ => T6_NONE,
    };
    // bo2zm: opaque surfaces first; blended ones after every opaque world
    // surface and prop, in their materials' sort order (as the game draws
    // them: decals, then blend layers, then glass).
    let opaque_order: Vec<usize> = (0..draws.len())
        .filter(|&i| !t6_blended(surface_draw(&draws[i])))
        .collect();
    let mut blended_order: Vec<usize> = (0..draws.len())
        .filter(|&i| t6_blended(surface_draw(&draws[i])))
        .collect();
    blended_order.sort_by_key(|&i| t6_sort_key(surface_draw(&draws[i])));
    let viewport = cam.viewport;
    let rect = [viewport.x, viewport.y, viewport.z, viewport.w].map(|v| v as f32);
    let mut tallies = PassCounts::default();
    let smodel_family = BodyKind {
        kind: ShadeKind::Smodel,
        vertex: geometry.smodel.vertex.as_ref(),
        index: geometry.smodel.index.as_ref(),
        ranges: &geometry.smodel.ranges,
        pipeline: model_pipeline,
    };
    let xmodel_family = BodyKind {
        kind: ShadeKind::XModel,
        vertex: geometry.xmodel.vertex.as_ref(),
        index: geometry.xmodel.index.as_ref(),
        ranges: &geometry.xmodel.ranges,
        pipeline: body_pipeline,
    };
    let mut prop_draws = 0u32;
    let mut prop_batch_draws = 0u32;
    // bo2zm 27c: cloud sheets draw in the effects pass, over the sky and
    // with soft edges (`BO2MP_T6_NO_SOFT`: here, hard-edged).
    let soft_sheets = std::env::var_os("BO2MP_T6_NO_SOFT").is_none();
    // bo2zm: world surfaces drawn untextured (diagnostic colours), a few ids.
    let mut untextured: Vec<u16> = Vec::new();
    let mut untextured_n = 0u32;
    for blended_phase in [false, true] {
        let order = if blended_phase {
            &blended_order
        } else {
            &opaque_order
        };
        for &index in order {
            let item = &draws[index];
            if !draws_in_overlay(&item.kind, item.key) {
                tallies.xmodel.skip();
                continue;
            }
            let (shade, first_ix, ix_count, inst_count) = match item.kind {
                HeldDrawKind::World { surf, .. } => {
                    let (Some(vert_buf), Some(ix_buf), Some(geo_pipe), Some(&(first_ix, ix_count))) = (
                        geometry.world.vertex.as_ref(),
                        geometry.world.index.as_ref(),
                        level_pipeline,
                        geometry.world.ranges.get(usize::from(surf)),
                    ) else {
                        tallies.world.skip();
                        continue;
                    };
                    let Some(binding) =
                        world_surface_binding(&geometry, &world_lightmaps, &world_textures, surf)
                    else {
                        tallies.world.skip();
                        continue;
                    };
                    let key = binding.key();
                    let textured = binding
                        .texture_group
                        .zip(t6_pipeline(key))
                        .zip(binding.lighting);
                    let shade = if let Some(((texture_group, textured_pipeline), (lighting, lighting_id))) =
                        textured
                    {
                        if bound_shade != Some(key) {
                            rp.set_render_pipeline(textured_pipeline);
                            bind_mesh(&mut rp, vert_buf, ix_buf);
                            if style_layers(binding.style)
                                && let Some(layer_vertex) = geometry.world_layer_vertex.as_ref()
                            {
                                rp.set_vertex_buffer(1, layer_vertex.slice(..));
                            }
                            if binding.style & STYLE_SHINE != 0
                                && let Some(probes) = world_textures.probes.as_ref().map(|p| &p.1)
                            {
                                rp.set_bind_group(3, probes, &[]);
                            }
                            bound_shade = Some(key);
                            bound_texture = None;
                            bound_lightmap = None;
                        }
                        if bound_texture != Some(binding.texture_key()) {
                            rp.set_bind_group(1, texture_group, &[]);
                            bound_texture = Some(binding.texture_key());
                        }
                        if bound_lightmap != Some(lighting_id) {
                            rp.set_bind_group(2, lighting, &[]);
                            bound_lightmap = Some(lighting_id);
                        }
                        key
                    } else {
                        untextured_n += 1;
                        if untextured.len() < 12 {
                            untextured.push(surf);
                        }
                        if bound_shade != Some(ShadeKind::World) {
                            rp.set_render_pipeline(geo_pipe);
                            bind_mesh(&mut rp, vert_buf, ix_buf);
                            bound_shade = Some(ShadeKind::World);
                        }
                        ShadeKind::World
                    };
                    tallies.world.draw();
                    let light = u32::from(at(&geometry.world_surface_primary, surf, 0));
                    let instance = if shade == key { binding.instance(light) } else { light };
                    (shade, first_ix, ix_count, instance..instance + 1)
                }
                HeldDrawKind::Smodel { surface, .. } => {
                    let Some((first_ix, ix_count, inst_buf)) = draw_body_surface(
                        &mut rp,
                        &mut bound_shade,
                        &smodel_family,
                        model_row_buffer.as_ref(),
                        surface as usize,
                        index,
                    ) else {
                        tallies.smodel.skip();
                        continue;
                    };
                    tallies.smodel.draw();
                    (ShadeKind::Smodel, first_ix, ix_count, inst_buf)
                }
                HeldDrawKind::XModel { surface, .. } => {
                    let Some((first_ix, ix_count, inst_data)) = draw_body_surface(
                        &mut rp,
                        &mut bound_shade,
                        &xmodel_family,
                        model_row_buffer.as_ref(),
                        surface as usize,
                        index,
                    ) else {
                        tallies.xmodel.skip();
                        continue;
                    };
                    tallies.xmodel.draw();
                    (ShadeKind::XModel, first_ix, ix_count, inst_data)
                }
                // The effect kinds, which `overlay_draws` already left out.
                _ => {
                    tallies.fx.skip();
                    continue;
                }
            };
            if ix_count == 0 {
                match shade {
                    ShadeKind::World | ShadeKind::WorldTextured(_) => {
                        tallies.world.retract();
                    }
                    ShadeKind::Smodel
                    | ShadeKind::PropTextured(_)
                    | ShadeKind::Sky => tallies.smodel.retract(),
                    ShadeKind::XModel => tallies.xmodel.retract(),
                }
                continue;
            }
            let draw_depth = DepthLayer::of_draw(&item.kind, item.key).viewport_depth();
            if bound_offset != Some(draw_depth) {
                apply_viewport(&mut rp, rect, draw_depth);
                bound_offset = Some(draw_depth);
            }
            let (what, indices) = match shade {
                ShadeKind::World | ShadeKind::WorldTextured(_) => ("world", geometry.world.index_count),
                ShadeKind::XModel => ("xmodel", geometry.xmodel.index_count),
                _ => ("static model", geometry.smodel.index_count),
            };
            if !retained_within(what, first_ix, ix_count, indices) {
                continue;
            }
            rp.draw_indexed(first_ix..first_ix.saturating_add(ix_count), 0, inst_count);
        }
        // bo2zm: Black Ops II props of this phase (blended ones by sort key).
        if let (Some(vert_buf), Some(ix_buf), Some(inst_count), Some(light_grp)) = (
            geometry.smodel.vertex.as_ref(),
            geometry.smodel.index.as_ref(),
            geometry.t6_prop_instance_buffer.as_ref(),
            world_lightmaps.lighting_group.as_ref(),
        ) {
            let mut bound_prop: Option<ShadeKind> = None;
            rp.set_bind_group(2, light_grp, &[]);
            let mut bound_slot = None;
            apply_viewport(&mut rp, rect, DepthLayer::World.viewport_depth());
            bind_mesh(&mut rp, vert_buf, ix_buf);
            // bo2mp: the opaque ones, a surface's placements per draw.
            if let Some(rows) = prop_rows_buffer.as_ref().filter(|_| !blended_phase) {
                rp.set_vertex_buffer(1, rows.slice(..));
                for b in &prop_batches {
                    let key = ShadeKind::PropTextured(b.style);
                    if bound_prop != Some(key) {
                        rp.set_render_pipeline(b.pipeline);
                        if let Some(probes) = b.probes {
                            rp.set_bind_group(3, probes, &[]);
                        }
                        bound_prop = Some(key);
                        bound_slot = None;
                    }
                    if bound_slot != Some(b.slot_tag) {
                        rp.set_bind_group(1, b.bind_group, &[]);
                        bound_slot = Some(b.slot_tag);
                    }
                    rp.draw_indexed(b.first_ix..b.first_ix.saturating_add(b.ix_count), 0, b.instances.clone());
                    prop_draws = prop_draws.saturating_add(b.instances.len() as u32);
                    prop_batch_draws += 1;
                }
            }
            rp.set_vertex_buffer(1, inst_count.slice(..));
            // The game's own per-placement draw distance (the fast quality
            // setting hides props at 60% of it).
            let eye = cam.world_from_view.translation();
            let cull_scale = if asset_core::t6_fast() { 0.6 } else { 1.0 };
            let instances_cpu = &geometry.t6_prop_instances;
            let view_planes = view_volume_planes(
                cam.clip_from_view
                    * cam.world_from_view.to_matrix().inverse(),
            );
            let props: &[usize] = if blended_phase {
                &geometry.t6_blended_props
            } else {
                &[]
            };
            for &i in props {
                let [surface, slot, code, instance] = geometry.t6_props[i];
                if code != T6_NONE && code & T6_BLEND_MASK >= T6_SHADOW_ONLY {
                    continue;
                }
                if soft_sheets && sheet_code(code) {
                    continue;
                }
                if let Some(row) = instances_cpu.get(instance as usize)
                    && row[22] > 0.0
                {
                    let d = Vec3::new(row[12], row[13], row[14]) - eye;
                    let cull = row[22] * cull_scale;
                    if d.length_squared() > cull * cull {
                        props_culled += 1;
                        continue;
                    }
                }
                if let (Some(planes), Some(&[x, y, z, radius])) =
                    (view_planes.as_ref(), geometry.prop_bounds.get(i))
                    && radius >= 0.0
                    && planes
                        .iter()
                        .any(|plane| plane.truncate().dot(Vec3::new(x, y, z)) + plane.w < -radius)
                {
                    props_unseen += 1;
                    continue;
                }
                let Some(&(first_ix, ix_count)) = geometry.smodel.ranges.get(surface as usize)
                else {
                    continue;
                };
                // Shine: a lit opaque prop with a normal or specular map.
                let shine = geometry.t6_prop_shine.get(i).copied().unwrap_or([u32::MAX; 3]);
                let flow = prop_style(code) & (STYLE_PROP_FLOW | STYLE_PROP_TILE) != 0
                    || (prop_style(code) & STYLE_PROP_FLAG != 0 && code & T6_UNLIT == 0);
                let shine_group = (prop_shine_code(code)
                    && (shine[0] != u32::MAX || shine[1] != u32::MAX))
                    .then(|| world_textures.probes.as_ref())
                    .flatten()
                    .and_then(|_| {
                        let aux = if flow { shine[2] } else { u32::MAX };
                        world_textures.shine_groups.get(&(slot, shine[0], shine[1], aux, [u32::MAX; 3]))
                    });
                // bo2mp: an animated screen with its constants.
                let anim_group = anim_code(code)
                    .then(|| world_textures.layer_groups.get(&(slot, shine[2], u32::MAX)))
                    .flatten();
                let style = prop_style(code)
                    | if shine_group.is_some() { STYLE_SHINE } else { 0 }
                    | if anim_group.is_some() { STYLE_ANIM } else { 0 };
                let key = ShadeKind::PropTextured(style);
                let (Some(geo_pipe), Some(view_bg)) = (
                    t6_pipeline(key),
                    shine_group
                        .or(anim_group)
                        .or_else(|| world_textures.bind_groups.get(&slot)),
                ) else {
                    continue;
                };
                if ix_count == 0 || !retained_within("prop", first_ix, ix_count, geometry.smodel.index_count) {
                    continue;
                }
                if bound_prop != Some(key) {
                    rp.set_render_pipeline(geo_pipe);
                    if shine_group.is_some()
                        && let Some(probes) = world_textures.probes.as_ref().map(|p| &p.1)
                    {
                        rp.set_bind_group(3, probes, &[]);
                    }
                    bound_prop = Some(key);
                    bound_slot = None;
                }
                let slot_key = (slot, shine_group.or(anim_group).map(|_| shine));
                if bound_slot != Some(slot_key) {
                    rp.set_bind_group(1, view_bg, &[]);
                    bound_slot = Some(slot_key);
                }
                rp.draw_indexed(first_ix..first_ix.saturating_add(ix_count), 0, instance..instance + 1);
                prop_draws = prop_draws.saturating_add(1);
            }
            // The world draws rebind their buffers, geo_pipe and viewport.
            bound_shade = None;
            bound_texture = None;
            bound_lightmap = None;
            bound_offset = None;
        }
        // bo2mp: the sky between the phases, as BO2 lays it: behind every
        // opaque surface, and under the blended ones, which leave depth
        // alone. Drawn last, it painted over Cargo's far see-through skyline
        // towers and left their opaque crowns floating in the sky.
        if !blended_phase
            && let (Some(geo_pipe), Some((_, sky))) = (sky_variant, world_lightmaps.sky.as_ref())
        {
            apply_viewport(&mut rp, rect, DepthLayer::World.viewport_depth());
            rp.set_render_pipeline(geo_pipe);
            rp.set_bind_group(1, sky, &[]);
            rp.draw(0..3, 0..1);
            bound_shade = None;
            bound_texture = None;
            bound_lightmap = None;
            bound_offset = None;
        }
    }
    // bo2zm: effects (over the sky, which only fills what nothing else
    // covered), then the first-person gun and arms in the viewmodel's depth
    // range; opaque surfaces first, then blended ones by sort key. The
    // world's blended effects draw in a pass of their own over a read-only
    // depth they also read: BO2's zfeather shaders fade a sprite where it
    // nears what is behind it.
    let soft_group = gpu_dev.create_bind_group(
        "bo2zm_soft_depth",
        &pipes.get_bind_group_layout(&diagnostic_pipeline.soft_layout),
        &BindGroupEntries::single(depth.view()),
    );
    let mut viewmodel_draws = 0u32;
    viewmodel_draws += draw_dynamic_list(
        &mut rp,
        dynamic.marks.as_ref(),
        world_lightmaps.lighting_group.as_ref(),
        DepthLayer::World,
        rect,
        &|_| true,
        None,
        &pipes,
        &view_bg.coded_pipelines,
        &world_textures,
    );
    viewmodel_draws += draw_dynamic_list(
        &mut rp,
        dynamic.missiles.as_ref(),
        world_lightmaps.lighting_group.as_ref(),
        DepthLayer::World,
        rect,
        &|_| true,
        None,
        &pipes,
        &view_bg.coded_pipelines,
        &world_textures,
    );
    viewmodel_draws += draw_dynamic_list(
        &mut rp,
        dynamic.script_static.as_ref(),
        world_lightmaps.lighting_group.as_ref(),
        DepthLayer::World,
        rect,
        &|_| true,
        None,
        &pipes,
        &view_bg.coded_pipelines,
        &world_textures,
    );
    viewmodel_draws += draw_dynamic_list(
        &mut rp,
        dynamic.effects.as_ref(),
        world_lightmaps.lighting_group.as_ref(),
        DepthLayer::World,
        rect,
        &|code| !t6_blended(code),
        None,
        &pipes,
        &view_bg.coded_pipelines,
        &world_textures,
    );
    drop(rp);
    let mut rp = begin_pass(
        &mut frame_ctx,
        "bo2zm_effects_pass",
        col_out,
        depth.read_only_attachment(),
        view_binds,
    );
    // bo2zm 27c: Black Ops II's cloud sheets (Die Rise's cloud floor, `scrolling_zfeather`), by
    // sort key: over the sky, fading where the scene behind them is near (its depth is read-only
    // here).
    if soft_sheets
        && let (Some(vert_buf), Some(idx_buf), Some(inst_count), Some(lighting_group)) = (
            geometry.smodel.vertex.as_ref(),
            geometry.smodel.index.as_ref(),
            geometry.t6_prop_instance_buffer.as_ref(),
            world_lightmaps.lighting_group.as_ref(),
        )
    {
        let eye = cam.world_from_view.translation();
        let cull_scale = if asset_core::t6_fast() { 0.6 } else { 1.0 };
        let mut bound = false;
        for &i in geometry.t6_blended_props.iter() {
            let [surface, slot, code, instance] = geometry.t6_props[i];
            if !sheet_code(code) {
                continue;
            }
            if let Some(row) = geometry.t6_prop_instances.get(instance as usize)
                && row[22] > 0.0
            {
                let d = Vec3::new(row[12], row[13], row[14]) - eye;
                let cull = row[22] * cull_scale;
                if d.length_squared() > cull * cull {
                    props_culled += 1;
                    continue;
                }
            }
            let Some(&(first_ix, ix_count)) = geometry.smodel.ranges.get(surface as usize) else {
                continue;
            };
            let (Some(geo_pipe), Some(view_bg)) = (
                t6_pipeline(ShadeKind::PropTextured(prop_style(code) | STYLE_SOFT)),
                world_textures.bind_groups.get(&slot),
            ) else {
                continue;
            };
            if ix_count == 0 {
                continue;
            }
            if !bound {
                apply_viewport(&mut rp, rect, DepthLayer::World.viewport_depth());
                rp.set_bind_group(2, lighting_group, &[]);
                rp.set_bind_group(3, &soft_group, &[]);
                bind_mesh(&mut rp, vert_buf, idx_buf);
                rp.set_vertex_buffer(1, inst_count.slice(..));
                bound = true;
            }
            rp.set_render_pipeline(geo_pipe);
            rp.set_bind_group(1, view_bg, &[]);
            rp.draw_indexed(first_ix..first_ix.saturating_add(ix_count), 0, instance..instance + 1);
            prop_draws = prop_draws.saturating_add(1);
        }
    }
    viewmodel_draws += draw_dynamic_list(
        &mut rp,
        dynamic.effects.as_ref(),
        world_lightmaps.lighting_group.as_ref(),
        DepthLayer::World,
        rect,
        &t6_blended,
        // (`BO2MP_T6_NO_SOFT`: hard edges, a test aid for comparing.)
        std::env::var_os("BO2MP_T6_NO_SOFT").is_none().then_some(&soft_group),
        &pipes,
        &view_bg.coded_pipelines,
        &world_textures,
    );
    drop(rp);
    let mut rp = begin_pass(
        &mut frame_ctx,
        "bo2zm_viewmodel_pass",
        col_out,
        depth.get_attachment(StoreOp::Store),
        view_binds,
    );
    viewmodel_draws += draw_dynamic_list(
        &mut rp,
        dynamic.viewmodel.as_ref(),
        world_lightmaps.lighting_group.as_ref(),
        DepthLayer::Gun,
        rect,
        &|_| true,
        None,
        &pipes,
        &view_bg.coded_pipelines,
        &world_textures,
    );
    viewmodel_draws += draw_dynamic_list(
        &mut rp,
        dynamic.viewmodel_effects.as_ref(),
        world_lightmaps.lighting_group.as_ref(),
        DepthLayer::Gun,
        rect,
        &|_| true,
        None,
        &pipes,
        &view_bg.coded_pipelines,
        &world_textures,
    );
    let _ = viewmodel_draws;
    drop(rp);
    gpu_span.end(frame_ctx.command_encoder());
    tallies.smodel.draws = tallies.smodel.draws.saturating_add(prop_draws);
    // bo2zm: frames per second, frame time and prop counts every 5 s.
    let now = Instant::now();
    if let Some(prev) = perf.last_frame {
        let millis = now.duration_since(prev).as_secs_f32() * 1000.0;
        perf.worst_ms = perf.worst_ms.max(millis);
        // BO2MP_PIPELINE_LOG=1: a frame that took over half a second, as it
        // happens (to line up with what was built or loaded then).
        static LOG: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
        if millis >= 500.0 && *LOG.get_or_init(|| std::env::var_os("BO2MP_PIPELINE_LOG").is_some()) {
            diag::info!(World, "slow frame: {millis:.0} ms");
        }
    }
    perf.last_frame = Some(now);
    perf.frame_n += 1;
    perf.props_drawn += u64::from(prop_draws);
    perf.prop_batches += u64::from(prop_batch_draws);
    perf.props_culled += u64::from(props_culled);
    perf.props_unseen += u64::from(props_unseen);
    let first_ix = *perf.window_start.get_or_insert(now);
    let span = now.duration_since(first_ix).as_secs_f32();
    // (`BO2MP_PERF_WINDOW=secs`: a shorter window, for tests that must end
    // before another window takes the foreground.)
    static WINDOW: std::sync::OnceLock<f32> = std::sync::OnceLock::new();
    let window = *WINDOW.get_or_init(|| {
        std::env::var("BO2MP_PERF_WINDOW")
            .ok()
            .and_then(|v| v.parse().ok())
            .filter(|v: &f32| *v >= 0.5)
            .unwrap_or(5.0)
    });
    if span >= window && perf.frame_n > 1 {
        let n = perf.frame_n as f32;
        diag::info!(
            World,
            "bo2zm perf: {:.1} fps, {:.2} ms average, {:.2} ms worst; props {:.0} drawn ({:.0} opaque draws), {:.0} hidden by distance, {:.0} out of view per frame; quality {}",
            (n - 1.0) / span,
            span * 1000.0 / (n - 1.0),
            perf.worst_ms,
            perf.props_drawn as f32 / n,
            perf.prop_batches as f32 / n,
            perf.props_culled as f32 / n,
            perf.props_unseen as f32 / n,
            if asset_core::t6_fast() { "fast" } else { "full" }
        );
        *perf = PacingWindow {
            last_census_at: perf.last_census_at,
            last_frame: perf.last_frame,
            ..Default::default()
        };
    }
    let loose_queued = geometry.loose_world_surfaces.len();
    let line = format!(
        "drawsurf geometry diagnostic: world_draws={} smodel_draws={} xmodel_draws={} fx_draws={} world_skipped={} smodel_skipped={} xmodel_skipped={} fx_skipped={} loose_queued={loose_queued} untextured={untextured_n} {untextured:?}",
        tallies.world.draws,
        tallies.smodel.draws,
        tallies.xmodel.draws,
        tallies.fx.draws,
        tallies.world.skip_n,
        tallies.smodel.skip_n,
        tallies.xmodel.skip_n,
        tallies.fx.skip_n,
    );
    // The census line changes nearly every frame, so it is written only when the render log is
    // switched on (BO2MP_RENDER_LOG), at most every five seconds: a slow write on this thread
    // would hold the frame up.
    static RENDER_LOG_ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    let logging = *RENDER_LOG_ON.get_or_init(|| std::env::var_os("BO2MP_RENDER_LOG").is_some());
    let waited_long_enough = perf
        .last_census_at
        .is_none_or(|at| now.duration_since(at).as_secs_f32() >= 5.0);
    let changed = last_census.as_ref() != Some(&line);
    if logging && waited_long_enough && changed {
        perf.last_census_at = Some(now);
        diag::warn!(World, "{line}");
        *last_census = Some(line);
    }
}

pub(super) fn register(host: &mut App) {
    if overlay_off() {
        return;
    }
    bevy::asset::embedded_asset!(host, "scene_geo.wgsl");
    bevy::asset::embedded_asset!(host, "t6_sun_shadow.wgsl");
    super::bo2_bloom::register(host);
    super::bo2_afterlife::register(host);
    super::bo2_dof::register(host);
    let Some(render_sub) = host.get_sub_app_mut(bevy::render::RenderApp) else {
        return;
    };
    // Each frame: copy the scene and the moving lists to the GPU, then bind
    // the cameras and the texture and light groups.
    let upload_sys = (mirror_scene_to_gpu, mirror_moving_to_gpu);
    let bindings = (bind_overlay_cameras, bind_texture_groups, bind_light_groups);
    // In the main pass: the sun's shadow map first, then the overlay itself,
    // after the opaque pass and the exact-colour draws.
    let shadow_first = sun_shadow::draw_sun_shadow.before(draw_overlay_scene);
    let overlay_pass = draw_overlay_scene.in_set(OverlaySceneDrawSet).after(main_opaque_pass_3d).after(super::draw::PreciseColourDrawSet);
    render_sub
        .init_resource::<SpecializedRenderPipelines<OverlayPipelines>>()
        .init_resource::<ExtractedInspectGeometry>()
        .init_resource::<ExtractedT6Dynamic>()
        .init_resource::<OverlayGpuScene>()
        .init_resource::<OverlayMovingLists>()
        .init_resource::<OverlayTextureGroups>()
        .init_resource::<OverlayLightGroups>()
        .init_resource::<skin::SkinCache>()
        .add_systems(RenderStartup, (make_overlay_layouts, sun_shadow::init, skin::init))
        .add_systems(Render, (upload_sys.in_set(RenderSystems::PrepareResources), bindings.in_set(RenderSystems::PrepareBindGroups)))
        .add_systems(Core3d, (shadow_first, overlay_pass).in_set(Core3dSystems::MainPass));
}

/// bo2zm: the diagnostic draw, for the guest weapons drawn after it.
#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub(super) struct OverlaySceneDrawSet;

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, HashMap};

    use bevy::shader::ShaderDefVal;
    use naga_oil::compose::{
        ComposableModuleDescriptor, Composer, NagaModuleDescriptor, ShaderDefValue, ShaderLanguage,
    };

    use super::*;

    /// The parts of Bevy's `View` the world shader reads (field names and
    /// types; the layout is Bevy's business, not this check's).
    const VIEW: &str = "#define_import_path bevy_render::view
struct View {
    clip_from_world: mat4x4<f32>,
    world_from_clip: mat4x4<f32>,
    world_from_view: mat4x4<f32>,
    view_from_world: mat4x4<f32>,
    view_from_clip: mat4x4<f32>,
    world_position: vec3<f32>,
    viewport: vec4<f32>,
}
";

    /// Every draw code whose bits can change a pipeline's flags: the four
    /// blends a draw keeps, then each flag bit on and off (the raster bits
    /// and the sort key change no flag), and the code of no draw.
    fn all_codes() -> Vec<u32> {
        let flags = [
            T6_UNLIT, T6_EFFECT, T6_LAYERED, T6_ALPHA_TEST, 0x1000, 0x2000, 0x4000, 0x8000,
            1 << T6_LAYER_SHIFT, 2 << T6_LAYER_SHIFT, 4 << T6_LAYER_SHIFT, 8 << T6_LAYER_SHIFT,
            T6_OBJECTIVE, T6_FOLIAGE, T6_WATER, T6_FLOW,
        ];
        let mut codes = vec![T6_NONE];
        for set in 0u32..1 << flags.len() {
            let bits = flags.iter().enumerate().filter(|&(i, _)| set & (1 << i) != 0).fold(0, |c, (_, f)| c | f);
            codes.extend((0..4).map(|blend| bits | blend));
        }
        codes
    }

    /// Every distinct shader build the world pipelines ask for: entry points
    /// and flags, with the first style that asks for it.
    fn all_builds() -> BTreeMap<(&'static str, &'static str, Vec<String>), ShadeKind> {
        let codes = all_codes();
        let props: Vec<[u32; 4]> = codes.iter().map(|&code| [0, 0, code, 0]).collect();
        let mut builds = BTreeMap::new();
        for shade in t6_styles(&codes, &props) {
            let defs = shade.shader_defs().iter().map(|def| format!("{def:?}")).collect();
            builds.entry((shade.vertex_entry(), shade.fragment_entry(), defs)).or_insert(shade);
        }
        builds
    }

    /// bo2mp: each world and prop pipeline's shader, built with its flags as
    /// Bevy builds it, must compile and validate: a variant that does not
    /// (the tree canopy once declared `tint` twice) leaves its surfaces
    /// undrawn, with the error on stderr only.
    #[test]
    fn every_world_shader_variant_compiles() {
        let source = include_str!("scene_geo.wgsl");
        let mut composer = Composer::default();
        composer
            .add_composable_module(ComposableModuleDescriptor {
                source: VIEW,
                file_path: "view.wgsl",
                language: ShaderLanguage::Wgsl,
                ..Default::default()
            })
            .unwrap();
        let builds = all_builds();
        let mut failed = Vec::new();
        for ((vertex, fragment, _), shade) in &builds {
            let shader_defs: HashMap<String, ShaderDefValue> = shade
                .shader_defs()
                .into_iter()
                .map(|def| match def {
                    ShaderDefVal::Bool(name, on) => (name, ShaderDefValue::Bool(on)),
                    ShaderDefVal::Int(name, value) => (name, ShaderDefValue::Int(value)),
                    ShaderDefVal::UInt(name, value) => (name, ShaderDefValue::UInt(value)),
                })
                .collect();
            let built = composer.make_naga_module(NagaModuleDescriptor {
                source,
                file_path: "scene_geo.wgsl",
                shader_defs,
                ..Default::default()
            });
            match built {
                Ok(module) => {
                    for entry in [vertex, fragment] {
                        if !module.entry_points.iter().any(|point| point.name == *entry) {
                            failed.push(format!("{shade:?}: no entry point `{entry}`"));
                        }
                    }
                }
                Err(error) => failed.push(format!("{shade:?}: {}", error.emit_to_string(&composer))),
            }
        }
        println!("{} world shader variants built, {} failed", builds.len(), failed.len());
        assert!(failed.is_empty(), "{} of {} variants fail:\n{}", failed.len(), builds.len(), failed.join("\n"));
    }
}
