//! Window 10: aiming down the sights.
//!
//! Server/predicted side (runs inside Pmove): the ADS flag, the aim-in/out lerp, hold breath,
//! anti-quickscope, variable zoom select. Client side ([`AdsView`]): the in/out direction latch
//! that picks the zoom fraction, the scope picture, the zoom FOV and the SCOPE_RAISE_CLIP / SCOPE_LOWER_CLIP
//! scrub. Timing spec with every number: `docs/gun/ADS.md`.

use super::Pm;
use crate::cmd::button;
use crate::env::pmf;
use crate::facts::{fire_type, gun_kind_ids};
use crate::player::{T6WeaponPs, wf};
use crate::state;
use fastfile_t6::layout::{WeaponDef, WeaponVariantDef};

/// `SIGNAL_SCOPE_RESTART`.
pub const SIGNAL_SCOPE_RESTART: u16 = 0x11;
/// The variable-zoom step event.
pub const EV_ADS_ZOOM_CHANGE: u16 = 0xdb;

/// View model anims the client scrubs by the aim fraction.
pub mod aim_clips {
    pub const SCOPE_RAISE_CLIP: i32 = 0x55;
    pub const SCOPE_LOWER_CLIP: i32 = 0x56;
    /// SCOPE_RAISE_CLIP's slot for a gun in its alt-ADS-only (Hybrid) mode.
    pub const ALT_ADS_UP: i32 = 0x57;
}

/// `weapOverlayReticle_t` 0: no scope picture.
pub const OVERLAY_NONE: i32 = 0;
/// `ATTACHMENT_VZOOM` (`eAttachment`): Variable Zoom.
pub const ATTACHMENT_VZOOM: i32 = 29;

/// Quick Draw (`perksEnum` 15): aim time x `perk_weapAdsMultiplier`.
const PERK_FASTADS: u32 = 15;
/// Extra breath (`perksEnum` 30).
const PERK_EXTRA_BREATH: u32 = 30;
/// `playerAnimType` of the sniper and sniper_rearclip anim sets: Quick Draw never speeds these.
const ANIM_TYPE_SNIPER: i32 = 3;
const ANIM_TYPE_SNIPER_REARCLIP: i32 = 17;
/// `eFlags2`: ADS forced off (frac drops to 0 at once).
const EF2_NO_ADS: u32 = 0x4_0000;
/// `linkFlags`: linked but the weapon still works; the frac snaps to the flag.
const LF_WEAPON_ALLOWED: u32 = 0x4;
/// `eFlags` / `eFlags2` pair that lets a no-clip / spectator / dead player aim.
const EF_VEHICLE: u32 = 0x4000;
const EF2_WEAPON_LOCKED: u32 = 0x1000_0000;
/// `otherFlags`: the weapon is usable at all.
const OF_WEAPON_USABLE: u32 = 0x4;
/// `weapFlags` 0x400: a stacked-fire gun mid-volley (keeps an ADS-fire-only gun aimed).
const WF_STACKED_FIRING: u64 = 0x400;
/// Water depth past which weapons (and ADS) go away with `player_disableWeaponsInWater`.
const WATER_UNDER: i32 = 3;
/// No `iAdsTransInTime` / `iAdsTransOutTime`: these times (ms).
const DEFAULT_IN_TIME: i32 = 300;
const DEFAULT_OUT_TIME: i32 = 500;
/// The zoom-select latch (ms).
const ZOOM_LATCH_MS: i32 = 40;

/// The dvars this file reads, with BO2's registered defaults (the MP scripts never change them).
pub mod dvar {
    pub const PLAYER_ADS_EXIT_DELAY: i32 = 0;
    pub const PLAYER_BREATH_HOLD_TIME: f32 = 4.5;
    pub const PLAYER_BREATH_GASP_TIME: f32 = 0.0;
    pub const PLAYER_BREATH_GASP_SCALE: f32 = 2.5;
    pub const PLAYER_BREATH_GASP_LERP: f32 = 4.0;
    pub const PLAYER_BREATH_HOLD_LERP: f32 = 1.5;
    pub const PLAYER_BREATH_FIRE_DELAY: f32 = 0.0;
    pub const PERK_EXTRA_BREATH: f32 = 5.0;
    pub const BG_AQS: bool = true;
    pub const BG_AQS_STYLE: i32 = 2;
    pub const CG_USE_WEAPON_BASED_VARIABLE_ZOOM: bool = true;
    /// Client dvar, not registered in the server build we read: 0 = four zoom steps.
    pub const CG_ADS_ZOOM_TOGGLE_STYLE: i32 = 0;
}

fn rd_i32(b: &[u8], at: usize) -> i32 {
    b.get(at..at + 4)
        .map(|s| i32::from_le_bytes([s[0], s[1], s[2], s[3]]))
        .unwrap_or(0)
}

fn rd_f32(b: &[u8], at: usize) -> f32 {
    f32::from_bits(rd_i32(b, at) as u32)
}

fn rd_bool(b: &[u8], off: usize) -> bool {
    b.get(off).is_some_and(|&x| x != 0)
}

/// Window 10's per-weapon facts.
///
/// `in_scale` / `out_scale`, `variable_zoom`, the zoom fractions and fields of view start as the
/// gun's own values; a `<gun>+<att>` row's attachments change them through `fold_in_addons`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct AdsFacts {
    pub scoped_aiming: bool,
    /// `iAdsTransInTime` / `iAdsTransOutTime` (ms).
    pub in_time: i32,
    pub out_time: i32,
    /// `BG_GetADSTransTimeScales`: 1 with no attachments.
    pub in_scale: f32,
    pub out_scale: f32,
    pub zoom_in_frac: f32,
    pub zoom_out_frac: f32,
    pub crosshair_in_frac: f32,
    pub crosshair_out_frac: f32,
    /// `fAdsZoomFov1..3`.
    pub zoom_fov: [f32; 3],
    pub hud_crosshair_layer: i32,
    /// The variant names an overlay material (a scope picture without an overlay reticle).
    pub hud_layer_surface: bool,
    pub hold_breath_to_steady: bool,
    pub fire_only_when_scoped: bool,
    pub block_aim_if_clip_dry: bool,
    pub reload_while_ads: bool,
    pub rack_bolt_when_scoped: bool,
    pub position_reload_trans_time: i32,
    pub staged_reload: bool,
    pub anti_quick_scope: bool,
    pub aqs_time: f32,
    pub aqs_scale: f32,
    pub aqs_spread_multiplier: f32,
    pub aqs_spread_max: f32,
    pub aqs_sway_factor: f32,
    /// `altScopeADSTransInTime` / `OutTime` (s): the Hybrid optic FOV blend.
    pub alt_in_time: f32,
    pub alt_out_time: f32,
    pub hip_idle: f32,
    pub ads_idle: f32,
    pub airburst: bool,
    pub player_motion_kind: i32,
    /// Has the variable-zoom attachment.
    pub variable_zoom: bool,
}

