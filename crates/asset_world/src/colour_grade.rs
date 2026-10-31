//! The screen-wide colour grading a map's vision file asks for.

/// Colour grading and glow applied over the finished frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ScreenGradeLook {
    pub enable: bool,
    pub contrast: f32,
    pub brightness: f32,
    /// How much colour is drained from bright parts, 0 to 1.
    pub color_drain: f32,
    /// How much colour is drained from dark parts, 0 to 1.
    pub fade_to_grey_shadow: f32,
    pub flip_tones: bool,
    /// Multiplied into the bright, middle and dark thirds of the picture.
    pub lit_tint: [f32; 3],
    pub midtone_hue: [f32; 3],
    pub shadow_hue: [f32; 3],
    pub bloom_enable: bool,
    pub bloom_radius: f32,
    /// Brightness above which a pixel starts to bloom.
    pub bloom_cutoff: f32,
    pub bloom_desaturation: f32,
    pub bloom_intensity: f32,
}

impl ScreenGradeLook {
    /// Grading that leaves the picture as rendered.
    const NEUTRAL: Self = {
        const OFF: bool = false;
        const NONE: f32 = 0.0;
        const UNIT_TINT: [f32; 3] = [1.0; 3];
        Self {
            enable: OFF,
            flip_tones: OFF,
            bloom_enable: OFF,
            contrast: 1.0,
            brightness: NONE,
            color_drain: NONE,
            fade_to_grey_shadow: NONE,
            lit_tint: UNIT_TINT,
            midtone_hue: UNIT_TINT,
            shadow_hue: UNIT_TINT,
            bloom_radius: NONE,
            bloom_cutoff: NONE,
            bloom_desaturation: NONE,
            bloom_intensity: NONE,
        }
    };
}

impl Default for ScreenGradeLook {
    fn default() -> Self {
        let neutral = Self::NEUTRAL;
        neutral
    }
}

/// Why a vision file could not be turned into a [`ScreenGradeLook`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GradeVisionDecodeFault {
    /// The file's bytes could not be read as text.
    Decode,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_grading_changes_nothing() {
        let vision = ScreenGradeLook::default();
        assert!(!vision.enable && !vision.bloom_enable && !vision.flip_tones);
        assert_eq!(vision.contrast, 1.0);
        assert_eq!(vision.lit_tint, [1.0; 3]);
    }
}
