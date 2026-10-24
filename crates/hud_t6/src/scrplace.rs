//! BO2 screen placement: 640x480 virtual units to real pixels by an align mode per axis
//! (scratch/hud-facts/scrplace.md).
//!
//! The real viewport always starts at (0, 0). Horizontal art maps through a centred 4:3 sub-screen so
//! wide displays do not stretch it. Safe-area insets shrink the "viewable" rect (modes 1, 3, 7); the
//! profile-tuned "tweakable" rect drives modes 8 to 10. In split screen widths and heights (and offsets
//! when cg_hudSplitscreenOffsetsUseScale is on) are scaled by the HUD split-screen scale.

pub const SCRPLACE_VIRTUAL_WIDTH: f32 = 640.0;
pub const SCRPLACE_VIRTUAL_HEIGHT: f32 = 480.0;

const ASPECT_4_3: f32 = 4.0 / 3.0;

/// BO2's hi-res X scale is a constant 1.
pub const HIRES_SCALE_X: f32 = 1.0;

pub const ALIGN_SUB: i32 = 0;
pub const ALIGN_VIEWABLE: i32 = 1;
pub const ALIGN_CENTER: i32 = 2;
pub const ALIGN_VIEWABLE_MAX: i32 = 3;
pub const ALIGN_FULLSCREEN: i32 = 4;
pub const ALIGN_NOSCALE: i32 = 5;
pub const ALIGN_TO_VIRTUAL: i32 = 6;
pub const ALIGN_CENTER_SAFE: i32 = 7;
pub const ALIGN_USER_MIN: i32 = 8;
pub const ALIGN_USER_CENTER: i32 = 9;
pub const ALIGN_USER_MAX: i32 = 10;

/// BO2 ScreenPlacement (0x7c bytes: fifteen vec2 then the split-screen scale).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ScreenPlacement {
    pub scale_virtual_to_real: [f32; 2],
    pub scale_virtual_to_full: [f32; 2],
    pub scale_real_to_virtual: [f32; 2],
    pub virtual_viewable_min: [f32; 2],
    pub virtual_viewable_max: [f32; 2],
    pub virtual_tweakable_min: [f32; 2],
    pub virtual_tweakable_max: [f32; 2],
    pub real_viewport_base: [f32; 2],
    pub real_viewport_size: [f32; 2],
    pub real_viewport_mid: [f32; 2],
    pub real_viewable_min: [f32; 2],
    pub real_viewable_max: [f32; 2],
    pub real_tweakable_min: [f32; 2],
    pub real_tweakable_max: [f32; 2],
    /// Left letterbox of the 4:3 sub-screen; y is always 0.
    pub sub_screen: [f32; 2],
    pub hud_splitscreen_scale: f32,
}

/// What a viewport setup reads from the video config, dvars and profile.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PlacementInputs {
    pub display_size: [f32; 2],
    /// safeArea_horizontal, safeArea_vertical (each 0..1).
    pub safe_area: [f32; 2],
    /// The gamer profile's safe area when a profile is initialised; None copies the viewable rect.
    pub profile_safe_area: Option<[f32; 2]>,
    /// Display-pixel aspect from the video config (must not be 0).
    pub aspect_display_pixel: f32,
    /// r_dualPlayEnable: forces the vertical safe ratio to 1.
    pub dual_play_enable: bool,
    /// cg_hudSplitscreenScale while split screen is on; None means scale 1.
    pub hud_splitscreen_scale: Option<f32>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AppliedRect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

/// Real min/max and virtual min/max of one safe-area rect.
struct SafeRect {
    real_min: [f32; 2],
    real_max: [f32; 2],
    virtual_min: [f32; 2],
    virtual_max: [f32; 2],
}

