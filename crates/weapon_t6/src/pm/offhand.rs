//! Window 5: grenades, equipment, detonators and other offhands. BO2's offhand states 22-29,
//! the detonate states (25, 35), cooking, cancel, the plant check and the empty-offhand hints.
//! Spec with every number: `docs/gun/OFFHAND.md`.
//!
//! Presses are edges against the last command (`cmd.old_buttons`); the detonate step reads
//! `pm.delay_fired`; the plant check reads the game's ground and plant answers in `env`.

use super::Pm;
use crate::cmd::button;
use crate::facts::{FactsTable, offhand_class, offhand_slot, weap_type};
use crate::player::{T6WeaponPs, wf};
use crate::{anim, base, events, state};
use fastfile_t6::layout::WeaponDef as D;

/// `weapFlags` bits only the offhand code reads.
const WF_OFFHAND_BLOCKED: u64 = 0x100;
/// Cleared when a tactical insertion is picked (a new one may be planted).
const WF_TAC_INSERT_PLACED: u64 = 0x200_0000;
/// Set by offhand init when a gun is in hand: the change check quick-drops it (window 1 names
/// this `WEAPONS_DISABLED`).
const WF_OFFHAND_FROM_GUN: u64 = wf::WEAPONS_DISABLED;

/// `pm_flags` bits.
const PMF_LADDER_OFFHAND_BLOCK: u32 = 0x8000;
/// Cleared by offhand init next to RESET_ADS.
const PMF_INIT_CLEARS: u32 = 0x10;
/// `eFlags`: deployed (mounted) weapon.
const EF_DEPLOYED: u32 = 0x300;
/// `eFlags`: no planting.
const EF_NO_PLANT: u32 = 0x4000;
/// `eFlags2`: weapon locked (vehicle / script).
const EF2_WEAPON_LOCKED: u32 = 0x1000_0000;
/// `linkFlags`: linked, weapon allowed.
const LF_WEAPON_ALLOWED: u32 = 0x4;
/// Water depth at which offhands stop (under water).
const WATER_UNDER: i32 = 4;
/// `pm_type` from which viewmodel anims stop (dead).
const PM_TYPE_DEAD: i32 = 10;

/// The two offhand keys.
const OFFHAND_KEYS: u32 = button::FRAG | button::SMOKE;

/// Window 5's per-weapon facts (read from `fastfile_t6::layout` offsets).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct OffhandFacts {
    /// `iHoldFireTime`: pin pull, ms.
    pub hold_fire: i32,
    /// `iFireTime`: the whole throw, ms.
    pub fire: i32,
    /// `iFireDelay`: press-release to the grenade leaving the hand, ms.
    pub fire_delay: i32,
    /// `fuseTime`, ms.
    pub fuse: i32,
    /// `bCookOffHold`: the fuse runs while held.
    pub cook_off_hold: bool,
    /// `offhandHoldIsCancelable`.
    pub cancelable: bool,
    /// `isHoldUseGrenade`: held and used, never thrown (Black Hat).
    pub hold_use: bool,
    /// `iDetonateTime` / `iDetonateDelay`, ms.
    pub detonate_time: i32,
    pub detonate_delay: i32,
    /// `iProjectileSpeedUp`: > 0 skips the plant check.
    pub projectile_speed_up: i32,
    pub timed_detonation: bool,
    pub tactical_insertion: bool,
}

fn rd_i32(b: &[u8], at: usize) -> i32 {
    b.get(at..at + 4)
        .map(|s| i32::from_le_bytes([s[0], s[1], s[2], s[3]]))
        .unwrap_or(0)
}

fn rd_bool(b: &[u8], at: usize) -> bool {
    b.get(at).is_some_and(|&x| x != 0)
}

impl OffhandFacts {
    pub fn read(variant: &[u8], def: &[u8]) -> Self {
        let _ = variant;
        Self {
            hold_fire: rd_i32(def, D::iHoldFireTime),
            fire: rd_i32(def, D::iFireTime),
            fire_delay: rd_i32(def, D::iFireDelay),
            fuse: rd_i32(def, D::fuseTime),
            cook_off_hold: rd_bool(def, D::bCookOffHold),
            cancelable: rd_bool(def, D::offhandHoldIsCancelable),
            hold_use: rd_bool(def, D::isHoldUseGrenade),
            detonate_time: rd_i32(def, D::iDetonateTime),
            detonate_delay: rd_i32(def, D::iDetonateDelay),
            projectile_speed_up: rd_i32(def, D::iProjectileSpeedUp),
            timed_detonation: rd_bool(def, D::timedDetonation),
            tactical_insertion: rd_bool(def, D::isTacticalInsertion),
        }
    }
}

/// No enemy grenade being thrown back.
fn clear_throw_back(ps: &mut T6WeaponPs) {
    ps.throw_back_grenade_time_left = 0;
    ps.throw_back_grenade_owner = None;
}

fn is_grenade_or_mine(t: u8) -> bool {
    t == weap_type::GRENADE || t == weap_type::MINE
}

fn ammo(ps: &T6WeaponPs, w: u32) -> i32 {
    ps.ammo_in_clip(w) + ps.ammo_not_in_clip(w)
}

/// BG_UseAmmo for one offhand: one out of the clip.
fn use_ammo(pm: &mut Pm, w: u32) {
    if pm.env.dvars.player_sustain_ammo {
        return;
    }
    let c = pm.ps.ammo_in_clip(w);
    pm.ps.set_ammo_in_clip(w, c - c.min(1));
}

