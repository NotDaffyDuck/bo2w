//! The one tessellated GPU list the HUD layers share: each layer turns its
//! quads into batches of vertices and indices, remembers its last result in a
//! latch, and appends it to the frame the renderer reads.

use std::hash::{DefaultHasher, Hash, Hasher};
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::time::Instant;

use bevy::prelude::*;
use bevy::ui::Display;
use bevy::ui::Node;

use crate::draw2d::{FlatPaintRect, QUAD_CORNER_ORDER};
use crate::images::HudPictures;
use crate::ui_emit::take_over_monitor;

const RELAXED: Ordering = Ordering::Relaxed;

/// One vertex as the GPU list stores it.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HudGpuVertex {
    pub clip_pos: [f32; 4],
    pub blue_green_red_alpha: [u8; 4],
    pub tex_st: [f32; 2],
    pub squeezed_facing: u32,
}

// How many vertices and indices one batch may take before it is cut. The
// game's real buffer sizes are not pinned down yet; these are placeholders.
const BATCH_VERTEX_CAP: u32 = 0x154a;
const BATCH_INDEX_CAP: u32 = 0x10_0000;

impl From<hud_t6::stretch_pic_cmd::OverlayCorner2d> for HudGpuVertex {
    fn from(vertex: hud_t6::stretch_pic_cmd::OverlayCorner2d) -> Self {
        // The game packs the colour as RGBA bytes; the GPU vertex wants BGRA.
        let [red, green, blue, alpha] = vertex.color.to_le_bytes();
        Self {
            clip_pos: vertex.quad,
            blue_green_red_alpha: [blue, green, red, alpha],
            tex_st: vertex.uv_semantic,
            squeezed_facing: vertex.squeezed_facing,
        }
    }
}

/// How a batch is shaded.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum HudStitchMethod {
    /// The image multiplied by the vertex colour.
    #[default]
    Modulate,
    /// The saved copy of the screen, tinted by the vertex colour.
    SavedScreen,
}

/// A run of indexed triangles drawn with one image.
#[derive(Clone, Debug)]
pub struct HudStitchBatch {
    pub image: Handle<Image>,
    pub technique: HudStitchMethod,
    pub blend_state_words: Option<[u32; 2]>,
    pub first_index: u32,
    pub index_count: u32,
    pub vertex_base: u32,
    pub vertex_count: u32,
}

impl HudStitchBatch {
    /// The same batch with its index and vertex starts pushed past `first_offset`
    /// and `vertex_base`.
    fn shifted_by(&self, first_offset: u32, vertex_base: u32) -> Self {
        Self {
            first_index: first_offset.saturating_add(self.first_index),
            vertex_base: vertex_base.saturating_add(self.vertex_base),
            ..self.clone()
        }
    }
}

/// Everything the renderer draws for the HUD this frame.
#[derive(Resource, Clone, Debug, Default)]
pub struct OverlayMeshBatch {
    pub vertices: Vec<HudGpuVertex>,
    pub indices: Vec<u16>,
    pub batches: Vec<HudStitchBatch>,
    pub canvas_w: f32,
    pub canvas_h: f32,
    pub visible: bool,
    pub saved_screen_sequence: u64,
}

impl OverlayMeshBatch {
    pub fn reset_geometry(&mut self) {
        let Self {
            vertices,
            indices,
            batches,
            ..
        } = self;
        vertices.clear();
        indices.clear();
        batches.clear();
    }

    /// Adds a layer's list after what the frame holds, moving its indices and
    /// vertex offsets past the existing ones.
    pub(crate) fn append_list(&mut self, list: &LayerList) {
        // The frame's vertex count is deliberately cut to 16 bits: indices are u16.
        let vertex_base = u32::from(self.vertices.len() as u16);
        let first_offset = self.indices.len() as u32;
        self.vertices.extend_from_slice(&list.vertices);
        self.indices.extend_from_slice(&list.index_list);
        let moved = list
            .batches
            .iter()
            .map(|run| run.shifted_by(first_offset, vertex_base));
        self.batches.extend(moved);
    }
}

/// A layer's finished vertices, indices and batches, offsets relative to it.
#[derive(Clone, Default)]
pub(crate) struct LayerList {
    vertices: Vec<HudGpuVertex>,
    index_list: Vec<u16>,
    batches: Vec<HudStitchBatch>,
}

impl LayerList {
    pub(crate) fn is_empty(&self) -> bool {
        Vec::is_empty(&self.vertices)
    }

