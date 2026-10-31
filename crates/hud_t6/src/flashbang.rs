//! BO2 flash and stun (shellshock) screen effect: the shock file's bg_shock_* parms, the
//! playerState-driven remaining time, flashbanged / flared / blind, the FLASHED whiteout and
//! screengrab blend, the BLURRED blend time, look control, the camera kick amplitude, the
//! server's blast fractions and duration, and the damage flash.
//! BO2's blind time itself is decided by GSC from the "flashbang" notify, not here.
//! Facts: scratch/hud-facts/flashbang.md.

extern crate alloc;

use alloc::borrow::ToOwned;
use alloc::format;
use alloc::string::String;

/// Mouse input at its usual strength.
const FULL_FEEL: f32 = 1.0;
/// No turn-rate cap.
const NO_CAP: f32 = 0.0;

/// ShockViewTypes (bg_shock_screenType).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(i32)]
pub enum ShockViewType {
    Blurred = 0,
    Flashed = 1,
    None = 2,
}

pub const SCREEN_BLEND_BLURRED: i32 = ShockViewType::Blurred as i32;
pub const SCREEN_BLEND_FLASHED: i32 = ShockViewType::Flashed as i32;
pub const SCREEN_BLEND_NONE: i32 = ShockViewType::None as i32;

impl ShockViewType {
    #[must_use]
    pub fn from_i32(code: i32) -> Option<Self> {
        match code {
            0 => Some(Self::Blurred),
            1 => Some(Self::Flashed),
            2 => Some(Self::None),
            _ => None,
        }
    }
}

pub const BG_SHOCK_SCREEN_BLUR_BLEND_TIME_DEFAULT: f32 = 0.4;
pub const BG_SHOCK_SCREEN_BLUR_BLEND_FADE_TIME_DEFAULT: f32 = 1.0;
pub const BG_SHOCK_SCREEN_FLASH_WHITE_FADE_TIME_DEFAULT: f32 = 1.0;
pub const BG_SHOCK_SCREEN_FLASH_SHOT_FADE_TIME_DEFAULT: f32 = 1.0;
pub const BG_SHOCK_VIEW_KICK_PERIOD_DEFAULT: f32 = 0.75;
pub const BG_SHOCK_VIEW_KICK_PERIOD_MIN: f32 = 0.001;
pub const BG_SHOCK_VIEW_KICK_RADIUS_DEFAULT: f32 = 0.05;
pub const BG_SHOCK_VIEW_KICK_FADE_TIME_DEFAULT: f32 = 3.0;
pub const BG_SHOCK_LOOK_CONTROL_DEFAULT: bool = true;
pub const BG_SHOCK_LOOK_CONTROL_MAX_PITCH_SPEED_DEFAULT: f32 = 90.0;
pub const BG_SHOCK_LOOK_CONTROL_MAX_YAW_SPEED_DEFAULT: f32 = 90.0;
pub const BG_SHOCK_LOOK_CONTROL_MOUSE_SENSITIVITY_SCALE_DEFAULT: f32 = 0.5;
pub const BG_SHOCK_LOOK_CONTROL_FADE_TIME_DEFAULT: f32 = 2.0;
pub const BG_SHOCK_MOVEMENT_DEFAULT: f32 = 1.0;
pub const BG_SHOCK_ANIMATION_DEFAULT: bool = false;
pub const BG_SHOCK_SOUND_DEFAULT: bool = true;
pub const BG_SHOCK_SOUND_LOOP_DEFAULT: &str = "shellshock_loop";
pub const BG_SHOCK_SOUND_LOOP_SILENT_DEFAULT: &str = "shellshock_loop_silent";
pub const BG_SHOCK_SOUND_END_DEFAULT: &str = "shellshock_end";
pub const BG_SHOCK_SOUND_END_ABORT_DEFAULT: &str = "shellshock_end_abort";
pub const BG_SHOCK_SOUND_FADE_IN_TIME_DEFAULT: f32 = 0.25;
pub const BG_SHOCK_SOUND_FADE_OUT_TIME_DEFAULT: f32 = 2.5;
pub const BG_SHOCK_SOUND_LOOP_FADE_TIME_DEFAULT: f32 = 1.5;
pub const BG_SHOCK_SOUND_LOOP_END_DELAY_DEFAULT: f32 = -3.0;
pub const BG_SHOCK_SOUND_ROOM_TYPE_DEFAULT: &str = "default";
pub const BG_SHOCK_SOUND_DRY_LEVEL_DEFAULT: f32 = 1.0;
pub const BG_SHOCK_SOUND_WET_LEVEL_DEFAULT: f32 = 0.5;
pub const BG_SHOCK_SOUND_MOD_END_DELAY_DEFAULT: f32 = 2.0;
pub const BG_SHOCK_SOUND_SNAPSHOT_DEFAULT: &str = "default";

