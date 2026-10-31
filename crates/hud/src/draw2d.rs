//! The 2D draw list the HUD passes build, and its expansion into quads.
//!
//! Pictures go through the same packed render-command buffer the renderer
//! uses (so colours quantise to bytes and an over-full buffer drops commands),
//! then come back out as textured quads in submission order.

use std::cell::RefCell;

use hud_t6::render_commands::{PaintQueueBytes, RC_STRETCH_PIC, walk_render_commands};
use hud_t6::stretch_pic_cmd::{
    PaintScaledImage, PaintScaledImageParams, RB_DRAW_STRETCH_PIC_INDICES, add_cmd_draw_stretch_pic,

};
use hud_t6::stretch_pic_cmd::draw_stretch_pic_quad;
use hud_t6::stretch_pic_cmd::parse_gfx_cmd_stretch_pic;
use hud_t6::stretch_pic_cmd::unpack_color_rgba;

/// What one entry of a [`FlatPaintQueue`] asks for.
#[derive(Clone, Debug, PartialEq)]
pub enum FlatPaintAction {
    /// An axis-aligned picture, with the entry's `s0..t1` as texture window.
    Stretched,
    /// A picture whose texture window is a rotated square around a centre.
    RotateSt {
        middle_u: f32,
        middle_v: f32,
        reach_uv: f32,
        stretch_end_u: f32,
        stretch_end_v: f32,
        deg: f32,
    },
    /// Narrows the clip box for every entry after it.
    ClipRect,
}

/// Which part of the game asked for a draw; carried along for diagnostics.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FlatPaintOrigin {
    MissionMarker,
    MapUsePrompt,
    CodePainted(i32),
    ScenePaint { site: &'static str },
    HudElem { elem_slot: i32 },
}

#[derive(Clone, Debug, PartialEq)]
pub struct FlatPaintEntry {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    pub s0: f32,
    pub t0: f32,
    pub s1: f32,
    pub t1: f32,
    pub rgba: [f32; 4],
    pub paint_material: String,
    pub surface_lineage: asset_core::AssetCategoryScope,
    pub op: FlatPaintAction,
    pub sample_origin_tag: FlatPaintOrigin,
    pub draw_layer: u8,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct FlatPaintQueue {
    pub cmds: Vec<FlatPaintEntry>,
}

/// One quad ready for the GPU list: corner positions, texture coordinates and
/// the clip box that was active when it was submitted.
#[derive(Clone, Debug, PartialEq)]
pub struct FlatPaintRect {
    pub corners: [[f32; 2]; 4],
    pub uv_corners: [[f32; 2]; 4],
    pub rgba: [f32; 4],
    pub paint_material: String,
    pub surface_lineage: asset_core::AssetCategoryScope,
    pub sample_origin_tag: FlatPaintOrigin,
    pub draw_layer: u8,
    /// `[left, top, right, bottom]`, or `None` when nothing clips the quad.
    pub clip_box: Option<[f32; 4]>,
}

/// Index order that turns the four corners of a [`FlatPaintRect`] into triangles.
pub const QUAD_CORNER_ORDER: [u16; 6] = RB_DRAW_STRETCH_PIC_INDICES;

/// What happened to the commands while a list was expanded.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FlatPaintTally {
    /// Bytes of the command buffer that were written.
    pub bytes_written: u32,
    /// Commands read back from the buffer.
    pub scanned_count: u32,
    pub stretch_pic_n: u32,
    /// Commands that could not be turned into a quad.
    pub skipped_count: u32,
    /// Commands the full buffer refused.
    pub spill_count: u32,
}

/// Corner positions plus texture coordinates of one picture, picture corner order.
struct PictureCorners {
    xy: [[f32; 2]; 4],
    st: [[f32; 2]; 4],
}

/// A square of texture space about `center`, half-side `radius`, stretched by
/// `scale` and turned by `deg` degrees; the four corners in picture order
/// (top-left, top-right, bottom-right, bottom-left).
fn turned_square(center: [f32; 2], half: f32, stretch: [f32; 2], deg: f32) -> [[f32; 2]; 4] {
    let (sin, cos) = deg.to_radians().sin_cos();
    // Half-side steps along the turned s and t axes.
    let along_s = [half * cos * stretch[0], half * sin * stretch[1]];
    let along_t = [-half * sin * stretch[0], half * cos * stretch[1]];
    let corner = |ks: f32, kt: f32| {
        let mut out = center;
        for axis in 0_usize..2 {
            out[axis] += ks * along_s[axis] + kt * along_t[axis];
        }
        out
    };
    [corner(-1.0, -1.0), corner(1.0, -1.0), corner(1.0, 1.0), corner(-1.0, 1.0)]
}

