//! Window 3: firing, burst, rechamber, continuous fire, charge shot, spin-up and fuel.
//!
//! BO2's fire path, hook by hook: the trigger (which button, pending pulls, burst limits),
//! StartFiring (fire time, last shot, intro shots, fire delay, kick restriction), FireWeapon
//! (ammo, events, the viewmodel fire anim), the bolt rechamber, the continuous-fire states
//! (Death Machine / War Machine style), minigun and jet gun spin, charge shots and fuel tanks.
//! Timing spec: docs/gun/FIRE.md.

use fastfile_t6::layout::{WeaponDef as D, WeaponVariantDef as V};

use super::Pm;
use crate::anim;
use crate::base;
use crate::cmd::button;
use crate::env::{ef2, pm_type};
use crate::events;
use crate::events::AnimScript;
use crate::facts::{fire_type, weap_type};
use crate::player::wf;
use crate::state;

/// eFlags2: the minigun / jet gun barrel is spinning.
const EF2_SPINNING: u32 = 0x40_0000;
/// eFlags2: the last shot used the clip up (UseAmmo left nothing).
const EF2_CLIP_USED_UP: u32 = 0x400;
/// eFlags bits that stop a shot from using ammo.
const EF_NO_AMMO_USE: u32 = 0x300;
/// otherFlags: hit by an EMP (charge shots queue only one bullet).
const OF_EMP: u32 = 0x40;
/// `guidedMissileType` for a TV-guided gun.
const GUIDED_TV: i32 = 6;
/// Highest shot count BO2 keeps for a burst.
const MAX_SHOT_COUNT: u32 = 7;
/// Intro shots are counted up to this.
const MAX_INTRO_SHOTS: i32 = 250;
/// Water this deep stops guns when `player_disableWeaponsInWater` is on.
const WATER_DEEP: i32 = 4;
/// Weapon position past this plays the ADS fire / rechamber anims.
const ADS_ANIM_FRAC: f32 = 0.75;
/// Time added after a dry-fire click.
const DRY_FIRE_MS: i32 = 500;
/// Time a grenade waits when it may not be planted here.
const NO_PLANT_MS: i32 = 500;

/// Window 3's per-weapon facts (read from `fastfile_t6::layout` offsets).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct FireFacts {
    /// `iFireDelay`: wait between the trigger and the first shot.
    pub fire_delay: i32,
    pub spin_up_time: i32,
    pub spin_down_time: i32,
    /// `iFireTime`: time between shots.
    pub fire_time: i32,
    /// `iLastFireTime`: fire time of the shot that empties the clip (0 = normal).
    pub last_fire_time: i32,
    pub rechamber_time: i32,
    /// `iRechamberBoltTime`: when the bolt is back (the brass ejects).
    pub rechamber_bolt_time: i32,
    pub hold_fire_time: i32,
    pub cont_fire_in_time: i32,
    pub cont_fire_out_time: i32,
    pub fuse_time: i32,
    pub require_lockon_to_fire: bool,
    pub rechamber_while_ads: bool,
    pub ads_fire_only: bool,
    pub continuous_fire: bool,
    pub no_ads_auto_reload: bool,
    /// `iShotsBeforeRechamber` (0 = rechamber after every shot).
    pub shots_before_rechamber: i32,
    pub freeze_movement_when_firing: bool,
    pub ads_kick_reduced_bullets: i32,
    pub hip_kick_reduced_bullets: i32,
    pub intro_fire_time: i32,
    /// `iIntroFireLength`: shots that use the intro fire time (0 = none).
    pub intro_fire_length: i32,
    pub overheat: bool,
    /// `overheatRate` / `cooldownRate`: heat (0..100) gained firing / lost a second.
    pub overheat_rate: f32,
    pub cooldown_rate: f32,
    /// `overheatEndVal`: an overheated gun fires again once it has cooled to this.
    pub overheat_end: f32,
    pub cool_while_firing: bool,
    pub fuel_tank: bool,
    pub tank_life_time: i32,
    /// Variant `fOOPosAnimLength[0]` (1 / ADS-in time).
    pub oo_pos_anim_length: f32,
}

fn rd_i32(b: &[u8], at: usize) -> i32 {
    b.get(at..at + 4)
        .map_or(0, |s| i32::from_le_bytes([s[0], s[1], s[2], s[3]]))
}

fn rd_f32(b: &[u8], at: usize) -> f32 {
    b.get(at..at + 4)
        .map_or(0.0, |s| f32::from_le_bytes([s[0], s[1], s[2], s[3]]))
}

fn rd_bool(b: &[u8], at: usize) -> bool {
    b.get(at).is_some_and(|&v| v != 0)
}

impl FireFacts {
    pub fn read(variant: &[u8], def: &[u8]) -> Self {
        Self {
            fire_delay: rd_i32(def, D::iFireDelay),
            spin_up_time: rd_i32(def, D::iSpinUpTime),
            spin_down_time: rd_i32(def, D::iSpinDownTime),
            fire_time: rd_i32(def, D::iFireTime),
            last_fire_time: rd_i32(def, D::iLastFireTime),
            rechamber_time: rd_i32(def, D::iRechamberTime),
            rechamber_bolt_time: rd_i32(def, D::iRechamberBoltTime),
            hold_fire_time: rd_i32(def, D::iHoldFireTime),
            cont_fire_in_time: rd_i32(def, D::contFireInTime),
            cont_fire_out_time: rd_i32(def, D::contFireOutTime),
            fuse_time: rd_i32(def, D::fuseTime),
            require_lockon_to_fire: rd_bool(def, D::requireLockonToFire),
            rechamber_while_ads: rd_bool(def, D::bRechamberWhileAds),
            ads_fire_only: rd_bool(def, D::adsFireOnly),
            continuous_fire: rd_bool(def, D::bContinuousFire),
            no_ads_auto_reload: rd_bool(def, D::bNoADSAutoReload),
            shots_before_rechamber: rd_i32(def, D::iShotsBeforeRechamber),
            freeze_movement_when_firing: rd_bool(def, D::freezeMovementWhenFiring),
            ads_kick_reduced_bullets: rd_i32(def, D::adsGunKickReducedKickBullets),
            hip_kick_reduced_bullets: rd_i32(def, D::hipGunKickReducedKickBullets),
            intro_fire_time: rd_i32(def, D::iIntroFireTime),
            intro_fire_length: rd_i32(def, D::iIntroFireLength),
            overheat: rd_i32(def, D::overheatWeapon) != 0,
            overheat_rate: rd_f32(def, D::overheatRate),
            cooldown_rate: rd_f32(def, D::cooldownRate),
            overheat_end: rd_f32(def, D::overheatEndVal),
            cool_while_firing: rd_bool(def, D::coolWhileFiring),
            fuel_tank: rd_bool(def, D::fuelTankWeapon),
            tank_life_time: rd_i32(def, D::iTankLifeTime),
            oo_pos_anim_length: rd_f32(variant, V::fOOPosAnimLength),
        }
    }
}

