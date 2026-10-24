//! BO2 2D text command (scratch/hud-facts/draw_text_cmd.md): the id-17 command with its 100-byte head,
//! the render flags, the legacy style map and the caret colour walk done at execute time.

use crate::font::COLOR_TABLE_RGBA;
use crate::render_commands::{GfxRenderCommandBuf, RC_DRAW_TEXT_2D};
use crate::stretch_pic_cmd::convert_color_to_bytes;

pub const GFX_CMD_DRAW_TEXT_TYPE: usize = 4;
pub const GFX_CMD_DRAW_TEXT_X: usize = 8;
pub const GFX_CMD_DRAW_TEXT_Y: usize = 12;
pub const GFX_CMD_DRAW_TEXT_W: usize = 16;
pub const GFX_CMD_DRAW_TEXT_H: usize = 20;
pub const GFX_CMD_DRAW_TEXT_ROTATION: usize = 24;
pub const GFX_CMD_DRAW_TEXT_FONT: usize = 28;
pub const GFX_CMD_DRAW_TEXT_X_SCALE: usize = 32;
pub const GFX_CMD_DRAW_TEXT_Y_SCALE: usize = 36;
pub const GFX_CMD_DRAW_TEXT_COLOR: usize = 40;
pub const GFX_CMD_DRAW_TEXT_MAX_CHARS: usize = 44;
pub const GFX_CMD_DRAW_TEXT_RENDER_FLAGS: usize = 48;
pub const GFX_CMD_DRAW_TEXT_CURSOR_POS: usize = 52;
pub const GFX_CMD_DRAW_TEXT_CURSOR_LETTER: usize = 56;
pub const GFX_CMD_DRAW_TEXT_GLOW_FORCE_COLOR: usize = 60;
pub const GFX_CMD_DRAW_TEXT_FX_BIRTH_TIME: usize = 64;
pub const GFX_CMD_DRAW_TEXT_FX_LETTER_TIME: usize = 68;
pub const GFX_CMD_DRAW_TEXT_FX_DECAY_START_TIME: usize = 72;
pub const GFX_CMD_DRAW_TEXT_FX_DECAY_DURATION: usize = 76;
pub const GFX_CMD_DRAW_TEXT_FX_REDACT_DECAY_START_TIME: usize = 80;
pub const GFX_CMD_DRAW_TEXT_FX_REDACT_DECAY_DURATION: usize = 84;
pub const GFX_CMD_DRAW_TEXT_FX_MATERIAL: usize = 88;
pub const GFX_CMD_DRAW_TEXT_FX_MATERIAL_GLOW: usize = 92;
pub const GFX_CMD_DRAW_TEXT_TEXT: usize = 100;

/// Layout types: 0 standard, 1 console text.
pub const DRAW_TEXT_TYPE_STANDARD: i32 = 0;
pub const DRAW_TEXT_TYPE_CONSOLE: i32 = 1;

pub const TEXT_RENDERFLAG_FORCEMONOSPACE: u32 = 0x1;
pub const TEXT_RENDERFLAG_CURSOR: u32 = 0x2;
pub const TEXT_RENDERFLAG_DROPSHADOW: u32 = 0x4;
pub const TEXT_RENDERFLAG_DROPSHADOW_EXTRA: u32 = 0x8;
pub const TEXT_RENDERFLAG_GLOW: u32 = 0x10;
pub const TEXT_RENDERFLAG_GLOW_FORCE_COLOR: u32 = 0x20;
pub const TEXT_RENDERFLAG_FX_DECODE: u32 = 0x40;
pub const TEXT_RENDERFLAG_PADDING: u32 = 0x80;
pub const TEXT_RENDERFLAG_SUBTITLETEXT: u32 = 0x100;
pub const TEXT_RENDERFLAG_CINEMATIC: u32 = 0x200;
pub const TEXT_RENDERFLAG_TYPEWRITER: u32 = 0x400;
pub const TEXT_RENDERFLAG_FADEIN: u32 = 0x800;
pub const TEXT_RENDERFLAG_COD7_TYPEWRITER: u32 = 0x1000;
pub const TEXT_RENDERFLAG_POP_IN_WRITE: u32 = 0x2000;
pub const TEXT_RENDERFLAG_REDACT: u32 = 0x4000;
pub const TEXT_RENDERFLAG_JITTERSCALE: u32 = 0x8000;
pub const TEXT_RENDERFLAG_COD7_DECODE: u32 = 0x1_0000;
pub const TEXT_RENDERFLAG_AUTOWRAP: u32 = 0x2_0000;