/// The weapon the cook / release code looks at: the offhand while one is in use, else the gun.
fn active_weapon(ps: &T6WeaponPs) -> u32 {
    if ps.weap_flags & wf::USING_OFFHAND != 0 {
        ps.off_hand_weapon
    } else {
        ps.weapon
    }
}

/// The gun in hand is a grenade held on the attack button (thrown on release).
fn is_holding_grenade(pm: &Pm) -> bool {
    let c = pm.weapon_facts().core;
    pm.ps.weapon != 0
        && is_grenade_or_mine(c.weap_type)
        && !c.hold_button_to_throw
        && pm.cmd.held(super::fire::fire_button(pm, false))
}

/// Which offhand a key picks. MP: lethal = the first held offhand, tactical = the second.
/// Zombies: the one filed in that slot. Only the first match is looked at; out of ammo picks
/// nothing, except a detonator (C4 out) or a lethal throw-back.
pub fn first_available_offhand(
    ps: &T6WeaponPs,
    facts: &FactsTable,
    zombies: bool,
    slot: u8,
) -> u32 {
    let mut skip = !zombies && slot == offhand_slot::TACTICAL_GRENADE;
    for h in &ps.held {
        let w = h.weapon;
        if w == 0 {
            continue;
        }
        let c = facts.get(w).core;
        let fits = if zombies {
            c.offhand_slot == slot
        } else {
            (offhand_slot::LETHAL_GRENADE..=offhand_slot::EQUIPMENT).contains(&c.offhand_slot)
        };
        if !fits {
            continue;
        }
        if skip {
            skip = false;
            continue;
        }
        let out = ammo(ps, w) < 1 && !c.has_detonator;
        let throw_back = slot == offhand_slot::LETHAL_GRENADE && ps.throw_back_grenade_time_left >= 1;
        return if out && !throw_back { 0 } else { w };
    }
    0
}

/// The first held offhand filed in `slot`, ammo or not (for the "none left" hint).
pub fn first_equipped_offhand(ps: &T6WeaponPs, facts: &FactsTable, slot: u8) -> u32 {
    ps.held
        .iter()
        .map(|h| h.weapon)
        .find(|&w| w != 0 && facts.get(w).core.offhand_slot == slot)
        .unwrap_or(0)
}

/// PM_WeaponAllowPlant: claymore, tactical insertion (and the Betty in the base zone).
/// `from_press` = the key press check, which never touches the gun anim.
pub fn allow_plant(pm: &mut Pm, w: u32, from_press: bool) -> bool {
    let f = pm.facts.get(w);
    if f.core.weap_type != weap_type::GRENADE || !f.core.plantable || f.offhand.projectile_speed_up > 0
    {
        return true;
    }
    if !pm.env.on_ground {
        if !from_press {
            base::start_anim(pm, anim::QUICK_RAISE, false);
        }
        return false;
    }
    if pm.env.e_flags & EF_NO_PLANT != 0 {
        return false;
    }
    if f.offhand.tactical_insertion && !pm.env.tac_insert_allowed {
        return false;
    }
    if pm.env.zombies {
        return true;
    }
    pm.env.plant_spot_ok
}

/// PM_UpdateGrenadeThrow: `true` ends the tick.
pub fn update_grenade_throw(pm: &mut Pm) -> bool {
    let w = active_weapon(pm.ps);
    if w == 0 {
        return false;
    }
    let f = pm.facts.get(w);
    if !is_grenade_or_mine(f.core.weap_type) || pm.ps.grenade_time_left <= 0 {
        return false;
    }
    // The cook fuse: raw tick time, never perk-scaled.
    if f.offhand.cook_off_hold {
        pm.ps.grenade_time_left -= pm.cmd.msec;
    }
    if pm.env.waterlevel >= WATER_UNDER
        && pm.ps.weap_flags & wf::USING_OFFHAND != 0
        && pm.ps.weaponstate == state::OFFHAND_START
    {
        // Under water while holding: forced to the release (which drops it).
        pm.ps.weaponstate = state::OFFHAND_HOLD;
        use_ammo(pm, w);
        return false;
    }
    if pm.ps.grenade_time_left > 0 {
        return false;
    }
    // Cooked off in hand.
    pm.ps.grenade_time_left = -1;
    let off = pm.ps.off_hand_weapon;
    pm.add_predictable_event(events::EV_GRENADE_SUICIDE, off);
    if pm.env.zombies {
        base::idle(pm);
        base::start_anim(pm, anim::FORCE_IDLE, false);
        clear_throw_back(pm.ps);
        if !pm.ps.has_weapon(w) {
            return true;
        }
    }
    if pm.ps.throw_back_grenade_owner.is_none() {
        use_ammo(pm, w);
    }
    true
}

/// The press gate: under water, mounted, ladder, vehicle gun, already throwing, dropping,
/// shield, dive, low ready, locked.
fn offhand_allowed(pm: &Pm) -> bool {
    let env = &*pm.env;
    let ps = &*pm.ps;
    let s = ps.weaponstate;
    env.waterlevel < WATER_UNDER
        && env.e_flags & EF_DEPLOYED == 0
        && ps.weap_flags & (wf::DISABLE_SWITCH_TO_NONE | WF_OFFHAND_BLOCKED | wf::CHANGE_LOCKED)
            == 0
        && !env.using_vehicle_weapon
        && env.pm_flags & PMF_LADDER_OFFHAND_BLOCK == 0
        && !is_holding_grenade(pm)
        && !(state::DROPPING..=state::DROPPING_ALTSWITCH).contains(&s)
        && !(state::OFFHAND_INIT..=state::OFFHAND_HOLDUSE).contains(&s)
        && !(state::DTP_IN..=state::DTP_OUT).contains(&s)
        && !(state::LOWREADY_RAISE..=state::LOWREADY_DROP).contains(&s)
        && !(state::RIOTSHIELD_PREPARE..=state::RIOTSHIELD_END).contains(&s)
        && env.e_flags2 & EF2_WEAPON_LOCKED == 0
        && env.link_flags & LF_WEAPON_ALLOWED == 0
}

