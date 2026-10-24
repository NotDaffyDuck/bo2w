//! BO2 fonts and text metrics (scratch/hud-facts/font.md): em scale, UI font handles, the hud elem font
//! table, glyph lookup, colour escapes and line wrapping.

extern crate alloc;

use alloc::string::String;

use crate::scrplace::{HIRES_SCALE_X, ScreenPlacement};

/// Every BO2 font is scaled to a 48-pixel em.
pub const R_TEXT_EM: f32 = 48.0;

/// ui_smallFont default (archived, 0..1).
pub const UI_SMALL_FONT_DEFAULT: f32 = 0.563;
/// ui_bigFont default (archived, 0..1). BO2 never registers ui_extraBigFont, so that branch is dead.
pub const UI_BIG_FONT_DEFAULT: f32 = 0.688;

/// r_fontResolution values: fonts load from fonts/<720|1080>/<name>.
pub const FONT_RESOLUTIONS: [u32; 2] = [720, 1080];

/// BO2 UI font handle enum (UI_GetFontHandle). 0 and 4 pick by size.
pub const UI_FONT_DEFAULT: i32 = 0;
pub const UI_FONT_NORMAL: i32 = 1;
pub const UI_FONT_BIG: i32 = 2;
pub const UI_FONT_SMALL: i32 = 3;
pub const UI_FONT_BOLD: i32 = 4;
pub const UI_FONT_CONSOLE: i32 = 5;
pub const UI_FONT_EXTRABIG: i32 = 6;
pub const UI_FONT_EXTRASMALL: i32 = 7;
pub const UI_FONT_FX: i32 = 8;

pub const FONT_NORMAL: &str = "fonts/normalfont";
pub const FONT_BIG: &str = "fonts/bigfont";
pub const FONT_SMALL: &str = "fonts/smallfont";
pub const FONT_BOLD: &str = "fonts/boldfont";
pub const FONT_CONSOLE: &str = "fonts/consoleFont";
pub const FONT_EXTRABIG: &str = "fonts/extrabigfont";
pub const FONT_EXTRASMALL: &str = "fonts/extrasmallfont";
pub const FONT_FX: &str = "fonts/distfont";

/// Script-facing hud elem font names, in index order.
pub const HUDELEM_FONT_NAMES: [&str; 8] = [
    "default",
    "bigfixed",
    "smallfixed",
    "objective",
    "big",
    "small",
    "extrabig",
    "extrasmall",
];

/// Glyph used when a code point is not in the font.
pub const GLYPH_FALLBACK_INDEX: usize = 0x0e;
const GLYPH_DIRECT_FIRST: u32 = 0x20;
const GLYPH_DIRECT_END: u32 = 0x80;
const GLYPH_SEARCH_FIRST: usize = 0x60;

/// R_NormalizedTextScale: scale * 48 / font pixel height.
#[must_use]
pub fn normalized_text_scale(pixel_height: i32, text_scale: f32) -> f32 {
    if pixel_height <= 0 {
        return 0.0;
    }
    text_scale * R_TEXT_EM / pixel_height as f32
}

/// UI_TextHeight: an integer, floor(pixelHeight * normalized + 0.5), which is round(scale * 48).
#[must_use]
pub fn ui_text_height(text_scale: f32) -> i32 {
    libm::floorf(text_scale * R_TEXT_EM + 0.5) as i32
}

/// Font path for a registered name at a font resolution: fonts/<res>/<file>.
#[must_use]
pub fn font_path(resolution: u32, name: &str) -> String {
    let file = name.strip_prefix("fonts/").unwrap_or(name);
    alloc::format!("fonts/{resolution}/{file}")
}

/// Developer print text scale at a font resolution.
#[must_use]
pub fn dev_print_text_scale(resolution: u32) -> f32 {
    720.0 / resolution as f32 * 0.7
}

/// UI_GetFontHandle. Size-based handles compare RealFromVirtual_Height(scale) against ui_smallFont and
/// ui_bigFont.
#[must_use]
pub fn ui_get_font_handle(
    font_enum: i32,
    placement: &ScreenPlacement,
    scale: f32,
    small_font: f32,
    big_font: f32,
) -> &'static str {
    match font_enum {
        UI_FONT_NORMAL => FONT_NORMAL,
        UI_FONT_BIG => FONT_BIG,
        UI_FONT_SMALL => FONT_SMALL,
        UI_FONT_CONSOLE => FONT_CONSOLE,
        UI_FONT_EXTRABIG => FONT_EXTRABIG,
        UI_FONT_EXTRASMALL => FONT_EXTRASMALL,
        UI_FONT_FX => FONT_FX,
        UI_FONT_BOLD => {
            let size = placement.real_from_virtual_height(scale);
            if size <= small_font {
                FONT_SMALL
            } else if size >= big_font {
                FONT_BOLD
            } else {
                FONT_NORMAL
            }
        }
        _ => {
            let size = placement.real_from_virtual_height(scale);
            if size <= small_font {
                FONT_SMALL
            } else if size >= big_font {
                FONT_BIG
            } else {
                FONT_NORMAL
            }
        }
    }
}