/// Subtitle ^2: RGB 0xe6, alpha scaled by this.
pub const SUBTITLE_GREEN_RGB: u8 = 0xe6;
pub const SUBTITLE_GREEN_ALPHA_SCALE: f32 = 0.86;

/// Command size for a text of `len` bytes: the 100-byte head plus the NUL, rounded to 4.
#[must_use]
pub fn gfx_cmd_draw_text_size(len: u32) -> u32 {
    (len + 0x68) & !3
}

/// Plain R_AddCmdDrawText style map: 3 drop shadow, 6 drop shadow + extra, 0x80 monospace, else 0.
#[must_use]
pub fn draw_text_render_flags(style: i32) -> u32 {
    match style {
        3 => TEXT_RENDERFLAG_DROPSHADOW,
        6 => TEXT_RENDERFLAG_DROPSHADOW | TEXT_RENDERFLAG_DROPSHADOW_EXTRA,
        0x80 => TEXT_RENDERFLAG_FORCEMONOSPACE,
        _ => 0,
    }
}

/// Cursor and effect fields; the plain variant leaves all of them zero.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct GfxCmdDrawTextFx {
    pub cursor_pos: i32,
    pub cursor_letter: u8,
    pub glow_force_color: u32,
    pub fx_birth_time: i32,
    pub fx_letter_time: i32,
    pub fx_decay_start_time: i32,
    pub fx_decay_duration: i32,
    pub fx_redact_decay_start_time: i32,
    pub fx_redact_decay_duration: i32,
    pub fx_material: u32,
    pub fx_material_glow: u32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GfxCmdDrawText2DArgs<'a> {
    /// Text bytes without the NUL; a NUL inside ends it.
    pub text: &'a [u8],
    pub max_chars: i32,
    pub font: u32,
    pub x: f32,
    pub y: f32,
    /// Vertex W; the plain variant writes 1.0.
    pub w: f32,
    pub rotation: f32,
    pub x_scale: f32,
    pub y_scale: f32,
    pub color: [f32; 4],
    pub render_flags: u32,
    pub fx: GfxCmdDrawTextFx,
    /// r_dualPlayActive: halves yScale before queueing.
    pub dual_play_active: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AddDrawTextCmd {
    /// Empty text: nothing queued.
    Empty,
    /// Text longer than a command can hold, or no noncritical room: silently dropped.
    Dropped,
    Wrote { offset: u32, size: u32 },
}

fn put(cmd: &mut [u8], at: usize, v: [u8; 4]) {
    cmd[at..at + 4].copy_from_slice(&v);
}