/// PM_Weapon_CheckForOffHand.
pub fn check_for_offhand(pm: &mut Pm) {
    if !offhand_allowed(pm) {
        return;
    }
    let asked = pm.cmd.off_hand_weapon;
    if asked != 0 && pm.ps.has_weapon(asked) {
        pm.ps.off_hand_weapon = asked;
    }
    let slot = if pm.cmd.pressed(button::FRAG) {
        offhand_slot::LETHAL_GRENADE
    } else if pm.cmd.pressed(button::SMOKE) {
        offhand_slot::TACTICAL_GRENADE
    } else {
        return;
    };
    let w = first_available_offhand(pm.ps, pm.facts, pm.env.zombies, slot);
    if w == 0 {
        // Nothing to throw: the "no grenades" hint.
        pm.add_event(events::EV_EMPTY_OFFHAND);
        if first_equipped_offhand(pm.ps, pm.facts, slot) != 0 {
            match slot {
                offhand_slot::LETHAL_GRENADE => pm.add_event(events::EV_NO_FRAG_GRENADE_HINT),
                offhand_slot::TACTICAL_GRENADE => pm.add_event(events::EV_NO_SPECIAL_GRENADE_HINT),
                _ => {}
            }
        }
        return;
    }
    let f = pm.facts.get(w);
    if f.offhand.tactical_insertion {
        pm.ps.weap_flags &= !WF_TAC_INSERT_PLACED;
    }
    if f.core.weap_type == weap_type::GRENADE && !allow_plant(pm, w, true) {
        pm.add_predictable_event(events::EV_CANNOTPLANT, w);
        return;
    }
    pm.add_predictable_event(events::EV_SWITCH_OFFHAND, w);
    pm.ps.off_hand_weapon = w;
    if pm.ps.cursor_hint_ent_index.is_none() && (pm.ps.weapon == 0 || pm.ps.weaponstate == state::OFFHAND_END) {
        // No gun to put away (or a throw just ended): the pin comes out now.
        offhand_prepare(pm);
    } else {
        offhand_init(pm);
    }
}

/// PM_Weapon_OffHandInit: start putting the gun away (window 2's change check does the drop).
fn offhand_init(pm: &mut Pm) {
    if pm.env.waterlevel >= WATER_UNDER {
        return;
    }
    let off = pm.ps.off_hand_weapon;
    pm.add_predictable_event(events::EV_WEAPON_SWITCH_STARTED, off);
    pm.ps.weap_flags &= !wf::USING_OFFHAND;
    pm.ps.weaponstate = state::OFFHAND_INIT;
    pm.ps.weapon_delay = 0;
    if pm.ps.weapon != 0 {
        pm.ps.weap_flags |= WF_OFFHAND_FROM_GUN;
    }
    pm.ps.throw_back_grenade_owner = None;
    pm.add_event(events::EV_RESET_ADS);
    pm.env.pm_flags &= !PMF_INIT_CLEARS;
}

/// PM_Detonate (states OFFHAND_DETONATE, DETONATING).
pub fn detonate(pm: &mut Pm) {
    if pm.delay_fired && active_weapon(pm.ps) != 0 {
        pm.add_event(events::EV_DETONATE);
    } else {
        base::idle(pm);
    }
}

/// PM_Weapon_CheckForDetonation: a detonator held as the gun, attack held.
pub fn check_for_detonation(pm: &mut Pm) {
    let f = pm.weapon_facts();
    if pm.ps.weapon == 0
        || !is_grenade_or_mine(f.core.weap_type)
        || !f.core.has_detonator
        || !pm.cmd.held(button::ATTACK)
    {
        return;
    }
    let s = pm.ps.weaponstate;
    let busy = s == state::DETONATING
        || (state::RAISING..=state::RECHAMBERING).contains(&s)
        || (state::RELOADING..=state::OFFHAND_END).contains(&s)
        || (state::RIOTSHIELD_PREPARE..=state::RIOTSHIELD_END).contains(&s)
        || s == state::CONT_FIRE_LOOP
        || (state::DTP_IN..=state::DTP_OUT).contains(&s);
    if busy {
        return;
    }
    pm.ps.weaponstate = state::DETONATING;
    pm.ps.weapon_time = f.offhand.detonate_time;
    pm.ps.weapon_delay = f.offhand.detonate_delay;
    base::start_anim(pm, anim::DETONATE, false);
}