impl AdsFacts {
    pub fn read(variant: &[u8], def: &[u8]) -> Self {
        Self {
            scoped_aiming: rd_bool(def, WeaponDef::aimDownSight),
            in_time: rd_i32(variant, WeaponVariantDef::iAdsTransInTime),
            out_time: rd_i32(variant, WeaponVariantDef::iAdsTransOutTime),
            in_scale: 1.0,
            out_scale: 1.0,
            zoom_in_frac: rd_f32(variant, WeaponVariantDef::fAdsZoomInFrac),
            zoom_out_frac: rd_f32(variant, WeaponVariantDef::fAdsZoomOutFrac),
            crosshair_in_frac: rd_f32(def, WeaponDef::fAdsCrosshairInFrac),
            crosshair_out_frac: rd_f32(def, WeaponDef::fAdsCrosshairOutFrac),
            zoom_fov: [
                rd_f32(variant, WeaponVariantDef::fAdsZoomFov1),
                rd_f32(variant, WeaponVariantDef::fAdsZoomFov2),
                rd_f32(variant, WeaponVariantDef::fAdsZoomFov3),
            ],
            hud_crosshair_layer: rd_i32(def, WeaponDef::overlayReticle),
            hud_layer_surface: rd_i32(variant, WeaponVariantDef::overlayMaterial) != 0,
            hold_breath_to_steady: rd_bool(def, WeaponDef::bHoldBreathToSteady),
            fire_only_when_scoped: rd_bool(def, WeaponDef::adsFireOnly),
            block_aim_if_clip_dry: rd_bool(def, WeaponDef::noAdsWhenMagEmpty),
            reload_while_ads: rd_bool(def, WeaponDef::bReloadWhileAds),
            rack_bolt_when_scoped: rd_bool(def, WeaponDef::bRechamberWhileAds),
            position_reload_trans_time: rd_i32(def, WeaponDef::iPositionReloadTransTime),
            staged_reload: rd_bool(def, WeaponDef::bSegmentedReload),
            anti_quick_scope: rd_bool(variant, WeaponVariantDef::bAntiQuickScope),
            aqs_time: rd_f32(def, WeaponDef::fAntiQuickScopeTime),
            aqs_scale: rd_f32(def, WeaponDef::fAntiQuickScopeScale),
            aqs_spread_multiplier: rd_f32(def, WeaponDef::fAntiQuickScopeSpreadMultiplier),
            aqs_spread_max: rd_f32(def, WeaponDef::fAntiQuickScopeSpreadMax),
            aqs_sway_factor: rd_f32(def, WeaponDef::fAntiQuickScopeSwayFactor),
            alt_in_time: rd_f32(def, WeaponDef::altScopeADSTransInTime),
            alt_out_time: rd_f32(def, WeaponDef::altScopeADSTransOutTime),
            hip_idle: rd_f32(def, WeaponDef::fHipIdleAmount),
            ads_idle: rd_f32(def, WeaponDef::fAdsIdleAmount),
            airburst: rd_bool(def, WeaponDef::bAirburstWeapon),
            player_motion_kind: rd_i32(def, WeaponDef::playerAnimType),
            variable_zoom: false,
        }
    }

    /// The attachments' share (up to 3, against the default attachment): BG_GetADSTransTimeScales
    /// (product of the time scales; BO2 asserts each is above 0, so a blank product leaves 1),
    /// BG_GetADSZoomInFrac / OutFrac (the first attachment whose value differs wins),
    /// BG_GetADSZoomFOV (each field of view separately: the last differing attachment wins) and
    /// BG_WeaponHasAttachment(VZOOM) for the variable-zoom key.
    pub fn fold_in_addons(&mut self, atts: &[&[u8]], default: &[u8], scale_in: f32, out_scale: f32) {
        use fastfile_t6::layout::WeaponAttachment;
        let atts = &atts[..atts.len().min(3)];
        if scale_in > 0.0 {
            self.in_scale = scale_in;
        }
        if out_scale > 0.0 {
            self.out_scale = out_scale;
        }
        let first = |at: usize, own: f32| {
            atts.iter()
                .map(|a| rd_f32(a, at))
                .find(|&x| x != rd_f32(default, at))
                .unwrap_or(own)
        };
        self.zoom_in_frac = first(WeaponAttachment::fAdsZoomInFrac, self.zoom_in_frac);
        self.zoom_out_frac = first(WeaponAttachment::fAdsZoomOutFrac, self.zoom_out_frac);
        for (fov, at) in self.zoom_fov.iter_mut().zip([WeaponAttachment::fAdsZoomFov1, WeaponAttachment::fAdsZoomFov2, WeaponAttachment::fAdsZoomFov3]) {
            for a in atts {
                if rd_f32(a, at) != rd_f32(default, at) {
                    *fov = rd_f32(a, at);
                }
            }
        }
        self.variable_zoom |= atts.iter().any(|a| rd_i32(a, WeaponAttachment::attachmentType) == ATTACHMENT_VZOOM);
    }

    /// Aim-in speed, frac per ms, before scales and perks.
    pub fn in_rate(&self) -> f32 {
        1.0 / (if self.in_time > 0 { self.in_time } else { DEFAULT_IN_TIME }) as f32
    }

    /// Aim-out speed, frac per ms, before scales and perks.
    pub fn out_rate(&self) -> f32 {
        1.0 / (if self.out_time > 0 { self.out_time } else { DEFAULT_OUT_TIME }) as f32
    }

    /// The gun has a scope picture.
    pub fn has_overlay(&self) -> bool {
        self.hud_crosshair_layer != OVERLAY_NONE || self.hud_layer_surface
    }

    /// Quick Draw leaves sniper anim sets alone.
    pub fn quick_draw_applies(&self) -> bool {
        self.player_motion_kind != ANIM_TYPE_SNIPER && self.player_motion_kind != ANIM_TYPE_SNIPER_REARCLIP
    }