/// R_AddCmdDrawText (and its fx/cursor variants through `fx`): queue one id-17 command.
pub fn add_cmd_draw_text(cmds: &mut GfxRenderCommandBuf<'_>, a: &GfxCmdDrawText2DArgs<'_>) -> AddDrawTextCmd {
    let text = a.text.iter().position(|&c| c == 0).map_or(a.text, |n| &a.text[..n]);
    if text.is_empty() {
        return AddDrawTextCmd::Empty;
    }
    debug_assert!(a.max_chars > 0, "maxChars must be positive");
    debug_assert!(a.x_scale > 0.0 && a.y_scale > 0.0, "text scales must be positive");
    let Ok(len) = u32::try_from(text.len()) else {
        return AddDrawTextCmd::Dropped;
    };
    let size = gfx_cmd_draw_text_size(len);
    let y_scale = if a.dual_play_active { a.y_scale * 0.5 } else { a.y_scale };
    let Some(cmd) = cmds.get_command_buffer(RC_DRAW_TEXT_2D, size) else {
        return AddDrawTextCmd::Dropped;
    };
    cmd[4..].fill(0);
    put(cmd, GFX_CMD_DRAW_TEXT_TYPE, DRAW_TEXT_TYPE_STANDARD.to_le_bytes());
    put(cmd, GFX_CMD_DRAW_TEXT_X, a.x.to_le_bytes());
    put(cmd, GFX_CMD_DRAW_TEXT_Y, a.y.to_le_bytes());
    put(cmd, GFX_CMD_DRAW_TEXT_W, a.w.to_le_bytes());
    put(cmd, GFX_CMD_DRAW_TEXT_ROTATION, a.rotation.to_le_bytes());
    put(cmd, GFX_CMD_DRAW_TEXT_FONT, a.font.to_le_bytes());
    put(cmd, GFX_CMD_DRAW_TEXT_X_SCALE, a.x_scale.to_le_bytes());
    put(cmd, GFX_CMD_DRAW_TEXT_Y_SCALE, y_scale.to_le_bytes());
    put(cmd, GFX_CMD_DRAW_TEXT_COLOR, convert_color_to_bytes(a.color).to_le_bytes());
    put(cmd, GFX_CMD_DRAW_TEXT_MAX_CHARS, a.max_chars.to_le_bytes());
    put(cmd, GFX_CMD_DRAW_TEXT_RENDER_FLAGS, a.render_flags.to_le_bytes());
    let fx = &a.fx;
    put(cmd, GFX_CMD_DRAW_TEXT_CURSOR_POS, fx.cursor_pos.to_le_bytes());
    cmd[GFX_CMD_DRAW_TEXT_CURSOR_LETTER] = fx.cursor_letter;
    put(cmd, GFX_CMD_DRAW_TEXT_GLOW_FORCE_COLOR, fx.glow_force_color.to_le_bytes());
    put(cmd, GFX_CMD_DRAW_TEXT_FX_BIRTH_TIME, fx.fx_birth_time.to_le_bytes());
    put(cmd, GFX_CMD_DRAW_TEXT_FX_LETTER_TIME, fx.fx_letter_time.to_le_bytes());
    put(cmd, GFX_CMD_DRAW_TEXT_FX_DECAY_START_TIME, fx.fx_decay_start_time.to_le_bytes());
    put(cmd, GFX_CMD_DRAW_TEXT_FX_DECAY_DURATION, fx.fx_decay_duration.to_le_bytes());
    put(cmd, GFX_CMD_DRAW_TEXT_FX_REDACT_DECAY_START_TIME, fx.fx_redact_decay_start_time.to_le_bytes());
    put(cmd, GFX_CMD_DRAW_TEXT_FX_REDACT_DECAY_DURATION, fx.fx_redact_decay_duration.to_le_bytes());
    put(cmd, GFX_CMD_DRAW_TEXT_FX_MATERIAL, fx.fx_material.to_le_bytes());
    put(cmd, GFX_CMD_DRAW_TEXT_FX_MATERIAL_GLOW, fx.fx_material_glow.to_le_bytes());
    let t = GFX_CMD_DRAW_TEXT_TEXT;
    cmd[t..t + text.len()].copy_from_slice(text);
    let offset = cmds.last.unwrap_or(0);
    AddDrawTextCmd::Wrote { offset, size }
}

/// The decoded command; `text` stops at the NUL.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GfxCmdDrawText2D<'a> {
    pub ty: i32,
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    pub rotation: f32,
    pub font: u32,
    pub x_scale: f32,
    pub y_scale: f32,
    pub color_rgba: u32,
    pub max_chars: i32,
    pub render_flags: u32,
    pub fx: GfxCmdDrawTextFx,
    pub text: &'a [u8],
}

fn word(cmd: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([cmd[at], cmd[at + 1], cmd[at + 2], cmd[at + 3]])
}

#[must_use]
pub fn parse_gfx_cmd_draw_text(cmd: &[u8]) -> Option<GfxCmdDrawText2D<'_>> {
    if cmd.len() <= GFX_CMD_DRAW_TEXT_TEXT || cmd[2] != RC_DRAW_TEXT_2D {
        return None;
    }
    let tail = &cmd[GFX_CMD_DRAW_TEXT_TEXT..];
    let text = &tail[..tail.iter().position(|&c| c == 0)?];
    let f = |at| f32::from_bits(word(cmd, at));
    let i = |at| word(cmd, at) as i32;
    Some(GfxCmdDrawText2D {
        ty: i(GFX_CMD_DRAW_TEXT_TYPE),
        x: f(GFX_CMD_DRAW_TEXT_X),
        y: f(GFX_CMD_DRAW_TEXT_Y),
        w: f(GFX_CMD_DRAW_TEXT_W),
        h: f(GFX_CMD_DRAW_TEXT_H),
        rotation: f(GFX_CMD_DRAW_TEXT_ROTATION),
        font: word(cmd, GFX_CMD_DRAW_TEXT_FONT),
        x_scale: f(GFX_CMD_DRAW_TEXT_X_SCALE),
        y_scale: f(GFX_CMD_DRAW_TEXT_Y_SCALE),
        color_rgba: word(cmd, GFX_CMD_DRAW_TEXT_COLOR),
        max_chars: i(GFX_CMD_DRAW_TEXT_MAX_CHARS),
        render_flags: word(cmd, GFX_CMD_DRAW_TEXT_RENDER_FLAGS),
        fx: GfxCmdDrawTextFx {
            cursor_pos: i(GFX_CMD_DRAW_TEXT_CURSOR_POS),
            cursor_letter: cmd[GFX_CMD_DRAW_TEXT_CURSOR_LETTER],
            glow_force_color: word(cmd, GFX_CMD_DRAW_TEXT_GLOW_FORCE_COLOR),
            fx_birth_time: i(GFX_CMD_DRAW_TEXT_FX_BIRTH_TIME),
            fx_letter_time: i(GFX_CMD_DRAW_TEXT_FX_LETTER_TIME),
            fx_decay_start_time: i(GFX_CMD_DRAW_TEXT_FX_DECAY_START_TIME),
            fx_decay_duration: i(GFX_CMD_DRAW_TEXT_FX_DECAY_DURATION),
            fx_redact_decay_start_time: i(GFX_CMD_DRAW_TEXT_FX_REDACT_DECAY_START_TIME),
            fx_redact_decay_duration: i(GFX_CMD_DRAW_TEXT_FX_REDACT_DECAY_DURATION),
            fx_material: word(cmd, GFX_CMD_DRAW_TEXT_FX_MATERIAL),
            fx_material_glow: word(cmd, GFX_CMD_DRAW_TEXT_FX_MATERIAL_GLOW),
        },
        text,
    })
}

