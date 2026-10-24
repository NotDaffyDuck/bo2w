//! The one tessellated GPU list the HUD layers share: each layer turns its
//! quads into runs of vertices and indices, remembers its last result in a
//! latch, and appends it to the frame the renderer reads.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::time::Instant;

use bevy::prelude::*;

use crate::draw2d::{DRAW2D_QUAD_INDICES, Draw2dQuad};
use crate::images::HudImages;
use crate::ui_write::adopt_display;

/// One vertex as the GPU list stores it.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HudTessVertex {
    pub xyzw: [f32; 4],
    pub color_bgra: [u8; 4],
    pub uv: [f32; 2],
    pub packed_normal: u32,
}

// How many vertices and indices one batch may take before it is cut. The
// game's real buffer sizes are not pinned down yet; these are placeholders.
const BATCH_VERTEX_CAP: u32 = 0x154a;
const BATCH_INDEX_CAP: u32 = 0x10_0000;

impl From<hud_t6::stretch_pic_cmd::GfxTessVertex2d> for HudTessVertex {
    fn from(vertex: hud_t6::stretch_pic_cmd::GfxTessVertex2d) -> Self {
        // The game packs the colour as RGBA bytes; the GPU vertex wants BGRA.
        let [red, green, blue, alpha] = vertex.color.to_le_bytes();
        Self {
            xyzw: vertex.xyzw,
            color_bgra: [blue, green, red, alpha],
            uv: vertex.texcoord,
            packed_normal: vertex.packed_normal,
        }
    }
}

/// How a batch is shaded.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum HudTessTechnique {
    /// The image multiplied by the vertex colour.
    #[default]
    Modulate,
    /// The saved copy of the screen, tinted by the vertex colour.
    SavedScreen,
}

/// A run of indexed triangles drawn with one image.
#[derive(Clone, Debug)]
pub struct HudTessBatch {
    pub image: Handle<Image>,
    pub technique: HudTessTechnique,
    pub state_bits: Option<[u32; 2]>,
    pub first_index: u32,
    pub index_count: u32,
    pub first_vertex: u32,
    pub vertex_count: u32,
}

/// Everything the renderer draws for the HUD this frame.
#[derive(Resource, Clone, Debug, Default)]
pub struct HudTessGpuFrame {
    pub vertices: Vec<HudTessVertex>,
    pub indices: Vec<u16>,
    pub batches: Vec<HudTessBatch>,
    pub surface_w: f32,
    pub surface_h: f32,
    pub visible: bool,
    pub saved_screen_sequence: u64,
}

impl HudTessGpuFrame {
    pub fn clear_geometry(&mut self) {
        self.vertices.clear();
        self.indices.clear();
        self.batches.clear();
    }

    /// Adds a layer's list after what the frame holds, moving its indices and
    /// vertex offsets past the existing ones.
    pub(crate) fn append_packed(&mut self, list: &PackedList) {
        let vertex_offset = u32::from(self.vertices.len() as u16);
        let index_offset = self.indices.len() as u32;
        self.vertices.extend_from_slice(&list.vertices);
        self.indices.extend_from_slice(&list.indices);
        self.batches
            .extend(list.batches.iter().map(|batch| HudTessBatch {
                image: batch.image.clone(),
                technique: batch.technique,
                state_bits: batch.state_bits,
                first_index: index_offset.saturating_add(batch.first_index),
                index_count: batch.index_count,
                first_vertex: vertex_offset.saturating_add(batch.first_vertex),
                vertex_count: batch.vertex_count,
            }));
    }
}

/// A layer's finished vertices, indices and batches, offsets relative to it.
#[derive(Clone, Default)]
pub(crate) struct PackedList {
    vertices: Vec<HudTessVertex>,
    indices: Vec<u16>,
    batches: Vec<HudTessBatch>,
}

impl PackedList {
    pub(crate) fn is_empty(&self) -> bool {
        self.vertices.is_empty()
    }
}