/// cg_drawShellshock: draws the shellshock and flashbang screen effects at all.
pub const CG_DRAW_SHELLSHOCK_DEFAULT: bool = true;
/// cg_flareVisionSetFadeDuration, ms.
pub const CG_FLARE_VISION_SET_FADE_DURATION_MS_DEFAULT: i32 = 2000;
pub const FLARE_DISABLE_EFFECTS_DEFAULT: bool = false;
/// perk_shellShockReduction: the shock duration scale for a player with the reduction perk.
pub const PERK_SHELL_SHOCK_REDUCTION_DEFAULT: f32 = 0.5;

/// parms.view.fadeTime is fixed at this, whatever bg_shock_viewKickFadeTime says.
pub const VIEW_KICK_FADE_MS: i32 = 3000;
/// The camera kick walks a table of this many vec2 perturbations.
pub const PERTURBATION_COUNT: usize = 128;
/// Kick table index = (duration * this + floor(phase)) & 0x7f.
pub const PERTURBATION_DURATION_STRIDE: i32 = 61;

/// Named shocks a level can precache (config strings CS_SHELLSHOCKS + 1..=15).
pub const MAX_SHELLSHOCKS: usize = 15;
pub const CS_SHELLSHOCKS: usize = 0x7fa;
/// ShellShock() takes 0..60 seconds.
pub const SHELLSHOCK_SECONDS_MAX: f32 = 60.0;
/// movement_state_bits bit set on a living player when shocked.
pub const PMF_SHELLSHOCKED: u32 = 0x10000;
/// Perk bit that scales a shock by perk_shellShockReduction (server side).
pub const PERK_SHELLSHOCK_REDUCTION: u32 = 0x38;
/// Perk bit that keeps a player from being flared.
pub const PERK_FLARE_IMMUNE: u32 = 0x1a;
/// A player counts as flared from this snapshot visionSetLerpRatio up.
pub const FLARED_LERP_RATIO: f32 = 0.25;
/// CG_InFlareVision: connection states above this only.
pub const FLARE_VISION_MIN_CONNECTION_STATE: i32 = 9;
pub const FLARE_VISION_NEXT_FIELD_MIN: f32 = 0.001;

/// Flashbang blast: the box that gathers candidates is +- this times the max radius.
pub const FLASHBANG_GATHER_BOX_SCALE: f32 = 1.4142;
pub const FLASHBANG_GATHER_MAX_ENTS: usize = 1024;
/// The shellshock flash material the backend stretches over the saved screen.
pub const SHELLSHOCK_FLASHED_MATERIAL: &str = "shellShockFlashed";

/// Damage flash: red overlay, amount capped at this.
pub const DAMAGE_FLASH_AMOUNT_MAX: f32 = 5.0;
pub const DAMAGE_FLASH_KICK_SCALE: f32 = 0.002;
pub const DAMAGE_FLASH_RGB: [f32; 3] = [0.2, 0.0, 0.0];
pub const DAMAGE_FLASH_ALPHA_SCALE: f32 = 0.2 * 0.7;
/// The overlay overhangs the screen by this on every side.
pub const DAMAGE_FLASH_OVERHANG: f32 = 10.0;
/// tu_isolateDamageFlash.
pub const TU_ISOLATE_DAMAGE_FLASH_DEFAULT: bool = true;

/// parms.screenBlend, times in ms.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ShellshockScreenBlend {
    pub blurred_fade_ms: i32,
    pub blurred_effect_ms: i32,
    pub flash_white_fade_ms: i32,
    pub flash_shot_fade_ms: i32,
    pub kind: ShockViewType,
}

/// parms.view: the camera kick.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ShellshockView {
    pub fade_for_ms: i32,
    /// Phase per ms: 0.001 / bg_shock_viewKickPeriod.
    pub kick_rate: f32,
    pub kick_radius: f32,
}

/// parms.lookControl.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ShellshockLookControl {
    pub touches: bool,
    pub fade_for_ms: i32,
    pub touchiness: f32,
    pub pitch_cap: f32,
    pub yaw_cap: f32,
}

/// parms.sound, times in ms.
#[derive(Clone, Debug, PartialEq)]
pub struct ShellshockSound {
    pub touches: bool,
    pub loop_alias: String,
    pub loop_silent_alias: String,
    pub end_alias: String,
    pub end_abort_alias: String,
    pub fade_in_ms: i32,
    pub fade_out_ms: i32,
    pub loop_fade_ms: i32,
    pub loop_end_delay_ms: i32,
    pub room_type: String,
    pub dry_level: f32,
    pub wet_level: f32,
    pub mod_end_delay_ms: i32,
    pub snapshot: String,
}