/// Where a list entry's quad will come from, in submission order.
enum Pending {
    /// The entry at this index of the list sits in the command buffer, with
    /// the clip box that was active for it.
    FromBuffer { at: usize, clip_box: Option<[f32; 4]> },
    /// Rotated pictures never touch the buffer; their quad is already made.
    Ready(FlatPaintRect),
}

/// The renderer sizes its command buffer at run time; this fixed size holds a
/// whole HUD frame with room to spare.
const SCRATCH_BYTES: usize = 0x18000;

thread_local! {
    static SCRATCH: RefCell<Vec<u8>> = const { RefCell::new(Vec::new()) };
}

/// The box both `current` and `next` agree on (`next` alone when nothing clips yet).
fn intersect_clip(current: Option<[f32; 4]>, next: [f32; 4]) -> [f32; 4] {
    match current {
        None => next,
        Some(old) => {
            let mut out = next;
            for edge in 0..2 {
                out[edge] = old[edge].max(next[edge]);
                out[edge + 2] = old[edge + 2].min(next[edge + 2]);
            }
            out
        }
    }
}

/// The buffer's index for `name`, adding it on first sight.
fn intern_material(names: &mut Vec<String>, wanted: &str) -> u32 {
    if let Some(at) = names.iter().position(|known| known == wanted) {
        return at as u32;
    }
    names.push(wanted.to_owned());
    (names.len() - 1) as u32
}

/// Builds the quad for a list entry from its corners, colour, material and clip.
fn make_quad(
    entry: &FlatPaintEntry,
    PictureCorners { xy: corners, st: uv_corners }: PictureCorners,
    rgba: [f32; 4],
    paint_material: String,
    clip_box: Option<[f32; 4]>,
) -> FlatPaintRect {
    FlatPaintRect {
        corners,
        uv_corners,
        rgba,
        paint_material,
        surface_lineage: entry.surface_lineage,
        sample_origin_tag: entry.sample_origin_tag.clone(),
        draw_layer: entry.draw_layer,
        clip_box,
    }
}

/// Expands a list into quads, in the order the entries were submitted.
pub fn expand_paint_list(list: &FlatPaintQueue) -> (Vec<FlatPaintRect>, FlatPaintTally) {
    SCRATCH.with(|slot| {
        let mut scratch = slot.borrow_mut();
        if scratch.len() < SCRATCH_BYTES {
            scratch.resize(SCRATCH_BYTES, 0);
        }
        let (quads, census) = expand(list, &mut scratch[..SCRATCH_BYTES]);
        // Hand the buffer on zeroed over whatever this call wrote.
        let dirty = SCRATCH_BYTES.min(census.bytes_written as usize);
        scratch[..dirty].fill(0);
        (quads, census)
    })
}

/// The axis-aligned screen corners of an entry, in picture order.
fn screen_box(entry: &FlatPaintEntry) -> [[f32; 2]; 4] {
    let (x0, y0) = (entry.x, entry.y);
    let (x1, y1) = (x0 + entry.w, y0 + entry.h);
    [[x0, y0], [x1, y0], [x1, y1], [x0, y1]]
}

/// Puts every picture of `list` into `buffer` (rotated ones become quads on the
/// spot) and returns what is pending plus how many the full buffer refused.
fn submit(
    list: &FlatPaintQueue,
    buffer: &mut PaintQueueBytes<'_>,
    names: &mut Vec<String>,
) -> (Vec<Pending>, u32) {
    let mut pending = Vec::with_capacity(list.cmds.len());
    let mut active_clip: Option<[f32; 4]> = None;
    let mut refused = 0_u32;
    for (index, entry) in list.cmds.iter().enumerate() {
        if let FlatPaintAction::RotateSt { middle_u, middle_v, reach_uv, stretch_end_u, stretch_end_v, deg } =
            entry.op
        {
            let st = turned_square([middle_u, middle_v], reach_uv, [stretch_end_u, stretch_end_v], deg);
            let corners = PictureCorners { xy: screen_box(entry), st };
            let material = entry.paint_material.clone();
            pending.push(Pending::Ready(make_quad(entry, corners, entry.rgba, material, active_clip)));
        } else if entry.op == FlatPaintAction::ClipRect {
            let [tl, _, br, _] = screen_box(entry);
            active_clip = Some(intersect_clip(active_clip, [tl[0], tl[1], br[0], br[1]]));
        } else {
            let FlatPaintEntry { x, y, w, h, s0, t0, s1, t1, rgba: color, .. } = *entry;
            let material = intern_material(names, &entry.paint_material);
            let args = PaintScaledImageParams { material, x, y, w, h, s0, t0, s1, t1, color };
            if add_cmd_draw_stretch_pic(buffer, &args) {
                pending.push(Pending::FromBuffer { at: index, clip_box: active_clip });
            } else {
                refused += 1;
            }
        }
    }
    (pending, refused)
}