    /// Adds one run of quads that share an image: their vertices and indices,
    /// and the batch that covers them.
    fn add_run(&mut self, head: &Placed, quads: &[Placed]) {
        let first_index = self.index_list.len() as u32;
        let first_vertex = self.vertices.len() as u16;
        for (nth, entry) in quads.iter().enumerate() {
            let base = (nth * 4).min(usize::from(u16::MAX)) as u16;
            self.vertices.extend(quad_vertices(&entry.quad));
            let steps = QUAD_CORNER_ORDER.iter().map(|&step| base.saturating_add(step));
            self.index_list.extend(steps);
        }
        self.batches.push(HudStitchBatch {
            image: head.image.clone(),
            technique: HudStitchMethod::Modulate,
            blend_state_words: head.blend_bits,
            first_index,
            index_count: 6 * quads.len() as u32,
            vertex_base: u32::from(first_vertex),
            vertex_count: 4 * quads.len() as u32,
        });
    }
}

/// What a layer's host node remembers between frames: the list it packed, the
/// fingerprint of the quads behind it, and whether it is currently hidden.
#[derive(Component, Default)]
pub struct ListLatch {
    list: LayerList,
    digest: u64,
    quad_total: usize,
    concealed: bool,
}

impl ListLatch {
    pub fn hide(&mut self) {
        self.concealed = true;
    }

    fn clear(&mut self) {
        self.concealed = true;
        self.list = LayerList::default();
    }

    /// Whether the list it holds is still the right one for these quads.
    fn is_current(&self, digest: u64, quad_total: usize) -> bool {
        !self.concealed
            && self.digest == digest
            && self.quad_total == quad_total
            && !self.list.is_empty()
    }
}

pub fn install(app: &mut App) {
    app.init_resource::<LayerRequests>()
        .init_resource::<OverlayMeshBatch>();
}

/// What a layer asks of the flush this frame.
#[derive(Default)]
pub enum LayerJob {
    /// Nothing new: draw whatever the latch still holds.
    #[default]
    Unchanged,
    Conceal,
    Fresh(Vec<FlatPaintRect>),
}

/// The layers' requests for this frame.
#[derive(Resource, Default)]
pub struct LayerRequests {
    pub names: LayerJob,
    pub elems_front: LayerJob,
    pub elems_behind: LayerJob,
}

// The cost of the flush systems' own bodies, summed over a frame. It is read
// from inside the bodies and not from the time between two systems: that gap
// belongs to the executor, which `.chain()` only orders, so it would measure
// the schedule and not the HUD.
static BODY_NANOS: AtomicU64 = AtomicU64::new(0);
static JOB_COUNT: AtomicU32 = AtomicU32::new(0);

/// Adds the time its scope lives to the frame's HUD tess total.
#[must_use = "the body is timed until this is dropped"]
pub struct BodyTimer(Instant);

impl BodyTimer {
    pub fn start() -> Self {
        Self(Instant::now())
    }
}

impl Drop for BodyTimer {
    fn drop(&mut self) {
        BODY_NANOS.fetch_add(self.0.elapsed().as_nanos() as u64, RELAXED);
    }
}

/// The frame's total in milliseconds and its job count; both restart at zero.
pub fn drain_body_cost() -> (f32, u32) {
    let nanos = BODY_NANOS.swap(0, RELAXED);
    let jobs = JOB_COUNT.swap(0, RELAXED);
    (nanos as f32 / 1e6, jobs)
}

/// Hands one layer's job for this frame to the GPU list.
pub fn flush_layer(
    frame: &mut OverlayMeshBatch,
    pictures: &mut HudPictures,
    image_store: &mut Assets<Image>,
    surface: (f32, f32),
    ui_node: &mut Node,
    latch: &mut ListLatch,
    job: LayerJob,
) {
    JOB_COUNT.fetch_add(1, RELAXED);
    match job {
        LayerJob::Fresh(quads) if !quads.is_empty() => {
            build_layer(LayerInput {
                frame,
                pictures,
                image_store,
                surface,
                quads: &quads[..],
                latch,
                ui_node,
            });
        }
        LayerJob::Unchanged => {
            if latch.has_picture() {
                frame.append_list(&latch.list);
            }
        }
        LayerJob::Conceal | LayerJob::Fresh(_) => conceal_layer(ui_node, latch),
    }
}

impl ListLatch {
    /// Whether an unchanged layer has something visible to put in the frame.
    fn has_picture(&self) -> bool {
        !self.concealed && !self.list.is_empty()
    }
}

pub fn conceal_node(node: &mut Node) {
    take_over_monitor(node, Display::None);
}

pub fn conceal_layer(host: &mut Node, latch: &mut ListLatch) {
    conceal_node(host);
    latch.clear();
}