/// Inset per axis is floor((1 - ratio) * 0.5 * display + 0.5); real offsets are clamped to the
/// viewport, virtual ones are scaled back to 640x480 (x also by the 4:3 aspect scale).
fn calc_safe_area_offsets(
    size: [f32; 2],
    display: [f32; 2],
    ratio: [f32; 2],
    dual_play_enable: bool,
    horz_aspect_scale: f32,
) -> SafeRect {
    debug_assert!((0.0..=1.0).contains(&ratio[0]) && (0.0..=1.0).contains(&ratio[1]));
    debug_assert!(display[0] > 0.0 && display[1] > 0.0);
    let ratio = [ratio[0], if dual_play_enable { 1.0 } else { ratio[1] }];
    let mut real_min = [0.0; 2];
    let mut real_max = [0.0; 2];
    for a in 0..2 {
        let inset = libm::floorf((1.0 - ratio[a]) * 0.5 * display[a] + 0.5);
        real_min[a] = inset.clamp(0.0, size[a]);
        real_max[a] = (display[a] - inset).clamp(0.0, size[a]);
    }
    let to_virtual = [
        SCRPLACE_VIRTUAL_WIDTH / size[0] * horz_aspect_scale,
        SCRPLACE_VIRTUAL_HEIGHT / size[1],
    ];
    SafeRect {
        real_min,
        real_max,
        virtual_min: [real_min[0] * to_virtual[0], real_min[1] * to_virtual[1]],
        virtual_max: [real_max[0] * to_virtual[0], real_max[1] * to_virtual[1]],
    }
}

impl ScreenPlacement {
    /// ScrPlace_SetupFloatViewport: a (w, h) viewport at (0, 0).
    pub fn setup_float_viewport(w: f32, h: f32, inp: &PlacementInputs) -> Self {
        assert!(
            inp.aspect_display_pixel != 0.0,
            "ScrPlace: display pixel aspect is 0; video init has not written it"
        );
        let sub_w = (h * ASPECT_4_3 / inp.aspect_display_pixel).min(w);
        let horz_aspect_scale = w / sub_w;
        let size = [w, h];
        let viewable = calc_safe_area_offsets(
            size,
            inp.display_size,
            inp.safe_area,
            inp.dual_play_enable,
            horz_aspect_scale,
        );
        let tweakable = match inp.profile_safe_area {
            Some(p) => calc_safe_area_offsets(
                size,
                inp.display_size,
                [p[0].clamp(0.0, 1.0), p[1].clamp(0.0, 1.0)],
                inp.dual_play_enable,
                horz_aspect_scale,
            ),
            None => SafeRect { ..viewable },
        };
        Self {
            scale_virtual_to_real: [sub_w / SCRPLACE_VIRTUAL_WIDTH, h / SCRPLACE_VIRTUAL_HEIGHT],
            scale_virtual_to_full: [w / SCRPLACE_VIRTUAL_WIDTH, h / SCRPLACE_VIRTUAL_HEIGHT],
            scale_real_to_virtual: [SCRPLACE_VIRTUAL_WIDTH / sub_w, SCRPLACE_VIRTUAL_HEIGHT / h],
            virtual_viewable_min: viewable.virtual_min,
            virtual_viewable_max: viewable.virtual_max,
            virtual_tweakable_min: tweakable.virtual_min,
            virtual_tweakable_max: tweakable.virtual_max,
            real_viewport_base: [0.0, 0.0],
            real_viewport_size: size,
            real_viewport_mid: [w * 0.5, h * 0.5],
            real_viewable_min: viewable.real_min,
            real_viewable_max: viewable.real_max,
            real_tweakable_min: tweakable.real_min,
            real_tweakable_max: tweakable.real_max,
            sub_screen: [(w - sub_w) * 0.5, 0.0],
            hud_splitscreen_scale: inp.hud_splitscreen_scale.unwrap_or(1.0),
        }
    }

    /// ScrPlace_SetupViewport: integer sizes cast to float.
    pub fn setup_viewport(w: i32, h: i32, inp: &PlacementInputs) -> Self {
        Self::setup_float_viewport(w as f32, h as f32, inp)
    }