/// Why a buffered picture produced no quad.
enum ReadMiss {
    /// The buffer stopped making sense; nothing after this can be read.
    Broken,
    /// This command was not a usable stretch pic.
    Unusable,
}

fn expand(list: &FlatPaintQueue, scratch: &mut [u8]) -> (Vec<FlatPaintRect>, FlatPaintTally) {
    let mut buffer = PaintQueueBytes::new(scratch, 0);
    let mut interned = Vec::<String>::new();
    let (queued, refused) = submit(list, &mut buffer, &mut interned);

    let mut census = FlatPaintTally::default();
    census.spill_count = refused;
    census.bytes_written = buffer.used_total;
    let mut commands = walk_render_commands(buffer.buf, buffer.used_total, None);
    let mut read_one = |census: &mut FlatPaintTally| -> Result<(PaintScaledImage, String), ReadMiss> {
        let view = match commands.next() {
            Some(Ok(view)) => view,
            _ => return Err(ReadMiss::Broken),
        };
        census.scanned_count += 1;
        if view.id != RC_STRETCH_PIC {
            return Err(ReadMiss::Unusable);
        }
        census.stretch_pic_n += 1;
        let pic = parse_gfx_cmd_stretch_pic(view.bytes).ok_or(ReadMiss::Unusable)?;
        let name = interned.get(pic.material as usize).cloned().ok_or(ReadMiss::Unusable)?;
        Ok((pic, name))
    };

    let mut quads = Vec::with_capacity(queued.len());
    for item in queued {
        let (index, clip) = match item {
            Pending::Ready(quad) => {
                quads.push(quad);
                continue;
            }
            Pending::FromBuffer { at, clip_box } => (at, clip_box),
        };
        match read_one(&mut census) {
            Ok((pic, name)) => {
                let color = unpack_color_rgba(pic.tint_rgba).map(|byte| f32::from(byte) / 255.0);
                let entry = &list.cmds[index];
                quads.push(make_quad(entry, picture_corners(&pic), color, name, clip));
            }
            Err(ReadMiss::Unusable) => census.skipped_count += 1,
            Err(ReadMiss::Broken) => {
                census.skipped_count += 1;
                break;
            }
        }
    }
    (quads, census)
}