/// PM_Weapon_CheckForGrenadeThrowCancel.
pub fn check_for_grenade_throw_cancel(pm: &mut Pm) {
    let cancel = pm.cmd.held(button::OFFHAND_SWITCH);
    let off = pm.facts.get(pm.ps.off_hand_weapon);
    match pm.ps.weaponstate {
        state::OFFHAND_PREPARE => {
            let end = if off.core.hold_button_to_throw {
                !pm.cmd.held(OFFHAND_KEYS)
            } else {
                off.offhand.cancelable && cancel
            };
            if !end {
                return;
            }
            offhand_end(pm);
        }
        state::OFFHAND_INIT if off.offhand.cancelable => {
            if cancel {
                offhand_end(pm);
            }
            return;
        }
        _ => {}
    }
    let s = pm.ps.weaponstate;
    let w = if (state::OFFHAND_INIT..=state::OFFHAND_END).contains(&s) {
        pm.ps.off_hand_weapon
    } else {
        pm.ps.weapon
    };
    let f = pm.facts.get(w);
    let gun_hold_throw = pm.ps.weapon != 0
        && matches!(
            s,
            state::FIRING
                | state::RECHAMBER_START
                | state::RECHAMBERING
                | state::CONT_FIRE_LOOP
                | state::MELEE_INIT
                | state::MELEE_FIRE
                | state::MELEE_END
        )
        && f.core.hold_button_to_throw;
    if is_grenade_or_mine(f.core.weap_type)
        && (f.offhand.cancelable || gun_hold_throw)
        && !pm.cmd.held(button::ATTACK)
        && (!f.offhand.cancelable || cancel)
        && f.core.offhand_slot != offhand_slot::EQUIPMENT
    {
        base::idle(pm);
        base::start_anim(pm, anim::FORCE_IDLE, false);
    }
}

/// PM_Weapon_CheckGrenadeHold: `true` blocks firing. BO2 looks only on the tick the hold-fire
/// delay runs out (`r`): the first press goes on to FireWeapon, which starts the hold_fire
/// anim and the iHoldFireTime wait; after that, a held trigger keeps the throw waiting.
pub fn check_grenade_hold(pm: &mut Pm, r: i32) -> bool {
    if r == 0 {
        return false;
    }
    if is_holding_grenade(pm) {
        pm.ps.weapon_delay = 1;
        return true;
    }
    false
}

/// PM_Weapon_OffHandPrepare (state OFFHAND_INIT): the gun is away, pull the pin.
pub fn offhand_prepare(pm: &mut Pm) {
    if pm.env.waterlevel >= WATER_UNDER || pm.ps.weapon != 0 {
        return;
    }
    let off = pm.ps.off_hand_weapon;
    let f = pm.facts.get(off);
    pm.ps.weaponstate = state::OFFHAND_PREPARE;
    pm.ps.weap_flags |= wf::USING_OFFHAND;
    pm.ps.weap_flags &= !WF_OFFHAND_FROM_GUN;
    pm.ps.weapon_time = f.offhand.hold_fire;
    pm.ps.weapon_delay = 0;
    if pm.ps.throw_back_grenade_owner.is_some() {
        base::start_anim(pm, anim::ALTSWITCHTO, false);
    } else if !f.core.has_detonator || ammo(pm.ps, off) != 0 {
        pm.add_predictable_event(events::EV_PREP_OFFHAND, off);
        if pm.env.pm_type < PM_TYPE_DEAD {
            pm.ps.weap_anim = anim::start(pm.ps.weap_anim, anim::HOLD_FIRE);
            pm.ps.weap_anim_left = anim::start(pm.ps.weap_anim_left, anim::HOLD_FIRE);
        }
        // Third person: gear (equipment, BO2's raise event) or a grenade prime (1/1; the
        // gun slot is always empty here).
        pm.out.scripts.push(if f.core.offhand_class == offhand_class::GEAR {
            events::AnimScript::PrimeGear
        } else {
            events::AnimScript::PrimeGrenade
        });
    } else {
        // A detonator with none left (C4 out): the key detonates.
        pm.ps.weaponstate = state::OFFHAND_DETONATE;
        pm.ps.weapon_time = f.offhand.detonate_time;
        pm.ps.weapon_delay = f.offhand.detonate_delay;
        base::start_anim(pm, anim::DETONATE, false);
    }
    base::set_prone_override(pm);
}

/// PM_Weapon_OffHandHold (state OFFHAND_PREPARE): the pin is out, the cook fuse starts.
pub fn offhand_hold(pm: &mut Pm) {
    pm.ps.weap_flags |= wf::USING_OFFHAND;
    pm.ps.weaponstate = state::OFFHAND_START;
    pm.ps.weapon_time = 0;
    pm.ps.weapon_delay = 0;
    if pm.ps.throw_back_grenade_owner.is_none() {
        pm.ps.grenade_time_left = pm.facts.get(pm.ps.off_hand_weapon).offhand.fuse;
    }
}

/// PM_Weapon_OffHandStart (state OFFHAND_START): held until the key comes up, then the throw.
pub fn offhand_start(pm: &mut Pm) {
    let off = pm.ps.off_hand_weapon;
    let f = pm.facts.get(off);
    if f.core.weap_type == weap_type::GRENADE && !allow_plant(pm, off, false) {
        pm.add_predictable_event(events::EV_CANNOTPLANT, off);
        offhand_end(pm);
        return;
    }
    // (BO2 also tests a T5 smoke-colour button here; no T6 binding sets it.)
    if !f.core.hold_button_to_throw
        && pm.cmd.old_buttons & OFFHAND_KEYS != 0
        && pm.cmd.held(OFFHAND_KEYS)
    {
        if f.offhand.cancelable && pm.cmd.held(button::OFFHAND_SWITCH) {
            offhand_end(pm);
        } else {
            // Still held: look again next tick.
            pm.ps.weapon_delay = 1;
        }
        return;
    }
    if f.offhand.hold_use {
        offhand_hold_use(pm);
        return;
    }
    pm.ps.weaponstate = state::OFFHAND_HOLD;
    pm.ps.weapon_time = f.offhand.fire;
    pm.ps.weapon_delay = f.offhand.fire_delay;
    pm.ps.weap_flags |= wf::USING_OFFHAND;
    base::start_anim(pm, anim::ATTACK, false);
    // Third person throw (BO2 sends its fire-weapon event here, 0/1).
    pm.out.scripts.push(events::AnimScript::OffHandStart);
}