    /// ScrPlace_SetupUnsafeViewport: both safe-area ratios forced to 1 for this setup.
    pub fn setup_unsafe_viewport(w: i32, h: i32, inp: &PlacementInputs) -> Self {
        let unsafe_inp = PlacementInputs { safe_area: [1.0, 1.0], ..*inp };
        Self::setup_viewport(w, h, &unsafe_inp)
    }

    /// ScrPlace_SetupUI3D: the whole window, no safe area, no sub-screen, scale 1.
    pub fn setup_ui3d(w: f32, h: f32) -> Self {
        let full = [w / SCRPLACE_VIRTUAL_WIDTH, h / SCRPLACE_VIRTUAL_HEIGHT];
        let virt = [SCRPLACE_VIRTUAL_WIDTH, SCRPLACE_VIRTUAL_HEIGHT];
        Self {
            scale_virtual_to_real: full,
            scale_virtual_to_full: full,
            scale_real_to_virtual: [SCRPLACE_VIRTUAL_WIDTH / w, SCRPLACE_VIRTUAL_HEIGHT / h],
            virtual_viewable_min: [0.0, 0.0],
            virtual_viewable_max: virt,
            virtual_tweakable_min: [0.0, 0.0],
            virtual_tweakable_max: virt,
            real_viewport_base: [0.0, 0.0],
            real_viewport_size: [w, h],
            real_viewport_mid: [0.0, 0.0],
            real_viewable_min: [0.0, 0.0],
            real_viewable_max: [w, h],
            real_tweakable_min: [0.0, 0.0],
            real_tweakable_max: [w, h],
            sub_screen: [0.0, 0.0],
            hud_splitscreen_scale: 1.0,
        }
    }

    /// One axis of ScrPlace_ApplyX / ApplyY. `axis` is 0 for x, 1 for y.
    pub fn apply_pos(&self, v: f32, align: i32, axis: usize, offsets_use_scale: bool) -> f32 {
        let v = if offsets_use_scale { v * self.hud_splitscreen_scale } else { v };
        let scaled = self.scale_virtual_to_real[axis] * v;
        match align {
            ALIGN_VIEWABLE => scaled + self.real_viewable_min[axis],
            ALIGN_CENTER => {
                scaled + self.real_viewport_size[axis] * 0.5 + self.real_viewport_base[axis]
            }
            ALIGN_VIEWABLE_MAX => scaled + self.real_viewable_max[axis],
            ALIGN_FULLSCREEN => {
                self.scale_virtual_to_full[axis] * v + self.real_viewport_base[axis]
            }
            ALIGN_NOSCALE => v,
            ALIGN_TO_VIRTUAL => self.scale_real_to_virtual[axis] * v,
            ALIGN_CENTER_SAFE => {
                scaled + (self.real_viewable_min[axis] + self.real_viewable_max[axis]) * 0.5
            }
            ALIGN_USER_MIN => scaled + self.real_tweakable_min[axis],
            ALIGN_USER_CENTER => {
                scaled + (self.real_tweakable_min[axis] + self.real_tweakable_max[axis]) * 0.5
            }
            ALIGN_USER_MAX => scaled + self.real_tweakable_max[axis],
            _ => {
                debug_assert!(align == ALIGN_SUB, "ScrPlace: bad align {align}");
                scaled + self.sub_screen[axis]
            }
        }
    }

    /// One axis of ScrPlace_ApplyW / ApplyH: always split-screen scaled first.
    pub fn apply_size(&self, v: f32, align: i32, axis: usize) -> f32 {
        let v = v * self.hud_splitscreen_scale;
        match align {
            ALIGN_FULLSCREEN => self.scale_virtual_to_full[axis] * v,
            ALIGN_NOSCALE => v,
            ALIGN_TO_VIRTUAL => self.scale_real_to_virtual[axis] * v,
            _ => self.scale_virtual_to_real[axis] * v,
        }
    }