/// parms.visionSet, times in ms.
#[derive(Clone, Debug, PartialEq)]
pub struct ShellshockVisionSet {
    pub name: String,
    pub in_ms: i32,
    pub out_ms: i32,
}

/// shellshock_parms_t: one precached shock, as BG_SetShellShockParmsFromDvars fills it.
#[derive(Clone, Debug, PartialEq)]
pub struct ShellshockParms {
    pub screen_blend: ShellshockScreenBlend,
    pub view: ShellshockView,
    pub sound: ShellshockSound,
    pub look_control: ShellshockLookControl,
    /// bg_shock_movement: a speed scale while shocked, not a switch.
    pub movement_scale: f32,
    /// bg_shock_animation: the shock plays its own third-person anim.
    pub animation: bool,
    pub vision_set: ShellshockVisionSet,
}

fn ms(seconds: f32) -> i32 {
    libm::roundf(seconds * 1000.0) as i32
}

impl Default for ShellshockParms {
    /// The parms from the bg_shock_* dvar defaults.
    fn default() -> Self {
        let period = BG_SHOCK_VIEW_KICK_PERIOD_DEFAULT;
        Self {
            screen_blend: ShellshockScreenBlend {
                blurred_fade_ms: ms(BG_SHOCK_SCREEN_BLUR_BLEND_FADE_TIME_DEFAULT),
                blurred_effect_ms: ms(BG_SHOCK_SCREEN_BLUR_BLEND_TIME_DEFAULT),
                flash_white_fade_ms: ms(BG_SHOCK_SCREEN_FLASH_WHITE_FADE_TIME_DEFAULT),
                flash_shot_fade_ms: ms(BG_SHOCK_SCREEN_FLASH_SHOT_FADE_TIME_DEFAULT),
                kind: ShockViewType::Blurred,
            },
            view: ShellshockView {
                fade_for_ms: VIEW_KICK_FADE_MS,
                kick_rate: view_kick_rate(period),
                kick_radius: BG_SHOCK_VIEW_KICK_RADIUS_DEFAULT,
            },
            sound: ShellshockSound {
                touches: BG_SHOCK_SOUND_DEFAULT,
                loop_alias: BG_SHOCK_SOUND_LOOP_DEFAULT.to_owned(),
                loop_silent_alias: BG_SHOCK_SOUND_LOOP_SILENT_DEFAULT.to_owned(),
                end_alias: BG_SHOCK_SOUND_END_DEFAULT.to_owned(),
                end_abort_alias: BG_SHOCK_SOUND_END_ABORT_DEFAULT.to_owned(),
                fade_in_ms: ms(BG_SHOCK_SOUND_FADE_IN_TIME_DEFAULT),
                fade_out_ms: ms(BG_SHOCK_SOUND_FADE_OUT_TIME_DEFAULT),
                loop_fade_ms: ms(BG_SHOCK_SOUND_LOOP_FADE_TIME_DEFAULT),
                loop_end_delay_ms: ms(BG_SHOCK_SOUND_LOOP_END_DELAY_DEFAULT),
                room_type: BG_SHOCK_SOUND_ROOM_TYPE_DEFAULT.to_owned(),
                dry_level: BG_SHOCK_SOUND_DRY_LEVEL_DEFAULT,
                wet_level: BG_SHOCK_SOUND_WET_LEVEL_DEFAULT,
                mod_end_delay_ms: ms(BG_SHOCK_SOUND_MOD_END_DELAY_DEFAULT),
                snapshot: BG_SHOCK_SOUND_SNAPSHOT_DEFAULT.to_owned(),
            },
            look_control: ShellshockLookControl {
                touches: BG_SHOCK_LOOK_CONTROL_DEFAULT,
                fade_for_ms: ms(BG_SHOCK_LOOK_CONTROL_FADE_TIME_DEFAULT),
                touchiness: BG_SHOCK_LOOK_CONTROL_MOUSE_SENSITIVITY_SCALE_DEFAULT,
                pitch_cap: BG_SHOCK_LOOK_CONTROL_MAX_PITCH_SPEED_DEFAULT,
                yaw_cap: BG_SHOCK_LOOK_CONTROL_MAX_YAW_SPEED_DEFAULT,
            },
            movement_scale: BG_SHOCK_MOVEMENT_DEFAULT,
            animation: BG_SHOCK_ANIMATION_DEFAULT,
            vision_set: ShellshockVisionSet { in_ms: 0, out_ms: 0, name: String::default() },
        }
    }
}

