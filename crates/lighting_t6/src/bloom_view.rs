//! Glow (bloom) settings: which values a view uses and what the post-effect
//! shader constants come to.

use crate::stage_light::gamma_encoded_to_linear;

/// Shader constant slot that receives [`BloomShaderConstants::prepare`].
pub const ENGINE_CONST_BLOOM_PREPARE: u16 = 0x2C;
/// Shader constant slot that receives [`BloomShaderConstants::compose`].
pub const ENGINE_CONST_BLOOM_COMPOSE: u16 = 0x2D;

/// Glow is on unless the `r_glow` setting is switched off.
pub const BLOOM_INITIAL_ON: bool = true;
/// `r_fullbright` is off by default.
pub const UNSHADED_MODE_INITIAL: bool = false;
/// A view uses its map's glow values unless the tweak setting is switched on.
pub const BLOOM_CUSTOM_SETTINGS_INITIAL: bool = false;

const TWEAK_ENABLE_DEFAULT: bool = false;
const TWEAK_RADIUS_DEFAULT: f32 = 5.0;
const TWEAK_INTENSITY_DEFAULT: f32 = 1.0;
const TWEAK_CUTOFF_DEFAULT: f32 = 0.5;

/// The two shader constant rows glow needs.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BloomShaderConstants {
    pub prepare: [f32; 4],
    pub compose: [f32; 4],
}

/// One set of glow values: what a map authors, or what the tweak settings hold.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GlowSettings {
    pub on: bool,
    pub glow_floor: f32,
    pub color_drain: f32,
    pub strength: f32,
    pub radius: f32,
}

impl GlowSettings {
    /// What the tweak settings hold before anyone changes them.
    pub const fn tweak_register_defaults() -> Self {
        Self {
            on: TWEAK_ENABLE_DEFAULT,
            glow_floor: TWEAK_CUTOFF_DEFAULT,
            color_drain: 0.0,
            strength: TWEAK_INTENSITY_DEFAULT,
            radius: TWEAK_RADIUS_DEFAULT,
        }
    }
}

/// The tweaks when they are in use, the map's own values otherwise.
pub fn select_glow_view_info(
    authored: GlowSettings,
    use_trims: bool,
    tweaks: GlowSettings,
) -> GlowSettings {
    if use_trims { tweaks } else { authored }
}

/// Whether the glow pass runs at all this frame: it needs to be enabled and
/// visible (some intensity and some radius), and it never runs in fullbright.
pub fn using_glow(wanted: bool, strength: f32, spread: f32, glow_on: bool, fullbright: bool) -> bool {
    let visible = strength != 0.0 && spread != 0.0;
    wanted && !fullbright && glow_on && visible
}

/// The constants for a glow with the given cutoff, color_drain and
/// intensity; `None` when the intensity is zero, which turns glow off.
///
/// The setup row carries the cutoff in linear light plus the factor that
/// stretches what is left above it back up to full range.
pub fn set_glow_info(cutoff: f32, color_drain: f32, strength: f32) -> Option<BloomShaderConstants> {
    (strength != 0.0).then(|| {
        let floor = gamma_encoded_to_linear(cutoff);
        let stretch = 1.0 / (1.0 - floor);
        BloomShaderConstants { prepare: [floor, stretch, 0.0, color_drain], compose: [0.0, 0.0, 0.0, strength] }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn authored() -> GlowSettings {
        GlowSettings { on: true, glow_floor: 0.8, color_drain: 0.25, strength: 2.0, radius: 9.0 }
    }

    #[test]
    fn the_tweaks_start_off_with_a_modest_glow() {
        let tweaks = GlowSettings::tweak_register_defaults();
        assert!(!tweaks.on);
        assert_eq!(tweaks.glow_floor, 0.5);
        assert_eq!(tweaks.color_drain, 0.0);
        assert_eq!(tweaks.strength, 1.0);
        assert_eq!(tweaks.radius, 5.0);
    }

    #[test]
    fn the_map_values_win_unless_tweaks_are_on() {
        let tweaks = GlowSettings::tweak_register_defaults();
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
        assert!(BLOOM_INITIAL_ON);
        assert!(!UNSHADED_MODE_INITIAL);
        assert!(!BLOOM_CUSTOM_SETTINGS_INITIAL);
    }

    #[test]
    fn zero_intensity_gives_no_constants() {
        assert_eq!(set_glow_info(0.5, 0.0, 0.0), None);
    }

    #[test]
    fn the_apply_row_holds_the_intensity_in_its_last_slot() {
        let consts = set_glow_info(0.5, 0.3, 4.0).unwrap();
        assert_eq!(consts.compose, [0.0, 0.0, 0.0, 4.0]);
    }

    #[test]
    fn the_setup_row_holds_the_linear_cutoff_and_its_rescale() {
        let consts = set_glow_info(0.5, 0.3, 4.0).unwrap();
        let linear = gamma_encoded_to_linear(0.5);
        assert_eq!(consts.prepare[0], linear);
        assert_eq!(consts.prepare[1], 1.0 / (1.0 - linear));
        assert_eq!(consts.prepare[2], 0.0);
        assert_eq!(consts.prepare[3], 0.3);
    }

    #[test]
    fn a_zero_cutoff_keeps_the_whole_range() {
        let consts = set_glow_info(0.0, 0.0, 1.0).unwrap();
        assert_eq!(consts.prepare[0], 0.0);
        assert_eq!(consts.prepare[1], 1.0);
    }

    #[test]
    fn the_constant_slots_are_adjacent() {
        assert_eq!(ENGINE_CONST_BLOOM_COMPOSE, ENGINE_CONST_BLOOM_PREPARE + 1);
    }
}
