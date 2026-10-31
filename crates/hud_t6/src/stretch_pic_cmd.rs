//! BO2 stretch pic command (scratch/hud-facts/stretch_pic_cmd.md): the 0x30-byte id-11 command, RGBA
//! colour packing, the 2D material checks and the tess quad it draws.

use crate::render_commands::{PaintQueueBytes, RC_STRETCH_PIC};

pub const GFX_CMD_STRETCH_PIC_SIZE: u32 = 0x30;
pub const GFX_CMD_STRETCH_PIC_MATERIAL: usize = 4;
pub const GFX_CMD_STRETCH_PIC_X: usize = 8;
pub const GFX_CMD_STRETCH_PIC_Y: usize = 12;
pub const GFX_CMD_STRETCH_PIC_W0: usize = 16;
pub const GFX_CMD_STRETCH_PIC_W: usize = 20;
pub const GFX_CMD_STRETCH_PIC_H: usize = 24;
pub const GFX_CMD_STRETCH_PIC_S0: usize = 28;
pub const GFX_CMD_STRETCH_PIC_T0: usize = 32;
pub const GFX_CMD_STRETCH_PIC_S1: usize = 36;
pub const GFX_CMD_STRETCH_PIC_T1: usize = 40;
pub const GFX_CMD_STRETCH_PIC_COLOR: usize = 44;

/// R_ConvertColorToBytes: each lane floor(clamp(c * 255, 0, 255)); memory order R, G, B, A, so the
/// little-endian u32 is 0xAABBGGRR.
#[must_use]
pub fn convert_color_to_bytes(c: [f32; 4]) -> u32 {
    let lane = |v: f32| {
        let v = v * 255.0;
        // NaN lands on 0.
        if v >= 255.0 {
            255u32
        } else if v > 0.0 {
            libm::floorf(v) as u32
        } else {
            0
        }
    };
    lane(c[0]) | lane(c[1]) << 8 | lane(c[2]) << 16 | lane(c[3]) << 24
}

#[must_use]
pub fn unpack_color_rgba(packed: u32) -> [u8; 4] {
    packed.to_le_bytes()
}

/// What the queue side needs to know about a material to accept it for 2D drawing.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Material2dCheck {
    /// Has a fogable (world) technique.
    pub fogable: bool,
    /// State flag 0x10.
    pub uses_depth_buffer: bool,
    /// Is the default material loaded under another name.
    pub default_alias: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Material2dReject {
    Fogable,
    UsesDepthBuffer,
    DefaultAlias,
}

/// Material choice at queue time: handle 0 (null) and rejected materials draw with the default
/// material. A rejection is returned so the caller can print BO2's "set materialType to 2d" warning.
#[must_use]
pub fn pick_2d_material(
    asked: u32,
    check: Material2dCheck,
    default_material: u32,
) -> (u32, Option<Material2dReject>) {
    if asked == 0 {
        return (default_material, None);
    }
    let reject = if check.fogable {
        Some(Material2dReject::Fogable)
    } else if check.uses_depth_buffer {
        Some(Material2dReject::UsesDepthBuffer)
    } else if check.default_alias {
        Some(Material2dReject::DefaultAlias)
    } else {
        None
    };
    (if reject.is_some() { default_material } else { asked }, reject)
}

/// Arguments in real (already placed) pixels. `material` is the handle after pick_2d_material.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PaintScaledImageParams {
    pub material: u32,
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    pub s0: f32,
    pub t0: f32,
    pub s1: f32,
    pub t1: f32,
    pub color: [f32; 4],
}

/// The decoded command.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PaintScaledImage {
    pub material: u32,
    pub x: f32,
    pub y: f32,
    pub w0: f32,
    pub w: f32,
    pub h: f32,
    pub s0: f32,
    pub t0: f32,
    pub s1: f32,
    pub t1: f32,
    pub tint_rgba: u32,
}

fn write_real32(cmd: &mut [u8], at: usize, v: f32) {
    cmd[at..at + 4].copy_from_slice(&v.to_le_bytes());
}

fn read_word32(cmd: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([cmd[at], cmd[at + 1], cmd[at + 2], cmd[at + 3]])
}

fn get_f32(cmd: &[u8], at: usize) -> f32 {
    f32::from_bits(read_word32(cmd, at))
}