/// parms.view.kickRate from bg_shock_viewKickPeriod (seconds, floored at 0.001).
#[must_use]
pub fn view_kick_rate(period_s: f32) -> f32 {
    0.001 / period_s.max(BG_SHOCK_VIEW_KICK_PERIOD_MIN)
}

impl ShellshockParms {
    /// A .shock file: `bg_shock_<name> "value"` lines set the dvars, then
    /// BG_SetShellShockParmsFromDvars reads them (seconds to rounded ms). A line the file leaves
    /// out keeps the dvar's default. Both blur times must end up above 0.
    pub fn parse(text: &str) -> Result<Self, String> {
        let mut p = Self::default();
        let mut period = BG_SHOCK_VIEW_KICK_PERIOD_DEFAULT;
        for row in text.lines().map(str::trim) {
            let Some((dvar, raw)) = row.split_once(char::is_whitespace) else { continue };
            let Some(name) = dvar.to_ascii_lowercase().strip_prefix("bg_shock_").map(str::to_owned) else {
                continue;
            };
            let value = raw.trim().trim_matches('"');
            let number = || -> Result<f32, String> {
                value.parse::<f32>().map_err(|_| format!("{dvar} \"{value}\" is not a number"))
            };
            let flag = || number().map(|v| v != 0.0);
            match name.as_str() {
                "screentype" => {
                    p.screen_blend.kind = match value.to_ascii_lowercase().as_str() {
                        "blurred" => ShockViewType::Blurred,
                        "flashed" => ShockViewType::Flashed,
                        "none" => ShockViewType::None,
                        other => return Err(format!("{dvar} \"{other}\" is not blurred, flashed or none")),
                    };
                }
                "screenblurblendtime" => p.screen_blend.blurred_effect_ms = ms(number()?),
                "screenblurblendfadetime" => p.screen_blend.blurred_fade_ms = ms(number()?),
                "screenflashwhitefadetime" => p.screen_blend.flash_white_fade_ms = ms(number()?),
                "screenflashshotfadetime" => p.screen_blend.flash_shot_fade_ms = ms(number()?),
                "viewkickperiod" => period = number()?,
                "viewkickradius" => p.view.kick_radius = number()?,
                // Registered, but parms.view.fadeTime stays VIEW_KICK_FADE_MS.
                "viewkickfadetime" => {}
                "lookcontrol" => p.look_control.touches = flag()?,
                "lookcontrol_maxpitchspeed" => p.look_control.pitch_cap = number()?,
                "lookcontrol_maxyawspeed" => p.look_control.yaw_cap = number()?,
                "lookcontrol_mousesensitivityscale" => p.look_control.touchiness = number()?,
                "lookcontrol_fadetime" => p.look_control.fade_for_ms = ms(number()?),
                "movement" => p.movement_scale = number()?,
                "animation" => p.animation = flag()?,
                "visionset_name" => p.vision_set.name = value.to_owned(),
                "visionset_intime" => p.vision_set.in_ms = ms(number()?),
                "visionset_outtime" => p.vision_set.out_ms = ms(number()?),
                "sound" => p.sound.touches = flag()?,
                "soundloop" => p.sound.loop_alias = value.to_owned(),
                "soundloopsilent" => p.sound.loop_silent_alias = value.to_owned(),
                "soundend" => p.sound.end_alias = value.to_owned(),
                "soundendabort" => p.sound.end_abort_alias = value.to_owned(),
                "soundfadeintime" => p.sound.fade_in_ms = ms(number()?),
                "soundfadeouttime" => p.sound.fade_out_ms = ms(number()?),
                "soundloopfadetime" => p.sound.loop_fade_ms = ms(number()?),
                "soundloopenddelay" => p.sound.loop_end_delay_ms = ms(number()?),
                "soundroomtype" => p.sound.room_type = value.to_owned(),
                "sounddrylevel" => p.sound.dry_level = number()?,
                "soundwetlevel" => p.sound.wet_level = number()?,
                "soundmodenddelay" => p.sound.mod_end_delay_ms = ms(number()?),
                "soundsnapshot" => p.sound.snapshot = value.to_owned(),
                _ => {}
            }
        }
        p.view.kick_rate = view_kick_rate(period);
        if p.screen_blend.blurred_effect_ms <= 0 || p.screen_blend.blurred_fade_ms <= 0 {
            return Err("bg_shock_screenBlurBlendTime and FadeTime must be above 0".to_owned());
        }
        Ok(p)
    }
}

/// CG_UpdateShellShock's gate and the remaining time every user reads:
/// startTime != 0 and cg.time - startTime >= 0, remaining = startTime - cg.time + duration (ms,
/// not clamped; users test < 1). None when the shock has not started.
#[must_use]
pub fn shellshock_left_ms(scene_clock_ms: i32, began: i32, lasts: i32) -> Option<i32> {
    (began != 0 && scene_clock_ms.wrapping_sub(began) >= 0)
        .then(|| began.wrapping_sub(scene_clock_ms).wrapping_add(lasts))
}

