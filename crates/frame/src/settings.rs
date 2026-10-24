use bevy::prelude::Resource;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DisplayResolution {
    pub width: u32,
    pub height: u32,
}

impl DisplayResolution {
    pub const HD: Self = Self {
        width: 1280,
        height: 720,
    };

    pub const fn new(width: u32, height: u32) -> Self {
        Self { width, height }
    }
}

impl core::fmt::Display for DisplayResolution {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{}x{}", self.width, self.height)
    }
}

#[derive(Resource, Clone, Debug, PartialEq)]
pub struct GameSettings {
    pub resolution: DisplayResolution,
    pub fullscreen: bool,
    pub vsync: bool,
    pub fov: f32,
    pub master_volume: f32,
    /// bo2mp: Black Ops II's other menu volumes (Options > Audio): each scales the sounds of
    /// the menu category it names, under the master volume.
    pub voice_volume: f32,
    pub music_volume: f32,
    pub sfx_volume: f32,
    pub cinematic_volume: f32,
    /// bo2mp: Black Ops II's audio preset (`snd_menu_presets`): 0 Treyarch mix, 1 bass
    /// boost, 2 high boost, 3 super crunch, 4 headphones.
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

    /// bo2zm: `bo2_quality=fast` (Black Ops II maps: half-size textures,
    /// single-layer ground, props hidden sooner); `full` by default.
    pub bo2_fast: bool,
    /// bo2zm: Controls > Move > "Omni Movement": sprint in any direction.
    /// Not Black Ops II's (it sprints forward only), so off by default.
    pub omni_movement: bool,

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
            player_name: "Player".to_owned(),
            pad_layout: 0,
            pad_stick_layout: 0,
            pad_sensitivity: Self::PAD_SENSITIVITY_DEFAULT,
            pad_ads_sensitivity: 1.0,
            pad_invert: false,
            pad_curve: 0,
            pad_aim_assist: 0,
            pad_prompts: 0,
            pad_vibration: true,
            pad_deadzone_left: 0.12,
            pad_deadzone_right: 0.12,
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
    pub const PAD_LAYOUT_CUSTOM: u8 = 255;

    /// bo2mp: the gain a Black Ops II sound category takes from its menu slider (`SndMenuCategory`:
    /// 0 effects, 1 music, 2 voice, 3 interface, 4 cinematic). As in Black Ops II the gain is
    /// the slider squared, and the interface sounds follow the effects slider. Sounds of the
    /// other games have no category (1).
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

    /// bo2mp: the gain every sound takes from the master slider: the slider squared, as in
    /// Black Ops II.
    pub fn master_gain(&self) -> f32 {
        self.master_volume * self.master_volume
    }

    pub fn touch(&mut self) {
        self.revision = self.revision.wrapping_add(1);
    }

    pub fn sanitize(&mut self) {
        self.resolution.width = self.resolution.width.clamp(640, 7680);
        self.resolution.height = self.resolution.height.clamp(480, 4320);
        self.fov = if self.fov.is_finite() {
            self.fov.clamp(Self::FOV_MIN, Self::FOV_MAX)
        } else {
            Self::FOV_DEFAULT
        };
        self.brightness = if self.brightness.is_finite() {
            self.brightness.clamp(-0.2, 0.2)
        } else {
            0.0
        };
        self.master_volume = self.master_volume.clamp(0.0, 1.0);
        for v in [
            &mut self.voice_volume,
            &mut self.music_volume,
            &mut self.sfx_volume,
            &mut self.cinematic_volume,
        ] {
            *v = if v.is_finite() { v.clamp(0.0, 1.0) } else { 1.0 };
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
        let finite = |v: f32, lo: f32, hi: f32, default: f32| {
            if v.is_finite() {
                v.clamp(lo, hi)
            } else {
                default
            }
        };
        self.pad_sensitivity = finite(
            self.pad_sensitivity,
            1.0,
            10.0,
            Self::PAD_SENSITIVITY_DEFAULT,
        );
        self.pad_ads_sensitivity = finite(self.pad_ads_sensitivity, 0.5, 1.5, 1.0);
        self.pad_deadzone_left = finite(self.pad_deadzone_left, 0.0, 0.4, 0.12);
        self.pad_deadzone_right = finite(self.pad_deadzone_right, 0.0, 0.4, 0.12);
        self.player_name = self.player_name.trim().chars().take(16).collect();
        if self.player_name.is_empty() {
            self.player_name = "Player".to_owned();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::GameSettings;

    #[test]
    fn bo2_sliders_give_their_square() {
        let s = GameSettings {
            master_volume: 0.5,
            sfx_volume: 0.5,
            music_volume: 0.25,
            voice_volume: 1.0,
            cinematic_volume: 0.0,
            ..GameSettings::default()
        };
        assert_eq!(s.master_gain(), 0.25);
        assert_eq!(s.menu_volume(Some(0)), 0.25);
        assert_eq!(s.menu_volume(Some(1)), 0.0625);
        assert_eq!(s.menu_volume(Some(2)), 1.0);
        // Interface sounds follow the effects slider.
        assert_eq!(s.menu_volume(Some(3)), 0.25);
        assert_eq!(s.menu_volume(Some(4)), 0.0);
        assert_eq!(s.menu_volume(None), 1.0);
    }
}