// ---- Stand-ins for pieces other windows own. Each is a one-line swap on merge. ----

/// `BG_GetFireTime`'s attachment scale (product of `fFireTimeScale`).
fn fire_time_attachment_scale(pm: &Pm, weapon: u32) -> f32 {
    pm.facts.get(weapon).att.fire_time
}

/// `BG_GetADSTransTimeScales` in-scale (product of `fAdsTransInTimeScale`).
fn ads_trans_in_scale(pm: &Pm, weapon: u32) -> f32 {
    pm.facts.get(weapon).att.ads_in
}

/// `PM_WeaponAllowPlant`: the game's plant probe.
fn allow_plant(pm: &Pm) -> bool {
    pm.env.plant_spot_ok
}

// ---- Small BO2 helpers. ----

/// The weapon in one hand (`left` = the dual-wield partner).
fn weapon_of(pm: &Pm, left: bool) -> u32 {
    if left {
        pm.weapon_facts().core.dual_wield_weapon
    } else {
        pm.ps.weapon
    }
}

fn is_thrown(t: u8) -> bool {
    t == weap_type::GRENADE || t == weap_type::MINE
}

/// BG_PlayerWeaponNeedsRechambered.
fn needs_rechamber(pm: &Pm, weapon: u32) -> bool {
    pm.ps.held(weapon).is_some_and(|h| h.needs_rechamber)
}

/// BG_PlayerWeaponSetNeedsRechamber.
fn set_needs_rechamber(pm: &mut Pm, weapon: u32, on: bool) {
    if let Some(h) = pm.ps.held_mut(weapon) {
        h.needs_rechamber = on;
    }
}

/// BG_PlayerWeaponOverheating: `weapon` overheats and the held gun is over its limit.
fn overheating(pm: &Pm, weapon: u32) -> bool {
    pm.facts.get(weapon).fire.overheat
        && pm
            .ps
            .held(pm.ps.weapon)
            .is_some_and(|h| h.over_heating || h.heat_percent >= 100.0)
}

/// Weapon_Overheat_Update (g_weapon.cpp), once a server frame of `frame_ms`: the gun in hand
/// heats while it fires (out of deep water); every other held gun, and one that cools while
/// firing, cools.
pub fn overheat_update(
    ps: &mut crate::player::T6WeaponPs,
    facts: &crate::facts::FactsTable,
    frame_ms: i32,
    waterlevel: i32,
) {
    let secs = frame_ms.max(0) as f32 * 0.001;
    let firing = matches!(ps.weaponstate, state::FIRING | state::CONT_FIRE_LOOP) && waterlevel < WATER_DEEP;
    let in_hand = ps.weapon;
    for h in ps.held.iter_mut().filter(|h| h.weapon != 0) {
        let f = facts.get(h.weapon).fire;
        let hot = firing && h.weapon == in_hand;
        if hot {
            add_heat(h, &f, secs * f.overheat_rate);
        }
        if !hot || f.cool_while_firing {
            add_heat(h, &f, -secs * f.cooldown_rate);
        }
    }
}

/// BG_PlayerWeaponOverheatUpdate: heat stays 0..100; at 100 the gun is overheated until it
/// cools to its end value (never, for a gun that does not overheat).
fn add_heat(h: &mut crate::player::HeldWeapon, f: &FireFacts, by: f32) {
    h.heat_percent += by;
    if h.heat_percent >= 100.0 {
        h.heat_percent = 100.0;
        h.over_heating = true;
    }
    if h.heat_percent < 0.0 {
        h.heat_percent = 0.0;
    }
    if h.heat_percent <= f.overheat_end || !f.overheat {
        h.over_heating = false;
    }
}

/// BG_GetFireTime.
fn fire_time(pm: &Pm, weapon: u32) -> i32 {
    let t = pm.facts.get(weapon).fire.fire_time;
    let t = (t as f32 * fire_time_attachment_scale(pm, weapon)) as i32;
    if pm.env.rapid_fire { (t / 3).max(30) } else { t }
}

/// The mod menu's Rapid Fire turns single-shot and burst guns into held-trigger guns.
fn rapid_auto(pm: &Pm, fire_type: u8) -> bool {
    pm.env.rapid_fire
        && (fire_type::SINGLESHOT..=fire_type::BURSTFIRE5).contains(&fire_type)
}

/// Ammo one shot needs (PM_Weapon_GetAmmoRequired).
fn ammo_required(pm: &Pm) -> i32 {
    let d = &pm.env.dvars;
    let charge = pm.weapon_facts().core.fire_type == fire_type::CHARGESHOT;
    if charge && d.bg_charge_shot_exponential_ammo_per_charge_level {
        charge_level_ammo(pm.ps.charge_shot_level)
    } else if charge && !d.bg_charge_shot_use_one_ammo_for_multiple_bullets {
        pm.ps.charge_shot_level
    } else {
        1
    }
}

/// PM_WeaponUseAmmo: the clip left (sustain ammo keeps it full and says 1).
fn use_ammo(pm: &mut Pm, weapon: u32, amount: i32) -> i32 {
    if pm.env.dvars.player_sustain_ammo {
        return 1;
    }
    let clip = pm.ps.ammo_in_clip(weapon);
    let left = (clip - amount.min(clip)).max(0);
    pm.ps.set_ammo_in_clip(weapon, left);
    left
}

/// PM_WeaponClipEmpty for the hand being run.
fn clip_empty(pm: &Pm) -> bool {
    !pm.weapon_facts().core.unlimited_ammo && pm.ps.ammo_in_clip(pm.hand_weapon()) == 0
}

/// BG_CheckIfLastShot: this shot (or this burst) empties the clip.
fn is_last_shot(pm: &Pm, weapon: u32) -> bool {
    let clip = pm.ps.ammo_in_clip(weapon);
    match pm.facts.get(weapon).core.fire_type {
        t @ fire_type::BURSTFIRE2..=fire_type::BURSTFIRE5 => clip < i32::from(t) + 1,
        _ => clip < 2,
    }
}

/// ShotLimitReached: the trigger has fired all the shots one pull allows.
fn shot_limit_reached(pm: &Pm, weapon: u32, count: u32) -> bool {
    let held = pm.weapon_facts().core;
    if held.fire_type == fire_type::STACKED {
        return held.clip_size <= count as i32;
    }
    match pm.facts.get(weapon).core.fire_type {
        fire_type::CHARGESHOT => pm.ps.charge_shot_level <= count as i32,
        t if rapid_auto(pm, t) => false,
        fire_type::SINGLESHOT => count >= 1,
        t @ fire_type::BURSTFIRE2..=fire_type::BURSTFIRE5 => count >= u32::from(t),
        _ => false,
    }
}

/// Spin progress per frame for a spin time (0 = instant).
fn spin_step(msec: i32, time: i32) -> f32 {
    if time <= 0 {
        1.0
    } else {
        msec as f32 / time as f32
    }
}

// ---- Hooks. ----