/// CG_Flashbanged: time left and the shock is FLASHED (a BLURRED stun is not). No startTime test.
#[must_use]
pub fn cg_flashbanged(scene_clock_ms: i32, began: i32, lasts: i32, kind: ShockViewType) -> bool {
    began.wrapping_sub(scene_clock_ms).wrapping_add(lasts) > 0 && kind == ShockViewType::Flashed
}

/// CG_Flared: not immune (perk 0x1a on the player or his viewmodel weapon) and the snapshot
/// playerState's visionSetLerpRatio at 0.25 or more.
#[must_use]
pub fn cg_flared(player_flare_immune: bool, weapon_flare_immune: bool, vision_set_lerp_ratio: f32) -> bool {
    !player_flare_immune && !weapon_flare_immune && vision_set_lerp_ratio >= FLARED_LERP_RATIO
}

/// What crosshair, names, waypoints, hiddenDuringFlashbang menus and the UI's flashbanged()
/// treat as blind: flashed or flared.
#[must_use]
pub fn cg_blinded(flashbanged: bool, flared: bool) -> bool {
    flashbanged || flared
}

/// CG_InFlareVision (feeds the vision set on the refdef).
#[must_use]
pub fn cg_in_flare_vision(
    connection_state: i32,
    flare_disable_effects: bool,
    vision_set_lerp_ratio: f32,
    next_field: f32,
) -> bool {
    connection_state > FLARE_VISION_MIN_CONNECTION_STATE
        && !flare_disable_effects
        && (vision_set_lerp_ratio != 0.0 || next_field >= FLARE_VISION_NEXT_FIELD_MIN)
}

/// The flash curves' ease: (sin((x - 0.5) * pi) + 1) * 0.5, 0 at 0 and 1 at 1.
#[must_use]
pub fn shellshock_flash_fade(x: f32) -> f32 {
    (libm::sinf((x - 0.5) * core::f32::consts::PI) + 1.0) * 0.5
}

/// One fade time's linear fraction: 1 while remaining >= fade, else remaining / fade.
#[must_use]
pub fn shellshock_fade_fraction(ms_to_go: i32, fade_ms: i32) -> f32 {
    if ms_to_go >= fade_ms { 1.0 } else { ms_to_go as f32 / fade_ms as f32 }
}

/// What the FLASHED draw does this frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum FlashDraw {
    /// Nothing drawn; the saved-screen flag is cleared.
    Clear,
    /// First frame of the effect: grab the screen only, then set the saved-screen flag.
    SaveScreen,
    /// blendFlashedParam: the whiteout and frozen-screengrab intensities.
    Blend { whiteout: f32, screengrab: f32 },
}

/// The FLASHED draw: needs startTime != 0, duration > 0 and time left.
#[must_use]
pub fn shellshock_flash_draw(
    began_at: i32,
    length_ms: i32,
    ms_to_go: i32,
    blend: &ShellshockScreenBlend,
    has_saved_screen: bool,
) -> FlashDraw {
    if began_at == 0 || length_ms <= 0 || ms_to_go <= 0 {
        return FlashDraw::Clear;
    }
    if !has_saved_screen {
        return FlashDraw::SaveScreen;
    }
    FlashDraw::Blend {
        whiteout: shellshock_flash_fade(shellshock_fade_fraction(ms_to_go, blend.flash_white_fade_ms)),
        screengrab: shellshock_flash_fade(shellshock_fade_fraction(ms_to_go, blend.flash_shot_fade_ms)),
    }
}

/// RB_SaveScreen_BlendFlashed's vertex colour on the shellShockFlashed stretch-pic: whiteout in
/// R, G and B, screengrab in A, each round(x * 255).
#[must_use]
pub fn flashed_vertex_color(whiteout: f32, screengrab: f32) -> [u8; 4] {
    let byte = |x: f32| libm::roundf(x.clamp(0.0, 1.0) * 255.0) as u8;
    let w = byte(whiteout);
    [w, w, w, byte(screengrab)]
}

/// The BLURRED draw's blend time: blurredEffectTime, shortened in the last blurredFadeTime ms.
#[must_use]
pub fn shellshock_blur_effect_ms(ms_to_go: i32, blend: &ShellshockScreenBlend) -> i32 {
    if ms_to_go < blend.blurred_fade_ms {
        libm::roundf(ms_to_go as f32 / blend.blurred_fade_ms as f32 * blend.blurred_effect_ms as f32) as i32
    } else {
        blend.blurred_effect_ms
    }
}

