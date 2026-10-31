//! The player's options, as one resource, with the limits every value must
//! stay inside.

use std::fmt;

use bevy::prelude::Resource;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ScreenPixelSize {
    pub width: u32,
    pub height: u32,
}

impl ScreenPixelSize {
    pub const HD: Self = Self::new(1280, 720);

    pub const fn new(width: u32, height: u32) -> Self {
        Self { width, height }
    }
}

impl fmt::Display for ScreenPixelSize {
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
pub struct PlayerPrefsBundle {
    pub resolution: ScreenPixelSize,
    pub full_screen: bool,
    pub sync_frames: bool,
    pub view_angle: f32,
    pub main_level: f32,
    /// The other volume sliders each scale one menu sound category, under the
    /// master volume (music, effects, voices, cinematics; 0..1).
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
    pub flip_look: bool,
    pub shown_name: String,

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

    /// bo2zm: more of Black Ops II's Settings rows: mature content (heads
    /// and limbs come off), how many corpses stay (3, 5, 10, 16), the fps
    /// counter, the frame cap (0 = none; only without sync), FXAA, the
    /// anti-aliasing row (1 = off), texture filtering (0 low .. 2 high) and
    /// texture quality (-1 automatic, 3 low .. 0 extra; low = `bo2_fast`).
    pub mature: bool,
    pub corpses: u8,
    pub draw_fps: bool,
    pub max_fps: u16,
    pub fxaa: bool,
    pub aa_samples: u8,
    pub tex_filter: u8,
    pub tex_quality: i8,

    /// Bumped by [`touch`](Self::touch) so readers can notice a change.
    pub revision: u64,
}

impl Default for PlayerPrefsBundle {
    fn default() -> Self {
        Self {
            resolution: ScreenPixelSize::HD,
            full_screen: Self::FULLSCREEN_DEFAULT,
            sync_frames: Self::VSYNC_DEFAULT,
            view_angle: Self::FOV_DEFAULT,
            main_level: Self::VOLUME_FULL,
            voice_volume: 1.0,
            music_volume: 1.0,
            sfx_volume: 1.0,
            cinematic_volume: 1.0,
            audio_preset: 0,
            brightness: Self::BRIGHTNESS_DEFAULT,
            shadows: true,
            depth_of_field: true,
            bloom: true,
            sensitivity: Self::SENSITIVITY_DEFAULT,
            flip_look: Self::INVERT_DEFAULT,
            shown_name: Self::NAME_DEFAULT.to_owned(),
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
            mature: true,
            corpses: 16,
            draw_fps: false,
            max_fps: 0,
            fxaa: false,
            aa_samples: 1,
            tex_filter: 2,
            tex_quality: -1,
            omni_movement: false,
            revision: u64::MIN,
        }
    }
}

impl PlayerPrefsBundle {
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
    const BRIGHTNESS_DEFAULT: f32 = 0.0;
    const SENSITIVITY_DEFAULT: f32 = 5.0;
    const VOLUME_FULL: f32 = 1.0;
    const FULLSCREEN_DEFAULT: bool = false;
    const VSYNC_DEFAULT: bool = true;
    const INVERT_DEFAULT: bool = false;

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
        self.main_level * self.main_level
    }

    pub fn touch(&mut self) {
        self.revision = u64::wrapping_add(self.revision, 1);
    }

