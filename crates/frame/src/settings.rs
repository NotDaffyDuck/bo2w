//! The player's options, as one resource, with the limits every value must
//! stay inside.

use std::fmt;

use bevy::prelude::Resource;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DisplayResolution {
    pub width: u32,
    pub height: u32,
}

impl DisplayResolution {
    pub const HD: Self = Self::new(1280, 720);

    pub const fn new(width: u32, height: u32) -> Self {
        Self { width, height }
    }
}

impl fmt::Display for DisplayResolution {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}x{}", self.width, self.height)
    }
}

/// `value` limited to `lo..=hi`; `fallback` when it is NaN or infinite.
fn clamp_or(value: f32, lo: f32, hi: f32, fallback: f32) -> f32 {
    if value.is_finite() {
        value.clamp(lo, hi)
    } else {
        fallback
    }
}

#[derive(Resource, Clone, Debug, PartialEq)]
pub struct GameSettings {
    pub resolution: DisplayResolution,
    pub fullscreen: bool,
    pub vsync: bool,
    pub fov: f32,
    pub master_volume: f32,
    /// The other volume sliders each scale one menu sound category, under the
    /// master volume.
    pub voice_volume: f32,
    pub music_volume: f32,
    pub sfx_volume: f32,
    pub cinematic_volume: f32,
    /// Index of the audio mix preset: 0 stock mix, 1 bass boost, 2 high boost,
    /// 3 super crunch, 4 headphones.
    pub audio_preset: u8,
    pub brightness: f32,
    pub shadows: bool,
    pub depth_of_field: bool,
    pub bloom: bool,
    pub sensitivity: f32,
    pub invert_mouse: bool,
    pub player_name: String,

    pub pad_layout: u8,
    pub pad_stick_layout: u8,
    pub pad_sensitivity: f32,
    pub pad_ads_sensitivity: f32,
    pub pad_invert: bool,
    pub pad_curve: u8,
    pub pad_aim_assist: u8,
    pub pad_prompts: u8,
    pub pad_vibration: bool,
    pub pad_deadzone_left: f32,
    pub pad_deadzone_right: f32,

    /// Reduced-detail map rendering: half-size textures, a single ground
    /// layer, props hidden sooner. Off means full quality.
    pub bo2_fast: bool,
    /// Sprint in any direction rather than forward only.
    pub omni_movement: bool,

    /// Bumped by [`touch`](Self::touch) so readers can notice a change.
    pub revision: u64,
}

impl Default for GameSettings {
    fn default() -> Self {
        Self {
            resolution: DisplayResolution::HD,
            fullscreen: false,
            vsync: true,
            fov: Self::FOV_DEFAULT,
            master_volume: 1.0,
            voice_volume: 1.0,
            music_volume: 1.0,
            sfx_volume: 1.0,
            cinematic_volume: 1.0,
            audio_preset: 0,
            brightness: 0.0,
            shadows: true,
            depth_of_field: true,
            bloom: true,
            sensitivity: 5.0,
            invert_mouse: false,
            player_name: Self::NAME_DEFAULT.to_owned(),
            pad_layout: 0,
            pad_stick_layout: 0,
            pad_sensitivity: Self::PAD_SENSITIVITY_DEFAULT,
            pad_ads_sensitivity: 1.0,
            pad_invert: false,
            pad_curve: 0,
            pad_aim_assist: 0,
            pad_prompts: 0,
            pad_vibration: true,
            pad_deadzone_left: Self::DEADZONE_DEFAULT,
            pad_deadzone_right: Self::DEADZONE_DEFAULT,
            bo2_fast: false,
            omni_movement: false,
            revision: 0,
        }
    }
}

impl GameSettings {
    pub const FOV_DEFAULT: f32 = 65.0;
    pub const FOV_MIN: f32 = 65.0;
    pub const FOV_MAX: f32 = 120.0;
    pub const PAD_SENSITIVITY_DEFAULT: f32 = 3.0;
    /// Marks a hand-built button layout, outside the numbered presets.
    pub const PAD_LAYOUT_CUSTOM: u8 = 255;

    const NAME_DEFAULT: &'static str = "Player";
    const NAME_LEN_MAX: usize = 16;
    const DEADZONE_DEFAULT: f32 = 0.12;
    const BRIGHTNESS_LIMIT: f32 = 0.2;

    /// The gain a menu sound category takes from its slider: the slider
    /// squared. Categories are 0 effects, 1 music, 2 voice, 3 interface and
    /// 4 cinematic; interface sounds follow the effects slider. Sounds with
    /// no category, or an unknown one, are not scaled.
    pub fn menu_volume(&self, category: Option<u8>) -> f32 {
        let slider = match category {
            Some(0 | 3) => self.sfx_volume,
            Some(1) => self.music_volume,
            Some(2) => self.voice_volume,
            Some(4) => self.cinematic_volume,
            _ => return 1.0,
        };
        slider * slider
    }

