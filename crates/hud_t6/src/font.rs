//! BO2 font metrics the HUD uses (scratch/hud-facts/font.md): the 48-pixel em, the UI font handle
//! numbers, and the size hud elem text is drawn at.

use super::scrplace::{DisplayAnchoring, HIRES_SCALE_X};

/// Every BO2 font is scaled to a 48-pixel em.
pub const GLYPH_EM_UNIT: f32 = 48.0;

/// BO2 UI font handle enum (UI_GetFontHandle). 0 and 4 pick by size.
pub const UI_FONT_DEFAULT: i32 = 0;
pub const UI_FONT_NORMAL: i32 = 1;
pub const UI_FONT_BIG: i32 = 2;
pub const UI_FONT_SMALL: i32 = 3;
pub const UI_FONT_BOLD: i32 = 4;
pub const UI_FONT_CONSOLE: i32 = 5;
pub const UI_FONT_EXTRABIG: i32 = 6;
pub const UI_FONT_EXTRASMALL: i32 = 7;

/// The hud elem fonts 0..7: the UI font each draws with and its base scale.
/// BO2 has no others and asserts on anything else.
const HUDELEM_FONTS: [(i32, f32); 8] = [
    (UI_FONT_DEFAULT, 0.25),
    (UI_FONT_BOLD, 0.5),
    (UI_FONT_CONSOLE, 1.0 / 3.0),
    (UI_FONT_NORMAL, 0.25),
    (UI_FONT_BIG, 0.25),
    (UI_FONT_SMALL, 0.25),
    (UI_FONT_EXTRABIG, 0.25),
    (UI_FONT_EXTRASMALL, 0.25),
];

/// Base scale for an unknown hud elem font.
const FALLBACK_BASE_SCALE: f32 = 0.25;

/// UI_TextHeight: an integer, floor(pixelHeight * normalized + 0.5), which is round(scale * 48).
#[must_use]
pub fn ui_text_height(text_scale: f32) -> i32 {
    let em_px = text_scale * GLYPH_EM_UNIT;
    libm::floorf(em_px + 0.5) as i32
}

/// Hud elem font index to (UI font handle, base scale).
#[must_use]
pub fn hudelem_font_info(elem_font: i32) -> Option<(i32, f32)> {
    usize::try_from(elem_font).ok().and_then(|slot| HUDELEM_FONTS.get(slot)).copied()
}

/// Final hud elem font scale: lerped scale * base * split-screen scale * virtual-to-real y * hi-res x.
#[must_use]
pub fn hudelem_text_scale(font_slot: i32, eased_scale: f32, place: &DisplayAnchoring) -> f32 {
    let base = match hudelem_font_info(font_slot) {
        Some((_, scale)) => scale,
        None => FALLBACK_BASE_SCALE,
    };
    // Multiplied left to right, as BO2 does.
    let factors = [base, place.hud_splitscreen_scale, place.layout_to_pixels[1], HIRES_SCALE_X];
    factors.iter().fold(eased_scale, |scale, factor| scale * factor)
}

/// Pixel height a hud elem's text is drawn at.
#[must_use]
pub fn hudelem_em_px(font_slot: i32, eased_scale: f32, place: &DisplayAnchoring) -> i32 {
    ui_text_height(hudelem_text_scale(font_slot, eased_scale, place))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scrplace::PlacementInputs;

    /// A 4:3 screen `height` pixels tall, nothing trimmed for safe area.
    fn screen(height: f32) -> DisplayAnchoring {
        let width = height * 4.0 / 3.0;
        let inputs = PlacementInputs {
            display_size: [width, height],
            safe_area: [1.0, 1.0],
            profile_safe_area: None,
            aspect_display_pixel: 1.0,
            dual_play_enable: false,
            hud_splitscreen_scale: None,
        };
        DisplayAnchoring::setup_float_viewport(width, height, &inputs)
    }

    #[test]
    fn text_height_rounds() {
        let heights = [0.25, 0.26, 0.27].map(ui_text_height);
        assert_eq!(heights, [12, 12, 13]);
    }

    #[test]
    fn hudelem_table() {
        assert_eq!(hudelem_font_info(1), Some((UI_FONT_BOLD, 0.5)));
        assert_eq!(hudelem_font_info(3), Some((UI_FONT_NORMAL, 0.25)));
        assert!(hudelem_font_info(8).is_none());
        assert!(hudelem_font_info(-1).is_none());
        let tall = screen(960.0);
        assert_eq!(hudelem_text_scale(1, 1.0, &tall), 1.0);
        assert_eq!(hudelem_em_px(1, 1.0, &tall), 48);
    }
}
