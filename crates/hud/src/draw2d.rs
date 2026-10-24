//! The 2D draw list the HUD passes build, and its expansion into quads.
//!
//! Pictures go through the same packed render-command buffer the renderer
//! uses (so colours quantise to bytes and an over-full buffer drops commands),
//! then come back out as textured quads in submission order.

use std::cell::RefCell;

use hud_t6::render_commands::{GfxRenderCommandBuf, RC_STRETCH_PIC, walk_render_commands};
use hud_t6::stretch_pic_cmd::{
    GfxCmdStretchPic, GfxCmdStretchPicArgs, RB_DRAW_STRETCH_PIC_INDICES, add_cmd_draw_stretch_pic,
    draw_stretch_pic_quad, parse_gfx_cmd_stretch_pic, unpack_color_rgba,
};

/// What one entry of a [`Draw2dList`] asks for.
#[derive(Clone, Debug, PartialEq)]
pub enum Draw2dOp {
    /// An axis-aligned picture, with the entry's `s0..t1` as texture window.
    StretchPic,
    /// A picture whose texture window is a rotated square around a centre.
    RotateSt {
        center_s: f32,
        center_t: f32,
        radius_st: f32,
        scale_final_s: f32,
        scale_final_t: f32,
        deg: f32,
    },
    /// Narrows the clip box for every entry after it.
    ClipRect,
}

/// Which part of the game asked for a draw; carried along for diagnostics.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Draw2dProvenance {
    Objective,
    MapUseHint,
    OwnerDraw(i32),
    CgDraw { site: &'static str },
    HudElem { index: i32 },
}

#[derive(Clone, Debug, PartialEq)]
pub struct Draw2dCmd {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    pub s0: f32,
    pub t0: f32,
    pub s1: f32,
    pub t1: f32,
    pub color: [f32; 4],
    pub material: String,
    pub material_namespace: asset_core::AssetNamespace,
    pub op: Draw2dOp,
    pub provenance: Draw2dProvenance,
    pub layer: u8,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Draw2dList {
    pub cmds: Vec<Draw2dCmd>,
}

/// One quad ready for the GPU list: corner positions, texture coordinates and
/// the clip box that was active when it was submitted.
#[derive(Clone, Debug, PartialEq)]
pub struct Draw2dQuad {
    pub xy: [[f32; 2]; 4],
    pub st: [[f32; 2]; 4],
    pub color: [f32; 4],
    pub material: String,
    pub material_namespace: asset_core::AssetNamespace,
    pub provenance: Draw2dProvenance,
    pub layer: u8,
    /// `[left, top, right, bottom]`, or `None` when nothing clips the quad.
    pub clip: Option<[f32; 4]>,
}

/// Index order that turns the four corners of a [`Draw2dQuad`] into triangles.
pub const DRAW2D_QUAD_INDICES: [u16; 6] = RB_DRAW_STRETCH_PIC_INDICES;

/// What happened to the commands while a list was expanded.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Draw2dCmdCensus {
    /// Bytes of the command buffer that were written.
    pub used: u32,
    /// Commands read back from the buffer.
    pub walk_n: u32,
    pub stretch_pic_n: u32,
    /// Commands that could not be turned into a quad.
    pub skip_n: u32,
    /// Commands the full buffer refused.
    pub overflow_n: u32,
}

/// The four corners of a square of texture space turned by `deg` degrees
/// around `(center_s, center_t)`, in picture corner order.
fn rotated_window(
    center_s: f32,
    center_t: f32,
    radius_st: f32,
    scale_final_s: f32,
    scale_final_t: f32,
    deg: f32,
) -> [[f32; 2]; 4] {
    let (sin, cos) = deg.to_radians().sin_cos();
    let along = [
        radius_st * cos * scale_final_s,
        radius_st * sin * scale_final_t,
    ];
    let across = [
        -radius_st * sin * scale_final_s,
        radius_st * cos * scale_final_t,
    ];
    let corner = |a: f32, b: f32| {
        [
            center_s + a * along[0] + b * across[0],
            center_t + a * along[1] + b * across[1],
        ]
    };
    [
        corner(-1.0, -1.0),
        corner(1.0, -1.0),
        corner(1.0, 1.0),
        corner(-1.0, 1.0),
    ]
}

/// Where a command goes once its picture has been through the buffer.
struct PicContext {
    material_namespace: asset_core::AssetNamespace,
    clip: Option<[f32; 4]>,
    provenance: Draw2dProvenance,
    layer: u8,
}

/// A list entry, in submission order, waiting to become a quad.
enum Pending {
    /// Written to the command buffer; the quad comes from reading it back.
    Packed(PicContext),
    /// Built directly (rotated pictures do not go through the buffer).
    Direct(Draw2dQuad),
}

/// The renderer sizes its command buffer at run time; this is a fixed size
/// that comfortably holds a HUD frame.
const CMD_BUF_BYTES: usize = 0x18000;

thread_local! {
    static CMD_BUF: RefCell<Vec<u8>> = const { RefCell::new(Vec::new()) };
}

fn overlap(current: Option<[f32; 4]>, next: [f32; 4]) -> [f32; 4] {
    match current {
        Some(c) => [
            c[0].max(next[0]),
            c[1].max(next[1]),
            c[2].min(next[2]),
            c[3].min(next[3]),
        ],
        None => next,
    }
}