/// Cuts a quad to its clip rectangle, trimming the texture coordinates to
/// match. `None` when nothing of it is left.
pub fn clip_quad(quad: &FlatPaintRect) -> Option<FlatPaintRect> {
    let Some([clip_l, clip_t, clip_r, clip_b]) = quad.clip_box else {
        return Some(FlatPaintRect::clone(quad));
    };
    let ([x0, y0], [x1, y1]) = (quad.corners[0], quad.corners[2]);
    let (cut_x0, cut_y0) = (x0.max(clip_l), y0.max(clip_t));
    let (cut_x1, cut_y1) = (x1.min(clip_r), y1.min(clip_b));
    if cut_x1 <= cut_x0 || cut_y1 <= cut_y0 {
        return None;
    }
    let shifts = [cut_x0 - x0, cut_y0 - y0, cut_x1 - x1, cut_y1 - y1];
    if shifts.iter().all(|shift| shift.abs() <= f32::EPSILON) {
        return Some(FlatPaintRect::clone(quad));
    }
    let (span_x, span_y) = (x1 - x0, y1 - y0);
    if span_x.abs() <= f32::EPSILON || span_y.abs() <= f32::EPSILON {
        return None;
    }
    // Texture coordinate at a fraction across and down the original quad.
    let [st0, st1, st2, st3] = quad.uv_corners;
    let st_at = |across: f32, down: f32| {
        let top_edge = lerp2(st0, st1, across);
        let bottom_edge = lerp2(st3, st2, across);
        lerp2(top_edge, bottom_edge, down)
    };
    let across = [(cut_x0 - x0) / span_x, (cut_x1 - x0) / span_x];
    let down = [(cut_y0 - y0) / span_y, (cut_y1 - y0) / span_y];
    Some(FlatPaintRect {
        corners: [
            [cut_x0, cut_y0],
            [cut_x1, cut_y0],
            [cut_x1, cut_y1],
            [cut_x0, cut_y1],
        ],
        uv_corners: [
            st_at(across[0], down[0]),
            st_at(across[1], down[0]),
            st_at(across[1], down[1]),
            st_at(across[0], down[1]),
        ],
        ..quad.clone()
    })
}

fn lerp2(from: [f32; 2], to: [f32; 2], amount: f32) -> [f32; 2] {
    std::array::from_fn(|axis| from[axis] + (to[axis] - from[axis]) * amount)
}

/// A quad that survived clipping and found its image.
struct Placed {
    image: Handle<Image>,
    blend_bits: Option<[u32; 2]>,
    quad: FlatPaintRect,
}

impl Placed {
    /// Whether `other` can join this one's batch.
    fn joins(&self, other: &Placed) -> bool {
        self.image.id() == other.image.id()
            && self.blend_bits == other.blend_bits
            && self.quad.paint_material == other.quad.paint_material
            && self.quad.surface_lineage == other.quad.surface_lineage
    }
}

/// Everything one layer's pass over its quads reads and writes.
struct LayerInput<'a> {
    frame: &'a mut OverlayMeshBatch,
    pictures: &'a mut HudPictures,
    image_store: &'a mut Assets<Image>,
    surface: (f32, f32),
    quads: &'a [FlatPaintRect],
    latch: &'a mut ListLatch,
    ui_node: &'a mut Node,
}

/// Packs the quads (or reuses the latched list when they are the same ones as
/// last time) and appends the result to the frame. Returns how many quads were
/// drawn.
fn build_layer(input: LayerInput<'_>) -> usize {
    let LayerInput {
        frame,
        pictures,
        image_store,
        surface: (width, height),
        quads,
        latch,
        ui_node: host,
    } = input;
    if width <= 1.0 || height <= 1.0 {
        conceal_layer(host, latch);
        return 0;
    }
    let placed: Vec<Placed> = quads
        .iter()
        .filter_map(|quad| place_quad(quad, pictures, image_store))
        .collect();
    let drawn = placed.len();
    let print = fingerprint(&placed);
    if latch.is_current(print, drawn) {
        frame.append_list(&latch.list);
        return drawn;
    }
    if drawn == 0 {
        conceal_layer(host, latch);
        return 0;
    }
    conceal_node(host);
    *latch = ListLatch {
        list: pack_batches(&placed),
        digest: print,
        quad_total: drawn,
        concealed: false,
    };
    frame.append_list(&latch.list);
    drawn
}