/// What a layer's host node remembers between frames: the list it packed, the
/// fingerprint of the quads behind it, and whether it is currently hidden.
#[derive(Component, Default)]
pub struct GpuListLatch {
    packed: PackedList,
    fingerprint: u64,
    shown: usize,
    hidden: bool,
}

impl GpuListLatch {
    pub fn mark_hidden(&mut self) {
        self.hidden = true;
    }

    fn clear(&mut self) {
        self.hidden = true;
        self.packed = PackedList::default();
    }
}

pub fn register(app: &mut App) {
    app.init_resource::<HudTessPass>()
        .init_resource::<HudTessGpuFrame>();
}

/// What a layer asks of the flush this frame.
#[derive(Default)]
pub enum TessJob {
    /// Nothing new: draw whatever the latch still holds.
    #[default]
    None,
    Hide,
    Quads(Vec<Draw2dQuad>),
}

/// The layers' requests for this frame.
#[derive(Resource, Default)]
pub struct HudTessPass {
    pub overhead_names: TessJob,
    pub hud_elems: TessJob,
    pub hud_elems_back: TessJob,
}

// The cost of the flush systems' own bodies, summed over a frame. It is read
// from inside the bodies and not from the time between two systems: that gap
// belongs to the executor, which `.chain()` only orders, so it would measure
// the schedule and not the HUD.
static BODY_NANOS: AtomicU64 = AtomicU64::new(0);
static JOB_COUNT: AtomicU32 = AtomicU32::new(0);

/// Adds the time its scope lives to the frame's HUD tess total.
#[must_use = "the body is timed until this is dropped"]
pub struct TessBody(Instant);

impl TessBody {
    pub fn open() -> Self {
        Self(Instant::now())
    }
}

impl Drop for TessBody {
    fn drop(&mut self) {
        BODY_NANOS.fetch_add(self.0.elapsed().as_nanos() as u64, Ordering::Relaxed);
    }
}

/// The frame's total in milliseconds and its job count; both restart at zero.
pub fn take_tess_body_cost() -> (f32, u32) {
    let nanos = BODY_NANOS.swap(0, Ordering::Relaxed);
    let jobs = JOB_COUNT.swap(0, Ordering::Relaxed);
    (nanos as f32 / 1e6, jobs)
}

#[allow(clippy::too_many_arguments)]
pub fn apply_tess_job(
    job: TessJob,
    host: &mut Node,
    latch: &mut GpuListLatch,
    hud_images: &mut HudImages,
    images: &mut Assets<Image>,
    frame: &mut HudTessGpuFrame,
    surface_w: f32,
    surface_h: f32,
) {
    JOB_COUNT.fetch_add(1, Ordering::Relaxed);
    match job {
        TessJob::None => {
            if !latch.hidden && !latch.packed.is_empty() {
                frame.append_packed(&latch.packed);
            }
        }
        TessJob::Hide => hide_list(host, latch),
        TessJob::Quads(quads) if quads.is_empty() => hide_list(host, latch),
        TessJob::Quads(quads) => {
            present_list(PresentInput {
                host,
                latch,
                quads: &quads,
                hud_images,
                images,
                frame,
                surface_w,
                surface_h,
            });
        }
    }
}

pub fn hide_host(node: &mut Node) {
    adopt_display(node, Display::None);
}

pub fn hide_list(host: &mut Node, latch: &mut GpuListLatch) {
    hide_host(host);
    latch.clear();
}

