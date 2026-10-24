//! Glow (bloom) settings: which values a view uses and what the post-effect
//! shader constants come to.

use crate::scene_light::color_srgb_to_linear;

/// Shader constant slot that receives [`GlowBloomConsts::setup`].
pub const CONST_SRC_CODE_GLOW_SETUP: u16 = 0x2C;
/// Shader constant slot that receives [`GlowBloomConsts::apply`].
pub const CONST_SRC_CODE_GLOW_APPLY: u16 = 0x2D;

/// Glow is on unless the `r_glow` setting is switched off.
pub const R_GLOW_DEFAULT: bool = true;
/// `r_fullbright` is off by default.
pub const R_FULLBRIGHT_DEFAULT: bool = false;
/// A view uses its map's glow values unless the tweak setting is switched on.
pub const R_GLOW_USE_TWEAKS_DEFAULT: bool = false;

const TWEAK_ENABLE_DEFAULT: bool = false;
const TWEAK_RADIUS_DEFAULT: f32 = 5.0;
const TWEAK_INTENSITY_DEFAULT: f32 = 1.0;
const TWEAK_CUTOFF_DEFAULT: f32 = 0.5;

/// The two shader constant rows glow needs.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GlowBloomConsts {
    pub setup: [f32; 4],
    pub apply: [f32; 4],
}

/// One set of glow values: what a map authors, or what the tweak settings hold.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GlowViewInfo {
    pub enable: bool,
    pub cutoff: f32,
    pub desaturation: f32,
    pub intensity: f32,
    pub radius: f32,
}

impl GlowViewInfo {
    /// What the tweak settings hold before anyone changes them.
    pub const fn tweak_register_defaults() -> Self {
        Self {
            enable: TWEAK_ENABLE_DEFAULT,
            cutoff: TWEAK_CUTOFF_DEFAULT,
            desaturation: 0.0,
            intensity: TWEAK_INTENSITY_DEFAULT,
            radius: TWEAK_RADIUS_DEFAULT,
        }
    }
}

/// The tweaks when they are in use, the map's own values otherwise.
pub fn select_glow_view_info(
    authored: GlowViewInfo,
    use_tweaks: bool,
    tweaks: GlowViewInfo,
) -> GlowViewInfo {
    if use_tweaks { tweaks } else { authored }
}

/// Whether the glow pass runs at all this frame: it needs to be enabled and
/// visible (some intensity and some radius), and it never runs in fullbright.
pub fn using_glow(
    enable: bool,
    intensity: f32,
    radius: f32,
    r_glow: bool,
    r_fullbright: bool,
) -> bool {
    enable && !r_fullbright && r_glow && intensity != 0.0 && radius != 0.0
}

/// The constants for a glow with the given cutoff, desaturation and
/// intensity; `None` when the intensity is zero, which turns glow off.
///
/// The setup row carries the cutoff in linear light plus the factor that
/// stretches what is left above it back up to full range.
pub fn set_glow_info(cutoff: f32, desaturation: f32, intensity: f32) -> Option<GlowBloomConsts> {
    if intensity == 0.0 {
        return None;
    }
    let linear_cutoff = color_srgb_to_linear(cutoff);
    Some(GlowBloomConsts {
        setup: [
            linear_cutoff,
            1.0 / (1.0 - linear_cutoff),
            0.0,
            desaturation,
        ],
        apply: [0.0, 0.0, 0.0, intensity],
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn authored() -> GlowViewInfo {
        GlowViewInfo {
            enable: true,
            cutoff: 0.8,
            desaturation: 0.25,
            intensity: 2.0,
            radius: 9.0,
        }
    }

    #[test]
    fn the_tweaks_start_off_with_a_modest_glow() {
        let tweaks = GlowViewInfo::tweak_register_defaults();
        assert!(!tweaks.enable);
        assert_eq!(tweaks.cutoff, 0.5);
        assert_eq!(tweaks.desaturation, 0.0);
        assert_eq!(tweaks.intensity, 1.0);
        assert_eq!(tweaks.radius, 5.0);
    }

    #[test]
    fn the_map_values_win_unless_tweaks_are_on() {
        let tweaks = GlowViewInfo::tweak_register_defaults();
        assert_eq!(select_glow_view_info(authored(), false, tweaks), authored());
        assert_eq!(select_glow_view_info(authored(), true, tweaks), tweaks);
    }

    #[test]
    fn glow_runs_only_when_everything_allows_it() {
        assert!(using_glow(true, 1.0, 5.0, true, false));
        assert!(!using_glow(false, 1.0, 5.0, true, false));
        assert!(!using_glow(true, 0.0, 5.0, true, false));
        assert!(!using_glow(true, 1.0, 0.0, true, false));
        assert!(!using_glow(true, 1.0, 5.0, false, false));
        assert!(!using_glow(true, 1.0, 5.0, true, true));
    }

    #[test]
    fn the_defaults_leave_glow_available() {
        assert!(R_GLOW_DEFAULT);
        assert!(!R_FULLBRIGHT_DEFAULT);
        assert!(!R_GLOW_USE_TWEAKS_DEFAULT);
    }

    #[test]
    fn zero_intensity_gives_no_constants() {
        assert_eq!(set_glow_info(0.5, 0.0, 0.0), None);
    }

    #[test]
    fn the_apply_row_holds_the_intensity_in_its_last_slot() {
        let consts = set_glow_info(0.5, 0.3, 4.0).unwrap();
        assert_eq!(consts.apply, [0.0, 0.0, 0.0, 4.0]);
    }

    #[test]
    fn the_setup_row_holds_the_linear_cutoff_and_its_rescale() {
        let consts = set_glow_info(0.5, 0.3, 4.0).unwrap();
        let linear = color_srgb_to_linear(0.5);
        assert_eq!(consts.setup[0], linear);
        assert_eq!(consts.setup[1], 1.0 / (1.0 - linear));
        assert_eq!(consts.setup[2], 0.0);
        assert_eq!(consts.setup[3], 0.3);
    }

    #[test]
    fn a_zero_cutoff_keeps_the_whole_range() {
        let consts = set_glow_info(0.0, 0.0, 1.0).unwrap();
        assert_eq!(consts.setup[0], 0.0);
        assert_eq!(consts.setup[1], 1.0);
    }

    #[test]
    fn the_constant_slots_are_adjacent() {
        assert_eq!(CONST_SRC_CODE_GLOW_APPLY, CONST_SRC_CODE_GLOW_SETUP + 1);
    }
}