    /// ScrPlace_ApplyRect.
    pub fn apply_rect(
        &self,
        r: AppliedRect,
        horz_align: i32,
        vert_align: i32,
        offsets_use_scale: bool,
    ) -> AppliedRect {
        AppliedRect {
            x: self.apply_pos(r.x, horz_align, 0, offsets_use_scale),
            y: self.apply_pos(r.y, vert_align, 1, offsets_use_scale),
            w: self.apply_size(r.w, horz_align, 0),
            h: self.apply_size(r.h, vert_align, 1),
        }
    }

    /// ScrPlace_RealFromVirtual_Width.
    pub fn real_from_virtual_width(&self, v: f32) -> f32 {
        self.hud_splitscreen_scale * v * HIRES_SCALE_X * self.scale_virtual_to_real[0]
    }

    /// ScrPlace_RealFromVirtual_Height.
    pub fn real_from_virtual_height(&self, v: f32) -> f32 {
        self.hud_splitscreen_scale * v * HIRES_SCALE_X * self.scale_virtual_to_real[1]
    }

    /// ScrPlace_VirtualFromReal_Height.
    pub fn virtual_from_real_height(&self, v: f32) -> f32 {
        v / self.hud_splitscreen_scale / HIRES_SCALE_X * self.scale_real_to_virtual[1]
    }
}