/// Clips the quad and looks up its image; `None` when either leaves nothing.
fn place_quad(
    quad: &FlatPaintRect,
    pictures: &mut HudPictures,
    image_store: &mut Assets<Image>,
) -> Option<Placed> {
    let cut = clip_quad(quad)?;
    let texture = pictures.get_data(cut.surface_lineage, &cut.paint_material, image_store)?;
    Some(Placed {
        image: texture,
        // The old zone material state bits left with the old menu catalog.
        blend_bits: None,
        quad: cut,
    })
}

/// One number that changes whenever any of the quads, or how many there are,
/// changes.
fn fingerprint(ready: &[Placed]) -> u64 {
    let mut digest = DefaultHasher::new();
    ready.len().hash(&mut digest);
    ready
        .iter()
        .for_each(|entry| feed_quad(&mut digest, &entry.quad));
    Hasher::finish(&digest)
}

fn feed_quad(digest: &mut impl Hasher, quad: &FlatPaintRect) {
    let corners = quad.corners.iter().chain(&quad.uv_corners).flatten();
    for value in corners.chain(&quad.rgba) {
        value.to_bits().hash(digest);
    }
    quad.paint_material.hash(digest);
    quad.surface_lineage.hash(digest);
    match quad.clip_box {
        Some(rect) => {
            1_u8.hash(digest);
            rect.iter().for_each(|edge| edge.to_bits().hash(digest));
        }
        None => 0_u8.hash(digest),
    }
}

fn quad_vertices(quad: &FlatPaintRect) -> [HudGpuVertex; 4] {
    let color = hud_t6::stretch_pic_cmd::convert_color_to_bytes(quad.rgba);
    std::array::from_fn(|corner| {
        let ([x, y], [s, t]) = (quad.corners[corner], quad.uv_corners[corner]);
        hud_t6::stretch_pic_cmd::set_vertex_2d(x, y, 1.0, s, t, color).into()
    })
}

/// How many quads from the front of `rest` go into one batch: those that share
/// the first one's image, up to the batch caps.
fn run_len(rest: &[Placed]) -> usize {
    let Some(head) = rest.first() else {
        return 0;
    };
    rest.iter()
        .enumerate()
        .take_while(|&(taken, entry)| {
            let (vertices, indices) = (4 * taken as u32, 6 * taken as u32);
            head.joins(entry)
                && !hud_t6::stretch_pic_cmd::tess_quad_overflows(
                    vertices,
                    indices,
                    BATCH_VERTEX_CAP,
                    BATCH_INDEX_CAP,
                )
        })
        .count()
}

/// Groups neighbouring quads that share an image, state and material into
/// batches, cutting a batch whenever it would outgrow the caps.
fn pack_batches(prepared: &[Placed]) -> LayerList {
    let mut packed = LayerList::default();
    let mut rest = prepared;
    while let Some(head) = rest.first() {
        // A run is never empty: the head always shares a batch with itself.
        let take = run_len(rest).max(1);
        let (run, tail) = rest.split_at(take);
        packed.add_run(head, run);
        rest = tail;
    }
    packed
}

/// A single quad drawn from the saved copy of the screen.
pub(crate) fn pack_saved_screen(quad: &FlatPaintRect, stand_in: Handle<Image>) -> LayerList {
    lone_quad(quad, stand_in, HudStitchMethod::SavedScreen)
}

/// A single quad drawn with one image.
pub(crate) fn pack_plain(quad: &FlatPaintRect, image: Handle<Image>) -> LayerList {
    lone_quad(quad, image, HudStitchMethod::Modulate)
}