/// BurstFirePending for one hand: a burst (or stacked / charge volley) still owes shots.
pub fn burst_fire_pending(pm: &Pm, left: bool) -> bool {
    let w = weapon_of(pm, left);
    if w == 0 {
        return false;
    }
    let ft = pm.facts.get(w).core.fire_type;
    if matches!(
        ft,
        fire_type::FULLAUTO | fire_type::MINIGUN | fire_type::JETGUN
    ) || rapid_auto(pm, ft)
    {
        return false;
    }
    let count = pm.ps.hand(left).shot_count;
    if count == 0 {
        return false;
    }
    let pool = super::dual::ammo_pool_weapon(pm.facts, w);
    if pm.ps.ammo_not_in_clip(pool) == 0 && pm.ps.ammo_in_clip(w) == 0 {
        return false;
    }
    !shot_limit_reached(pm, w, count)
}

/// PM_Weapon_BurstFireWaitingForTriggerRelease (the hand being run): the pull is used up
/// and the trigger is still held.
pub fn burst_waiting_for_trigger_release(pm: &Pm) -> bool {
    let left = pm.left();
    let h = pm.hand();
    if (state::OFFHAND_INIT..=state::OFFHAND_END).contains(&h.state)
        || (state::CRAWL_IN..=state::CRAWL_LEFT).contains(&h.state)
        || pm.ps.weap_flags & wf::WEAPONS_DISABLED != 0
    {
        return false;
    }
    let w = pm.hand_weapon();
    let hc = pm.hand_facts().core;
    let limit = !trigger_pull_pending(pm, left) && shot_limit_reached(pm, w, h.shot_count);
    let hold = is_thrown(hc.weap_type) && hc.hold_button_to_throw;
    if !(limit || hold) {
        return false;
    }
    let same_gun = pm.ps.weapon == pm.cmd.weapon
        || (crate::switch::is_alt_switch(pm.ps, pm.facts, pm.ps.weapon, pm.cmd.weapon)
            && pm.weapon_facts().core.alt_weapon_disable_switching);
    pm.cmd.held(fire_button(pm, left))
        && same_gun
        && (pm.ps.ammo_in_clip(w) != 0 || hc.unlimited_ammo)
}

/// PM_Weapon_TriggerPullPending for one hand.
pub fn trigger_pull_pending(pm: &Pm, left: bool) -> bool {
    let bit = if left {
        wf::PENDING_PULL_LEFT
    } else {
        wf::PENDING_PULL_RIGHT
    };
    pm.ps.weap_flags & bit != 0
}

/// PM_GetWeaponFireButton: the button that fires this hand of the held weapon.
/// A thrown weapon with a detonator fires on aim (BO2 MP's UAV ADS fix off, or Zombies). Dual wield
/// picks by the last input device (window 8, [`super::dual::fire_button`]).
pub fn fire_button(pm: &Pm, left: bool) -> u32 {
    let c = pm.weapon_facts().core;
    if (pm.env.zombies || !pm.env.dvars.tu5_uav_ads_fix)
        && is_thrown(c.weap_type)
        && c.has_detonator
    {
        return button::ADS;
    }
    super::dual::fire_button(c.dual_wield, left, super::dual::last_input_was_pad(pm.cmd))
}

/// PM_Weapon_UpdatePendingTriggerPull: a fresh pull on a burst / semi-stacked gun.
pub fn update_pending_trigger_pull(pm: &mut Pm) {
    let ft = pm.weapon_facts().core.fire_type;
    if ft == fire_type::FULLAUTO || ft == fire_type::SINGLESHOT {
        return;
    }
    if pm.cmd.pressed(fire_button(pm, false)) {
        pm.ps.weap_flags |= wf::PENDING_PULL_RIGHT;
    }
    if pm.cmd.pressed(fire_button(pm, true)) {
        pm.ps.weap_flags |= wf::PENDING_PULL_LEFT;
    }
}

/// PM_UpdateSpinLerp: minigun spin-up (0..1) and jet gun spin (-1..0).
pub fn update_spin(pm: &mut Pm) {
    let wfacts = pm.weapon_facts();
    let f = wfacts.fire;
    let msec = pm.cmd.msec;
    match wfacts.core.fire_type {
        fire_type::MINIGUN => {
            let mut lerp = pm.ps.weapon_spin_lerp;
            if pm.cmd.held(button::ATTACK | button::ADS) {
                pm.env.e_flags2 |= EF2_SPINNING;
                lerp += spin_step(msec, f.spin_up_time);
            } else {
                pm.env.e_flags2 &= !EF2_SPINNING;
                lerp -= spin_step(msec, f.spin_down_time);
            }
            pm.ps.weapon_spin_lerp = lerp.clamp(0.0, 1.0);
        }
        fire_type::JETGUN => {
            let rate = if (state::RAISING..=state::DROPPING_ALTSWITCH).contains(&pm.ps.weaponstate)
            {
                pm.env.e_flags2 &= !EF2_SPINNING;
                0.0
            } else {
                let on = pm.cmd.held(button::ATTACK) && !overheating(pm, pm.ps.weapon);
                if on {
                    pm.env.e_flags2 |= EF2_SPINNING;
                } else {
                    pm.env.e_flags2 &= !EF2_SPINNING;
                }
                match (f.spin_up_time > 0, on) {
                    (true, true) => spin_step(msec, f.spin_up_time),
                    (true, false) => -spin_step(msec, f.spin_down_time),
                    (false, true) => 0.5 * msec as f32,
                    (false, false) => -0.5 * msec as f32,
                }
            };
            pm.ps.weapon_spin_lerp = (pm.ps.weapon_spin_lerp - rate).clamp(-1.0, 0.0);
        }
        _ => pm.ps.weapon_spin_lerp = 0.0,
    }
}

fn clear_charge(pm: &mut Pm) {
    pm.ps.weap_flags &= !wf::CHARGE_SHOT;
    pm.ps.charge_shot_timer = 0;
    pm.ps.charge_shot_level = 0;
}

/// The rounds a charge level takes with the exponential ammo dvar on: 1, 2, 6.
fn charge_level_ammo(level: i32) -> i32 {
    (1..=level.max(1)).product()
}