/// Rotation (degrees) as (sin, cos), as RB_DrawText2DCmd does before walking the glyphs.
#[must_use]
pub fn rotation_sin_cos(degrees: f32) -> (f32, f32) {
    let r = degrees * (core::f32::consts::PI / 180.0);
    (libm::sinf(r), libm::cosf(r))
}

/// Byte colour (packed RGBA as in the command) for caret digit index 0..9.
#[must_use]
pub fn lookup_color_rgba(index: u32, allies: u32, axis: u32) -> u32 {
    match index {
        0..=7 => u32::from_le_bytes(COLOR_TABLE_RGBA[index as usize].to_be_bytes()),
        8 => allies,
        9 => axis,
        _ => 0xffff_ffff,
    }
}

/// One drawn byte from the caret walk.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TextDrawByte {
    pub index: usize,
    pub byte: u8,
    pub color: u32,
    /// State of the ^F toggle at this byte.
    pub f_toggle: bool,
}

/// DrawText2D's escape handling: ^0..^9 set the colour (^7 back to the command colour, subtitle ^2
/// pale), ^F toggles a flag, ^^ prints one caret; REDACT makes every caret literal.
pub struct TextColorWalk<'a> {
    text: &'a [u8],
    at: usize,
    base: u32,
    color: u32,
    flags: u32,
    allies: u32,
    axis: u32,
    f_toggle: bool,
}

impl<'a> TextColorWalk<'a> {
    #[must_use]
    pub fn new(cmd: &GfxCmdDrawText2D<'a>, allies: u32, axis: u32) -> Self {
        Self {
            text: cmd.text,
            at: 0,
            base: cmd.color_rgba,
            color: cmd.color_rgba,
            flags: cmd.render_flags,
            allies,
            axis,
            f_toggle: false,
        }
    }

    fn digit_color(&self, d: u32) -> u32 {
        if d == 7 {
            return self.base;
        }
        if d == 2 && self.flags & TEXT_RENDERFLAG_SUBTITLETEXT != 0 {
            let a = (f32::from((self.base >> 24) as u8) * SUBTITLE_GREEN_ALPHA_SCALE) as u32;
            let c = u32::from(SUBTITLE_GREEN_RGB);
            return c | c << 8 | c << 16 | a << 24;
        }
        lookup_color_rgba(d, self.allies, self.axis)
    }
}

impl Iterator for TextColorWalk<'_> {
    type Item = TextDrawByte;

    fn next(&mut self) -> Option<TextDrawByte> {
        loop {
            let i = self.at;
            let &c = self.text.get(i)?;
            self.at += 1;
            if c == b'^' && self.flags & TEXT_RENDERFLAG_REDACT == 0 {
                match self.text.get(i + 1) {
                    Some(&d @ b'0'..=b'9') => {
                        self.color = self.digit_color(u32::from(d - b'0'));
                        self.at += 1;
                        continue;
                    }
                    Some(b'F') => {
                        self.f_toggle = !self.f_toggle;
                        self.at += 1;
                        continue;
                    }
                    Some(b'^') => self.at += 1,
                    _ => {}
                }
            }
            return Some(TextDrawByte { index: i, byte: c, color: self.color, f_toggle: self.f_toggle });
        }
    }
}