/// Hud elem font index to (UI font handle, base scale). BO2 has 0..7 only and asserts on anything else.
#[must_use]
pub fn hudelem_font_info(elem_font: i32) -> Option<(i32, f32)> {
    Some(match elem_font {
        0 => (UI_FONT_DEFAULT, 0.25),
        1 => (UI_FONT_BOLD, 0.5),
        2 => (UI_FONT_CONSOLE, 1.0 / 3.0),
        3 => (UI_FONT_NORMAL, 0.25),
        4 => (UI_FONT_BIG, 0.25),
        5 => (UI_FONT_SMALL, 0.25),
        6 => (UI_FONT_EXTRABIG, 0.25),
        7 => (UI_FONT_EXTRASMALL, 0.25),
        _ => return None,
    })
}

/// Final hud elem font scale: lerped scale * base * split-screen scale * virtual-to-real y * hi-res x.
#[must_use]
pub fn hudelem_text_scale(elem_font: i32, lerped_scale: f32, placement: &ScreenPlacement) -> f32 {
    let base = hudelem_font_info(elem_font).map_or(0.25, |(_, b)| b);
    lerped_scale
        * base
        * placement.hud_splitscreen_scale
        * placement.scale_virtual_to_real[1]
        * HIRES_SCALE_X
}

/// Pixel height a hud elem's text is drawn at.
#[must_use]
pub fn hudelem_em_px(elem_font: i32, lerped_scale: f32, placement: &ScreenPlacement) -> i32 {
    ui_text_height(hudelem_text_scale(elem_font, lerped_scale, placement))
}

/// R_GetCharacterGlyph: 0x20..0x7f index directly; anything else is a binary search from index 0x60
/// over the sorted letters; a miss gives the fallback glyph. `letters` is each glyph's letter in order.
#[must_use]
pub fn glyph_index(letters: &[u16], letter: u32) -> usize {
    if (GLYPH_DIRECT_FIRST..GLYPH_DIRECT_END).contains(&letter) {
        return (letter - GLYPH_DIRECT_FIRST) as usize;
    }
    if letters.len() > GLYPH_SEARCH_FIRST
        && let Ok(i) = letters[GLYPH_SEARCH_FIRST..].binary_search_by(|&l| u32::from(l).cmp(&letter))
    {
        return GLYPH_SEARCH_FIRST + i;
    }
    GLYPH_FALLBACK_INDEX
}

pub const G_COLOR_TABLE: [[f32; 4]; 8] = [
    [0.0, 0.0, 0.0, 1.0],
    [1.0, 0.2, 0.2, 1.0],
    [0.0, 1.0, 0.0, 1.0],
    [1.0, 1.0, 0.5, 1.0],
    [0.0, 0.0, 1.0, 1.0],
    [0.0, 1.0, 1.0, 1.0],
    [1.0, 0.36, 1.0, 1.0],
    [1.0, 1.0, 1.0, 1.0],
];

/// Render-side byte colours, RGBA.
pub const COLOR_TABLE_RGBA: [u32; 8] = [
    0x0000_00ff,
    0xff33_33ff,
    0x00ff_00ff,
    0xffff_80ff,
    0x0000_ffff,
    0x00ff_ffff,
    0xff5c_ffff,
    0xffff_ffff,
];

/// ColorIndex: c - '0' when that is 0..9, else 7.
#[must_use]
pub fn color_index(c: u8) -> u32 {
    let i = u32::from(c).wrapping_sub(u32::from(b'0'));
    if i <= 9 { i } else { 7 }
}

/// RB_LookupColor for a caret escape: 0..7 the table, ^8 allies, ^9 axis, anything else white.
#[must_use]
pub fn color_from_caret(c: u8, allies: [f32; 4], axis: [f32; 4]) -> [f32; 4] {
    match color_index(c) {
        i @ 0..=7 => G_COLOR_TABLE[i as usize],
        8 => allies,
        9 => axis,
        _ => [1.0; 4],
    }
}

