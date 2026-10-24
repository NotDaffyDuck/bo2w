//! Bitmap font sheets: where each letter sits on its sheet and how far the
//! pen moves after drawing it.

/// Glyph slots 0 to 0x5f hold the printable ASCII range from the space up,
/// in order, so a character in that range is found by position alone.
const ASCII_FIRST: u32 = 0x20;
const ASCII_SLOTS: usize = 0x60;

/// The slot a letter missing from the font is drawn from. Slot 14 is the
/// full stop.
const MISSING_LETTER_SLOT: usize = 14;

/// One letter of a font sheet.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FontGlyph {
    /// The character this glyph draws.
    pub letter: u16,
    /// Where the glyph's box starts, in pixels from the pen.
    pub x0: i8,
    pub y0: i8,
    /// How far the pen moves on after the glyph, in pixels.
    pub dx: u8,
    pub pixel_width: u8,
    pub pixel_height: u8,
    /// The glyph's corners on the sheet, as fractions of its width and height.
    pub s0: f32,
    pub t0: f32,
    pub s1: f32,
    pub t1: f32,
}

/// A font: its sheet's material and every glyph on it.
#[derive(Clone, Debug, Default)]
pub struct FontDef {
    pub name: String,
    /// Height of a line of text in pixels.
    pub pixel_height: i32,
    /// The material the sheet is drawn with.
    pub material: String,
    /// The material the text's glow is drawn with.
    pub glow_material: String,
    /// The ASCII slots first, then every other letter sorted by character.
    pub glyphs: Vec<FontGlyph>,
}

impl FontDef {
    /// The glyph that draws `letter`. A letter the font lacks comes back as
    /// the missing-letter glyph; an ASCII letter in a font too short to hold
    /// it comes back as `None`.
    pub fn glyph(&self, letter: u32) -> Option<&FontGlyph> {
        if let Some(slot) = letter.checked_sub(ASCII_FIRST) {
            if (slot as usize) < ASCII_SLOTS {
                return self.glyphs.get(slot as usize);
            }
        }
        let beyond_ascii = self.glyphs.get(ASCII_SLOTS..).unwrap_or_default();
        match beyond_ascii.binary_search_by_key(&letter, |glyph| u32::from(glyph.letter)) {
            Ok(found) => beyond_ascii.get(found),
            Err(_) => self.glyphs.get(MISSING_LETTER_SLOT),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn glyph(letter: u16) -> FontGlyph {
        FontGlyph {
            letter,
            x0: 0,
            y0: 0,
            dx: 1,
            pixel_width: 1,
            pixel_height: 1,
            s0: 0.0,
            t0: 0.0,
            s1: 1.0,
            t1: 1.0,
        }
    }

    fn font_with_extras(extras: &[u16]) -> FontDef {
        let mut glyphs: Vec<FontGlyph> = (0..ASCII_SLOTS as u16).map(|n| glyph(0x20 + n)).collect();
        glyphs.extend(extras.iter().map(|&letter| glyph(letter)));
        FontDef {
            glyphs,
            ..FontDef::default()
        }
    }

    #[test]
    fn ascii_is_found_by_position() {
        let font = font_with_extras(&[]);
        assert_eq!(font.glyph(u32::from(b'A')).map(|g| g.letter), Some(0x41));
        assert_eq!(font.glyph(0x20).map(|g| g.letter), Some(0x20));
    }

    #[test]
    fn other_letters_are_searched_by_character() {
        let font = font_with_extras(&[0xa9, 0xe9, 0x2022]);
        assert_eq!(font.glyph(0xe9).map(|g| g.letter), Some(0xe9));
        assert_eq!(font.glyph(0x2022).map(|g| g.letter), Some(0x2022));
    }

    #[test]
    fn an_unknown_letter_draws_as_the_missing_glyph() {
        let font = font_with_extras(&[0xa9]);
        assert_eq!(font.glyph(0x4e2d).map(|g| g.letter), Some(0x2e));
        assert_eq!(font.glyph(0x1f).map(|g| g.letter), Some(0x2e));
    }

    #[test]
    fn a_font_without_glyphs_has_nothing_to_draw() {
        assert!(FontDef::default().glyph(u32::from(b'A')).is_none());
        assert!(FontDef::default().glyph(0x4e2d).is_none());
    }
}