/// Look control this frame: shellshock.sensitivity and the CL_CapTurnRate caps (0 = no cap).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ShellshockLook {
    pub touchiness: f32,
    pub pitch_cap: f32,
    pub yaw_cap: f32,
}

/// Look control ended: sensitivity 1, caps cleared.
pub const SHELLSHOCK_LOOK_ENDED: ShellshockLook =
    ShellshockLook { touchiness: FULL_FEEL, pitch_cap: NO_CAP, yaw_cap: NO_CAP };

/// Look control from the time left: full inside the fade window's start, then the sensitivity
/// eases back to 1 and the caps loosen (divided by the fade).
#[must_use]
pub fn shellshock_look_control(ms_to_go: Option<i32>, look: &ShellshockLookControl) -> ShellshockLook {
    let Some(remaining) = ms_to_go.filter(|_| look.touches) else { return SHELLSHOCK_LOOK_ENDED };
    let fade = if remaining < look.fade_for_ms {
        if remaining <= 0 {
            return SHELLSHOCK_LOOK_ENDED;
        }
        remaining as f32 / look.fade_for_ms as f32
    } else {
        1.0
    };
    let sensitivity = if fade == 1.0 { look.touchiness } else { (look.touchiness - 1.0) * fade + 1.0 };
    ShellshockLook {
        touchiness: sensitivity,
        pitch_cap: look.pitch_cap / fade,
        yaw_cap: look.yaw_cap / fade,
    }
}

/// The camera kick's amplitude: smoothstep(min(1, remaining / view.fadeTime)) * kickRadius;
/// None (viewDelta cleared) once remaining < 1.
#[must_use]
pub fn shellshock_kick_amplitude(ms_to_go: i32, view: &ShellshockView) -> Option<f32> {
    if ms_to_go < 1 {
        return None;
    }
    let x = (ms_to_go as f32 / view.fade_for_ms as f32).min(1.0);
    Some(x * x * (3.0 - 2.0 * x) * view.kick_radius)
}

/// The camera kick's phase (elapsed * kickRate) and its first perturbation index; the kick
/// interpolates cubically through the table from there.
/// TODO(fact): the table's contents and the exact cubic weights are not in the facts.
#[must_use]
pub fn shellshock_kick_phase(elapsed_ms: i32, lasts: i32, view: &ShellshockView) -> (f32, usize) {
    let phase = elapsed_ms as f32 * view.kick_rate;
    let index = lasts
        .wrapping_mul(PERTURBATION_DURATION_STRIDE)
        .wrapping_add(libm::floorf(phase) as i32)
        & (PERTURBATION_COUNT as i32 - 1);
    (phase, index as usize)
}

/// ShellShock(): the stored duration in ms. Seconds are 0..60; with the perk applied (third
/// argument 1 or absent) and the reduction perk, scaled by perk_shellShockReduction.
#[must_use]
pub fn shellshock_duration_ms(seconds: f32, apply_perk: bool, has_reduction_perk: bool, reduction: f32) -> Option<i32> {
    if !(0.0..=SHELLSHOCK_SECONDS_MAX).contains(&seconds) {
        return None;
    }
    let duration = ms(seconds);
    Some(if apply_perk && has_reduction_perk { libm::roundf(duration as f32 * reduction) as i32 } else { duration })
}

/// Flashbang blast radius floors: min >= 1, max >= min.
#[must_use]
pub fn flashbang_radii(max_radius: f32, min_radius: f32) -> (f32, f32) {
    let min = min_radius.max(1.0);
    (max_radius.max(min), min)
}

/// The "flashbang" notify's percent_distance: 1 inside the min radius, then falling to 0 at max.
#[must_use]
pub fn flashbang_distance_fraction(distance: f32, max_radius: f32, min_radius: f32) -> f32 {
    if distance <= min_radius { 1.0 } else { 1.0 - (distance - min_radius) / (max_radius - min_radius) }
}

/// The "flashbang" notify's percent_angle: (dot(view forward, unit eye-to-blast) + 1) / 2.
#[must_use]
pub fn flashbang_angle_fraction(eye_forward: [f32; 3], eye_to_blast_unit: [f32; 3]) -> f32 {
    let [fx, fy, fz] = eye_forward;
    let facing = fx * eye_to_blast_unit[0] + fy * eye_to_blast_unit[1] + fz * eye_to_blast_unit[2];
    0.5 * (facing + 1.0)
}