/// Next drawn letter: a caret followed by a digit or F is a zero-width escape.
pub fn next_letter(chars: &mut core::iter::Peekable<core::str::Chars<'_>>) -> Option<u32> {
    loop {
        let c = chars.next()?;
        if c == '^'
            && let Some(n) = chars.peek().copied()
            && (n.is_ascii_digit() || n == 'F')
        {
            chars.next();
            continue;
        }
        return Some(c as u32);
    }
}

/// One UTF-8 code point at `pos`: (code point, byte length). A bad lead byte reads as one byte.
fn read_char(text: &[u8], pos: usize) -> (u32, usize) {
    let b = text[pos];
    let (len, init) = match b {
        0x00..=0x7f => return (u32::from(b), 1),
        0xc0..=0xdf => (2, u32::from(b & 0x1f)),
        0xe0..=0xef => (3, u32::from(b & 0x0f)),
        0xf0..=0xf7 => (4, u32::from(b & 0x07)),
        _ => return (u32::from(b), 1),
    };
    if pos + len > text.len() {
        return (u32::from(b), 1);
    }
    let cp = text[pos + 1..pos + len]
        .iter()
        .fold(init, |acc, &c| (acc << 6) | u32::from(c & 0x3f));
    (cp, len)
}

fn is_c_space(cp: u32) -> bool {
    matches!(cp, 0x20 | 0x09..=0x0d)
}