/// PM_UpdateChargeShot: holding the trigger queues bullets, one per queue time.
pub fn update_charge_shot(pm: &mut Pm) {
    let wfacts = pm.weapon_facts();
    if wfacts.core.fire_type != fire_type::CHARGESHOT {
        return clear_charge(pm);
    }
    let d = pm.env.dvars;
    let clip = pm.ps.ammo_in_clip(pm.ps.weapon);
    let mut most = clip.min(d.bg_charge_shot_max_bullets_in_queue);
    if d.bg_charge_shot_exponential_ammo_per_charge_level {
        most = (1..=most).rev().find(|&l| charge_level_ammo(l) <= clip).unwrap_or(most);
    }
    let s = pm.ps.weaponstate;
    let busy = (state::RELOADING..=state::RELOAD_QUICK_EMPTY).contains(&s)
        || (state::CRAWL_IN..=state::CRAWL_OUT).contains(&s)
        || s == state::RECHAMBER_START
        || s == state::RECHAMBERING
        || (state::RAISING..=state::DROPPING_ALTSWITCH).contains(&s)
        || (state::DTP_IN..=state::DTP_OUT).contains(&s);
    if !pm.cmd.held(button::ATTACK) {
        return;
    }
    if !pm.cmd.was_held(button::ATTACK) && clip != 0 && !busy {
        pm.ps.weap_flags |= wf::CHARGE_SHOT;
        pm.ps.charge_shot_level = 0;
        pm.ps.charge_shot_timer = 0;
        return;
    }
    if pm.ps.weap_flags & wf::CHARGE_SHOT == 0 || busy {
        return;
    }
    pm.ps.charge_shot_timer += pm.cmd.msec;
    let mut level = pm.ps.charge_shot_timer / d.bg_charge_shot_queue_time + 1;
    let f = wfacts.fire;
    if wfacts.core.bolt_action && f.shots_before_rechamber != 0 {
        most = most.min(f.shots_before_rechamber - pm.ps.shots_fired_from_chamber);
    }
    if pm.env.other_flags & OF_EMP != 0 {
        most = most.min(d.bg_charge_shot_max_bullet_queue_on_emp);
    }
    if most <= level {
        level = most;
        if d.bg_charge_shot_discharge_when_queue_reaches_max
            && d.bg_charge_shot_auto_discharge_delay
                < pm.ps.charge_shot_timer - (most - 1) * d.bg_charge_shot_queue_time
        {
            pm.add_predictable_event(events::EV_CHARGESHOT_DISCHARGEWEAPON, 0);
            return clear_charge(pm);
        }
    }
    if pm.ps.charge_shot_level < level {
        pm.add_predictable_event(
            events::EV_INC_CHARGESHOT_LEVEL,
            pm.ps.charge_shot_level as u32,
        );
    }
    pm.ps.charge_shot_level = level;
}

/// ShouldFireChargeShotWeapon: let go of a charged trigger, or the queue is full.
fn should_fire_charge_shot(pm: &Pm) -> bool {
    let d = &pm.env.dvars;
    let level = pm.ps.charge_shot_level;
    let released = !pm.cmd.held(button::ATTACK) && pm.cmd.was_held(button::ATTACK);
    let full = level > 1
        && d.bg_charge_shot_fire_when_queue_reaches_max
        && d.bg_charge_shot_max_bullets_in_queue <= level
        && d.bg_charge_shot_auto_fire_delay
            < pm.ps.charge_shot_timer - (level - 1) * d.bg_charge_shot_queue_time;
    (released || full) && pm.ps.weap_flags & wf::CHARGE_SHOT != 0
}

/// G_GetWeaponDefBasedOnNumberOfBullets: the weapon a charge shot of `bullets` (the fire
/// event's parm) fires: one step down the alt chain per bullet past the first. Any other
/// weapon fires itself.
pub fn weapon_for_bullets(facts: &crate::facts::FactsTable, bullets: u32, weapon: u32) -> u32 {
    let c = facts.get(weapon).core;
    if c.fire_type != fire_type::CHARGESHOT || c.alt_weapon == 0 {
        return weapon;
    }
    let mut w = weapon;
    for _ in 1..bullets {
        let c = facts.get(w).core;
        if c.inventory_type != crate::facts::inventory::ALTMODE && c.alt_weapon != 0 {
            w = c.alt_weapon;
        }
    }
    w
}

/// PM_Weapon_CheckForFuelTime: a fuel-tank gun burns fuel while it fires.
pub fn check_for_fuel_time(pm: &mut Pm) {
    let f = pm.weapon_facts().fire;
    if !f.fuel_tank {
        return;
    }
    let mut h = pm.hand();
    if !matches!(
        h.state,
        state::FIRING | state::RECHAMBER_START | state::RECHAMBERING | state::CONT_FIRE_LOOP
    ) {
        return;
    }
    let w = pm.ps.weapon;
    // The game reads a command byte here; BO2 adds the frame time.
    let msec = pm.cmd.msec;
    let Some(held) = pm.ps.held_mut(w) else {
        return;
    };
    held.fuel_tank_time += msec;
    if held.fuel_tank_time > f.tank_life_time {
        held.fuel_tank_time = f.tank_life_time;
        pm.ps.set_ammo_in_clip(w, 0);
        h.state = state::READY;
        pm.set_hand(h);
    }
}

fn cont_fire_in(pm: &mut Pm) {
    pm.ps.weaponstate = state::CONT_FIRE_IN;
    pm.ps.weapon_time = pm.weapon_facts().fire.cont_fire_in_time;
    pm.ps.weapon_delay = 0;
    base::start_anim(pm, anim::CONT_FIRE_IN, false);
}

fn cont_fire_out(pm: &mut Pm) {
    pm.ps.weaponstate = state::CONT_FIRE_OUT;
    pm.ps.weapon_time = pm.weapon_facts().fire.cont_fire_out_time;
    pm.ps.weapon_delay = 0;
    base::start_anim(pm, anim::CONT_FIRE_OUT, false);
}

/// PM_Weapon_CheckForContinuousFire: wind a continuous-fire gun in and out.
pub fn check_for_continuous_fire(pm: &mut Pm, r: i32) {
    let wfacts = pm.weapon_facts();
    if pm.cmd.weapon == 0 || !wfacts.fire.continuous_fire {
        return;
    }
    if pm.env.waterlevel >= WATER_DEEP && pm.env.dvars.player_disable_weapons_in_water {
        return;
    }
    let s = pm.ps.weaponstate;
    let firing = matches!(
        s,
        state::FIRING | state::RECHAMBER_START | state::RECHAMBERING
    );
    let melee = (state::MELEE_INIT..=state::MELEE_END).contains(&s);
    if s != state::CONT_FIRE_LOOP && (firing || melee) {
        return;
    }
    if melee
        || (state::RAISING..=state::DROPPING_ALTSWITCH).contains(&s)
        || (state::OFFHAND_INIT..=state::OFFHAND_END).contains(&s)
        || (state::RIOTSHIELD_PREPARE..=state::RIOTSHIELD_END).contains(&s)
        || (state::DTP_IN..=state::DTP_OUT).contains(&s)
        || s == state::DETONATING
    {
        return;
    }
    let wants = should_be_firing_owed(pm, r != 0, true);
    if (wfacts.fire.overheat && overheating(pm, pm.ps.weapon)) || !wants {
        if s == state::CONT_FIRE_IN || s == state::CONT_FIRE_LOOP {
            cont_fire_out(pm);
        }
    } else if !(state::CONT_FIRE_IN..=state::CONT_FIRE_OUT).contains(&s) {
        cont_fire_in(pm);
    }
}

/// ContinuousFire_State_Loop (state CONT_FIRE_IN finished): the loop runs until the trigger
/// lets go.
pub fn continuous_fire_loop(pm: &mut Pm) {
    pm.ps.weaponstate = state::CONT_FIRE_LOOP;
    pm.ps.weapon_time = 0;
    pm.ps.weapon_delay = 0;
    base::start_anim(pm, anim::CONT_FIRE_LOOP, false);
}