/// Cuts a quad to its clip rectangle, trimming the texture coordinates to
/// match. `None` when nothing of it is left.
pub fn clip_aa_quad(quad: &Draw2dQuad) -> Option<Draw2dQuad> {
    let Some([clip_left, clip_top, clip_right, clip_bottom]) = quad.clip else {
        return Some(quad.clone());
    };
    let [left, top] = quad.xy[0];
    let [right, bottom] = quad.xy[2];
    let kept_left = left.max(clip_left);
    let kept_top = top.max(clip_top);
    let kept_right = right.min(clip_right);
    let kept_bottom = bottom.min(clip_bottom);
    if kept_right <= kept_left || kept_bottom <= kept_top {
        return None;
    }
    let unchanged = [
        kept_left - left,
        kept_top - top,
        kept_right - right,
        kept_bottom - bottom,
    ]
    .iter()
    .all(|shift| shift.abs() <= f32::EPSILON);
    if unchanged {
        return Some(quad.clone());
    }
    let width = right - left;
    let height = bottom - top;
    if width.abs() <= f32::EPSILON || height.abs() <= f32::EPSILON {
        return None;
    }
    let along_x = |x: f32| (x - left) / width;
    let along_y = |y: f32| (y - top) / height;
    // Texture coordinate at a fraction across and down the original quad.
    let texel = |across: f32, down: f32| {
        let upper = mix(quad.st[0], quad.st[1], across);
        let lower = mix(quad.st[3], quad.st[2], across);
        mix(upper, lower, down)
    };
    let (u0, u1) = (along_x(kept_left), along_x(kept_right));
    let (v0, v1) = (along_y(kept_top), along_y(kept_bottom));
    let mut cut = quad.clone();
    cut.xy = [
        [kept_left, kept_top],
        [kept_right, kept_top],
        [kept_right, kept_bottom],
        [kept_left, kept_bottom],
    ];
    cut.st = [texel(u0, v0), texel(u1, v0), texel(u1, v1), texel(u0, v1)];
    Some(cut)
}

fn mix(from: [f32; 2], to: [f32; 2], amount: f32) -> [f32; 2] {
    [
        from[0] + (to[0] - from[0]) * amount,
        from[1] + (to[1] - from[1]) * amount,
    ]
}

/// A quad that survived clipping and found its image.
struct Prepared {
    texture: Handle<Image>,
    state_bits: Option<[u32; 2]>,
    quad: Draw2dQuad,
}

pub struct PresentInput<'a> {
    pub host: &'a mut Node,
    pub latch: &'a mut GpuListLatch,
    pub quads: &'a [Draw2dQuad],
    pub hud_images: &'a mut HudImages,
    pub images: &'a mut Assets<Image>,
    pub frame: &'a mut HudTessGpuFrame,
    pub surface_w: f32,
    pub surface_h: f32,
}

/// Packs the quads (or reuses the latched list when they are the same ones as
/// last time) and appends the result to the frame. Returns how many quads were
/// drawn.
pub fn present_list(input: PresentInput<'_>) -> usize {
    let PresentInput {
        host,
        latch,
        quads,
        hud_images,
        images,
        frame,
        surface_w,
        surface_h,
    } = input;
    if surface_w <= 1.0 || surface_h <= 1.0 {
        hide_list(host, latch);
        return 0;
    }
    let ready: Vec<Prepared> = quads
        .iter()
        .filter_map(|quad| {
            let clipped = clip_aa_quad(quad)?;
            let texture =
                hud_images.get_native(clipped.material_namespace, &clipped.material, images)?;
            Some(Prepared {
                texture,
                // The old zone material state bits left with the old menu catalog.
                state_bits: None,
                quad: clipped,
            })
        })
        .collect();
    let drawn = ready.len();
    let fingerprint = fingerprint_quads(drawn, ready.iter().map(|entry| &entry.quad));
    let unchanged = !latch.hidden
        && latch.fingerprint == fingerprint
        && latch.shown == drawn
        && !latch.packed.is_empty();
    if unchanged {
        frame.append_packed(&latch.packed);
        return drawn;
    }
    if drawn == 0 {
        hide_list(host, latch);
        return 0;
    }
    hide_host(host);
    *latch = GpuListLatch {
        packed: pack_runs(&ready),
        fingerprint,
        shown: drawn,
        hidden: false,
    };
    frame.append_packed(&latch.packed);
    drawn
}