fn material_slot(names: &mut Vec<String>, name: &str) -> u32 {
    let at = names.iter().position(|known| known == name);
    at.unwrap_or_else(|| {
        names.push(name.to_owned());
        names.len() - 1
    }) as u32
}

fn unit_rgba(packed: u32) -> [f32; 4] {
    unpack_color_rgba(packed).map(|byte| f32::from(byte) / 255.0)
}

/// Expands a list into quads, in the order the entries were submitted.
pub fn tessellate(list: &Draw2dList) -> (Vec<Draw2dQuad>, Draw2dCmdCensus) {
    // Borrow the thread's scratch buffer for the duration of the call, and
    // give it back zeroed over the part that was written.
    let mut scratch = CMD_BUF.with(|cell| std::mem::take(&mut *cell.borrow_mut()));
    scratch.resize(CMD_BUF_BYTES, 0);
    let (quads, census) = expand(list, &mut scratch);
    let dirty = (census.used as usize).min(scratch.len());
    scratch[..dirty].fill(0);
    CMD_BUF.with(|cell| *cell.borrow_mut() = scratch);
    (quads, census)
}

fn expand(list: &Draw2dList, scratch: &mut [u8]) -> (Vec<Draw2dQuad>, Draw2dCmdCensus) {
    let mut buffer = GfxRenderCommandBuf::new(scratch, 0);
    let mut names: Vec<String> = Vec::new();
    let mut pending: Vec<Pending> = Vec::with_capacity(list.cmds.len());
    let mut clip: Option<[f32; 4]> = None;
    let mut overflow_n = 0u32;

    for cmd in &list.cmds {
        match &cmd.op {
            Draw2dOp::ClipRect => {
                clip = Some(overlap(clip, [cmd.x, cmd.y, cmd.x + cmd.w, cmd.y + cmd.h]));
            }
            Draw2dOp::StretchPic => {
                let args = GfxCmdStretchPicArgs {
                    material: material_slot(&mut names, &cmd.material),
                    x: cmd.x,
                    y: cmd.y,
                    w: cmd.w,
                    h: cmd.h,
                    s0: cmd.s0,
                    t0: cmd.t0,
                    s1: cmd.s1,
                    t1: cmd.t1,
                    color: cmd.color,
                };
                if add_cmd_draw_stretch_pic(&mut buffer, &args) {
                    pending.push(Pending::Packed(PicContext {
                        material_namespace: cmd.material_namespace,
                        clip,
                        provenance: cmd.provenance.clone(),
                        layer: cmd.layer,
                    }));
                } else {
                    overflow_n += 1;
                }
            }
            Draw2dOp::RotateSt {
                center_s,
                center_t,
                radius_st,
                scale_final_s,
                scale_final_t,
                deg,
            } => {
                let (right, bottom) = (cmd.x + cmd.w, cmd.y + cmd.h);
                pending.push(Pending::Direct(Draw2dQuad {
                    xy: [[cmd.x, cmd.y], [right, cmd.y], [right, bottom], [cmd.x, bottom]],
                    st: rotated_window(
                        *center_s,
                        *center_t,
                        *radius_st,
                        *scale_final_s,
                        *scale_final_t,
                        *deg,
                    ),
                    color: cmd.color,
                    material: cmd.material.clone(),
                    material_namespace: cmd.material_namespace,
                    provenance: cmd.provenance.clone(),
                    layer: cmd.layer,
                    clip,
                }));
            }
        }
    }

    let mut census = Draw2dCmdCensus {
        used: buffer.used_total,
        overflow_n,
        ..Draw2dCmdCensus::default()
    };
    let mut readback = walk_render_commands(buffer.buf, buffer.used_total, None);
    let mut quads = Vec::with_capacity(pending.len());
    for entry in pending {
        let ctx = match entry {
            Pending::Direct(quad) => {
                quads.push(quad);
                continue;
            }
            Pending::Packed(ctx) => ctx,
        };
        // A buffer that stops making sense ends the readback for good.
        let Some(Ok(view)) = readback.next() else {
            census.skip_n += 1;
            break;
        };
        census.walk_n += 1;
        if view.id != RC_STRETCH_PIC {
            census.skip_n += 1;
            continue;
        }
        census.stretch_pic_n += 1;
        let packed = parse_gfx_cmd_stretch_pic(view.bytes)
            .and_then(|pic| names.get(pic.material as usize).map(|name| (pic, name)));
        let Some((pic, name)) = packed else {
            census.skip_n += 1;
            continue;
        };
        let (xy, st) = picture_corners(&pic);
        quads.push(Draw2dQuad {
            xy,
            st,
            color: unit_rgba(pic.color_rgba),
            material: name.clone(),
            material_namespace: ctx.material_namespace,
            provenance: ctx.provenance,
            layer: ctx.layer,
            clip: ctx.clip,
        });
    }
    (quads, census)
}

/// Corner positions and texture coordinates in the renderer's stretch-pic
/// order: top-left, top-right, bottom-right, bottom-left.
fn picture_corners(pic: &GfxCmdStretchPic) -> ([[f32; 2]; 4], [[f32; 2]; 4]) {
    let verts = draw_stretch_pic_quad(&GfxCmdStretchPic {
        material: 0,
        color_rgba: 0,
        w0: 1.0,
        ..*pic
    });
    (
        verts.map(|v| [v.xyzw[0], v.xyzw[1]]),
        verts.map(|v| v.texcoord),
    )
}