fn lone_quad(quad: &FlatPaintRect, image: Handle<Image>, technique: HudStitchMethod) -> LayerList {
    LayerList {
        vertices: quad_vertices(quad).to_vec(),
        index_list: QUAD_CORNER_ORDER.to_vec(),
        batches: vec![HudStitchBatch {
            image,
            technique,
            blend_state_words: None,
            first_index: 0,
            index_count: 6,
            vertex_base: 0_u32,
            vertex_count: 4,
        }],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::draw2d::FlatPaintOrigin;

    fn quad_at(x: f32, y: f32, size: f32) -> FlatPaintRect {
        FlatPaintRect {
            corners: [[x, y], [x + size, y], [x + size, y + size], [x, y + size]],
            uv_corners: [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]],
            rgba: [1.0_f32; 4],
            paint_material: "white".into(),
            surface_lineage: Default::default(),
            sample_origin_tag: FlatPaintOrigin::MissionMarker,
            draw_layer: 0,
            clip_box: None,
        }
    }

    fn ready(quad: FlatPaintRect) -> Placed {
        Placed {
            image: Handle::default(),
            blend_bits: None,
            quad,
        }
    }

    #[test]
    fn an_unclipped_quad_passes_through_unchanged() {
        let quad = quad_at(10.0, 10.0, 20.0);
        assert_eq!(clip_quad(&quad), Some(quad));
    }

    #[test]
    fn a_quad_wholly_outside_its_clip_is_dropped() {
        let quad = FlatPaintRect {
            clip_box: Some([100.0, 100.0, 200.0, 200.0]),
            ..quad_at(0.0, 0.0, 10.0)
        };
        assert_eq!(clip_quad(&quad), None);
    }

    #[test]
    fn clipping_the_right_half_trims_the_texture_to_match() {
        let quad = FlatPaintRect {
            clip_box: Some([0.0, 0.0, 10.0, 20.0]),
            ..quad_at(0.0, 0.0, 20.0)
        };
        let cut = clip_quad(&quad).expect("half of it is inside");
        assert_eq!(cut.corners[1], [10.0, 0.0]);
        assert_eq!(cut.corners[2], [10.0, 20.0]);
        assert_eq!(cut.uv_corners[1], [0.5, 0.0]);
        assert_eq!(cut.uv_corners[2], [0.5, 1.0]);
        assert_eq!(cut.uv_corners[0], quad.uv_corners[0]);
    }

    #[test]
    fn lerp2_runs_each_axis_alone() {
        assert_eq!(lerp2([0.0, 10.0], [4.0, 20.0], 0.5), [2.0, 15.0]);
    }

    #[test]
    fn a_single_quad_is_one_batch_of_four_vertices_and_six_indices() {
        let list = pack_plain(&quad_at(0.0, 0.0, 1.0), Handle::default());
        assert_eq!((list.vertices.len(), list.index_list.len()), (4, 6));
        assert_eq!(list.batches.len(), 1);
        assert_eq!(list.batches[0].technique, HudStitchMethod::Modulate);
        let screen = pack_saved_screen(&quad_at(0.0, 0.0, 1.0), Handle::default());
        assert_eq!(screen.batches[0].technique, HudStitchMethod::SavedScreen);
    }

    #[test]
    fn neighbours_with_one_material_pack_into_one_run() {
        let quads: Vec<Placed> = (0..3).map(|n| ready(quad_at(n as f32, 0.0, 1.0))).collect();
        let list = pack_batches(&quads);
        assert_eq!(list.batches.len(), 1);
        assert_eq!(list.batches[0].vertex_count, 12);
        assert_eq!(list.batches[0].index_count, 18);
        // Each quad's indices start four vertices after the last one's.
        assert_eq!(list.index_list[6], QUAD_CORNER_ORDER[0] + 4);
    }

    #[test]
    fn a_material_change_starts_a_new_run() {
        let other = FlatPaintRect {
            paint_material: "grey".into(),
            ..quad_at(0.0, 0.0, 1.0)
        };
        let quads = vec![ready(quad_at(0.0, 0.0, 1.0)), ready(other)];
        let list = pack_batches(&quads);
        assert_eq!(list.batches.len(), 2);
        assert_eq!(list.batches[1].first_index, 6);
        assert_eq!(list.batches[1].vertex_base, 4);
    }

    #[test]
    fn appending_moves_a_list_past_what_the_frame_holds() {
        let one = pack_plain(&quad_at(0.0, 0.0, 1.0), Handle::default());
        let mut frame = OverlayMeshBatch::default();
        frame.append_list(&one);
        frame.append_list(&one);
        assert_eq!(frame.vertices.len(), 8);
        assert_eq!(frame.indices.len(), 12);
        assert_eq!(frame.batches[1].first_index, 6);
        assert_eq!(frame.batches[1].vertex_base, 4);
        frame.reset_geometry();
        assert!(frame.vertices.is_empty() && frame.batches.is_empty());
    }

    #[test]
    fn the_digest_follows_the_quads() {
        let a = vec![ready(quad_at(0.0, 0.0, 1.0))];
        let b = vec![ready(quad_at(0.0, 0.0, 2.0))];
        assert_eq!(fingerprint(&a), fingerprint(&a));
        assert_ne!(fingerprint(&a), fingerprint(&b));
    }

    #[test]
    fn a_latch_matches_only_while_shown_and_unchanged() {
        let mut latch = ListLatch {
            list: pack_plain(&quad_at(0.0, 0.0, 1.0), Handle::default()),
            digest: 7,
            quad_total: 1,
            concealed: false,
        };
        assert!(latch.is_current(7, 1));
        assert!(!latch.is_current(8, 1));
        assert!(!latch.is_current(7, 2));
        latch.hide();
        assert!(!latch.is_current(7, 1));
        assert!(!latch.has_picture());
    }
}