    /// `fAntiQuickScopeTime` in whole ms (0 = no anti-quickscope).
    pub fn aqs_ms(&self) -> i32 {
        (self.aqs_time * 1000.0) as i32
    }
}

/// BO2's DiffTrack: ease `cur` toward `tgt` at `rate` per second; snaps when close or overshooting.
pub fn diff_track(tgt: f32, cur: f32, rate: f32, secs: f32) -> f32 {
    let step = (tgt - cur) * rate * secs;
    let gap = (tgt - cur).abs();
    if gap > 0.001 && step.abs() <= gap { cur + step } else { tgt }
}

/// PM_IsAdsAllowed, for the gun in view (`ps.weapon`).
pub fn is_ads_allowed(pm: &Pm) -> bool {
    use crate::env::movement_mode;
    let env = &*pm.env;
    // Linked / no-clip / spectator / dead: a weapon-allowed link aims; otherwise only a vehicle
    // gunner with its weapon locked may. (BO2's Wii U pointer refusal never applies on PC.)
    let ghost = matches!(
        env.movement_mode,
        movement_mode::NOCLIP | movement_mode::UFO | movement_mode::SPECTATOR | movement_mode::INTERMISSION | movement_mode::DEAD | movement_mode::DEAD_LINKED
    );
    if (ghost || env.movement_mode == movement_mode::NORMAL_LINKED) && env.attach_bits & LF_WEAPON_ALLOWED != 0 {
        return true;
    }
    if ghost && (env.ent_bits & EF_VEHICLE == 0 || env.e_flags2 & EF2_WEAPON_LOCKED == 0) {
        return false;
    }
    if env.secondary_state_bits & OF_WEAPON_USABLE == 0 {
        return false;
    }
    let f = pm.weapon_facts();
    if !f.ads.scoped_aiming {
        return false;
    }
    if env.dvars.player_disable_weapons_in_water
        && env.waterlevel > WATER_UNDER
        && env.movement_mode != movement_mode::NORMAL_LINKED
    {
        return false;
    }
    let st = pm.ps.gun_phase;
    if (state::OFFHAND_INIT..=state::RIOTSHIELD_END).contains(&st) {
        return false;
    }
    match st {
        state::RAISING | state::DROPPING | state::DROPPING_QUICK => return false,
        state::RAISING_ALTSWITCH | state::DROPPING_ALTSWITCH if !f.core.alt_weapon_ads_only => {
            return false;
        }
        _ => {}
    }
    !(state::DTP_IN..=state::DTP_OUT).contains(&st)
        && env.movement_state_bits & pmf::MANTLE == 0
        && pm.ps.weapon_state_bits & wf::AIM_DISABLED_BIT == 0
        && !(f.ads.block_aim_if_clip_dry && pm.ps.ammo_in_clip(pm.facts, pm.ps.weapon) == 0)
}

/// PM_UpdateAimDownSightFlag: once per Pmove, before sprint and both weapon passes.
pub fn update_ads_flag(pm: &mut Pm) {
    pm.env.movement_state_bits &= !pmf::ADS;
    let mut allowed = is_ads_allowed(pm);
    let cmd = *pm.cmd;
    let a = pm.weapon_facts().ads;
    // PC: the sprint key drops ADS, unless toggle-ADS is on or a scoped gun is holding breath.
    if !pm.env.is_console
        && cmd.held(button::SPRINT)
        && !cmd.held(button::TOGGLE_ADS)
        && (a.hud_crosshair_layer == OVERLAY_NONE || !cmd.held(button::STEADY_AIM_HOLD_BIT))
    {
        pm.add_event(SIGNAL_SCOPE_RESTART);
        pm.env.movement_state_bits &= !pmf::ADS;
        allowed = false;
    }
    if !cmd.held(button::AIM_BIT) || !allowed {
        return;
    }
    if pm.env.movement_state_bits & pmf::LYING_FLAT == 0
        || (a.hud_crosshair_layer != OVERLAY_NONE && pm.ps.weapon_raise_fraction >= 1.0)
    {
        pm.env.movement_state_bits |= pmf::ADS;
    } else {
        // Prone: a fresh press always aims; a held one only while not crawling.
        if cmd.last_tick_buttons & button::AIM_BIT != 0 && (cmd.thrust_ahead != 0 || cmd.thrust_lateral != 0) {
            return;
        }
        pm.env.movement_state_bits |= pmf::ADS | pmf::PRONE_MOVE_OVERRIDE;
    }
}

/// PM_ExitAimDownSight.
pub fn exit_ads(pm: &mut Pm) {
    pm.add_event(SIGNAL_SCOPE_RESTART);
    pm.env.movement_state_bits &= !pmf::ADS;
}

/// PM_UpdateAimDownSightLerp: move `fWeaponPosFrac` toward the ADS flag.
pub fn update_ads_lerp(pm: &mut Pm) {
    let f = pm.weapon_facts();
    let a = f.ads;
    let ps_flags = pm.env.movement_state_bits;
    if pm.env.e_flags2 & EF2_NO_ADS != 0 && pm.ps.weapon_raise_fraction > 0.0 {
        exit_ads(pm);
        pm.ps.weapon_raise_fraction = 0.0;
        pm.ps.scope_in_wait = 0;
        return;
    }
    if pm.env.attach_bits & LF_WEAPON_ALLOWED != 0 {
        pm.ps.weapon_raise_fraction = if ps_flags & pmf::ADS != 0 { 1.0 } else { 0.0 };
        pm.ps.scope_in_wait = 0;
        return;
    }
    // (player_scopeExitOnDamage is off in BO2: taking damage never drops the scope.)
    if !a.scoped_aiming {
        pm.ps.weapon_raise_fraction = 0.0;
        pm.ps.scope_in_wait = 0;
        return;
    }

    let st = pm.ps.gun_phase;
    let past_trans = pm.ps.gun_timer_ms > a.position_reload_trans_time;
    let reload_blocks = !a.reload_while_ads
        && if a.staged_reload {
            (state::RELOADING..=state::RELOAD_START_INTERUPT).contains(&st)
                || (st == state::RELOAD_END && past_trans)
        } else {
            st == state::RELOADING && past_trans
        };
    let rechamber_blocks =
        !a.rack_bolt_when_scoped && (st == state::RECHAMBER_START || st == state::RECHAMBERING);
    let mut want = !reload_blocks && !rechamber_blocks && ps_flags & pmf::ADS != 0;
    if a.fire_only_when_scoped
        && (pm.ps.weapon_ready_wait != 0
            || (f.core.fire_type == fire_type::STACKED && pm.ps.weapon_state_bits & WF_STACKED_FIRING != 0))
        && (st == state::FIRING || st == state::CONT_FIRE_LOOP)
    {
        want = true;
    }

    let frac = pm.ps.weapon_raise_fraction;
    let exit_delay = dvar::PLAYER_ADS_EXIT_DELAY;
    if frac == 1.0 && !want && exit_delay > 0 {
        if pm.ps.scope_in_wait == 0 {
            pm.ps.scope_in_wait = pm.cmd.authority_clock_ms + exit_delay;
        }
        if pm.cmd.authority_clock_ms < pm.ps.scope_in_wait {
            return;
        }
        pm.ps.scope_in_wait = 0;
    } else {
        pm.ps.scope_in_wait = 0;
        if (want && frac == 1.0) || (!want && frac == 0.0) {
            return;
        }
    }

    let mut rate = if want {
        a.in_rate() / a.in_scale
    } else {
        -a.out_rate() / a.out_scale
    };
    if pm.has_perk(PERK_FASTADS) && a.quick_draw_applies() {
        rate /= pm.env.dvars.perk_weap_ads_multiplier;
    }
    pm.ps.weapon_raise_fraction = (pm.cmd.frame_ms as f32 * rate + frac).clamp(0.0, 1.0);
}