/// PM_Weapon_CheckForRechamber: `true` ends the tick.
pub fn check_for_rechamber(pm: &mut Pm, r: i32) -> bool {
    let s = pm.ps.weaponstate;
    if (state::OFFHAND_INIT..=state::OFFHAND_END).contains(&s)
        || (state::DTP_IN..=state::DTP_OUT).contains(&s)
        || (state::RIOTSHIELD_PREPARE..=state::RIOTSHIELD_END).contains(&s)
    {
        return false;
    }
    let wfacts = pm.weapon_facts();
    if !wfacts.core.bolt_action || !needs_rechamber(pm, pm.ps.weapon) {
        return false;
    }
    if s == state::RECHAMBER_START && r != 0 {
        // The bolt is back: the brass comes out.
        pm.ps.weaponstate = state::RECHAMBERING;
        pm.add_event(events::EV_EJECT_BRASS);
        if pm.ps.weapon_time != 0 {
            return true;
        }
    } else if pm.ps.weapon_time != 0
        && (matches!(
            s,
            state::FIRING
                | state::RECHAMBER_START
                | state::RECHAMBERING
                | state::CONT_FIRE_LOOP
                | state::MELEE_INIT
                | state::MELEE_FIRE
                | state::MELEE_END
        ) || pm.ps.weapon_delay != 0)
    {
        return false;
    }
    let s = pm.ps.weaponstate;
    if s == state::RECHAMBERING {
        finish_rechamber(pm);
        return false;
    }
    if s == state::READY || (state::CRAWL_IN..=state::CRAWL_OUT).contains(&s) {
        let f = wfacts.fire;
        let a = if pm.ps.f_weapon_pos_frac <= ADS_ANIM_FRAC {
            anim::RECHAMBER
        } else {
            anim::ADS_RECHAMBER
        };
        base::start_anim(pm, a, false);
        pm.ps.weaponstate = state::RECHAMBER_START;
        pm.ps.weapon_time = f.rechamber_time;
        if f.shots_before_rechamber != 0 {
            pm.ps.shots_fired_from_chamber = 0;
        }
        pm.ps.weapon_delay =
            if f.rechamber_bolt_time != 0 && f.rechamber_bolt_time < f.rechamber_time {
                f.rechamber_bolt_time
            } else {
                1
            };
        pm.add_event(events::EV_RECHAMBER_WEAPON);
    }
    false
}

/// PM_Weapon_FinishRechamber.
pub fn finish_rechamber(pm: &mut Pm) {
    let left = pm.left();
    let right_playing = pm.ps.weap_anim & !anim::TOGGLE != 0;
    let left_playing = pm.ps.weap_anim_left & !anim::TOGGLE != 0;
    if (right_playing || left) && (left_playing || !left) && pm.env.pm_type < pm_type::DEAD {
        pm.ps.weap_anim = !pm.ps.weap_anim & anim::TOGGLE;
    }
    let w = pm.ps.weapon;
    set_needs_rechamber(pm, w, false);
    pm.ps.weaponstate = state::READY;
    let wfacts = pm.weapon_facts();
    if wfacts.core.bolt_action && wfacts.fire.shots_before_rechamber != 0 {
        pm.ps.shots_fired_from_chamber = 0;
    }
    if wfacts.core.fire_type == fire_type::CHARGESHOT {
        clear_charge(pm);
    }
}

/// PM_Weapon_ShouldBeFiring. When not firing and `check_only` is false, BO2 drops
/// FIRING back to READY (the left hand on a dual-wield left pass).
pub fn should_be_firing(pm: &mut Pm, check_only: bool) -> bool {
    let owed = pm.delay_fired;
    should_be_firing_owed(pm, owed, check_only)
}

/// ShouldBeFiring with the "delay just ran out" flag passed in (CheckForContinuousFire
/// asks with its own).
fn should_be_firing_owed(pm: &mut Pm, delay_fired: bool, check_only: bool) -> bool {
    if pm.env.e_flags2 & ef2::WEAPON_LOCKED != 0 || pm.env.in_vehicle {
        return false;
    }
    let wfacts = pm.weapon_facts();
    if wfacts.core.weap_type == weap_type::RIOTSHIELD {
        return false;
    }
    let left = pm.left();
    let mut held = pm.cmd.held(fire_button(pm, left));
    if wfacts.fire.freeze_movement_when_firing && !pm.env.on_ground {
        held = false;
    }
    // As the game behaves: a gun marked usable in vehicles never fires from the trigger here.
    if wfacts.core.can_use_in_vehicle {
        held = false;
    }
    let owed = delay_fired || burst_fire_pending(pm, left);
    match wfacts.core.fire_type {
        fire_type::MINIGUN => held = held && pm.ps.weapon_spin_lerp >= 1.0,
        fire_type::CHARGESHOT => held = should_fire_charge_shot(pm),
        _ => {}
    }
    let chamber_empty = wfacts.core.bolt_action
        && wfacts.fire.shots_before_rechamber != 0
        && needs_rechamber(pm, pm.ps.weapon);
    if !chamber_empty && (held || owed) {
        return true;
    }
    if check_only {
        return false;
    }
    if left && wfacts.core.dual_wield {
        if pm.ps.weaponstate_left == state::FIRING {
            base::continue_anim(pm, anim::IDLE);
        }
        pm.ps.weaponstate_left = state::READY;
        return false;
    }
    if pm.ps.weaponstate == state::FIRING && !pm.env.mantle_weapon_equipped {
        base::continue_anim(pm, anim::IDLE);
    }
    pm.ps.weaponstate = state::READY;
    false
}

/// PM_Weapon_CheckFriendlyFire: `true` blocks the shot. Zombies-only in BO2; MP never blocks.
pub fn check_friendly_fire(pm: &Pm) -> bool {
    let _ = pm;
    false
}

/// PM_Weapon_CheckFiringAmmo: enough in the clip for this shot. If not, start a reload or
/// click dry (with a 500 ms wait).
fn check_firing_ammo(pm: &mut Pm) -> bool {
    let w = pm.hand_weapon();
    let hc = pm.hand_facts();
    let needed = ammo_required(pm);
    if needed <= pm.ps.ammo_in_clip(w) {
        return true;
    }
    // A dual-wield left gun spends its partner's spare ammo.
    let stock = pm
        .ps
        .ammo_not_in_clip(super::dual::ammo_pool_weapon(pm.facts, w));
    if !hc.fire.no_ads_auto_reload || pm.ps.f_weapon_pos_frac != 1.0 {
        if needed <= stock {
            crate::pm::reload::begin_weapon_reload(pm);
            return false;
        }
        if !is_thrown(hc.core.weap_type) {
            let ev = if pm.left() {
                events::EV_NOAMMO_LEFT
            } else {
                events::EV_NOAMMO
            };
            pm.add_event(ev);
        }
    }
    if !pm.env.mantle_weapon_equipped {
        set_needs_rechamber(pm, w, false);
        base::continue_anim(pm, anim::IDLE);
        if !is_thrown(hc.core.weap_type) {
            let mut h = pm.hand();
            h.time += DRY_FIRE_MS;
            pm.set_hand(h);
        }
    }
    false
}