/// R_TextLineWrapPosition: the byte offset where this line ends. `glyph_dx` gives a letter's advance.
/// '\r' resets the width, '\n' ends the line after itself, ^digit and ^F are zero width, ^H and ^I
/// are inline images (a break chance). Breaks are remembered after whitespace past the first char.
/// When width * scale passes `max_width` the line ends at the last break, else where that letter starts.
/// `max_chars` <= 0 is unlimited.
pub fn text_line_wrap_position(
    text: &[u8],
    max_chars: i32,
    max_width: f32,
    scale: f32,
    pixel_height: i32,
    glyph_dx: impl Fn(u32) -> u8,
) -> usize {
    let mut pos = 0;
    let mut width = 0i32;
    let mut last_break = None;
    let mut count = 0;
    while pos < text.len() && text[pos] != 0 {
        if max_chars > 0 && count >= max_chars {
            return pos;
        }
        let start = pos;
        let (cp, n) = read_char(text, pos);
        pos += n;
        count += 1;
        match cp {
            0x0d => {
                width = 0;
                continue;
            }
            0x0a => return pos,
            0x5e if pos < text.len() => match text[pos] {
                b'0'..=b'9' | b'F' => {
                    pos += 1;
                    continue;
                }
                b'H' | b'I' if start + 3 < text.len() => {
                    let c = i32::from(text[start + 2]);
                    width += ((c - 0x10) * pixel_height + 0x10) / 32;
                    pos = (start + 4 + usize::from(text[start + 3])).min(text.len());
                    last_break = Some(pos);
                }
                _ => width += i32::from(glyph_dx(cp)),
            },
            _ => width += i32::from(glyph_dx(cp)),
        }
        if width as f32 * scale > max_width {
            return last_break.unwrap_or(start);
        }
        if start > 0 && cp < 0x100 && is_c_space(cp) {
            last_break = Some(pos);
        }
    }
    pos
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scrplace::PlacementInputs;

    fn placement(h: f32) -> ScreenPlacement {
        ScreenPlacement::setup_float_viewport(
            h * 4.0 / 3.0,
            h,
            &PlacementInputs {
                display_size: [h * 4.0 / 3.0, h],
                safe_area: [1.0, 1.0],
                profile_safe_area: None,
                aspect_display_pixel: 1.0,
                dual_play_enable: false,
                hud_splitscreen_scale: None,
            },
        )
    }

    #[test]
    fn text_height_rounds() {
        assert_eq!(ui_text_height(0.25), 12);
        assert_eq!(ui_text_height(0.26), 12);
        assert_eq!(ui_text_height(0.27), 13);
        assert_eq!(normalized_text_scale(24, 0.5), 1.0);
        assert_eq!(normalized_text_scale(0, 0.5), 0.0);
    }

    #[test]
    fn font_path_by_resolution() {
        assert_eq!(font_path(1080, FONT_BIG), "fonts/1080/bigfont");
        assert_eq!(font_path(720, FONT_FX), "fonts/720/distfont");
        assert!((dev_print_text_scale(1080) - 0.466_666_7).abs() < 1e-6);
    }

    #[test]
    fn size_based_handles_use_dvars_on_real_height() {
        let p = placement(480.0);
        let d = |s| ui_get_font_handle(0, &p, s, UI_SMALL_FONT_DEFAULT, UI_BIG_FONT_DEFAULT);
        assert_eq!(d(0.5), FONT_SMALL);
        assert_eq!(d(0.6), FONT_NORMAL);
        assert_eq!(d(0.7), FONT_BIG);
        let b = |s| ui_get_font_handle(4, &p, s, UI_SMALL_FONT_DEFAULT, UI_BIG_FONT_DEFAULT);
        assert_eq!(b(0.7), FONT_BOLD);
        // At 1080p the same script scale reads bigger.
        let p = placement(1080.0);
        assert_eq!(ui_get_font_handle(0, &p, 0.25, 0.563, 0.688), FONT_SMALL);
        assert_eq!(ui_get_font_handle(0, &p, 0.3, 0.563, 0.688), FONT_NORMAL);
        assert_eq!(ui_get_font_handle(8, &p, 1.0, 0.563, 0.688), FONT_FX);
    }

    #[test]
    fn hudelem_table() {
        assert_eq!(hudelem_font_info(1), Some((UI_FONT_BOLD, 0.5)));
        assert_eq!(hudelem_font_info(3), Some((UI_FONT_NORMAL, 0.25)));
        assert_eq!(hudelem_font_info(8), None);
        let p = placement(960.0);
        assert_eq!(hudelem_text_scale(1, 1.0, &p), 1.0);
        assert_eq!(hudelem_em_px(1, 1.0, &p), 48);
        assert_eq!(HUDELEM_FONT_NAMES[3], "objective");
    }

    #[test]
    fn glyph_lookup() {
        let mut letters: alloc::vec::Vec<u16> = (0x20u16..0x80).collect();
        letters.extend([0xe9, 0x3a9, 0x4e2d]);
        assert_eq!(glyph_index(&letters, u32::from(b'A')), 0x21);
        assert_eq!(glyph_index(&letters, 0x3a9), 0x61);
        assert_eq!(glyph_index(&letters, 0x4e2d), 0x62);
        assert_eq!(glyph_index(&letters, 0x1234), GLYPH_FALLBACK_INDEX);
    }

    #[test]
    fn caret_colours() {
        let allies = [0.1, 0.2, 0.3, 1.0];
        let axis = [0.4, 0.5, 0.6, 1.0];
        assert_eq!(color_from_caret(b'1', allies, axis), [1.0, 0.2, 0.2, 1.0]);
        assert_eq!(color_from_caret(b'3', allies, axis), [1.0, 1.0, 0.5, 1.0]);
        assert_eq!(color_from_caret(b'8', allies, axis), allies);
        assert_eq!(color_from_caret(b'9', allies, axis), axis);
        assert_eq!(color_from_caret(b'x', allies, axis), G_COLOR_TABLE[7]);
        assert_eq!(color_index(b' '), 7);
    }

    #[test]
    fn next_letter_skips_digit_and_f_escapes() {
        let mut it = "^1a^Fb^Hc".chars().peekable();
        let got: alloc::vec::Vec<u32> = core::iter::from_fn(|| next_letter(&mut it)).collect();
        assert_eq!(got, [97, 98, 94, 72, 99]);
    }

    #[test]
    fn line_wrap() {
        let dx = |_| 10u8;
        // 10 px per letter, 45 px wide: "ab cd" breaks after the space.
        assert_eq!(text_line_wrap_position(b"ab cdef", 0, 45.0, 1.0, 48, dx), 3);
        // No break chance: end where the overflowing letter starts.
        assert_eq!(text_line_wrap_position(b"abcdef", 0, 45.0, 1.0, 48, dx), 4);
        assert_eq!(text_line_wrap_position(b"ab\ncd", 0, 1000.0, 1.0, 48, dx), 3);
        assert_eq!(text_line_wrap_position(b"^1abcd", 0, 45.0, 1.0, 48, dx), 6);
        assert_eq!(text_line_wrap_position(b"abcdef", 2, 1000.0, 1.0, 48, dx), 2);
        assert_eq!(text_line_wrap_position(b"ab\rcdef", 0, 45.0, 1.0, 48, dx), 7);
        // Inline image ^H, width byte 0x20, name length 2: ((0x10 * 48) + 16) / 32 = 24 px.
        let img = [b'a', b'^', b'H', 0x20, 2, b'x', b'y', b'b'];
        assert_eq!(text_line_wrap_position(&img, 0, 1000.0, 1.0, 48, dx), img.len());
        assert_eq!(text_line_wrap_position(&img, 0, 40.0, 1.0, 48, dx), 7);
    }
}