#[cfg(test)]
mod tests {
    extern crate alloc;
    use alloc::vec::Vec;

    use super::*;

    fn args(text: &[u8]) -> GfxCmdDrawText2DArgs<'_> {
        GfxCmdDrawText2DArgs {
            text,
            max_chars: 0x7fff_ffff,
            font: 3,
            x: 1.0,
            y: 2.0,
            w: 1.0,
            rotation: 0.0,
            x_scale: 0.5,
            y_scale: 0.5,
            color: [1.0, 1.0, 1.0, 1.0],
            render_flags: draw_text_render_flags(3),
            fx: GfxCmdDrawTextFx::default(),
            dual_play_active: false,
        }
    }

    #[test]
    fn size_and_style_map() {
        assert_eq!(gfx_cmd_draw_text_size(0), 0x68);
        assert_eq!(gfx_cmd_draw_text_size(3), 0x68);
        assert_eq!(gfx_cmd_draw_text_size(4), 0x6c);
        assert_eq!(draw_text_render_flags(6), 0xc);
        assert_eq!(draw_text_render_flags(0x80), 1);
        assert_eq!(draw_text_render_flags(7), 0);
        assert_eq!(draw_text_render_flags(0x84), 0);
    }

    #[test]
    fn queue_and_parse() {
        let mut mem = [0u8; 0x4000];
        let mut b = GfxRenderCommandBuf::new(&mut mem, 0);
        assert_eq!(add_cmd_draw_text(&mut b, &args(b"")), AddDrawTextCmd::Empty);
        let mut a = args(b"Hi^1x\0junk");
        a.dual_play_active = true;
        assert_eq!(add_cmd_draw_text(&mut b, &a), AddDrawTextCmd::Wrote { offset: 0, size: 0x6c });
        let p = parse_gfx_cmd_draw_text(&mem[..0x6c]).unwrap();
        assert_eq!(p.text, b"Hi^1x");
        assert_eq!(p.y_scale, 0.25);
        assert_eq!(p.render_flags, TEXT_RENDERFLAG_DROPSHADOW);
        assert_eq!(p.color_rgba, 0xffff_ffff);
        assert_eq!(p.fx, GfxCmdDrawTextFx::default());
        assert_eq!(p.w, 1.0);
    }

    #[test]
    fn caret_walk() {
        let mut mem = [0u8; 0x4000];
        let mut b = GfxRenderCommandBuf::new(&mut mem, 0);
        let mut a = args(b"a^1b^7c^^d^8e^Ff");
        a.color = [0.0, 0.0, 0.0, 1.0];
        add_cmd_draw_text(&mut b, &a);
        let p = parse_gfx_cmd_draw_text(&mem[..0x80]).unwrap();
        let w: Vec<_> = TextColorWalk::new(&p, 0x1111_1111, 0x2222_2222).collect();
        let bytes: Vec<u8> = w.iter().map(|d| d.byte).collect();
        assert_eq!(bytes, b"abc^def");
        assert_eq!(w[0].color, 0xff00_0000);
        assert_eq!(w[1].color, 0xff33_33ff);
        assert_eq!(w[2].color, 0xff00_0000);
        assert_eq!(w[5].color, 0x1111_1111);
        assert!(!w[5].f_toggle && w[6].f_toggle);
    }

    #[test]
    fn subtitle_green_and_redact() {
        let mut mem = [0u8; 0x4000];
        let mut b = GfxRenderCommandBuf::new(&mut mem, 0);
        let mut a = args(b"^2s");
        a.render_flags = TEXT_RENDERFLAG_SUBTITLETEXT;
        add_cmd_draw_text(&mut b, &a);
        let mut a = args(b"^2s");
        a.render_flags = TEXT_RENDERFLAG_REDACT;
        add_cmd_draw_text(&mut b, &a);
        let p = parse_gfx_cmd_draw_text(&mem[..0x68]).unwrap();
        let w: Vec<_> = TextColorWalk::new(&p, 0, 0).collect();
        assert_eq!(w.len(), 1);
        // alpha 255 * 0.86 = 219.3 -> 219.
        assert_eq!(w[0].color, 0xdbe6_e6e6);
        let p = parse_gfx_cmd_draw_text(&mem[0x68..0xd0]).unwrap();
        assert_eq!(TextColorWalk::new(&p, 0, 0).count(), 3);
        let (s, c) = rotation_sin_cos(90.0);
        assert!((s - 1.0).abs() < 1e-6 && c.abs() < 1e-6);
    }
}