/// PM_UpdateScopeZoom: the variable-zoom key steps the held gun's zoom when fully in.
pub fn update_scope_zoom(pm: &mut Pm) {
    let a = pm.weapon_facts().ads;
    if !a.variable_zoom {
        return;
    }
    let now = pm.cmd.authority_clock_ms;
    let pressed = pm.cmd.held(button::CHANGE_ZOOM);
    let since = now - pm.ps.ads_zoom_latch_time;
    let weapon = pm.ps.weapon;
    if !pm.ps.ads_zoom_latch_state && since > ZOOM_LATCH_MS && pressed && pm.ps.weapon_raise_fraction >= 1.0 {
        pm.ps.ads_prev_zoom_select = pm.ps.held(weapon).map_or(0, |h| h.ads_zoom_select);
        pm.ps.ads_prev_zoom_select_time = now;
        if dvar::CG_USE_WEAPON_BASED_VARIABLE_ZOOM {
            let Some(h) = pm.ps.held_mut(weapon) else {
                return;
            };
            let steps = if dvar::CG_ADS_ZOOM_TOGGLE_STYLE != 0 { 2 } else { 4 };
            h.ads_zoom_select = (h.ads_zoom_select + 1).rem_euclid(steps);
        } else {
            pm.ps.ads_zoom_select += 1;
        }
        pm.ps.ads_zoom_latch_state = true;
        pm.ps.ads_zoom_latch_time = now;
        pm.add_event(EV_ADS_ZOOM_CHANGE);
    } else if since > ZOOM_LATCH_MS && !pressed && using_sniper_scope(pm.ps, &a) {
        pm.ps.ads_zoom_latch_state = false;
        pm.ps.ads_zoom_latch_time = now;
    }
}

/// BG_UsingSniperScope.
pub fn using_sniper_scope(ps: &T6WeaponPs, a: &AdsFacts) -> bool {
    a.hud_crosshair_layer != OVERLAY_NONE && ps.weapon_raise_fraction >= 1.0
}

fn breath_hold_ms(pm: &Pm) -> i32 {
    let mut hold = (dvar::PLAYER_BREATH_HOLD_TIME * 1000.0) as i32;
    if pm.has_perk(PERK_EXTRA_BREATH) {
        hold += (dvar::PERK_EXTRA_BREATH * 1000.0) as i32;
    }
    hold
}

/// PM_UpdateHoldBreath.
pub fn update_hold_breath(pm: &mut Pm) {
    let f = pm.weapon_facts();
    let hold = breath_hold_ms(pm);
    let gasp = (dvar::PLAYER_BREATH_GASP_TIME * 1000.0) as i32;
    if hold < 1 || f.ads.airburst {
        pm.ps.weapon_state_bits &= !wf::HOLDING_BREATH;
        pm.ps.steady_aim_factor = 1.0;
        pm.ps.hold_breath_timer = 0;
        return;
    }
    let ps = &mut *pm.ps;
    if ps.weapon_raise_fraction == 1.0
        && f.core.weap_class != gun_kind_ids::ITEM
        && f.ads.hold_breath_to_steady
        && pm.cmd.held(button::STEADY_AIM_HOLD_BIT)
    {
        if ps.quick_scope_timer > 0 && dvar::BG_AQS {
            ps.weapon_state_bits &= !wf::HOLDING_BREATH;
        } else if ps.hold_breath_timer == 0 {
            ps.weapon_state_bits |= wf::HOLDING_BREATH;
        }
    } else {
        ps.weapon_state_bits &= !wf::HOLDING_BREATH;
    }
    let holding = ps.weapon_state_bits & wf::HOLDING_BREATH != 0;
    ps.hold_breath_timer += if holding { pm.cmd.frame_ms } else { -pm.cmd.frame_ms };
    ps.hold_breath_timer = ps.hold_breath_timer.max(0);
    if holding && ps.hold_breath_timer > hold {
        ps.hold_breath_timer = gasp + hold;
        ps.weapon_state_bits &= !wf::HOLDING_BREATH;
    }
    let (goal, speed) = if ps.weapon_state_bits & wf::HOLDING_BREATH == 0 {
        let t = (dvar::PLAYER_BREATH_GASP_SCALE - 1.0)
            * (ps.hold_breath_timer as f32 / (gasp + hold) as f32)
            + 1.0;
        (t, dvar::PLAYER_BREATH_GASP_LERP)
    } else {
        (0.0, dvar::PLAYER_BREATH_HOLD_LERP)
    };
    let dt = pm.cmd.frame_ms as f32 * 0.001;
    ps.steady_aim_factor = diff_track(
        (goal - 1.0) * ps.weapon_raise_fraction + 1.0,
        ps.steady_aim_factor,
        speed,
        dt,
    );
}