/// Damage flash amount while cg.time is before its end: |(end - time) * kick * 0.002|, at most 5.
#[must_use]
pub fn damage_flash_amount(scene_clock_ms: i32, finish_stamp: i32, kick: f32) -> Option<f32> {
    (scene_clock_ms < finish_stamp)
        .then(|| ((finish_stamp - scene_clock_ms) as f32 * kick * DAMAGE_FLASH_KICK_SCALE).abs().min(DAMAGE_FLASH_AMOUNT_MAX))
}

/// Damage flash colour: (0.2, 0, 0) at alpha amount * 0.2 * 0.7.
#[must_use]
pub fn damage_flash_color(amount: f32) -> [f32; 4] {
    let [r, g, b] = DAMAGE_FLASH_RGB;
    [r, g, b, amount * DAMAGE_FLASH_ALPHA_SCALE]
}

#[cfg(test)]
mod tests {
    use super::*;

    // BO2's common_mp shock/flashbang.shock and shock/concussion_grenade_mp.shock
    // (the lines that matter).
    const FLASHBANG: &str = r#"
bg_shock_screenType "flashed"
bg_shock_screenFlashWhiteFadeTime "3.5"
bg_shock_screenFlashShotFadeTime "1.0"
bg_shock_viewKickPeriod ".75"
bg_shock_viewKickRadius ".05"
bg_shock_viewKickFadeTime "3"
bg_shock_sound "1"
bg_shock_soundLoop "chr_flashbang_tinnitus_loop"
bg_shock_soundEnd ""
bg_shock_soundEndAbort ""
bg_shock_lookControl "0"
bg_shock_lookControl_maxpitchspeed "90"
bg_shock_lookControl_maxyawspeed "90"
bg_shock_lookControl_mousesensitivityscale "0.5"
bg_shock_lookControl_fadeTime "2"
bg_shock_movement "0.8"
bg_shock_visionset_name ""
bg_shock_visionset_inTime "0"
bg_shock_visionset_outTime "0"
"#;

    const CONCUSSION: &str = r#"
bg_shock_screenType "flashed"
bg_shock_screenFlashWhiteFadeTime "7.5"
bg_shock_screenFlashShotFadeTime "15"
bg_shock_viewKickPeriod ".75"
bg_shock_viewKickRadius ".1"
bg_shock_lookControl "1"
bg_shock_lookControl_maxpitchspeed "22"
bg_shock_lookControl_maxyawspeed "22"
bg_shock_lookControl_mousesensitivityscale "0.1"
bg_shock_lookControl_fadeTime "2"
bg_shock_movement "0.32"
bg_shock_animation "1"
bg_shock_visionset_name "concussion_grenade"
bg_shock_visionset_inTime "0"
bg_shock_visionset_outTime "3.5"
"#;

    #[test]
    fn shock_files_parse_over_the_dvar_defaults() {
        let f = ShellshockParms::parse(FLASHBANG).unwrap();
        assert_eq!(f.screen_blend.kind, ShockViewType::Flashed);
        assert_eq!(f.screen_blend.flash_white_fade_ms, 3500);
        assert_eq!(f.movement_scale, 0.8);
        assert_eq!(f.sound.end_alias, "");
        assert!(!f.animation && !f.look_control.touches);
        assert_eq!(f.view.fade_for_ms, VIEW_KICK_FADE_MS);
        assert!((f.view.kick_rate - 0.001 / 0.75).abs() < 1e-9);
        // Lines the file leaves out keep the defaults.
        assert_eq!(f.screen_blend.blurred_effect_ms, 400);
        assert_eq!(f.sound.loop_silent_alias, BG_SHOCK_SOUND_LOOP_SILENT_DEFAULT);
        let c = ShellshockParms::parse(CONCUSSION).unwrap();
        assert!(c.look_control.touches && c.animation);
        assert_eq!(c.look_control.yaw_cap, 22.0);
        assert_eq!(c.vision_set.name, "concussion_grenade");
        assert_eq!(c.vision_set.out_ms, 3500);
        assert!(ShellshockParms::parse("bg_shock_screenType \"sparkly\"").is_err());
        assert!(ShellshockParms::parse("bg_shock_screenBlurBlendTime \"0\"").is_err());
    }

    #[test]
    fn remaining_and_flashbanged() {
        assert_eq!(shellshock_left_ms(1500, 1000, 2000), Some(1500));
        assert_eq!(shellshock_left_ms(900, 1000, 2000), None);
        assert_eq!(shellshock_left_ms(1500, 0, 2000), None);
        assert!(cg_flashbanged(1500, 1000, 2000, ShockViewType::Flashed));
        assert!(!cg_flashbanged(1500, 1000, 2000, ShockViewType::Blurred));
        assert!(!cg_flashbanged(3000, 1000, 2000, ShockViewType::Flashed));
    }