/// PM_Weapon_StartFiring: fire time, delay, kick restriction, state FIRING, burst count.
fn start_firing(pm: &mut Pm) {
    let w = pm.hand_weapon();
    let wfacts = pm.facts.get(w);
    let (c, f) = (wfacts.core, wfacts.fire);
    let mut h = pm.hand();
    if h.state != state::FIRING {
        pm.ps.intro_shots_fired = 0;
    }
    let mut script = true;
    if is_thrown(c.weap_type) {
        if !pm.delay_fired {
            if pm.ps.ammo_in_clip(w) != 0 {
                pm.ps.grenade_time_left = f.fuse_time;
                if pm.env.pm_type < pm_type::DEAD {
                    pm.ps.weap_anim = anim::start(pm.ps.weap_anim, anim::HOLD_FIRE);
                    pm.ps.weap_anim_left = anim::start(pm.ps.weap_anim_left, anim::HOLD_FIRE);
                }
                let parm = pm.ps.weapon;
                pm.add_predictable_event(events::EV_PULLBACK_WEAPON, parm);
            }
            pm.ps.weapon_delay = f.hold_fire_time;
            pm.ps.weapon_time = 0;
            pm.server_print("end weapon (start fire)");
            script = false;
        }
    } else {
        h.time = if is_last_shot(pm, w) && f.last_fire_time != 0 {
            f.last_fire_time
        } else if f.intro_fire_length != 0 && pm.ps.intro_shots_fired < f.intro_fire_length {
            if pm.env.rapid_fire { f.intro_fire_time / 3 } else { f.intro_fire_time }
        } else {
            fire_time(pm, w)
        };
        if c.bolt_action && f.shots_before_rechamber == 0 {
            set_needs_rechamber(pm, w, true);
        }
        if h.state != state::FIRING && h.state != state::CONT_FIRE_LOOP {
            h.delay = if pm.env.rapid_fire { 0 } else { f.fire_delay };
            let kick_bullets = if pm.ps.f_weapon_pos_frac < 1.0 {
                f.hip_kick_reduced_bullets
            } else {
                f.ads_kick_reduced_bullets
            };
            pm.ps.weapon_restrict_kick_time = fire_time(pm, w) * kick_bullets + f.fire_delay;
        }
        if f.ads_fire_only && c.fire_type != fire_type::MINIGUN {
            // Fire waits until the sights are up.
            let ads_in_ms = 1.0 / (f.oo_pos_anim_length / ads_trans_in_scale(pm, w));
            let wait = (ads_in_ms * (1.0 - pm.ps.f_weapon_pos_frac)) as i32;
            h.delay = h.delay.max(wait);
        }
        if c.guided_missile_type == GUIDED_TV {
            pm.ps.weap_flags |= wf::TV_GUIDED_START;
        }
        pm.set_hand(h);
        if !pm.env.view_height_settled {
            script = false;
        }
    }
    if script {
        pm.out.scripts.push(AnimScript::FireWeapon);
    }
    base::set_prone_override(pm);
    let mut h = pm.hand();
    if !(state::CONT_FIRE_IN..=state::CONT_FIRE_OUT).contains(&h.state) {
        h.state = state::FIRING;
    }
    if c.bolt_action && f.shots_before_rechamber != 0 {
        pm.ps.shots_fired_from_chamber += if c.fire_type == fire_type::CHARGESHOT {
            pm.ps.charge_shot_level
        } else {
            1
        };
        if f.shots_before_rechamber <= pm.ps.shots_fired_from_chamber {
            set_needs_rechamber(pm, w, true);
        }
    }
    if h.delay == 0 && c.fire_type != fire_type::FULLAUTO {
        h.shot_count = (h.shot_count + 1).min(MAX_SHOT_COUNT);
    }
    pm.set_hand(h);
}

/// PM_Weapon_SetFPSFireAnim: hip or ADS fire anim, last-shot version on an empty clip.
fn set_fps_fire_anim(pm: &mut Pm) {
    if pm.ps.weap_anim & !anim::TOGGLE == anim::MANTLE {
        return;
    }
    let empty = clip_empty(pm);
    let a = match (pm.ps.f_weapon_pos_frac <= ADS_ANIM_FRAC, empty) {
        (true, false) => anim::ATTACK,
        (true, true) => anim::ATTACK_LASTSHOT,
        (false, false) => anim::ADS_ATTACK,
        (false, true) => anim::ADS_ATTACK_LASTSHOT,
    };
    let left = pm.left();
    base::start_anim(pm, a, left);
}

/// PM_Weapon_FireWeapon: one shot (or the start of a delayed one).
pub fn fire_weapon(pm: &mut Pm) {
    let wfacts = pm.weapon_facts();
    let (c, f) = (wfacts.core, wfacts.fire);
    if pm.env.waterlevel >= WATER_DEEP && pm.env.dvars.player_disable_weapons_in_water {
        return;
    }
    if c.weap_type == weap_type::MELEE {
        return;
    }
    if c.weap_type == weap_type::GRENADE && !allow_plant(pm) {
        pm.ps.weapon_time = NO_PLANT_MS;
        pm.add_event(events::EV_CANNOTPLANT);
    }
    if f.overheat && overheating(pm, pm.ps.weapon) {
        return;
    }
    if c.guided_missile_type == GUIDED_TV && !pm.env.on_ground {
        return;
    }
    if !c.unlimited_ammo && !check_firing_ammo(pm) {
        return;
    }
    if c.fire_type == fire_type::CHARGESHOT {
        pm.ps.weap_flags &= !wf::CHARGE_SHOT;
        pm.ps.charge_shot_timer = 0;
    }
    start_firing(pm);
    if pm.hand().delay != 0 {
        return;
    }
    pm.ps.weap_flags &= !(wf::PENDING_PULL_RIGHT | wf::PENDING_PULL_LEFT);
    if f.require_lockon_to_fire {
        let lock = pm.env.weap_lock_flags;
        if lock & 0x2 == 0 {
            pm.add_event(events::EV_LOCKON_REQUIRED_HINT);
        }
        if lock & 0x10 != 0 {
            pm.add_event(events::EV_TARGET_TOO_CLOSE_HINT);
        }
        if lock & 0x20 != 0 {
            pm.add_event(events::EV_TARGET_NOT_ENOUGH_CLEARANCE);
        }
    }
    let left = pm.left();
    let hw = pm.hand_weapon();
    if pm.ps.ammo_in_clip(hw) != 0 && !f.fuel_tank && pm.env.e_flags & EF_NO_AMMO_USE == 0 {
        let need = ammo_required(pm);
        if !left || !c.dual_wield {
            let w = pm.ps.weapon;
            if use_ammo(pm, w, need) == 0 {
                pm.env.e_flags2 |= EF2_CLIP_USED_UP;
            }
        } else {
            use_ammo(pm, c.dual_wield_weapon, need);
        }
    }
    if is_thrown(c.weap_type) {
        pm.ps.weapon_time = fire_time(pm, pm.ps.weapon);
    }
    pm.ps.intro_shots_fired = (pm.ps.intro_shots_fired + 1).clamp(0, MAX_INTRO_SHOTS);
    if !(state::CONT_FIRE_IN..=state::CONT_FIRE_OUT).contains(&pm.ps.weaponstate) {
        set_fps_fire_anim(pm);
    }
    if c.guided_missile_type == GUIDED_TV {
        pm.ps.weap_flags |= wf::TV_GUIDED_FIRED;
    }
    let parm = if c.fire_type == fire_type::CHARGESHOT {
        pm.ps.charge_shot_level as u32
    } else {
        pm.hand().shot_count
    };
    let ev = match (clip_empty(pm), left) {
        (false, false) => events::EV_FIRE_WEAPON,
        (false, true) => events::EV_FIRE_WEAPON_LEFT,
        (true, false) => events::EV_FIRE_WEAPON_LASTSHOT,
        (true, true) => events::EV_FIRE_WEAPON_LASTSHOT_LEFT,
    };
    pm.add_predictable_event(ev, parm);
    crate::pm::ads::hold_breath_fire(pm);
    crate::pm::spread::add_firing_aim_spread(pm);
    crate::tick::switch_if_empty(pm);
}