    /// Pulls every value back inside its limits. Anything that is not a
    /// finite number takes its default.
    pub fn pull_into_limits(&mut self) {
        self.resolution.width = u32::clamp(self.resolution.width, 640, 7680);
        self.resolution.height = u32::clamp(self.resolution.height, 480, 4320);
        self.view_angle = clamp_or(self.view_angle, Self::FOV_MIN, Self::FOV_MAX, Self::FOV_DEFAULT);
        self.brightness = clamp_or(
            self.brightness,
            -Self::BRIGHTNESS_LIMIT,
            Self::BRIGHTNESS_LIMIT,
            0.0,
        );
        self.main_level = f32::clamp(self.main_level, 0.0, 1.0);
        for slider in [
            &mut self.voice_volume,
            &mut self.music_volume,
            &mut self.sfx_volume,
            &mut self.cinematic_volume,
        ] {
            *slider = clamp_or(*slider, 0.0, 1.0, 1.0);
        }
        self.audio_preset = self.audio_preset.min(4);
        self.sensitivity = f32::clamp(self.sensitivity, 0.1, 30.0);

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
        if ![3, 5, 10, 16].contains(&self.corpses) {
            self.corpses = 16;
        }
        self.max_fps = self.max_fps.min(1000);
        if ![1, 2, 4, 8, 16, 17, 18].contains(&self.aa_samples) {
            self.aa_samples = 1;
        }
        self.tex_filter = self.tex_filter.min(2);
        self.tex_quality = self.tex_quality.clamp(-1, 3);
        self.pad_deadzone_left = clamp_or(self.pad_deadzone_left, 0.0, 0.4, Self::DEADZONE_DEFAULT);
        self.pad_deadzone_right =
            clamp_or(self.pad_deadzone_right, 0.0, 0.4, Self::DEADZONE_DEFAULT);

        let trimmed = self.shown_name.trim();
        self.shown_name = if trimmed.is_empty() {
            Self::NAME_DEFAULT.to_owned()
        } else {
            trimmed.chars().take(Self::NAME_LEN_MAX).collect()
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sanitized(edit: impl FnOnce(&mut PlayerPrefsBundle)) -> PlayerPrefsBundle {
        let mut prefs = PlayerPrefsBundle::default();
        edit(&mut prefs);
        prefs.pull_into_limits();
        prefs
    }

    #[test]
    fn resolution_prints_as_width_x_height() {
        assert_eq!(ScreenPixelSize::HD.to_string(), "1280x720");
        assert_eq!(ScreenPixelSize::new(1920, 1080).to_string(), "1920x1080");
    }

    #[test]
    fn defaults_are_already_inside_every_limit() {
        let defaults = PlayerPrefsBundle::default();
        let mut cleaned = defaults.clone();
        cleaned.pull_into_limits();
        assert_eq!(cleaned, defaults);
    }

    #[test]
    fn touching_counts_up_and_wraps() {
        let mut prefs = PlayerPrefsBundle::default();
        prefs.touch();
        prefs.touch();
        assert_eq!(prefs.revision, 2);
        prefs.revision = u64::MAX;
        prefs.touch();
        assert_eq!(prefs.revision, 0);
    }

    #[test]
    fn sliders_give_their_square() {
        let settings = PlayerPrefsBundle {
            main_level: 0.5,
            sfx_volume: 0.5,
            music_volume: 0.25,
            voice_volume: 1.0,
            cinematic_volume: 0.0,
            ..PlayerPrefsBundle::default()
        };
        assert_eq!(settings.master_gain(), 0.25);
        assert_eq!(settings.menu_volume(Some(0)), 0.25);
        assert_eq!(settings.menu_volume(Some(1)), 0.0625);
        assert_eq!(settings.menu_volume(Some(2)), 1.0);
        assert_eq!(settings.menu_volume(Some(4)), 0.0);
    }

    #[test]
    fn interface_sounds_follow_the_effects_slider() {
        let settings = PlayerPrefsBundle {
            sfx_volume: 0.5,
            ..PlayerPrefsBundle::default()
        };
        assert_eq!(settings.menu_volume(Some(3)), settings.menu_volume(Some(0)));
    }

    #[test]
    fn uncategorised_sounds_are_not_scaled() {
        let settings = PlayerPrefsBundle {
            sfx_volume: 0.1,
            music_volume: 0.1,
            ..PlayerPrefsBundle::default()
        };
        assert_eq!(settings.menu_volume(None), 1.0);
        assert_eq!(settings.menu_volume(Some(5)), 1.0);
        assert_eq!(settings.menu_volume(Some(255)), 1.0);
    }

    #[test]
    fn resolution_is_held_between_the_smallest_and_largest_supported() {
        let small = sanitized(|s| s.resolution = ScreenPixelSize::new(10, 10));
        assert_eq!(small.resolution, ScreenPixelSize::new(640, 480));
        let huge = sanitized(|s| s.resolution = ScreenPixelSize::new(100_000, 100_000));
        assert_eq!(huge.resolution, ScreenPixelSize::new(7680, 4320));
    }

    #[test]
    fn fov_and_brightness_are_clamped_and_nan_takes_the_default() {
        assert_eq!(sanitized(|s| s.view_angle = 10.0).view_angle, PlayerPrefsBundle::FOV_MIN);
        assert_eq!(sanitized(|s| s.view_angle = 500.0).view_angle, PlayerPrefsBundle::FOV_MAX);
        assert_eq!(
            sanitized(|s| s.view_angle = f32::NAN).view_angle,
            PlayerPrefsBundle::FOV_DEFAULT
        );
        assert_eq!(sanitized(|s| s.brightness = 3.0).brightness, 0.2);
        assert_eq!(sanitized(|s| s.brightness = -3.0).brightness, -0.2);
        assert_eq!(sanitized(|s| s.brightness = f32::INFINITY).brightness, 0.0);
    }

    #[test]
    fn volumes_stay_between_silent_and_full() {
        let loud = sanitized(|s| {
            s.main_level = 4.0;
            s.voice_volume = 9.0;
            s.music_volume = -1.0;
            s.sfx_volume = f32::NAN;
            s.cinematic_volume = f32::NEG_INFINITY;
        });
        assert_eq!(loud.main_level, 1.0);
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
        let custom = sanitized(|s| s.pad_layout = PlayerPrefsBundle::PAD_LAYOUT_CUSTOM);
        assert_eq!(custom.pad_layout, PlayerPrefsBundle::PAD_LAYOUT_CUSTOM);
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
            PlayerPrefsBundle::PAD_SENSITIVITY_DEFAULT
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
            sanitized(|s| s.shown_name = "  Ada  ".into()).shown_name,
            "Ada"
        );
        assert_eq!(
            sanitized(|s| s.shown_name = "   ".into()).shown_name,
            "Player"
        );
        assert_eq!(
            sanitized(|s| s.shown_name = String::new()).shown_name,
            "Player"
        );
        let long = sanitized(|s| s.shown_name = "abcdefghijklmnopqrstuvwxyz".into());
        assert_eq!(long.shown_name, "abcdefghijklmnop");
    }

    #[test]
    fn the_name_limit_counts_characters_not_bytes() {
        let wide = "é".repeat(20);
        let cut = sanitized(|s| s.shown_name = wide);
        assert_eq!(cut.shown_name.chars().count(), 16);
    }
}

impl PlayerPrefsBundle {
    /// The name he plays under (lobby, scoreboard, the game's own
    /// messages): a name his settings give (the family launcher's
    /// profile), else the Steam account this PC signs into.
    pub fn shown_name(&self) -> String {
        if self.shown_name == "Player" {
            steam_name().unwrap_or_else(|| self.shown_name.clone())
        } else {
            self.shown_name.clone()
        }
    }
}

/// The Steam name of the account Steam signs into on this PC (its
/// `config/loginusers.vdf`: the auto-login one, else the newest), as BO2
/// shows the player's Steam name; read once.
pub fn steam_name() -> Option<String> {
    static NAME: std::sync::OnceLock<Option<String>> = std::sync::OnceLock::new();
    NAME.get_or_init(|| {
        let root = std::env::var_os("ProgramFiles(x86)")?;
        let path = std::path::Path::new(&root).join("Steam/config/loginusers.vdf");
        let text = std::fs::read_to_string(path).ok()?;
        // Each account is a { } block of "key" "value" lines.
        let mut chosen: Option<(bool, u64, String)> = None;
        for entry in text.split('{').skip(2) {
            let entry = entry.split('}').next().unwrap_or("");
            let value_of = |wanted: &str| {
                entry.lines().find_map(|l| {
                    let mut parts = l.split('"').filter(|p| !p.trim().is_empty());
                    (parts.next()? == wanted).then(|| parts.next().map(str::to_owned)).flatten()
                })
            };
            let Some(name) = value_of("PersonaName").filter(|n| !n.trim().is_empty()) else {
                continue;
            };
            let auto = value_of("AutoLogin").as_deref() == Some("1");
            let time = value_of("Timestamp").and_then(|t| t.parse().ok()).unwrap_or(0);
            if chosen.as_ref().is_none_or(|b| (auto, time) > (b.0, b.1)) {
                chosen = Some((auto, time, name));
            }
        }
        chosen.map(|(_, _, n)| n.trim().chars().take(16).collect())
    })
    .clone()
}