/// PM_HoldBreathFire: firing while scoped ends a held breath. Called right after the fire event.
pub fn hold_breath_fire(pm: &mut Pm) {
    let f = pm.weapon_facts();
    if pm.ps.weapon_raise_fraction != 1.0
        || f.ads.hud_crosshair_layer == OVERLAY_NONE
        || f.core.weap_class == gun_kind_ids::ITEM
    {
        return;
    }
    // BO2 caps at the plain hold time here (no extra-breath perk).
    let hold = (dvar::PLAYER_BREATH_HOLD_TIME * 1000.0) as i32;
    let delay = (dvar::PLAYER_BREATH_FIRE_DELAY * 1000.0) as i32;
    if pm.ps.hold_breath_timer < hold {
        pm.ps.hold_breath_timer = (pm.ps.hold_breath_timer + delay).min(hold);
    }
    pm.ps.weapon_state_bits &= !wf::HOLDING_BREATH;
}

/// PM_UpdateQuickScope. `prev_frac` is `fWeaponPosFrac` from before this tick's lerp.
pub fn update_quickscope(pm: &mut Pm, prev_frac: f32) {
    let a = pm.weapon_facts().ads;
    let t = a.aqs_ms();
    let ps = &mut *pm.ps;
    if t < 1 || !a.anti_quick_scope {
        ps.quick_scope_timer = 0;
        ps.quick_scope_scale = 1.0;
        return;
    }
    let frac = ps.weapon_raise_fraction;
    ps.quick_scope_timer = if prev_frac > frac || frac < 0.001 {
        0
    } else if frac < 1.0 {
        t
    } else {
        ps.quick_scope_timer - pm.cmd.frame_ms
    }
    .max(0);
    let target = (ps.quick_scope_timer as f32 / t as f32) * (a.aqs_scale - 1.0) * frac + 1.0;
    let dt = pm.cmd.frame_ms as f32 * 0.001;
    let s = diff_track(target, ps.quick_scope_scale, dvar::PLAYER_BREATH_GASP_LERP, dt);
    ps.quick_scope_scale = if s < 1.0 { 1.0 } else { s };
}

/// A shot now counts as quickscoped: the scale is up and the spread is still under the cap
/// (window 12 adds the spread itself).
pub fn quickscope_spread_active(ps: &T6WeaponPs, a: &AdsFacts) -> bool {
    dvar::BG_AQS && ps.quick_scope_scale > 1.0 && ps.sight_spread_factor <= a.aqs_spread_max
}

/// The extra idle-sway wobble a quickscope adds (bg_aqsStyle 2), for window 12's sway.
pub fn quickscope_sway(ps: &T6WeaponPs, a: &AdsFacts) -> f32 {
    if dvar::BG_AQS && dvar::BG_AQS_STYLE == 2 {
        (ps.quick_scope_scale - 1.0) * a.aqs_sway_factor * 0.01
    } else {
        0.0
    }
}

/// Airburst range lasing. No MP gun is an airburst gun, so this never runs.
pub fn update_airburst_lase(pm: &mut Pm) {
    let _ = pm;
}

/// Airburst mark. No MP gun is an airburst gun, so this never runs.
pub fn check_for_airburst_mark(pm: &mut Pm) {
    let _ = pm;
}

/// Trace columns: aim frac, quickscope timer/scale, breath timer/scale. Empty while the gun is
/// at the hip with nothing running, so non-ADS traces stay as they were.
pub fn trace_columns(ps: &T6WeaponPs) -> String {
    let holding = ps.weapon_state_bits & wf::HOLDING_BREATH != 0;
    if ps.weapon_raise_fraction == 0.0
        && ps.quick_scope_timer == 0
        && ps.hold_breath_timer == 0
        && !holding
        && ps.quick_scope_scale == 1.0
        && ps.steady_aim_factor == 1.0
    {
        return String::new();
    }
    format!(
        "frac {:.4} qs {} {:.3} breath {} {:.3}{}",
        ps.weapon_raise_fraction,
        ps.quick_scope_timer,
        ps.quick_scope_scale,
        ps.hold_breath_timer,
        ps.steady_aim_factor,
        if holding { " held" } else { "" },
    )
}

/// What the client draws for the aim this frame.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct AdsFrame {
    /// Zoom progress 0..1 (the scope / FOV window of the lerp).
    pub zoom_frac: f32,
    /// Crosshair fade 0..1.
    pub crosshair: f32,
    /// The scope picture is drawn (and the view model hidden).
    pub overlay: bool,
    pub view_fov: f32,
    /// The view model anim being scrubbed and its time (0..1).
    pub anim: i32,
    pub anim_time: f32,
}

/// Client-side aim state (cg): the in/out direction latch and the Hybrid optic blend.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AdsView {
    /// `bPositionToADS`: aiming in since the gun was last fully at the hip.
    pub to_ads: bool,
    /// Hybrid optic blend 0..1 toward the alt optic.
    pub alt_blend: f32,
}

impl Default for AdsView {
    fn default() -> Self {
        Self {
            to_ads: true,
            alt_blend: 0.0,
        }
    }
}

/// `z = (frac - (1 - f)) / f`, 0 when not positive, at most 1.
fn zoom_window(frac: f32, f: f32) -> f32 {
    if frac == 0.0 {
        return 0.0;
    }
    let z = frac - (1.0 - f);
    if z > 0.0 { (z / f).min(1.0) } else { 0.0 }
}