#[cfg(test)]
mod tests {
    //! Round test numbers, not BO2's: the real per-gun numbers come from the facts dump.
    use crate::cmd::{T6Cmd, button};
    use crate::env::T6Env;
    use crate::facts::{FactsTable, T6WeaponFacts, fire_type};
    use crate::player::T6WeaponPs;
    use crate::{anim, events, state, tick};

    const GUN: u32 = 1;

    struct Rig {
        ps: T6WeaponPs,
        env: T6Env,
        cmd: T6Cmd,
        facts: FactsTable,
    }

    impl Rig {
        fn new(edit: impl FnOnce(&mut T6WeaponFacts)) -> Self {
            let mut f = T6WeaponFacts::default();
            f.core.clip_size = 30;
            f.core.fire_type = fire_type::FULLAUTO;
            f.fire.fire_time = 100;
            edit(&mut f);
            let mut facts = FactsTable::new();
            facts.set(GUN, "gun", f);
            let mut ps = T6WeaponPs::default();
            ps.give(GUN);
            ps.set_ammo_in_clip(GUN, f.core.clip_size);
            ps.set_ammo_not_in_clip(GUN, 90);
            ps.weapon = GUN;
            ps.set_ever_had_all(true);
            Self {
                ps,
                env: T6Env {
                    is_server: true,
                    view_height_settled: true,
                    on_ground: true,
                    plant_spot_ok: true,
                    tac_insert_allowed: true,
                    ..T6Env::default()
                },
                cmd: T6Cmd {
                    msec: 50,
                    weapon: GUN,
                    ..T6Cmd::default()
                },
                facts,
            }
        }

        /// One 50 ms tick; the (event, parm) pairs it raised.
        fn step(&mut self) -> Vec<(u16, u32)> {
            self.cmd.server_time += self.cmd.msec;
            let mut out = tick::pre_weapon(&mut self.ps, &self.cmd, &mut self.env, &self.facts);
            out.append(tick::run(
                &mut self.ps,
                &self.cmd,
                &mut self.env,
                &self.facts,
            ));
            self.cmd.old_buttons = self.cmd.buttons;
            out.events.iter().map(|e| (e.id, e.parm)).collect()
        }

        /// Run `n` ticks; the tick index (from 0) of every tick that raised `id`.
        fn ticks_with(&mut self, n: usize, id: u16) -> Vec<usize> {
            (0..n)
                .filter(|_| self.step().iter().any(|e| e.0 == id))
                .collect()
        }

        fn hold(&mut self) {
            self.cmd.buttons |= button::ATTACK;
        }

        fn release(&mut self) {
            self.cmd.buttons &= !button::ATTACK;
        }

        fn clip(&self) -> i32 {
            self.ps.ammo_in_clip(GUN)
        }
    }

    #[test]
    fn full_auto_fires_once_per_fire_time() {
        let mut r = Rig::new(|_| {});
        r.hold();
        let shots = r.ticks_with(20, events::EV_FIRE_WEAPON);
        assert_eq!(shots, vec![0, 2, 4, 6, 8, 10, 12, 14, 16, 18]);
        assert_eq!(r.clip(), 20);
        assert_eq!(r.ps.weaponstate, state::FIRING);
        r.release();
        r.step();
        r.step();
        assert_eq!(r.ps.weaponstate, state::READY);
    }

    #[test]
    fn semi_auto_waits_for_the_trigger() {
        let mut r = Rig::new(|f| f.core.fire_type = fire_type::SINGLESHOT);
        r.hold();
        assert_eq!(r.ticks_with(20, events::EV_FIRE_WEAPON), vec![0]);
        r.release();
        r.step();
        r.hold();
        assert_eq!(r.ticks_with(1, events::EV_FIRE_WEAPON), vec![0]);
        assert_eq!(r.clip(), 28);
    }

    #[test]
    fn burst_fires_three_then_waits_for_the_trigger() {
        let mut r = Rig::new(|f| {
            f.core.fire_type = fire_type::BURSTFIRE3;
            f.core.burst_delay_time = 200;
            f.fire.fire_time = 50;
        });
        r.hold();
        // Held: one burst, no second until the trigger comes back.
        assert_eq!(r.ticks_with(20, events::EV_FIRE_WEAPON), vec![0, 1, 2]);
        assert_eq!(r.clip(), 27);
        // A tap fires the whole burst.
        r.release();
        r.step();
        r.hold();
        let first = r.step();
        r.release();
        assert!(
            first.iter().any(|e| e.0 == events::EV_FIRE_WEAPON),
            "{first:?}"
        );
        let rest = r.ticks_with(20, events::EV_FIRE_WEAPON);
        assert_eq!(rest, vec![0, 1], "{rest:?}");
        assert_eq!(r.clip(), 24);
    }

    #[test]
    fn bolt_action_rechambers_then_readies() {
        let mut r = Rig::new(|f| {
            f.core.fire_type = fire_type::SINGLESHOT;
            f.core.bolt_action = true;
            f.fire.rechamber_time = 1000;
            f.fire.rechamber_bolt_time = 400;
        });
        r.hold();
        let ev = r.step();
        assert!(ev.iter().any(|e| e.0 == events::EV_FIRE_WEAPON), "{ev:?}");
        r.release();
        let mut seen = Vec::new();
        for i in 0..40 {
            for e in r.step() {
                if e.0 == events::EV_RECHAMBER_WEAPON || e.0 == events::EV_EJECT_BRASS {
                    seen.push((i, e.0));
                }
            }
            let brass = seen.iter().any(|e| e.1 == events::EV_EJECT_BRASS);
            if brass && r.ps.weaponstate == state::READY {
                seen.push((i, 0));
                break;
            }
        }
        // The shot ends (100), the rechamber starts the tick after READY, the bolt comes back
        // 400 in, the chamber is full 1000 after the start.
        assert_eq!(
            seen,
            vec![
                (2, events::EV_RECHAMBER_WEAPON),
                (10, events::EV_EJECT_BRASS),
                (22, 0)
            ]
        );
        assert!(!r.ps.held(GUN).unwrap().needs_rechamber);
    }