    /// The gain every sound takes from the master slider: the slider squared.
    pub fn master_gain(&self) -> f32 {
        self.master_volume * self.master_volume
    }

    pub fn touch(&mut self) {
        self.revision = self.revision.wrapping_add(1);
    }

    /// Pulls every value back inside its limits. Anything that is not a
    /// finite number takes its default.
    pub fn sanitize(&mut self) {
        self.resolution.width = self.resolution.width.clamp(640, 7680);
        self.resolution.height = self.resolution.height.clamp(480, 4320);
        self.fov = clamp_or(self.fov, Self::FOV_MIN, Self::FOV_MAX, Self::FOV_DEFAULT);
        self.brightness = clamp_or(
            self.brightness,
            -Self::BRIGHTNESS_LIMIT,
            Self::BRIGHTNESS_LIMIT,
            0.0,
        );
        self.master_volume = self.master_volume.clamp(0.0, 1.0);
        for slider in [
            &mut self.voice_volume,
            &mut self.music_volume,
            &mut self.sfx_volume,
            &mut self.cinematic_volume,
        ] {
            *slider = clamp_or(*slider, 0.0, 1.0, 1.0);
        }
        self.audio_preset = self.audio_preset.min(4);
        self.sensitivity = self.sensitivity.clamp(0.1, 30.0);

        if self.pad_layout != Self::PAD_LAYOUT_CUSTOM {
            self.pad_layout = self.pad_layout.min(4);
        }
        self.pad_stick_layout = self.pad_stick_layout.min(3);
        self.pad_curve = self.pad_curve.min(2);
        self.pad_aim_assist = 0;
        self.pad_prompts = self.pad_prompts.min(3);
        self.pad_sensitivity = clamp_or(
            self.pad_sensitivity,
            1.0,
            10.0,
            Self::PAD_SENSITIVITY_DEFAULT,
        );
        self.pad_ads_sensitivity = clamp_or(self.pad_ads_sensitivity, 0.5, 1.5, 1.0);
        self.pad_deadzone_left = clamp_or(self.pad_deadzone_left, 0.0, 0.4, Self::DEADZONE_DEFAULT);
        self.pad_deadzone_right =
            clamp_or(self.pad_deadzone_right, 0.0, 0.4, Self::DEADZONE_DEFAULT);

        let trimmed = self.player_name.trim();
        self.player_name = if trimmed.is_empty() {
            Self::NAME_DEFAULT.to_owned()
        } else {
            trimmed.chars().take(Self::NAME_LEN_MAX).collect()
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sanitized(edit: impl FnOnce(&mut GameSettings)) -> GameSettings {
        let mut settings = GameSettings::default();
        edit(&mut settings);
        settings.sanitize();
        settings
    }

    #[test]
    fn resolution_prints_as_width_x_height() {
        assert_eq!(DisplayResolution::HD.to_string(), "1280x720");
        assert_eq!(DisplayResolution::new(1920, 1080).to_string(), "1920x1080");
    }

    #[test]
    fn defaults_are_already_inside_every_limit() {
        let defaults = GameSettings::default();
        let mut cleaned = defaults.clone();
        cleaned.sanitize();
        assert_eq!(cleaned, defaults);
    }

    #[test]
    fn touching_counts_up_and_wraps() {
        let mut settings = GameSettings::default();
        settings.touch();
        settings.touch();
        assert_eq!(settings.revision, 2);
        settings.revision = u64::MAX;
        settings.touch();
        assert_eq!(settings.revision, 0);
    }

    #[test]
    fn sliders_give_their_square() {
        let settings = GameSettings {
            master_volume: 0.5,
            sfx_volume: 0.5,
            music_volume: 0.25,
            voice_volume: 1.0,
            cinematic_volume: 0.0,
            ..GameSettings::default()
        };
        assert_eq!(settings.master_gain(), 0.25);
        assert_eq!(settings.menu_volume(Some(0)), 0.25);
        assert_eq!(settings.menu_volume(Some(1)), 0.0625);
        assert_eq!(settings.menu_volume(Some(2)), 1.0);
        assert_eq!(settings.menu_volume(Some(4)), 0.0);
    }

    #[test]
    fn interface_sounds_follow_the_effects_slider() {
        let settings = GameSettings {
            sfx_volume: 0.5,
            ..GameSettings::default()
        };
        assert_eq!(settings.menu_volume(Some(3)), settings.menu_volume(Some(0)));
    }

    #[test]
    fn uncategorised_sounds_are_not_scaled() {
        let settings = GameSettings {
            sfx_volume: 0.1,
            music_volume: 0.1,
            ..GameSettings::default()
        };
        assert_eq!(settings.menu_volume(None), 1.0);
        assert_eq!(settings.menu_volume(Some(5)), 1.0);
        assert_eq!(settings.menu_volume(Some(255)), 1.0);
    }

    #[test]
    fn resolution_is_held_between_the_smallest_and_largest_supported() {
        let small = sanitized(|s| s.resolution = DisplayResolution::new(10, 10));
        assert_eq!(small.resolution, DisplayResolution::new(640, 480));
        let huge = sanitized(|s| s.resolution = DisplayResolution::new(100_000, 100_000));
        assert_eq!(huge.resolution, DisplayResolution::new(7680, 4320));
    }

    #[test]
    fn fov_and_brightness_are_clamped_and_nan_takes_the_default() {
        assert_eq!(sanitized(|s| s.fov = 10.0).fov, GameSettings::FOV_MIN);
        assert_eq!(sanitized(|s| s.fov = 500.0).fov, GameSettings::FOV_MAX);
        assert_eq!(
            sanitized(|s| s.fov = f32::NAN).fov,
            GameSettings::FOV_DEFAULT
        );
        assert_eq!(sanitized(|s| s.brightness = 3.0).brightness, 0.2);
        assert_eq!(sanitized(|s| s.brightness = -3.0).brightness, -0.2);
        assert_eq!(sanitized(|s| s.brightness = f32::INFINITY).brightness, 0.0);
    }

    #[test]
    fn volumes_stay_between_silent_and_full() {
        let loud = sanitized(|s| {
            s.master_volume = 4.0;
            s.voice_volume = 9.0;
            s.music_volume = -1.0;
            s.sfx_volume = f32::NAN;
            s.cinematic_volume = f32::NEG_INFINITY;
        });
        assert_eq!(loud.master_volume, 1.0);
        assert_eq!(loud.voice_volume, 1.0);
        assert_eq!(loud.music_volume, 0.0);
        assert_eq!(loud.sfx_volume, 1.0);
        assert_eq!(loud.cinematic_volume, 1.0);
    }

    #[test]
    fn small_enumerations_are_capped() {
        let capped = sanitized(|s| {
            s.audio_preset = 200;
            s.pad_stick_layout = 200;
            s.pad_curve = 200;
            s.pad_prompts = 200;
            s.pad_layout = 100;
        });
        assert_eq!(capped.audio_preset, 4);
        assert_eq!(capped.pad_stick_layout, 3);
        assert_eq!(capped.pad_curve, 2);
        assert_eq!(capped.pad_prompts, 3);
        assert_eq!(capped.pad_layout, 4);
    }

    #[test]
    fn the_custom_pad_layout_marker_survives() {
        let custom = sanitized(|s| s.pad_layout = GameSettings::PAD_LAYOUT_CUSTOM);
        assert_eq!(custom.pad_layout, GameSettings::PAD_LAYOUT_CUSTOM);
    }

    #[test]
    fn aim_assist_is_always_forced_off() {
        assert_eq!(sanitized(|s| s.pad_aim_assist = 2).pad_aim_assist, 0);
    }

    #[test]
    fn pad_tuning_is_clamped_with_per_field_fallbacks() {
        let tuned = sanitized(|s| {
            s.pad_sensitivity = 99.0;
            s.pad_ads_sensitivity = 0.0;
            s.pad_deadzone_left = 5.0;
            s.pad_deadzone_right = f32::NAN;
        });
        assert_eq!(tuned.pad_sensitivity, 10.0);
        assert_eq!(tuned.pad_ads_sensitivity, 0.5);
        assert_eq!(tuned.pad_deadzone_left, 0.4);
        assert_eq!(tuned.pad_deadzone_right, 0.12);

        let broken = sanitized(|s| {
            s.pad_sensitivity = f32::NAN;
            s.pad_ads_sensitivity = f32::NAN;
        });
        assert_eq!(
            broken.pad_sensitivity,
            GameSettings::PAD_SENSITIVITY_DEFAULT
        );
        assert_eq!(broken.pad_ads_sensitivity, 1.0);
    }

    #[test]
    fn mouse_sensitivity_has_a_floor_and_a_ceiling() {
        assert_eq!(sanitized(|s| s.sensitivity = 0.0).sensitivity, 0.1);
        assert_eq!(sanitized(|s| s.sensitivity = 1000.0).sensitivity, 30.0);
    }

    #[test]
    fn names_are_trimmed_cut_to_sixteen_characters_and_never_empty() {
        assert_eq!(
            sanitized(|s| s.player_name = "  Ada  ".into()).player_name,
            "Ada"
        );
        assert_eq!(
            sanitized(|s| s.player_name = "   ".into()).player_name,
            "Player"
        );
        assert_eq!(
            sanitized(|s| s.player_name = String::new()).player_name,
            "Player"
        );
        let long = sanitized(|s| s.player_name = "abcdefghijklmnopqrstuvwxyz".into());
        assert_eq!(long.player_name, "abcdefghijklmnop");
    }

    #[test]
    fn the_name_limit_counts_characters_not_bytes() {
        let wide = "é".repeat(20);
        let cut = sanitized(|s| s.player_name = wide);
        assert_eq!(cut.player_name.chars().count(), 16);
    }
}