    #[test]
    fn flared_and_blind() {
        assert!(cg_flared(false, false, 0.25));
        assert!(!cg_flared(false, false, 0.24));
        assert!(!cg_flared(true, false, 1.0));
        assert!(!cg_flared(false, true, 1.0));
        assert!(cg_blinded(false, true) && cg_blinded(true, false) && !cg_blinded(false, false));
        assert!(cg_in_flare_vision(10, false, 0.1, 0.0));
        assert!(!cg_in_flare_vision(9, false, 0.1, 0.0));
        assert!(!cg_in_flare_vision(10, true, 0.1, 0.0));
        assert!(cg_in_flare_vision(10, false, 0.0, 0.001));
    }

    #[test]
    fn flash_draw_grabs_first_then_blends() {
        let blend = ShellshockParms::parse(FLASHBANG).unwrap().screen_blend;
        assert_eq!(shellshock_flash_draw(1000, 2000, 500, &blend, false), FlashDraw::SaveScreen);
        assert_eq!(shellshock_flash_draw(0, 2000, 500, &blend, true), FlashDraw::Clear);
        assert_eq!(shellshock_flash_draw(1000, 2000, 0, &blend, true), FlashDraw::Clear);
        let FlashDraw::Blend { whiteout, screengrab } = shellshock_flash_draw(1000, 4000, 1750, &blend, true) else {
            panic!()
        };
        // Halfway through the white fade; past the shot fade window's start.
        assert!((whiteout - 0.5).abs() < 1e-4 && screengrab == 1.0);
        assert_eq!(flashed_vertex_color(0.5, 1.0), [128, 128, 128, 255]);
        assert_eq!(shellshock_flash_fade(0.0), 0.0);
    }

    #[test]
    fn blur_time_shortens_in_the_fade() {
        let blend = ShellshockParms::default().screen_blend;
        assert_eq!(shellshock_blur_effect_ms(5000, &blend), 400);
        assert_eq!(shellshock_blur_effect_ms(500, &blend), 200);
    }

    #[test]
    fn look_control_eases_out() {
        let look = ShellshockParms::parse(CONCUSSION).unwrap().look_control;
        let full = shellshock_look_control(Some(5000), &look);
        assert_eq!(full, ShellshockLook { touchiness: 0.1, pitch_cap: 22.0, yaw_cap: 22.0 });
        let half = shellshock_look_control(Some(1000), &look);
        assert!((half.touchiness - 0.55).abs() < 1e-5 && half.yaw_cap == 44.0);
        assert_eq!(shellshock_look_control(Some(0), &look), SHELLSHOCK_LOOK_ENDED);
        assert_eq!(shellshock_look_control(None, &look), SHELLSHOCK_LOOK_ENDED);
        let off = ShellshockParms::parse(FLASHBANG).unwrap().look_control;
        assert_eq!(shellshock_look_control(Some(5000), &off), SHELLSHOCK_LOOK_ENDED);
    }

    #[test]
    fn kick_fades_with_smoothstep() {
        let view = ShellshockParms::parse(CONCUSSION).unwrap().view;
        assert_eq!(shellshock_kick_amplitude(0, &view), None);
        assert!((shellshock_kick_amplitude(6000, &view).unwrap() - 0.1).abs() < 1e-6);
        assert!((shellshock_kick_amplitude(1500, &view).unwrap() - 0.05).abs() < 1e-6);
        let (phase, index) = shellshock_kick_phase(1500, 2, &view);
        assert!((phase - 2.0).abs() < 1e-4);
        assert_eq!(index, (2 * 61 + 2) & 0x7f);
    }

    #[test]
    fn server_side_rules() {
        assert_eq!(shellshock_duration_ms(4.0, true, true, 0.5), Some(2000));
        assert_eq!(shellshock_duration_ms(4.0, false, true, 0.5), Some(4000));
        assert_eq!(shellshock_duration_ms(61.0, true, false, 0.5), None);
        assert_eq!(flashbang_radii(0.0, 0.0), (1.0, 1.0));
        assert_eq!(flashbang_distance_fraction(100.0, 800.0, 200.0), 1.0);
        assert!((flashbang_distance_fraction(500.0, 800.0, 200.0) - 0.5).abs() < 1e-6);
        assert_eq!(flashbang_angle_fraction([1.0, 0.0, 0.0], [-1.0, 0.0, 0.0]), 0.0);
        assert_eq!(flashbang_angle_fraction([1.0, 0.0, 0.0], [1.0, 0.0, 0.0]), 1.0);
        assert_eq!(damage_flash_amount(1000, 1000, 10.0), None);
        assert_eq!(damage_flash_amount(0, 1000, 10.0), Some(5.0));
        assert!((damage_flash_color(5.0)[3] - 0.7).abs() < 1e-6);
    }
}