fn fingerprint_quads<'a>(count: usize, quads: impl Iterator<Item = &'a Draw2dQuad>) -> u64 {
    let mut hasher = DefaultHasher::new();
    count.hash(&mut hasher);
    for quad in quads {
        hash_quad(&mut hasher, quad);
    }
    hasher.finish()
}

fn hash_quad(hasher: &mut impl Hasher, quad: &Draw2dQuad) {
    for corner in quad.xy.iter().chain(quad.st.iter()) {
        corner[0].to_bits().hash(hasher);
        corner[1].to_bits().hash(hasher);
    }
    for channel in quad.color {
        channel.to_bits().hash(hasher);
    }
    quad.material.hash(hasher);
    quad.material_namespace.hash(hasher);
    match quad.clip {
        Some(rect) => {
            1u8.hash(hasher);
            for edge in rect {
                edge.to_bits().hash(hasher);
            }
        }
        None => 0u8.hash(hasher),
    }
}

fn quad_vertices(quad: &Draw2dQuad) -> [HudTessVertex; 4] {
    let color = hud_t6::stretch_pic_cmd::convert_color_to_bytes(quad.color);
    std::array::from_fn(|corner| {
        hud_t6::stretch_pic_cmd::set_vertex_2d(
            quad.xy[corner][0],
            quad.xy[corner][1],
            1.0,
            quad.st[corner][0],
            quad.st[corner][1],
            color,
        )
        .into()
    })
}

/// Groups neighbouring quads that share an image, state and material into
/// batches, cutting a batch whenever it would outgrow the caps.
fn pack_runs(prepared: &[Prepared]) -> PackedList {
    let mut packed = PackedList::default();
    let mut start = 0;
    while start < prepared.len() {
        let head = &prepared[start];
        let first_index = packed.indices.len() as u32;
        let first_vertex = packed.vertices.len() as u16;
        let mut vertex_count = 0u32;
        let mut index_count = 0u32;
        let mut end = start;
        while let Some(next) = prepared.get(end) {
            let same_run = next.texture.id() == head.texture.id()
                && next.state_bits == head.state_bits
                && next.quad.material == head.quad.material
                && next.quad.material_namespace == head.quad.material_namespace;
            if !same_run
                || hud_t6::stretch_pic_cmd::tess_quad_overflows(
                    vertex_count,
                    index_count,
                    BATCH_VERTEX_CAP,
                    BATCH_INDEX_CAP,
                )
            {
                break;
            }
            let base = (vertex_count as usize).min(usize::from(u16::MAX)) as u16;
            packed.vertices.extend(quad_vertices(&next.quad));
            packed
                .indices
                .extend(DRAW2D_QUAD_INDICES.iter().map(|&step| base.saturating_add(step)));
            vertex_count = vertex_count.saturating_add(4);
            index_count = index_count.saturating_add(6);
            end += 1;
        }
        packed.batches.push(HudTessBatch {
            image: head.texture.clone(),
            technique: HudTessTechnique::Modulate,
            state_bits: head.state_bits,
            first_index,
            index_count,
            first_vertex: u32::from(first_vertex),
            vertex_count,
        });
        start = end;
    }
    packed
}

/// A single quad drawn from the saved copy of the screen.
pub(crate) fn pack_saved_screen(quad: &Draw2dQuad, stand_in: Handle<Image>) -> PackedList {
    let mut packed = pack_modulate(quad, stand_in);
    packed
        .batches
        .iter_mut()
        .for_each(|batch| batch.technique = HudTessTechnique::SavedScreen);
    packed
}

/// A single quad drawn with one image.
pub(crate) fn pack_modulate(quad: &Draw2dQuad, image: Handle<Image>) -> PackedList {
    PackedList {
        vertices: quad_vertices(quad).to_vec(),
        indices: DRAW2D_QUAD_INDICES.to_vec(),
        batches: vec![HudTessBatch {
            image,
            technique: HudTessTechnique::Modulate,
            state_bits: None,
            first_index: 0,
            index_count: 6,
            first_vertex: 0,
            vertex_count: 4,
        }],
    }
}