/// PM_Weapon_OffHand_HoldUse (state OFFHAND_HOLDUSE): a held device is put away, not thrown.
pub fn offhand_hold_use(pm: &mut Pm) {
    let off = pm.ps.off_hand_weapon;
    pm.ps.weapon_time = pm.facts.get(off).switch.quick_drop;
    pm.ps.weapon_delay = 0;
    base::start_anim(pm, anim::DROP, false);
    pm.out.scripts.push(events::AnimScript::DropWeapon);
    pm.ps.weap_flags |= wf::USING_OFFHAND;
    pm.ps.weaponstate = state::OFFHAND;
}

/// PM_Weapon_OffHand (state OFFHAND_HOLD): the delay ran out, the grenade leaves the hand.
pub fn offhand(pm: &mut Pm) {
    let off = pm.ps.off_hand_weapon;
    if pm.env.waterlevel >= WATER_UNDER {
        pm.add_predictable_event(events::EV_GRENADE_DROP, off);
        pm.ps.weap_flags &= !wf::USING_OFFHAND;
        pm.ps.weap_flags |= wf::SWIM_LOWERED;
        pm.env.pm_flags &= !crate::env::pmf::PRONE_MOVE_OVERRIDE;
        pm.ps.weaponstate = state::SWIM_IN;
        return;
    }
    let f = pm.facts.get(off);
    if f.core.weap_type == weap_type::GRENADE && !allow_plant(pm, off, false) {
        pm.add_predictable_event(events::EV_CANNOTPLANT, off);
        offhand_end(pm);
        return;
    }
    pm.add_predictable_event(events::EV_USE_OFFHAND, off);
    if pm.ps.throw_back_grenade_owner.is_none() {
        if ammo(pm.ps, off) == 0 {
            pm.add_event(events::EV_EMPTY_OFFHAND);
        } else {
            use_ammo(pm, off);
        }
    }
    pm.ps.weap_flags |= wf::USING_OFFHAND;
    pm.ps.weaponstate = state::OFFHAND;
}

/// PM_Weapon_OffHandEnd (state OFFHAND): the throw is over, the gun comes back.
pub fn offhand_end(pm: &mut Pm) {
    if pm.ps.weapon == 0 {
        // The gun comes up through the change check, quick.
        pm.ps.weap_flags |= wf::FORCE_QUICK_SWITCH;
        pm.ps.weapon_time = 0;
        pm.ps.weapon_delay = 1;
    } else {
        let gun = pm.weapon_facts();
        pm.ps.weapon_time = gun.switch.quick_raise;
        pm.ps.weapon_delay = 0;
        let empty = !gun.core.unlimited_ammo && pm.ps.ammo_in_clip(pm.ps.weapon) == 0;
        let a = if empty {
            anim::EMPTY_RAISE
        } else {
            anim::QUICK_RAISE
        };
        base::start_anim(pm, a, false);
        if gun.core.dual_wield {
            base::start_anim(pm, anim::QUICK_RAISE, true);
        }
    }
    pm.ps.weap_flags &= !(wf::USING_OFFHAND | WF_OFFHAND_FROM_GUN);
    pm.env.pm_flags &= !crate::env::pmf::PRONE_MOVE_OVERRIDE;
    clear_throw_back(pm.ps);
    pm.ps.weaponstate = state::OFFHAND_END;
}

/// The client's double tap of Use/Reload (alt-detonate for C4): a tap is a press released
/// within `max_hold` ms, and a second tap within `max_delay` ms of the last one counts.
#[derive(Clone, Copy, Debug, Default)]
pub struct DoubleTap {
    pressed_at: Option<i32>,
    last_tap_at: Option<i32>,
}

impl DoubleTap {
    /// `cl_dblTapMaxHoldTime` / `cl_dblTapMaxDelayTime` defaults.
    pub const MAX_HOLD: i32 = 500;
    pub const MAX_DELAY: i32 = 500;