impl AdsView {
    /// One client frame. `frac` is the predicted `fWeaponPosFrac`, `hip_fov` the view FOV
    /// without aiming, `zoom_select` the held gun's zoom step, `alt_ads_mode` the gun is the
    /// alt-ADS-only half of a Hybrid pair, `dt` the frame time (s).
    pub fn frame(
        &mut self,
        a: &AdsFacts,
        frac: f32,
        hip_fov: f32,
        zoom_select: i32,
        alt_ads_mode: bool,
        dt: f32,
    ) -> AdsFrame {
        if frac == 1.0 {
            self.to_ads = false;
        } else if frac == 0.0 {
            self.to_ads = true;
        }
        let step = dt.min(0.1);
        let (len, goal) = if alt_ads_mode { (a.alt_in_time, 1.0) } else { (a.alt_out_time, 0.0) };
        self.alt_blend = if len > 0.0 {
            let d = step / len;
            if goal > self.alt_blend { (self.alt_blend + d).min(goal) } else { (self.alt_blend - d).max(goal) }
        } else {
            goal
        };

        let zf = if self.to_ads { a.zoom_in_frac } else { a.zoom_out_frac };
        let cf = if self.to_ads { a.crosshair_in_frac } else { a.crosshair_out_frac };
        let zoom = zoom_window(frac, zf);
        let i = zoom_select.clamp(0, 2) as usize;
        let zoom_fov = a.zoom_fov[i] + (a.zoom_fov[0] - a.zoom_fov[i]) * self.alt_blend;
        let fov = if frac == 1.0 {
            zoom_fov
        } else if zoom > 0.0 {
            hip_fov + (zoom_fov - hip_fov) * zoom
        } else {
            hip_fov
        };
        let (anim, anim_time) = if self.to_ads {
            (if alt_ads_mode { aim_clips::ALT_ADS_UP } else { aim_clips::SCOPE_RAISE_CLIP }, frac)
        } else {
            (aim_clips::SCOPE_LOWER_CLIP, 1.0 - frac)
        };
        AdsFrame {
            zoom_frac: zoom,
            crosshair: zoom_window(frac, cf),
            overlay: a.has_overlay() && zoom > 0.01,
            view_fov: fov,
            anim,
            anim_time,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cmd::T6Cmd;
    use crate::env::T6Env;
    use crate::facts::{FactsTable, T6WeaponFacts};
    use crate::pm::FrameOut;

    const GUN: u32 = 1;

    fn gun(in_time: i32, out_time: i32, zin: f32, zout: f32) -> T6WeaponFacts {
        let mut f = T6WeaponFacts::default();
        f.core.magazine_capacity = 10;
        f.ads = AdsFacts {
            scoped_aiming: true,
            in_time,
            out_time,
            in_scale: 1.0,
            out_scale: 1.0,
            zoom_in_frac: zin,
            zoom_out_frac: zout,
            zoom_fov: [65.0, 65.0, 65.0],
            ..AdsFacts::default()
        };
        f
    }

    /// DSR 50: 400 / 600 ms, zoom in 0, out 0.05, scope picture, holds breath.
    fn dsr() -> T6WeaponFacts {
        let mut f = gun(400, 600, 0.0, 0.05);
        f.ads.hud_crosshair_layer = 1;
        f.ads.hold_breath_to_steady = true;
        f.ads.player_motion_kind = ANIM_TYPE_SNIPER_REARCLIP;
        f.ads.zoom_fov = [10.0, 10.0, 10.0];
        f
    }

    /// XPR-50: DSR timing plus anti-quickscope 100 ms, scale 4.
    fn xpr() -> T6WeaponFacts {
        let mut f = dsr();
        f.ads.player_motion_kind = ANIM_TYPE_SNIPER;
        f.ads.anti_quick_scope = true;
        f.ads.aqs_time = 0.1;
        f.ads.aqs_scale = 4.0;
        f.ads.aqs_sway_factor = 20.0;
        f.ads.aqs_spread_max = 25.0;
        f
    }

    struct Rig {
        ps: T6WeaponPs,
        env: T6Env,
        facts: FactsTable,
        view: AdsView,
        time: i32,
        last_tick_buttons: u32,
    }

    impl Rig {
        fn new(f: T6WeaponFacts) -> Self {
            let mut facts = FactsTable::new();
            facts.set(GUN, "gun", f);
            let mut ps = T6WeaponPs {
                weapon: GUN,
                ..T6WeaponPs::default()
            };
            ps.give(GUN);
            ps.set_clip_slot(GUN, 10);
            Self {
                ps,
                env: T6Env {
                    secondary_state_bits: OF_WEAPON_USABLE,
                    ..T6Env::default()
                },
                facts,
                view: AdsView::default(),
                time: 1000,
                last_tick_buttons: 0,
            }
        }

        /// One 50 ms server tick in BO2's order: flag, then the PM_Weapon ADS hooks.
        fn tick(&mut self, buttons: u32) -> (FrameOut, AdsFrame) {
            self.tick_ms(buttons, 50)
        }

        fn tick_ms(&mut self, held_buttons: u32, msec: i32) -> (FrameOut, AdsFrame) {
            self.time += msec;
            let cmd = T6Cmd {
                frame_ms: msec,
                authority_clock_ms: self.time,
                weapon: GUN,
                buttons: held_buttons,
                last_tick_buttons: self.last_tick_buttons,
                ..T6Cmd::default()
            };
            self.last_tick_buttons = held_buttons;
            let mut out = FrameOut::default();
            let mut pm = Pm {
                ps: &mut self.ps,
                cmd: &cmd,
                env: &mut self.env,
                facts: &self.facts,
                out: &mut out,
                delay_fired: false,
            };
            update_ads_flag(&mut pm);
            let prev = pm.ps.weapon_raise_fraction;
            update_ads_lerp(&mut pm);
            update_scope_zoom(&mut pm);
            update_hold_breath(&mut pm);
            update_quickscope(&mut pm, prev);
            let a = self.facts.get(GUN).ads;
            let frame = self.view.frame(&a, self.ps.weapon_raise_fraction, 65.0, 0, false, msec as f32 * 0.001);
            (out, frame)
        }
    }

    #[test]
    fn rifle_aims_in_250_out_250() {
        let mut r = Rig::new(gun(250, 250, 0.5, 0.4));
        for i in 1..=5 {
            r.tick(button::AIM_BIT);
            assert!((r.ps.weapon_raise_fraction - i as f32 * 0.2).abs() < 1e-5);
        }
        assert_eq!(r.ps.weapon_raise_fraction, 1.0);
        assert_ne!(r.env.movement_state_bits & pmf::ADS, 0);
        for _ in 0..5 {
            r.tick(0);
        }
        // f32 steps of 0.2 leave a crumb; BO2 lands on 0 one tick later, same as here.
        assert!(r.ps.weapon_raise_fraction < 1e-6);
        r.tick(0);
        assert_eq!(r.ps.weapon_raise_fraction, 0.0);
    }

    #[test]
    fn quick_draw_halves_rifles_not_snipers() {
        let mut r = Rig::new(gun(250, 250, 0.5, 0.4));
        r.env.perks |= 1 << PERK_FASTADS;
        r.tick(button::AIM_BIT);
        assert!((r.ps.weapon_raise_fraction - 0.4).abs() < 1e-5);
        let mut s = Rig::new(dsr());
        s.env.perks |= 1 << PERK_FASTADS;
        s.tick(button::AIM_BIT);
        assert!((s.ps.weapon_raise_fraction - 0.125).abs() < 1e-5);
    }

    #[test]
    fn a_gun_that_grants_quick_draw_aims_fast_without_the_perk() {
        // BG_HasPerk: the gun in the arms can grant the perk through its attachments.
        let mut f = gun(250, 250, 0.5, 0.4);
        f.spread.perks = 1 << PERK_FASTADS;
        let mut r = Rig::new(f);
        r.tick(button::AIM_BIT);
        assert!((r.ps.weapon_raise_fraction - 0.4).abs() < 1e-5);
    }

    #[test]
    fn sniper_scope_pops_at_full_and_goes_in_first_5_percent_out() {
        let mut r = Rig::new(dsr());
        for _ in 0..7 {
            let (_, fr) = r.tick(button::AIM_BIT);
            assert!(!fr.overlay, "scope must not show before frac 1");
            assert_eq!(fr.view_fov, 65.0);
        }
        let (_, fr) = r.tick(button::AIM_BIT); // 400 ms
        assert_eq!(r.ps.weapon_raise_fraction, 1.0);
        assert!(fr.overlay);
        assert_eq!(fr.view_fov, 10.0);
        // Out at 600 ms: 50 ms = 0.0833 > 5%: the picture is gone on the first tick out.
        let (_, fr) = r.tick(0);
        assert!(!fr.overlay);
        assert_eq!(fr.view_fov, 65.0);
    }

    #[test]
    fn sniper_released_early_never_shows_scope() {
        let mut r = Rig::new(dsr());
        for _ in 0..7 {
            r.tick(button::AIM_BIT);
        }
        for _ in 0..8 {
            let (_, fr) = r.tick(0);
            assert!(!fr.overlay);
        }
    }

    #[test]
    fn iron_sight_fov_window_starts_half_way() {
        let mut r = Rig::new(gun(250, 250, 0.5, 0.4));
        r.facts.set(GUN, "gun", {
            let mut f = gun(250, 250, 0.5, 0.4);
            f.ads.zoom_fov = [55.0; 3];
            f
        });
        let (_, f1) = r.tick(button::AIM_BIT); // 0.2
        let (_, f2) = r.tick(button::AIM_BIT); // 0.4
        let (_, f3) = r.tick(button::AIM_BIT); // 0.6
        assert_eq!(f1.view_fov, 65.0);
        assert_eq!(f2.view_fov, 65.0);
        assert!((f3.zoom_frac - 0.2).abs() < 1e-5);
        assert!((f3.view_fov - 63.0).abs() < 1e-4);
    }

    #[test]
    fn sprint_key_drops_ads_on_pc() {
        let mut r = Rig::new(gun(250, 250, 0.5, 0.4));
        r.tick(button::AIM_BIT);
        let (out, _) = r.tick(button::AIM_BIT | button::SPRINT);
        assert_eq!(r.env.movement_state_bits & pmf::ADS, 0);
        assert_eq!(out.events[0].id, SIGNAL_SCOPE_RESTART);
        assert!(r.ps.weapon_raise_fraction < 0.2);
    }

    #[test]
    fn prone_crawl_keeps_held_ads_off() {
        let mut r = Rig::new(gun(250, 250, 0.5, 0.4));
        r.env.movement_state_bits = pmf::LYING_FLAT;
        r.tick(button::AIM_BIT);
        assert_eq!(r.env.movement_state_bits & (pmf::ADS | pmf::PRONE_MOVE_OVERRIDE), pmf::ADS | pmf::PRONE_MOVE_OVERRIDE);
        r.env.movement_state_bits = pmf::LYING_FLAT;
        let cmd = T6Cmd {
            frame_ms: 50,
            buttons: button::AIM_BIT,
            last_tick_buttons: button::AIM_BIT,
            thrust_ahead: 127,
            ..T6Cmd::default()
        };
        let mut out = FrameOut::default();
        let mut pm = Pm {
            ps: &mut r.ps,
            cmd: &cmd,
            env: &mut r.env,
            facts: &r.facts,
            out: &mut out,
            delay_fired: false,
        };
        update_ads_flag(&mut pm);
        assert_eq!(pm.env.movement_state_bits & pmf::ADS, 0);
    }

    #[test]
    fn reload_past_trans_aims_out() {
        let mut f = gun(250, 250, 0.5, 0.4);
        f.ads.position_reload_trans_time = 200;
        let mut r = Rig::new(f);
        for _ in 0..5 {
            r.tick(button::AIM_BIT);
        }
        r.ps.gun_phase = state::RELOADING;
        r.ps.gun_timer_ms = 200;
        r.tick(button::AIM_BIT);
        assert_eq!(r.ps.weapon_raise_fraction, 1.0);
        r.ps.gun_timer_ms = 201;
        r.tick(button::AIM_BIT);
        assert!((r.ps.weapon_raise_fraction - 0.8).abs() < 1e-5);
    }

    #[test]
    fn raising_refuses_ads_unless_alt_ads_only() {
        let mut r = Rig::new(gun(250, 250, 0.5, 0.4));
        r.ps.gun_phase = state::RAISING_ALTSWITCH;
        r.tick(button::AIM_BIT);
        assert_eq!(r.ps.weapon_raise_fraction, 0.0);
        let mut f = gun(250, 250, 0.5, 0.4);
        f.core.alt_weapon_ads_only = true;
        let mut h = Rig::new(f);
        h.ps.gun_phase = state::RAISING_ALTSWITCH;
        h.tick(button::AIM_BIT);
        assert!(h.ps.weapon_raise_fraction > 0.0);
    }

    #[test]
    fn xpr_quickscope_timer_runs_100_ms_after_full() {
        let mut r = Rig::new(xpr());
        for _ in 0..7 {
            r.tick(button::AIM_BIT);
            assert_eq!(r.ps.quick_scope_timer, 100);
        }
        r.tick(button::AIM_BIT); // frac 1 this tick: timer -= 50
        assert_eq!(r.ps.weapon_raise_fraction, 1.0);
        assert_eq!(r.ps.quick_scope_timer, 50);
        assert!(r.ps.quick_scope_scale > 1.0);
        r.tick(button::AIM_BIT);
        assert_eq!(r.ps.quick_scope_timer, 0);
        // Hold breath refused while the timer ran; works once it is 0.
        r.tick(button::AIM_BIT | button::STEADY_AIM_HOLD_BIT);
        assert_ne!(r.ps.weapon_state_bits & wf::HOLDING_BREATH, 0);
    }

    #[test]
    fn dsr_has_no_quickscope() {
        let mut r = Rig::new(dsr());
        for _ in 0..8 {
            r.tick(button::AIM_BIT);
            assert_eq!(r.ps.quick_scope_timer, 0);
            assert_eq!(r.ps.quick_scope_scale, 1.0);
        }
    }

    #[test]
    fn hold_breath_lasts_4500_then_gasps() {
        let mut r = Rig::new(dsr());
        for _ in 0..8 {
            r.tick(button::AIM_BIT);
        }
        let mut held = 0;
        let mut steadiest = 1.0f32;
        for _ in 0..200 {
            r.tick(button::AIM_BIT | button::STEADY_AIM_HOLD_BIT);
            if r.ps.weapon_state_bits & wf::HOLDING_BREATH == 0 {
                break;
            }
            held += 50;
            steadiest = steadiest.min(r.ps.steady_aim_factor);
        }
        assert_eq!(held, 4500);
        assert_eq!(r.ps.hold_breath_timer, 4500);
        // Held breath eased the sway toward 0.
        assert!(steadiest < 0.01);
        // Out of breath: can't hold again until the timer is back at 0; sway climbs toward 2.5.
        let before = r.ps.steady_aim_factor;
        r.tick(button::AIM_BIT | button::STEADY_AIM_HOLD_BIT);
        assert_eq!(r.ps.weapon_state_bits & wf::HOLDING_BREATH, 0);
        assert!(r.ps.steady_aim_factor > before);
        for _ in 0..20 {
            r.tick(button::AIM_BIT | button::STEADY_AIM_HOLD_BIT);
        }
        assert!(r.ps.steady_aim_factor > 2.0);
    }

    #[test]
    fn zoom_select_steps_with_latch() {
        let mut f = dsr();
        f.ads.variable_zoom = true;
        let mut r = Rig::new(f);
        for _ in 0..8 {
            r.tick(button::AIM_BIT);
        }
        let (out, _) = r.tick(button::AIM_BIT | button::CHANGE_ZOOM);
        assert_eq!(out.events.last().map(|e| e.id), Some(EV_ADS_ZOOM_CHANGE));
        assert_eq!(r.ps.held(GUN).unwrap().ads_zoom_select, 1);
        r.tick(button::AIM_BIT | button::CHANGE_ZOOM);
        assert_eq!(r.ps.held(GUN).unwrap().ads_zoom_select, 1);
        r.tick(button::AIM_BIT);
        r.tick(button::AIM_BIT | button::CHANGE_ZOOM);
        assert_eq!(r.ps.held(GUN).unwrap().ads_zoom_select, 2);
    }

    #[test]
    fn diff_track_snaps_close() {
        assert_eq!(diff_track(1.0, 0.9995, 4.0, 0.05), 1.0);
        assert!((diff_track(2.0, 1.0, 4.0, 0.05) - 1.2).abs() < 1e-6);
    }

    /// Frame trace for the proof: `cargo test -p weapon_t6 ads_frame_trace -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn ads_frame_trace() {
        let cases: [(&str, T6WeaponFacts, &[(u32, i32)]); 4] = [
            ("rifle 250/250 zoom 0.5/0.4", {
                let mut f = gun(250, 250, 0.5, 0.4);
                f.ads.zoom_fov = [55.0; 3];
                f
            }, &[(button::AIM_BIT, 7), (0, 7)]),
            ("dsr50 400/600", dsr(), &[(button::AIM_BIT, 10), (0, 3)]),
            ("xpr50 400/600 aqs 100", xpr(), &[(button::AIM_BIT, 12), (button::AIM_BIT | button::STEADY_AIM_HOLD_BIT, 3)]),
            ("dsr50 release at 300", dsr(), &[(button::AIM_BIT, 6), (0, 6)]),
        ];
        for (name, f, script) in cases {
            println!("== {name} (50 ms ticks)");
            let mut r = Rig::new(f);
            let mut t = 0;
            for &(buttons, n) in script {
                for _ in 0..n {
                    let (_, fr) = r.tick(buttons);
                    t += 50;
                    println!(
                        "t {t:4} {:<4} ads {} {} | z {:.3} overlay {} fov {:.2} anim {:#x} {:.3}",
                        if buttons & button::AIM_BIT != 0 { "ads" } else { "-" },
                        u8::from(r.env.movement_state_bits & pmf::ADS != 0),
                        trace_columns(&r.ps),
                        fr.zoom_frac,
                        u8::from(fr.overlay),
                        fr.view_fov,
                        fr.anim,
                        fr.anim_time,
                    );
                }
            }
        }
    }

    /// The client side on BO2's own numbers: `ADS_FACTS_DUMP=dump.tsv cargo test -p weapon_t6
    /// ads_view_trace_real -- --ignored --nocapture` (dump lines: name, variant hex, def hex).
    #[test]
    #[ignore]
    fn ads_view_trace_real() {
        let Ok(path) = std::env::var("ADS_FACTS_DUMP") else {
            println!("ADS_FACTS_DUMP not set");
            return;
        };
        let table = FactsTable::from_dump(&std::fs::read_to_string(path).expect("dump"));
        for i in 1..table.len() as u32 {
            let f = table.get(i);
            let a = f.ads;
            println!(
                "== {} in {} out {} zoom frac {} / {} fov {} overlay {} aqs {} {} ms",
                table.name(i),
                a.in_time,
                a.out_time,
                a.zoom_in_frac,
                a.zoom_out_frac,
                a.zoom_fov[0],
                a.hud_crosshair_layer,
                u8::from(a.anti_quick_scope),
                a.aqs_ms(),
            );
            let mut r = Rig::new(f);
            let ins = a.in_time.max(1) / 50 + 1;
            let outs = a.out_time.max(1) / 50 + 1;
            let mut t = 0;
            for (buttons, n) in [(button::AIM_BIT, ins + 1), (0, outs)] {
                for _ in 0..n {
                    let (_, fr) = r.tick(buttons);
                    t += 50;
                    println!(
                        "t {t:4} {:<3} {} | z {:.3} overlay {} fov {:.2} anim {:#x} {:.3}",
                        if buttons != 0 { "ads" } else { "-" },
                        trace_columns(&r.ps),
                        fr.zoom_frac,
                        u8::from(fr.overlay),
                        fr.view_fov,
                        fr.anim,
                        fr.anim_time,
                    );
                }
            }
        }
    }
}