    #[test]
    fn last_shot_uses_its_time_anim_and_event() {
        let mut r = Rig::new(|f| f.fire.last_fire_time = 300);
        r.ps.set_ammo_in_clip(GUN, 1);
        r.ps.set_ammo_not_in_clip(GUN, 0);
        r.hold();
        let ev = r.step();
        assert!(
            ev.iter().any(|e| e.0 == events::EV_FIRE_WEAPON_LASTSHOT),
            "{ev:?}"
        );
        assert_eq!(anim::number(r.ps.weap_anim), anim::ATTACK_LASTSHOT);
        assert_eq!(r.ps.weapon_time, 300);
    }

    #[test]
    fn dry_fire_clicks_and_waits() {
        let mut r = Rig::new(|_| {});
        r.ps.set_ammo_in_clip(GUN, 0);
        r.ps.set_ammo_not_in_clip(GUN, 0);
        r.hold();
        let ev = r.step();
        assert!(ev.iter().any(|e| e.0 == events::EV_NOAMMO), "{ev:?}");
        assert_eq!(r.ps.weapon_time, 500);
    }

    #[test]
    fn fire_delay_holds_the_first_shot() {
        let mut r = Rig::new(|f| f.fire.fire_delay = 150);
        r.hold();
        r.step();
        assert_eq!(r.ps.weapon_restrict_kick_time, 150);
        let shots = r.ticks_with(5, events::EV_FIRE_WEAPON);
        assert_eq!(shots.first(), Some(&2), "{shots:?}");
    }

    #[test]
    fn minigun_spins_up_before_firing() {
        let mut r = Rig::new(|f| {
            f.core.fire_type = fire_type::MINIGUN;
            f.fire.spin_up_time = 500;
            f.fire.spin_down_time = 500;
        });
        r.hold();
        let shots = r.ticks_with(14, events::EV_FIRE_WEAPON);
        assert_eq!(shots.first(), Some(&9), "{shots:?}");
        r.release();
        r.step();
        assert!(r.ps.weapon_spin_lerp < 1.0);
    }

    #[test]
    fn continuous_fire_winds_in_loops_and_out() {
        let mut r = Rig::new(|f| {
            f.fire.continuous_fire = true;
            f.fire.cont_fire_in_time = 200;
            f.fire.cont_fire_out_time = 300;
        });
        r.hold();
        r.step();
        assert_eq!(r.ps.weaponstate, state::CONT_FIRE_IN);
        let mut states = Vec::new();
        for _ in 0..10 {
            r.step();
            states.push(r.ps.weaponstate);
        }
        assert!(states.contains(&state::CONT_FIRE_LOOP), "{states:?}");
        assert!(r.clip() < 30);
        r.release();
        r.step();
        assert_eq!(r.ps.weaponstate, state::CONT_FIRE_OUT);
        for _ in 0..8 {
            r.step();
        }
        assert_eq!(r.ps.weaponstate, state::READY);
    }

    #[test]
    fn charge_shot_queues_while_held_and_fires_on_release() {
        let mut r = Rig::new(|f| f.core.fire_type = fire_type::CHARGESHOT);
        r.hold();
        let mut fired = Vec::new();
        for _ in 0..6 {
            fired.extend(r.step());
        }
        assert!(
            !fired.iter().any(|e| e.0 == events::EV_FIRE_WEAPON),
            "{fired:?}"
        );
        // 250 ms held after the press tick: 250/128 + 1 = 2 queued.
        assert_eq!(r.ps.charge_shot_level, 2);
        r.release();
        let ev = r.step();
        let shot = ev.iter().find(|e| e.0 == events::EV_FIRE_WEAPON);
        assert_eq!(shot.map(|e| e.1), Some(2), "{ev:?}");
        assert_eq!(r.clip(), 29);
    }

    /// A dual-wield left gun has no pool of its own: the spare ammo it checks is its
    /// partner's (fiveseven_lh_mp reads fiveseven_dw_mp's), as BO2's shared ammo pool does.
    fn dual_left_rig(edit: impl Fn(&mut T6WeaponFacts)) -> (T6WeaponPs, T6Env, T6Cmd, FactsTable) {
        let mut facts = crate::pm::dual::demo_facts();
        for (w, name) in [(1, "fiveseven_dw_mp"), (2, "fiveseven_lh_mp")] {
            let mut f = facts.get(w);
            edit(&mut f);
            facts.set(w, name, f);
        }
        let mut ps = T6WeaponPs::default();
        ps.give(1);
        ps.give(2);
        ps.weapon = 1;
        ps.weaponstate = state::READY;
        ps.weaponstate_left = state::READY;
        ps.set_ammo_in_clip(1, 12);
        ps.set_ammo_in_clip(2, 0);
        ps.set_ammo_not_in_clip(1, 25);
        ps.b_run_left_gun = true;
        let cmd = T6Cmd {
            msec: 16,
            weapon: 1,
            ..T6Cmd::default()
        };
        (
            ps,
            T6Env {
                is_server: true,
                ..T6Env::default()
            },
            cmd,
            facts,
        )
    }

    #[test]
    fn dual_left_gun_empty_clip_reloads_from_the_partner_pool() {
        use crate::pm::{FrameOut, Pm};
        let (mut ps, mut env, cmd, facts) = dual_left_rig(|_| {});
        let mut out = FrameOut::default();
        let mut pm = Pm {
            ps: &mut ps,
            cmd: &cmd,
            env: &mut env,
            facts: &facts,
            out: &mut out,
            delay_fired: false,
        };
        assert!(!super::check_firing_ammo(&mut pm));
        assert_eq!(pm.ps.weaponstate_left, state::RELOADING);
        assert_eq!(pm.ps.weaponstate, state::READY);
        let ids: Vec<u16> = out.events.iter().map(|e| e.id).collect();
        assert!(!ids.contains(&events::EV_NOAMMO_LEFT), "{ids:?}");
    }

    #[test]
    fn dual_left_burst_keeps_going_on_the_partner_pool() {
        use crate::pm::{FrameOut, Pm};
        let (mut ps, mut env, cmd, facts) =
            dual_left_rig(|f| f.core.fire_type = fire_type::BURSTFIRE3);
        ps.weapon_shot_count_left = 1;
        let mut out = FrameOut::default();
        let pm = Pm {
            ps: &mut ps,
            cmd: &cmd,
            env: &mut env,
            facts: &facts,
            out: &mut out,
            delay_fired: false,
        };
        assert!(super::burst_fire_pending(&pm, true));
    }
}