    /// Feed the key state at `now` ms; `true` on the release that completes a double tap.
    pub fn update(&mut self, held: bool, now: i32) -> bool {
        match (held, self.pressed_at) {
            (true, None) => {
                self.pressed_at = Some(now);
                false
            }
            (false, Some(at)) => {
                self.pressed_at = None;
                if now - at > Self::MAX_HOLD {
                    self.last_tap_at = None;
                    return false;
                }
                match self.last_tap_at {
                    Some(last) if at - last <= Self::MAX_DELAY => {
                        self.last_tap_at = None;
                        true
                    }
                    _ => {
                        self.last_tap_at = Some(now);
                        false
                    }
                }
            }
            _ => false,
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use crate::cmd::button;
    use crate::facts::{FactsTable, T6WeaponFacts, offhand_class, offhand_slot, weap_type};
    use crate::player::T6WeaponPs;
    use crate::{env::T6Env, events, state, tick};

    const SMG: u32 = 1;
    const FRAG: u32 = 2;
    const SEMTEX: u32 = 3;
    const C4: u32 = 4;
    const CLAYMORE: u32 = 5;
    const SMOKE: u32 = 6;

    fn grenade(slot: u8, class: u8, quick_drop: i32) -> T6WeaponFacts {
        let mut f = T6WeaponFacts::default();
        f.core.weap_type = weap_type::GRENADE;
        f.core.offhand_slot = slot;
        f.core.offhand_class = class;
        f.core.clip_size = 2;
        f.switch.quick_drop = quick_drop;
        f
    }

    /// BO2 MP numbers (docs/gun/OFFHAND.md section 14); the gun is a round test gun.
    fn table() -> FactsTable {
        let mut t = FactsTable::new();
        let mut smg = T6WeaponFacts::default();
        smg.core.clip_size = 30;
        smg.switch.quick_raise = 300;
        smg.switch.quick_drop = 250;
        smg.switch.raise = 500;
        smg.switch.drop = 400;
        t.set(SMG, "smg", smg);

        let mut frag = grenade(offhand_slot::LETHAL_GRENADE, offhand_class::FRAG_GRENADE, 150);
        frag.offhand.hold_fire = 550;
        frag.offhand.fire = 365;
        frag.offhand.fire_delay = 120;
        frag.offhand.fuse = 3500;
        frag.offhand.cook_off_hold = true;
        t.set(FRAG, "frag_grenade_mp", frag);

        let mut sem = grenade(offhand_slot::LETHAL_GRENADE, offhand_class::FRAG_GRENADE, 150);
        sem.offhand.hold_fire = 600;
        sem.offhand.fire = 360;
        sem.offhand.fire_delay = 130;
        sem.offhand.fuse = 2000;
        sem.offhand.cancelable = true;
        t.set(SEMTEX, "sticky_grenade_mp", sem);

        let mut c4 = grenade(offhand_slot::EQUIPMENT, offhand_class::GEAR, 200);
        c4.core.has_detonator = true;
        c4.offhand.hold_fire = 16;
        c4.offhand.fire = 600;
        c4.offhand.fire_delay = 150;
        c4.offhand.detonate_time = 500;
        c4.offhand.detonate_delay = 230;
        c4.offhand.cancelable = true;
        t.set(C4, "satchel_charge_mp", c4);

        let mut cl = grenade(offhand_slot::EQUIPMENT, offhand_class::GEAR, 150);
        cl.core.plantable = true;
        cl.offhand.hold_fire = 200;
        cl.offhand.fire = 480;
        cl.offhand.fire_delay = 160;
        cl.offhand.cancelable = true;
        t.set(CLAYMORE, "claymore_mp", cl);

        let mut sm = grenade(offhand_slot::TACTICAL_GRENADE, offhand_class::SMOKE_GRENADE, 150);
        sm.offhand.hold_fire = 200;
        sm.offhand.fire = 300;
        sm.offhand.fire_delay = 150;
        sm.offhand.fuse = 10;
        sm.offhand.cancelable = true;
        t.set(SMOKE, "willy_pete_mp", sm);
        t
    }

    /// A player holding the test gun, READY, with the given offhands (2 each).
    struct Sim {
        ps: T6WeaponPs,
        env: T6Env,
        cmd: crate::cmd::T6Cmd,
        facts: FactsTable,
        now: i32,
        /// (ms, state, event names) per tick.
        log: Vec<(i32, i32, Vec<String>)>,
        /// The cook fuse after each tick.
        fuse: Vec<i32>,
        /// (ms, third-person script) for every script pushed.
        scripts: Vec<(i32, events::AnimScript)>,
    }

    impl Sim {
        fn new(offhands: &[u32]) -> Self {
            let facts = table();
            let mut ps = T6WeaponPs::default();
            for &w in [SMG].iter().chain(offhands) {
                ps.give(w);
                ps.set_ammo_in_clip(w, facts.get(w).core.clip_size);
            }
            ps.weapon = SMG;
            ps.weaponstate = state::READY;
            let env = T6Env {
                is_server: true,
                view_height_settled: true,
                on_ground: true,
                plant_spot_ok: true,
                tac_insert_allowed: true,
                ..T6Env::default()
            };
            let cmd = crate::cmd::T6Cmd {
                msec: 16,
                weapon: SMG,
                ..Default::default()
            };
            Self {
                ps,
                env,
                cmd,
                facts,
                now: 0,
                log: Vec::new(),
                fuse: Vec::new(),
                scripts: Vec::new(),
            }
        }

        fn tick(&mut self) {
            self.now += self.cmd.msec;
            self.cmd.server_time = self.now;
            // The trace runner's order: the ADS flag, then the weapon tick.
            let mut out = tick::pre_weapon(&mut self.ps, &self.cmd, &mut self.env, &self.facts);
            out.append(tick::run(&mut self.ps, &self.cmd, &mut self.env, &self.facts));
            self.cmd.old_buttons = self.cmd.buttons;
            let evs = out.events.iter().map(|e| events::name(e.id).to_owned()).collect();
            self.log.push((self.now, self.ps.weaponstate, evs));
            self.fuse.push(self.ps.grenade_time_left);
            let now = self.now;
            self.scripts.extend(out.scripts.iter().map(|&a| (now, a)));
        }

        fn run(&mut self, n: usize) {
            for _ in 0..n {
                self.tick();
            }
        }

        fn hold(&mut self, b: u32) {
            self.cmd.buttons |= b;
        }

        fn release(&mut self, b: u32) {
            self.cmd.buttons &= !b;
        }

        /// The ms of the first tick that raised `ev` (by name), after `from` ms.
        fn when(&self, ev: &str, from: i32) -> Option<i32> {
            self.log
                .iter()
                .find(|(t, _, e)| *t > from && e.iter().any(|x| x == ev))
                .map(|(t, _, _)| *t)
        }

        /// The ms of the first tick that ended in state `s`, after `from` ms.
        fn when_state(&self, s: i32, from: i32) -> Option<i32> {
            self.log
                .iter()
                .find(|(t, st, _)| *t > from && *st == s)
                .map(|(t, _, _)| *t)
        }

        fn dump(&self) -> String {
            self.log
                .iter()
                .map(|(t, s, e)| format!("{t:>5} {:<16} {}", state::name(*s), e.join(" ")))
                .collect::<Vec<_>>()
                .join("\n")
        }
    }

    /// Ticks of `msec` needed for a timer of `ms` to run out (counted from the tick after it
    /// is set).
    fn ticks(ms: i32, msec: i32) -> i32 {
        (ms + msec - 1) / msec
    }

    #[test]
    fn third_person_scripts_and_throw_back_cleared() {
        use events::AnimScript as A;
        let mut s = Sim::new(&[FRAG, CLAYMORE]);
        s.ps.throw_back_grenade_owner = Some(7);
        s.ps.throw_back_grenade_time_left = 900;
        s.run(1);
        s.hold(button::FRAG);
        s.tick();
        // Init clears the owner (the time left stays until the end).
        assert_eq!(s.ps.throw_back_grenade_owner, None);
        s.release(button::FRAG);
        s.run(120);
        let log = s.dump();
        let prep = s.when("PREP_OFFHAND", 0).expect("prep");
        let used = s.when("USE_OFFHAND", 0).expect("thrown");
        let got: Vec<_> = s.scripts.iter().filter(|(_, a)| *a != A::DropWeapon).collect();
        let hold = s.when_state(state::OFFHAND_START, 0).expect("held");
        assert_eq!(got.first(), Some(&&(prep, A::PrimeGrenade)), "{log} {got:?}");
        assert_eq!(got.get(1), Some(&&(hold + 16, A::OffHandStart)), "{log} {got:?}");
        assert!(used > hold, "{log}");
        assert_eq!(s.ps.throw_back_grenade_time_left, 0, "{log}");

        // Equipment primes as gear, not as a grenade.
        let mut s = Sim::new(&[CLAYMORE]);
        s.run(1);
        s.hold(button::FRAG);
        s.run(60);
        assert!(s.scripts.iter().any(|(_, a)| *a == A::PrimeGear), "{}", s.dump());
        assert!(!s.scripts.iter().any(|(_, a)| *a == A::PrimeGrenade), "{}", s.dump());
    }

    #[test]
    fn frag_tap_from_ready() {
        let mut s = Sim::new(&[FRAG]);
        s.run(2);
        s.hold(button::FRAG);
        s.tick(); // the press
        let press = s.now;
        s.release(button::FRAG);
        s.run(120);
        let log = s.dump();
        let t = 16;
        // Gun down (quickDrop 150), then prepare the tick after the drop ends.
        let prep = press + (ticks(150, t) + 1) * t;
        assert_eq!(s.when("PREP_OFFHAND", press), Some(prep), "{log}");
        // Pin pull 550, hold the tick it ends, throw starts the tick after.
        let hold = prep + ticks(550, t) * t;
        assert_eq!(s.when_state(state::OFFHAND_START, press), Some(hold), "{log}");
        let throw = hold + t;
        assert_eq!(s.when_state(state::OFFHAND_HOLD, press), Some(throw), "{log}");
        // Leaves at the fire delay (120).
        let leaves = throw + ticks(120, t) * t;
        assert_eq!(s.when("USE_OFFHAND", press), Some(leaves), "{log}");
        assert!((840..=900).contains(&(leaves - press)), "{} {log}", leaves - press);
        // Throw ends at fireTime (365), then idle, then the gun comes up.
        let end = throw + ticks(365, t) * t;
        assert_eq!(s.when_state(state::OFFHAND_END, press), Some(end), "{log}");
        assert_eq!(s.ps.ammo_in_clip(FRAG), 1, "{log}");
        assert_eq!(s.ps.weapon, SMG, "{log}");
        assert_eq!(s.ps.weaponstate, state::READY, "{log}");
        // The fuse the server reads at the throw: 3500 minus the cook from hold to release.
        let at = s.log.iter().position(|l| l.0 == leaves).unwrap();
        assert_eq!(s.fuse[at], 3500 - (leaves - hold), "{log}");
        // The gun change clears it.
        assert_eq!(s.ps.grenade_time_left, 0);
    }

    #[test]
    fn frag_cook_off_kills() {
        let mut s = Sim::new(&[FRAG]);
        s.run(1);
        s.hold(button::FRAG);
        s.run(400);
        let log = s.dump();
        let hold = s.when_state(state::OFFHAND_START, 0).expect("held");
        let boom = s.when("GRENADE_SUICIDE", 0).expect("cook-off");
        // The fuse starts at hold and counts the raw tick: 3500 / 16 -> 219 ticks.
        assert_eq!(boom - hold, ticks(3500, 16) * 16, "{log}");
        assert_eq!(s.ps.ammo_in_clip(FRAG), 1, "{log}");
        assert!(s.when("USE_OFFHAND", 0).is_none(), "{log}");
    }

    #[test]
    fn semtex_cancel_while_held() {
        let mut s = Sim::new(&[SEMTEX]);
        s.run(1);
        s.hold(button::FRAG);
        s.run(60);
        assert_eq!(s.ps.weaponstate, state::OFFHAND_START, "{}", s.dump());
        s.hold(button::OFFHAND_SWITCH);
        s.tick();
        s.release(button::OFFHAND_SWITCH | button::FRAG);
        s.run(60);
        let log = s.dump();
        assert!(s.when("USE_OFFHAND", 0).is_none(), "{log}");
        assert_eq!(s.ps.ammo_in_clip(SEMTEX), 2, "{log}");
        assert_eq!(s.ps.weapon, SMG, "{log}");
        assert_eq!(s.ps.weaponstate, state::READY, "{log}");
    }

    #[test]
    fn frag_cannot_cancel() {
        let mut s = Sim::new(&[FRAG]);
        s.run(1);
        s.hold(button::FRAG);
        s.run(60);
        s.hold(button::OFFHAND_SWITCH);
        s.tick();
        assert_eq!(s.ps.weaponstate, state::OFFHAND_START, "{}", s.dump());
    }

    #[test]
    fn claymore_plant_fail_on_press() {
        let mut s = Sim::new(&[CLAYMORE]);
        s.env.plant_spot_ok = false;
        s.run(1);
        s.hold(button::FRAG);
        s.tick();
        let log = s.dump();
        assert!(s.when("CANNOTPLANT", 0).is_some(), "{log}");
        assert_eq!(s.ps.weaponstate, state::READY, "{log}");
    }

    #[test]
    fn c4_out_detonates() {
        let mut s = Sim::new(&[C4]);
        s.ps.set_ammo_in_clip(C4, 0);
        s.run(1);
        s.hold(button::FRAG);
        s.tick();
        let press = s.now;
        s.release(button::FRAG);
        s.run(80);
        let log = s.dump();
        // Gun down (C4 quickDrop 200), detonate state the tick after, DETONATE at 230.
        let det_state = press + (ticks(200, 16) + 1) * 16;
        assert_eq!(s.when_state(state::OFFHAND_DETONATE, press), Some(det_state), "{log}");
        assert_eq!(s.when("DETONATE", press), Some(det_state + ticks(230, 16) * 16), "{log}");
        assert!(s.when("PREP_OFFHAND", 0).is_none(), "{log}");
        assert_eq!(s.ps.weapon, SMG, "{log}");
    }

    #[test]
    fn empty_lethal_hint() {
        let mut s = Sim::new(&[FRAG]);
        s.ps.set_ammo_in_clip(FRAG, 0);
        s.run(1);
        s.hold(button::FRAG);
        s.tick();
        let log = s.dump();
        assert!(s.when("EMPTY_OFFHAND", 0).is_some(), "{log}");
        assert!(s.log.last().is_some_and(|l| l.2.len() == 2), "16 + 123: {log}");
        assert_eq!(s.ps.weaponstate, state::READY, "{log}");
    }

    #[test]
    fn tactical_key_takes_the_second_offhand() {
        let mut s = Sim::new(&[FRAG, SMOKE]);
        s.run(1);
        s.hold(button::SMOKE);
        s.tick();
        assert_eq!(s.ps.off_hand_weapon, SMOKE);
    }

    #[test]
    fn throw_mid_reload_and_back_to_back() {
        let mut s = Sim::new(&[FRAG]);
        s.ps.weaponstate = state::RELOADING;
        s.ps.weapon_time = 2000;
        s.run(1);
        s.hold(button::FRAG);
        s.tick();
        s.release(button::FRAG);
        let log = s.dump();
        assert_eq!(s.ps.weaponstate, state::DROPPING_QUICK, "{log}");
        assert_eq!(s.ps.weapon_time, 150, "grenade quickDrop replaces the reload: {log}");
        // Press again on the tick the throw ends: back-to-back, no gun drop.
        while s.ps.weaponstate != state::OFFHAND_END {
            s.tick();
        }
        s.hold(button::FRAG);
        s.tick();
        let log = s.dump();
        // No gun to drop, so it preps at once; the end's one-tick delay fired this tick, so the
        // dispatch also runs the hold: a press on that exact tick skips the pin pull.
        assert!(s.log.last().unwrap().2.iter().any(|e| e == "PREP_OFFHAND"), "{log}");
        assert_eq!(s.ps.weaponstate, state::OFFHAND_START, "{log}");
        assert_eq!(s.ps.weapon, 0, "{log}");
    }

    #[test]
    fn press_while_dropping_is_lost() {
        let mut s = Sim::new(&[FRAG]);
        s.ps.weaponstate = state::DROPPING;
        s.ps.weapon_time = 300;
        s.hold(button::FRAG);
        s.tick();
        assert!(s.when("SWITCH_OFFHAND", 0).is_none(), "{}", s.dump());
    }

    #[test]
    fn fast_toss_halves_the_pin_pull() {
        let mut s = Sim::new(&[FRAG]);
        s.env.perks |= 1 << crate::env::perk::FASTTOSS;
        s.run(1);
        s.hold(button::FRAG);
        s.tick();
        s.release(button::FRAG);
        s.run(100);
        let prep = s.when("PREP_OFFHAND", 0).expect("prep");
        let hold = s.when_state(state::OFFHAND_START, 0).expect("hold");
        // 550 at 32 ms a tick.
        assert_eq!(hold - prep, ticks(550, 32) * 16, "{}", s.dump());
    }

    #[test]
    fn double_tap() {
        let mut d = super::DoubleTap::default();
        assert!(!d.update(true, 0));
        assert!(!d.update(false, 100));
        assert!(!d.update(true, 300));
        assert!(d.update(false, 400));
        // Too slow between taps.
        assert!(!d.update(true, 1000));
        assert!(!d.update(false, 1100));
        assert!(!d.update(true, 1700));
        assert!(!d.update(false, 1800));
    }
}
