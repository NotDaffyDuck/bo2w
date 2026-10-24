//! The screen-wide colour grading a map's vision file asks for.

/// Colour grading and glow applied over the finished frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FilmVision {
    pub enable: bool,
    pub contrast: f32,
    pub brightness: f32,
    /// How much colour is drained from bright parts, 0 to 1.
    pub desaturation: f32,
    /// How much colour is drained from dark parts, 0 to 1.
    pub desaturation_dark: f32,
    pub invert: bool,
    /// Multiplied into the bright, middle and dark thirds of the picture.
    pub light_tint: [f32; 3],
    pub medium_tint: [f32; 3],
    pub dark_tint: [f32; 3],
    pub glow_enable: bool,
    pub glow_radius: f32,
    /// Brightness above which a pixel starts to bloom.
    pub glow_bloom_cutoff: f32,
    pub glow_bloom_desaturation: f32,
    pub glow_bloom_intensity: f32,
}

impl FilmVision {
    /// Grading that leaves the picture as rendered.
    const NEUTRAL: Self = Self {
        enable: false,
        contrast: 1.0,
        brightness: 0.0,
        desaturation: 0.0,
        desaturation_dark: 0.0,
        invert: false,
        light_tint: [1.0; 3],
        medium_tint: [1.0; 3],
        dark_tint: [1.0; 3],
        glow_enable: false,
        glow_radius: 0.0,
        glow_bloom_cutoff: 0.0,
        glow_bloom_desaturation: 0.0,
        glow_bloom_intensity: 0.0,
    };
}

impl Default for FilmVision {
    fn default() -> Self {
        Self::NEUTRAL
    }
}

/// Why a vision file could not be turned into a [`FilmVision`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FilmVisionParseError {
    /// The file's bytes could not be read as text.
    Decode,
    /// A number field did not parse; names the field.
    InvalidScalar(&'static str),
    /// A colour or vector field did not parse; names the field.
    InvalidVector(&'static str),
    /// A field the grading needs was absent; names the field.
    MissingField(&'static str),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_grading_changes_nothing() {
        let vision = FilmVision::default();
        assert!(!vision.enable && !vision.glow_enable && !vision.invert);
        assert_eq!(vision.contrast, 1.0);
        assert_eq!(vision.light_tint, [1.0; 3]);
    }
}