/// R_AddCmdDrawStretchPicW: queue one pic with vertex W `w0`. False when the noncritical command was
/// dropped (nothing drawn).
pub fn add_cmd_draw_stretch_pic_w(cmds: &mut PaintQueueBytes<'_>, a: &PaintScaledImageParams, w0: f32) -> bool {
    let Some(cmd) = cmds.get_command_buffer(RC_STRETCH_PIC, GFX_CMD_STRETCH_PIC_SIZE) else {
        return false;
    };
    cmd[GFX_CMD_STRETCH_PIC_MATERIAL..GFX_CMD_STRETCH_PIC_MATERIAL + 4].copy_from_slice(&a.material.to_le_bytes());
    write_real32(cmd, GFX_CMD_STRETCH_PIC_X, a.x);
    write_real32(cmd, GFX_CMD_STRETCH_PIC_Y, a.y);
    write_real32(cmd, GFX_CMD_STRETCH_PIC_W0, w0);
    write_real32(cmd, GFX_CMD_STRETCH_PIC_W, a.w);
    write_real32(cmd, GFX_CMD_STRETCH_PIC_H, a.h);
    write_real32(cmd, GFX_CMD_STRETCH_PIC_S0, a.s0);
    write_real32(cmd, GFX_CMD_STRETCH_PIC_T0, a.t0);
    write_real32(cmd, GFX_CMD_STRETCH_PIC_S1, a.s1);
    write_real32(cmd, GFX_CMD_STRETCH_PIC_T1, a.t1);
    let c = convert_color_to_bytes(a.color);
    cmd[GFX_CMD_STRETCH_PIC_COLOR..GFX_CMD_STRETCH_PIC_COLOR + 4].copy_from_slice(&c.to_le_bytes());
    true
}

/// R_AddCmdDrawStretchPic: the plain variant, w0 = 1.0.
pub fn add_cmd_draw_stretch_pic(cmds: &mut PaintQueueBytes<'_>, a: &PaintScaledImageParams) -> bool {
    add_cmd_draw_stretch_pic_w(cmds, a, 1.0)
}

/// Decode a whole id-11 command (header included).
#[must_use]
pub fn parse_gfx_cmd_stretch_pic(cmd: &[u8]) -> Option<PaintScaledImage> {
    if cmd.len() < GFX_CMD_STRETCH_PIC_SIZE as usize || cmd[2] != RC_STRETCH_PIC {
        return None;
    }
    Some(PaintScaledImage {
        material: read_word32(cmd, GFX_CMD_STRETCH_PIC_MATERIAL),
        x: get_f32(cmd, GFX_CMD_STRETCH_PIC_X),
        y: get_f32(cmd, GFX_CMD_STRETCH_PIC_Y),
        w0: get_f32(cmd, GFX_CMD_STRETCH_PIC_W0),
        w: get_f32(cmd, GFX_CMD_STRETCH_PIC_W),
        h: get_f32(cmd, GFX_CMD_STRETCH_PIC_H),
        s0: get_f32(cmd, GFX_CMD_STRETCH_PIC_S0),
        t0: get_f32(cmd, GFX_CMD_STRETCH_PIC_T0),
        s1: get_f32(cmd, GFX_CMD_STRETCH_PIC_S1),
        t1: get_f32(cmd, GFX_CMD_STRETCH_PIC_T1),
        tint_rgba: read_word32(cmd, GFX_CMD_STRETCH_PIC_COLOR),
    })
}

pub const OVERLAY_CORNER_BYTES: usize = 0x20;
/// Packed normal of every 2D vertex.
pub const GFX_TESS_2D_PACKED_NORMAL: u32 = 0x3ffe_7f7f;
/// Quad indices relative to the quad's first vertex.
pub const RB_DRAW_STRETCH_PIC_INDICES: [u16; 6] = {
    let (a, b, c, d) = (0, 1, 2, 3);
    [d, a, c, c, a, b]
};

/// One 2D tess vertex: xyz (z = 0) and w, colour, st, packed normal.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct OverlayCorner2d {
    pub quad: [f32; 4],
    pub color: u32,
    pub uv_semantic: [f32; 2],
    pub squeezed_facing: u32,
}

impl OverlayCorner2d {
    #[must_use]
    pub fn to_bytes(&self) -> [u8; OVERLAY_CORNER_BYTES] {
        let [x, y, z, w] = self.quad.map(f32::to_bits);
        let [s, t] = self.uv_semantic.map(f32::to_bits);
        let words = [x, y, z, w, self.color, s, t, self.squeezed_facing];
        let mut packed = [0; OVERLAY_CORNER_BYTES];
        for (slot, word) in packed.chunks_exact_mut(4).zip(words) {
            slot.copy_from_slice(&word.to_le_bytes());
        }
        packed
    }
}

#[must_use]
pub fn set_vertex_2d(x: f32, y: f32, w0: f32, s: f32, t: f32, color: u32) -> OverlayCorner2d {
    OverlayCorner2d { quad: [x, y, 0.0, w0], color, uv_semantic: [s, t], squeezed_facing: GFX_TESS_2D_PACKED_NORMAL }
}