/// Corner positions and texture coordinates in the renderer's stretch-pic
/// order: top-left, top-right, bottom-right, bottom-left.
fn picture_corners(pic: &PaintScaledImage) -> PictureCorners {
    // Only the geometry matters here: unit w, no material or colour.
    let mut geometry = *pic;
    geometry.w0 = 1.0;
    geometry.tint_rgba = 0;
    geometry.material = 0;
    let vertex_points = draw_stretch_pic_quad(&geometry);
    let mut xy = [[0.0; 2]; 4];
    let mut st = [[0.0; 2]; 4];
    for (slot, vert) in vertex_points.iter().enumerate() {
        xy[slot] = [vert.quad[0], vert.quad[1]];
        st[slot] = vert.uv_semantic;
    }
    PictureCorners { xy, st }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A 10 x 20 picture over the whole texture, on layer 3.
    fn pic(name: &str, x: f32, y: f32, rgba: [f32; 4]) -> FlatPaintEntry {
        let (w, h) = (10.0, 20.0);
        let (s0, t0, s1, t1) = (0.0, 0.0, 1.0, 1.0);
        let paint_material = name.to_owned();
        let surface_lineage = asset_core::AssetCategoryScope::T6;
        let (op, sample_origin_tag) = (FlatPaintAction::Stretched, FlatPaintOrigin::MissionMarker);
        FlatPaintEntry { x, y, w, h, s0, t0, s1, t1, rgba, paint_material, surface_lineage, op, sample_origin_tag, draw_layer: 3 }
    }

    fn of(cmds: Vec<FlatPaintEntry>) -> FlatPaintQueue {
        FlatPaintQueue { cmds }
    }

    fn close(a: [f32; 2], b: [f32; 2]) -> bool {
        a.iter().zip(b).all(|(l, r)| (l - r).abs() < 1e-4)
    }

    #[test]
    fn clip_boxes_narrow_to_their_overlap() {
        let wide = [0.0, 0.0, 100.0, 100.0];
        assert_eq!(intersect_clip(None, wide), wide);
        let narrowed = intersect_clip(Some(wide), [10.0, 20.0, 200.0, 50.0]);
        assert_eq!(narrowed, [10.0, 20.0, 100.0, 50.0]);
    }

    #[test]
    fn materials_get_stable_indices() {
        let mut names = Vec::<String>::new();
        let got: Vec<u32> = ["a", "b", "a"].iter().map(|n| intern_material(&mut names, n)).collect();
        assert_eq!(got, [0, 1, 0]);
        assert_eq!(names, ["a", "b"]);
    }

    #[test]
    fn unturned_square_spans_its_radius() {
        let corners = turned_square([0.5, 0.5], 0.25, [1.0, 1.0], 0.0);
        let want = [[0.25, 0.25], [0.75, 0.25], [0.75, 0.75], [0.25, 0.75]];
        assert!(corners.iter().zip(want).all(|(c, w)| close(*c, w)));
    }

    #[test]
    fn quarter_turn_moves_corners_round() {
        let turned = turned_square([0.0, 0.0], 1.0, [1.0, 1.0], 90.0);
        assert!(close(turned[0], [1.0, -1.0]));
    }

    #[test]
    fn stretch_pics_come_back_as_quads_in_order() {
        let red = pic("one", 0.0, 0.0, [1.0, 0.0, 0.0, 1.0]);
        let green = pic("two", 5.0, 6.0, [0.0, 1.0, 0.0, 1.0]);
        let (quads, census) = expand_paint_list(&of(vec![red, green]));
        assert_eq!(quads.len(), 2);
        let names: Vec<&str> = quads.iter().map(|q| q.paint_material.as_str()).collect();
        assert_eq!(names, ["one", "two"]);
        assert!(close(quads[1].corners[0], [5.0, 6.0]) && close(quads[1].corners[2], [15.0, 26.0]));
        assert_eq!(quads[0].rgba, [1.0, 0.0, 0.0, 1.0]);
        assert_eq!(quads[0].draw_layer, 3);
        assert_eq!([census.stretch_pic_n, census.skipped_count, census.spill_count], [2, 0, 0]);
    }

    #[test]
    fn clip_rect_applies_to_later_pictures_only() {
        let white = [1.0_f32; 4];
        let clip = FlatPaintEntry { op: FlatPaintAction::ClipRect, ..pic("", 1.0, 2.0, white) };
        let (quads, _) =
            expand_paint_list(&of(vec![pic("before", 0.0, 0.0, white), clip, pic("after", 0.0, 0.0, white)]));
        assert_eq!(quads[0].clip_box, None);
        assert_eq!(quads[1].clip_box, Some([1.0, 2.0, 11.0, 22.0]));
    }

    #[test]
    fn rotated_pictures_skip_the_buffer() {
        let (half, one) = (0.5, 1.0);
        let spin = FlatPaintAction::RotateSt {
            middle_u: half,
            middle_v: half,
            reach_uv: half,
            stretch_end_u: one,
            stretch_end_v: one,
            deg: 0.0,
        };
        let turned = FlatPaintEntry { op: spin, ..pic("spin", 0.0, 0.0, [1.0; 4]) };
        let (quads, census) = expand_paint_list(&of(vec![turned]));
        assert_eq!(quads.len(), 1);
        assert!(close(quads[0].uv_corners[0], [0.0, 0.0]));
        assert_eq!([census.scanned_count, census.bytes_written], [0, 0]);
    }

    #[test]
    fn a_full_buffer_refuses_and_counts() {
        let crowd = (0..5000).map(|n| pic("many", n as f32, 0.0, [1.0; 4])).collect();
        let (quads, census) = expand_paint_list(&of(crowd));
        assert!(census.spill_count > 0);
        assert_eq!(quads.len() as u32 + census.spill_count, 5000);
        // The scratch buffer is clean for the next call.
        let (again, next) = expand_paint_list(&of(vec![pic("x", 0.0, 0.0, [1.0; 4])]));
        assert_eq!((again.len(), next.spill_count), (1, 0));
    }
}