/// ScrPlace_SetLegacySplitscreenScaling: the global HUD scale is cg_hudLegacySplitscreenScale in
/// split screen, else 1. (SetNormalSplitscreenScaling is always 1.)
pub fn legacy_splitscreen_scale(splitscreen: bool, legacy_scale: f32) -> f32 {
    if splitscreen { legacy_scale } else { 1.0 }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn inputs(w: f32, h: f32) -> PlacementInputs {
        PlacementInputs {
            display_size: [w, h],
            safe_area: [0.9, 0.9],
            profile_safe_area: None,
            aspect_display_pixel: 1.0,
            dual_play_enable: false,
            hud_splitscreen_scale: None,
        }
    }

    #[test]
    fn widescreen_uses_centred_4_3_sub_screen() {
        let p = ScreenPlacement::setup_float_viewport(1920.0, 1080.0, &inputs(1920.0, 1080.0));
        assert_eq!(p.sub_screen, [240.0, 0.0]);
        assert_eq!(p.scale_virtual_to_real, [2.25, 2.25]);
        assert_eq!(p.scale_virtual_to_full, [3.0, 2.25]);
        assert_eq!(p.real_viewport_mid, [960.0, 540.0]);
        // Inset floor(0.05 * 1920 + 0.5) = 96, floor(0.05 * 1080 + 0.5) = 54.
        assert_eq!(p.real_viewable_min, [96.0, 54.0]);
        assert_eq!(p.real_viewable_max, [1824.0, 1026.0]);
        // No profile: tweakable copies viewable.
        assert_eq!(p.real_tweakable_min, p.real_viewable_min);
        assert_eq!(p.virtual_tweakable_max, p.virtual_viewable_max);
        // Virtual x carries the 4:3 aspect scale (1920 / 1440).
        assert!((p.virtual_viewable_min[0] - 96.0 * (640.0 / 1920.0) * (1920.0 / 1440.0)).abs() < 1e-4);
        assert!((p.virtual_viewable_min[1] - 24.0).abs() < 1e-4);
    }

    #[test]
    fn dual_play_forces_full_vertical_safe_area() {
        let mut inp = inputs(1280.0, 720.0);
        inp.dual_play_enable = true;
        let p = ScreenPlacement::setup_float_viewport(1280.0, 360.0, &inp);
        assert_eq!(p.real_viewable_min[1], 0.0);
        assert_eq!(p.real_viewable_max[1], 360.0);
        assert!(p.real_viewable_min[0] > 0.0);
    }

    #[test]
    fn profile_safe_area_is_clamped_and_drives_tweakable() {
        let mut inp = inputs(1000.0, 1000.0);
        inp.aspect_display_pixel = 0.75;
        inp.profile_safe_area = Some([2.0, 0.8]);
        let p = ScreenPlacement::setup_float_viewport(1000.0, 1000.0, &inp);
        assert_eq!(p.real_tweakable_min, [0.0, 100.0]);
        assert_eq!(p.real_tweakable_max, [1000.0, 900.0]);
    }

    #[test]
    fn unsafe_viewport_has_no_insets() {
        let p = ScreenPlacement::setup_unsafe_viewport(1280, 720, &inputs(1280.0, 720.0));
        assert_eq!(p.real_viewable_min, [0.0, 0.0]);
        assert_eq!(p.real_viewable_max, [1280.0, 720.0]);
    }

    #[test]
    fn apply_modes() {
        let p = ScreenPlacement::setup_float_viewport(1920.0, 1080.0, &inputs(1920.0, 1080.0));
        assert_eq!(p.apply_pos(10.0, ALIGN_SUB, 0, false), 22.5 + 240.0);
        assert_eq!(p.apply_pos(10.0, ALIGN_SUB, 1, false), 22.5);
        assert_eq!(p.apply_pos(0.0, ALIGN_CENTER, 0, false), 960.0);
        assert_eq!(p.apply_pos(10.0, ALIGN_VIEWABLE, 0, false), 22.5 + 96.0);
        assert_eq!(p.apply_pos(-10.0, ALIGN_VIEWABLE_MAX, 1, false), -22.5 + 1026.0);
        assert_eq!(p.apply_pos(10.0, ALIGN_FULLSCREEN, 0, false), 30.0);
        assert_eq!(p.apply_pos(10.0, ALIGN_NOSCALE, 0, false), 10.0);
        assert_eq!(p.apply_pos(0.0, ALIGN_CENTER_SAFE, 0, false), 960.0);
        assert_eq!(p.apply_pos(0.0, ALIGN_USER_CENTER, 1, false), 540.0);
        assert_eq!(p.apply_size(10.0, ALIGN_FULLSCREEN, 0), 30.0);
        assert_eq!(p.apply_size(10.0, ALIGN_CENTER, 0), 22.5);
    }

    #[test]
    fn splitscreen_scale_rules() {
        let mut inp = inputs(1280.0, 720.0);
        inp.hud_splitscreen_scale = Some(0.5);
        let p = ScreenPlacement::setup_float_viewport(1280.0, 720.0, &inp);
        let r = AppliedRect { x: 100.0, y: 100.0, w: 40.0, h: 40.0 };
        let off = p.apply_rect(r, ALIGN_SUB, ALIGN_SUB, false);
        let on = p.apply_rect(r, ALIGN_SUB, ALIGN_SUB, true);
        // Sizes always halve; offsets only when the dvar says so.
        assert_eq!(off.w, 20.0 * p.scale_virtual_to_real[0]);
        assert_eq!(off.h, on.h);
        assert_eq!(off.y, 100.0 * 1.5);
        assert_eq!(on.y, 50.0 * 1.5);
        assert_eq!(p.real_from_virtual_height(40.0), 30.0);
        assert_eq!(p.virtual_from_real_height(30.0), 40.0 * 1.0 / 0.5 / 0.5 * 0.5 * 0.5);
        assert_eq!(legacy_splitscreen_scale(false, 0.6), 1.0);
        assert_eq!(legacy_splitscreen_scale(true, 0.6), 0.6);
    }

    #[test]
    fn ui3d_covers_whole_window() {
        let p = ScreenPlacement::setup_ui3d(320.0, 240.0);
        assert_eq!(p.scale_virtual_to_real, [0.5, 0.5]);
        assert_eq!(p.real_viewable_max, [320.0, 240.0]);
        assert_eq!(p.apply_pos(640.0, ALIGN_VIEWABLE, 0, false), 320.0);
    }
}