/// RB_DrawStretchPic corners: v0 top-left, v1 top-right, v2 bottom-right, v3 bottom-left.
#[must_use]
pub fn draw_stretch_pic_quad(p: &PaintScaledImage) -> [OverlayCorner2d; 4] {
    let (x1, y1) = (p.x + p.w, p.y + p.h);
    [
        set_vertex_2d(p.x, p.y, p.w0, p.s0, p.t0, p.tint_rgba),
        set_vertex_2d(x1, p.y, p.w0, p.s1, p.t0, p.tint_rgba),
        set_vertex_2d(x1, y1, p.w0, p.s1, p.t1, p.tint_rgba),
        set_vertex_2d(p.x, y1, p.w0, p.s0, p.t1, p.tint_rgba),
    ]
}

/// RB_TessAddQuadIndices: indices for a quad starting at `vertex_count` (masked to 16 bits).
#[must_use]
pub fn quad_indices(vertex_count: u32) -> [u16; 6] {
    let base = (vertex_count & 0xffff) as u16;
    RB_DRAW_STRETCH_PIC_INDICES.map(|i| base.wrapping_add(i))
}

/// RB_CheckTessOverflow: true when one more quad does not fit and the surface must end and restart
/// with the same material. BO2's limits are not in the facts, so the caller passes its own.
#[must_use]
pub fn tess_quad_overflows(verts: u32, indices: u32, vert_cap: u32, index_cap: u32) -> bool {
    verts + 4 > vert_cap || indices + 6 > index_cap
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn color_is_rgba_floor() {
        assert_eq!(convert_color_to_bytes([1.0, 0.2, 0.2, 1.0]), 0xff33_33ff);
        assert_eq!(unpack_color_rgba(0xff33_33ff), [0xff, 0x33, 0x33, 0xff]);
        // 0.5 * 255 = 127.5 floors to 127; out of range clamps.
        assert_eq!(convert_color_to_bytes([0.5, -1.0, 2.0, f32::NAN]), 0x00ff_007f);
    }

    #[test]
    fn material_fallbacks() {
        let ok = Material2dCheck::default();
        assert_eq!(pick_2d_material(0, ok, 9), (9, None));
        assert_eq!(pick_2d_material(5, ok, 9), (5, None));
        let fog = Material2dCheck { fogable: true, ..ok };
        assert_eq!(pick_2d_material(5, fog, 9), (9, Some(Material2dReject::Fogable)));
        let depth = Material2dCheck { uses_depth_buffer: true, ..ok };
        assert_eq!(pick_2d_material(5, depth, 9), (9, Some(Material2dReject::UsesDepthBuffer)));
    }

    #[test]
    fn command_round_trip() {
        let mut mem = [0u8; 0x4000];
        let mut b = PaintQueueBytes::new(&mut mem, 0);
        let mut tint = [1.0f32; 4];
        tint[3] = 0.5;
        let a = PaintScaledImageParams {
            material: 7,
            x: 10.0,
            y: 20.0,
            w: 30.0,
            h: 40.0,
            s0: 0.0,
            t0: 0.0,
            s1: 1.0,
            t1: 1.0,
            color: tint,
        };
        assert!(add_cmd_draw_stretch_pic(&mut b, &a));
        assert_eq!(b.used_total, 0x30);
        let p = parse_gfx_cmd_stretch_pic(&mem[..0x30]).unwrap();
        assert_eq!(p.material, 7);
        assert_eq!(p.w0, 1.0);
        assert_eq!((p.x, p.y, p.w, p.h), (10.0, 20.0, 30.0, 40.0));
        assert_eq!(p.tint_rgba, 0x7fff_ffff);
        let q = draw_stretch_pic_quad(&p);
        assert_eq!(q[1].quad, [40.0, 20.0, 0.0, 1.0]);
        assert_eq!(q[3].uv_semantic, [0.0, 1.0]);
        let bytes = q[2].to_bytes();
        assert_eq!(&bytes[28..32], &GFX_TESS_2D_PACKED_NORMAL.to_le_bytes());
        assert_eq!(quad_indices(0x1_0004), [7, 4, 6, 6, 4, 5]);
    }

    #[test]
    fn dropped_when_noncritical_space_is_gone() {
        let mut mem = [0u8; 0x2020];
        let mut b = PaintQueueBytes::new(&mut mem, 0);
        let white = [1.0f32; 4];
        let a = PaintScaledImageParams {
            material: 1,
            x: 0.0,
            y: 0.0,
            w: 1.0,
            h: 1.0,
            s0: 0.0,
            t0: 0.0,
            s1: 1.0,
            t1: 1.0,
            color: white,
        };
        assert!(!add_cmd_draw_stretch_pic(&mut b, &a));
        assert!(tess_quad_overflows(0x3fe, 0, 0x400, 0x600));
    }
}
