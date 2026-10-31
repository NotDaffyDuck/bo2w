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
    /// The character this letter_shape draws.
    pub glyph_char: u16,
    /// Where the letter_shape's box starts, in pixels from the pen.
    pub x0: i8,
    pub y0: i8,
    /// How far the pen moves on after the letter_shape, in pixels.
    pub step_along_x: u8,
    pub texel_span: u8,
    pub raster_rows: u8,
    /// The letter_shape's corners on the sheet, as fractions of its width and height.
    pub s0: f32,
    pub t0: f32,
    pub s1: f32,
    pub t1: f32,
}

/// A font: its sheet's material and every letter_shape on it.
#[derive(Clone, Debug, Default)]
pub struct TypefaceSpec {
    pub name: String,
    /// Height of a line of text in pixels.
    pub raster_rows: i32,
    /// The material the sheet is drawn with.
    pub mat_name: String,
    /// The ASCII slots first, then every other letter sorted by character.
    pub glyphs: Vec<FontGlyph>,
}

impl TypefaceSpec {
    /// The letter_shape that draws `letter`. A letter the font lacks comes back as
    /// the missing-letter letter_shape; an ASCII letter in a font too short to hold
    /// it comes back as `None`.
    pub fn letter_shape(&self, letter: u32) -> Option<&FontGlyph> {
        if let Some(slot) = letter.checked_sub(ASCII_FIRST) {
            if (slot as usize) < ASCII_SLOTS {
                return self.glyphs.get(slot as usize);
            }
        }
        let beyond_ascii = self.glyphs.get(ASCII_SLOTS..).unwrap_or_default();
        match beyond_ascii.binary_search_by_key(&letter, |letter_shape| u32::from(letter_shape.glyph_char)) {
            Ok(found) => beyond_ascii.get(found),
            Err(_) => self.glyphs.get(MISSING_LETTER_SLOT),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn letter_shape(letter: u16) -> FontGlyph {
        FontGlyph {
            glyph_char: letter,
            x0: 0,
            y0: 0,
            step_along_x: 1,
            texel_span: 1,
            raster_rows: 1,
            s0: 0.0,
            t0: 0.0,
            s1: 1.0,
            t1: 1.0,
        }
    }

    fn font_with_extras(extras: &[u16]) -> TypefaceSpec {
        let mut glyphs: Vec<FontGlyph> = (0..ASCII_SLOTS as u16).map(|n| letter_shape(0x20 + n)).collect();
        glyphs.extend(extras.iter().map(|&letter| letter_shape(letter)));
        TypefaceSpec {
            glyphs,
            ..TypefaceSpec::default()
        }
    }

    #[test]
    fn ascii_is_found_by_position() {
        let font = font_with_extras(&[]);
        assert_eq!(font.letter_shape(u32::from(b'A')).map(|g| g.glyph_char), Some(0x41));
        assert_eq!(font.letter_shape(0x20).map(|g| g.glyph_char), Some(0x20));
    }

    #[test]
    fn other_letters_are_searched_by_character() {
        let font = font_with_extras(&[0xa9, 0xe9, 0x2022]);
        assert_eq!(font.letter_shape(0xe9).map(|g| g.glyph_char), Some(0xe9));
        assert_eq!(font.letter_shape(0x2022).map(|g| g.glyph_char), Some(0x2022));
    }

    #[test]
    fn an_unknown_letter_draws_as_the_missing_glyph() {
        let font = font_with_extras(&[0xa9]);
        assert_eq!(font.letter_shape(0x4e2d).map(|g| g.glyph_char), Some(0x2e));
        assert_eq!(font.letter_shape(0x1f).map(|g| g.glyph_char), Some(0x2e));
    }

    #[test]
    fn a_font_without_glyphs_has_nothing_to_draw() {
        assert!(TypefaceSpec::default().letter_shape(u32::from(b'A')).is_none());
        assert!(TypefaceSpec::default().letter_shape(0x4e2d).is_none());
    }
}
