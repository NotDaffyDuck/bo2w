//! Weapon switching, by BO2's rules (docs/gun/SWITCH.md has the full timing spec and the real numbers).
//!
//! Server side (runs inside the weapon tick): the change check, the drop, the raise, the fast-hands timer step and
//! the out-of-ammo nudge. Client side ([`ClientSwitch`]): which gun the command asks for on next/previous weapon,
//! a direct pick and the attachment (alt) toggle.
//!
//! Tick contract (SWITCH.md section 2): the frame counts the timers down with [`timer_step`], runs the checks in
//! BO2's order with [`check_for_change`] between the riot shield check and the reload check, and when both timers
//! are 0 at the end calls [`finish_if_due`], which ends a drop or a raise on that same tick.

use crate::anim;
use crate::cmd::T6Cmd;
use crate::env::{T6Env, perk, movement_mode, pmf};
use crate::events::{
    AnimScript, Event, EV_WEAPON_BREAKING_DOWN, EV_STOP_WEAPON_SOUND, EV_WEAPON_SWITCH_STARTED,
    EV_WEAPON_ALT, EV_PUTAWAY_WEAPON, EV_RAISE_PICKUP_WEAPON, EV_FIRST_RAISE_WEAPON,
    EV_RAISE_WEAPON, EV_NOAMMO,
};
use crate::facts::{
    CoreFacts, FactsTable, SwitchFacts, inventory, offhand_slot, gun_kind_ids, gun_family_code,
};
use crate::player::{MAX_HELD, T6WeaponPs, wf};
use crate::state;

/// BO2 cmd button 46: lets an equipment offhand hand over to a switch.
pub const OFFHAND_SWITCH_BUTTON: u32 = crate::cmd::button::OFFHAND_SWITCH;

/// `eFlags` bits that mean a deployed weapon is up.
const EF_DEPLOYED: u32 = 0x300;
/// `eFlags2`: the raise is from an empty clip (third person reads it).
pub const EF2_EMPTY_RAISE: u32 = 0x400;
/// `eFlags2`: script locked the weapon.
const EF2_WEAPON_LOCKED: u32 = 0x1000_0000;
/// `linkFlags`: linked to something that holds the weapon.
const LF_WEAPON_LOCKED: u32 = 4;
/// `otherFlags`: in a vehicle seat that refuses switches.
const OF_VEHICLE_NO_SWITCH: u32 = 0x400;
/// `mantleState.flags`: the mantle forces quick switches.
const MANTLE_QUICK: u32 = 0x100;
/// `mantleState.flags`: a drop started mid-mantle ends no later than the mantle does.
const MANTLE_CAP_DROP: u32 = 0x2000;
/// `guidedMissileType_t` values that keep a gun in hand at 0 ammo (the player is still steering the shot).
const GUIDED_WIRE: i32 = 5;
const GUIDED_TV: i32 = 6;
/// `movement_state_bits` that stop the third-person drop anim.
const PMF_NO_DROP_SCRIPT: u32 = 0x8004;
/// `movement_state_bits` that stop the third-person raise anim.
const PMF_NO_RAISE_SCRIPT: u32 = 0xa008;
/// Aim spread after a normal raise, and the floor after an alt raise.
const SPREAD_RAISE: f32 = 255.0;
const SPREAD_ALT_MIN: f32 = 128.0;

/// What a switch call produced besides the player-state change, in BO2's order.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SwitchOut {
    pub events: Vec<Event>,
    pub scripts: Vec<AnimScript>,
    /// `Some(v)`: set (`true`) or clear the empty-raise bit in `eFlags2` ([`EF2_EMPTY_RAISE`]).
    pub empty_raise_flag: Option<bool>,
    /// Clear the deployed bits in `eFlags` (a switch asked a deployed weapon with no break-down pending).
    pub clear_deployed: bool,
    /// The new gun in hand is a riot shield: show its plain model again (not the can't-plant one).
    pub reset_riotshield_model: bool,
    /// PM_SetProneMovementOverride: a drop or raise started, so a prone player stops crawling until the gun is idle.
    pub prone_override: bool,
}

impl SwitchOut {
    fn push_id(&mut self, id: u16) {
        self.events.push(Event {
            id,
            parm: 0,
            predictable: false,
        });
    }

    fn add_predictable(&mut self, id: u16, parm: u32) {
        self.events.push(Event {
            id,
            parm,
            predictable: true,
        });
    }
}

fn core(facts: &FactsTable, weapon: u32) -> CoreFacts {
    facts.get(weapon).core
}

fn times(facts: &FactsTable, weapon: u32) -> SwitchFacts {
    facts.get(weapon).switch
}

fn alive(env: &T6Env) -> bool {
    env.movement_mode < movement_mode::DEAD
}

fn start_anim(ps: &mut T6WeaponPs, env: &T6Env, a: i32, left_too: bool) {
    if alive(env) {
        ps.gun_clip = anim::start(ps.gun_clip, a);
        if left_too {
            ps.weap_anim_left = anim::start(ps.weap_anim_left, a);
        }
    }
}

/// The gun-down part of BO2's idle: both hands to READY with no time left, anim IDLE.
fn idle(ps: &mut T6WeaponPs, env: &T6Env) {
    ps.weapon_state_bits &= !wf::USING_OFFHAND;
    ps.gun_timer_ms = Default::default();
    ps.weapon_ready_wait = 0;
    ps.weapon_time_left = 0;
    ps.weapon_delay_left = 0;
    ps.gun_phase = state::READY;
    ps.weaponstate_left = state::READY;
    start_anim(ps, env, anim::REST, true);
}

// ---------------------------------------------------------------------------------------------------------------
// The tests (SWITCH.md section 6)
// ---------------------------------------------------------------------------------------------------------------

/// Quick switch: quick drop and quick raise. `state_now` is the weapon state when asked (during the raise half it is
/// the drop state that just ended, so a quick drop always gets a quick raise).
pub fn is_quick_switch(facts: &FactsTable, old: u32, new: u32, state_now: i32) -> bool {
    let o = core(facts, old);
    let n = core(facts, new);
    if new != 0
        && matches!(
            n.weap_class,
            gun_kind_ids::PISTOL | gun_kind_ids::PISTOL_SPREAD | gun_kind_ids::CLOSE_COMBAT
        )
    {
        return true;
    }
    state_now == state::DROPPING_QUICK
        || (old == 0 && n.gun_family_code == gun_family_code::PROJECTILE)
        || o.can_use_in_vehicle
        || n.can_use_in_vehicle
        || o.offhand_slot != offhand_slot::NONE
        || n.offhand_slot != offhand_slot::NONE
}

/// Fast hands may speed this state up: drop or raise, and not a specific-use offhand.
pub fn quick_swap_ok(facts: &FactsTable, gun: u32, weaponstate: i32) -> bool {
    (state::RAISING..=state::DROPPING_ALTSWITCH).contains(&weaponstate)
        && core(facts, gun).offhand_slot != offhand_slot::SPECIFIC_USE
}

/// Attachment switch: both real guns and the new one is the old one's other mode.
pub fn is_alt_switch(ps: &T6WeaponPs, facts: &FactsTable, old: u32, new: u32) -> bool {
    if old == 0 || new == 0 {
        return false;
    }
    let o = core(facts, old);
    new == o.alt_weapon
        || (o.carry_slot_kind == inventory::ALTMODE && ps.last_weapon_alt_mode_switch == new)
}

/// Both guns are ADS-only modes of each other (dual optic): no drop/raise anims, the change is instant.
pub fn is_ads_only_change(facts: &FactsTable, old: u32, new: u32) -> bool {
    let o = core(facts, old);
    let n = core(facts, new);
    let paired = (old != 0 && o.alt_weapon == new) || (new != 0 && n.alt_weapon == old);
    paired && o.alt_weapon_ads_only && n.alt_weapon_ads_only
}

/// Clip of the hand being run is empty (unlimited ammo is never empty).
fn clip_empty(player: &T6WeaponPs, facts: &FactsTable) -> bool {
    let c = core(facts, player.weapon);
    if c.unlimited_ammo {
        return false;
    }
    let w = if player.b_run_left_gun {
        c.dual_wield_weapon
    } else {
        player.weapon
    };
    player.ammo_in_clip(facts, w) == 0
}

// ---------------------------------------------------------------------------------------------------------------
// Timer step (SWITCH.md section 2, step 1)
// ---------------------------------------------------------------------------------------------------------------

/// How many ms come off the weapon timers this tick. Fast hands runs drops and raises at `1 / multiplier` speed
/// (0.5 → twice as fast), rounded to the nearest ms. Other states get the plain `msec` (their own perks live in
/// their own files).
pub fn timer_step(ps: &T6WeaponPs, env: &T6Env, facts: &FactsTable, msec: i32) -> i32 {
    let mult = env.dvars.perk_weap_switch_multiplier;
    if env.has_perk(perk::FASTSWITCH)
        && mult > 0.0
        && quick_swap_ok(facts, ps.weapon, ps.gun_phase)
    {
        (msec as f32 / mult).round() as i32
    } else {
        msec
    }
}

// ---------------------------------------------------------------------------------------------------------------
// Change check (SWITCH.md section 3)
// ---------------------------------------------------------------------------------------------------------------

fn switch_blocked(ps: &T6WeaponPs, env: &T6Env) -> bool {
    let s = ps.gun_phase;
    (env.waterlevel > 3 && env.dvars.player_disable_weapons_in_water)
        || (state::MELEE_INIT..=state::MELEE_END).contains(&s)
        || (state::DTP_IN..=state::DTP_OUT).contains(&s)
        || s == state::NIGHTVISION_WEAR
        || s == state::NIGHTVISION_REMOVE
        || (state::RIOTSHIELD_PREPARE..=state::RIOTSHIELD_END).contains(&s)
        || env.e_flags2 & EF2_WEAPON_LOCKED != 0
        || ps.weapon_state_bits & wf::CHANGE_LOCKED != 0
        || env.attach_bits & LF_WEAPON_LOCKED != 0
        || env.mantle_weapon_equipped
}

/// A running timer holds the switch only while firing, in the continuous-fire loop, swinging a knife, or with a
/// delay pending. Reloads (11-18) and rechambers (7-8) never hold it.
fn timer_holds_switch(ps: &T6WeaponPs) -> bool {
    let s = ps.gun_phase;
    if ps.gun_timer_ms == 0
        || state::is_reload_family(s)
        || s == state::RECHAMBER_START
        || s == state::RECHAMBERING
    {
        return false;
    }
    s == state::FIRING
        || s == state::CONT_FIRE_LOOP
        || (state::MELEE_INIT..=state::MELEE_END).contains(&s)
        || ps.weapon_ready_wait != 0
}

/// BO2's change-weapon check: compares the command's gun with the gun in hand and starts a drop when they differ.
pub fn check_for_change(
    ps: &mut T6WeaponPs,
    cmd: &T6Cmd,
    env: &T6Env,
    facts: &FactsTable,
    out: &mut SwitchOut,
) {
    if switch_blocked(ps, env) || timer_holds_switch(ps) {
        return;
    }

    // Forced drops to no weapon.
    if env.gun_stowed_while_climbing
        || env.movement_state_bits & pmf::LADDER != 0
        || ps.weapon_state_bits & wf::WEAPONS_DISABLED != 0
    {
        if 0 != ps.weapon {
            begin_change(ps, cmd, env, facts, 0, true, out);
        }
        return;
    }
    if ps.weapon_state_bits & wf::DISABLE_SWITCH_TO_NONE != 0 {
        if 0 != ps.weapon {
            begin_change(ps, cmd, env, facts, 0, false, out);
        }
        return;
    }

    let wanted = cmd.weapon;
    let frozen = env.movement_state_bits & (pmf::FROZEN | pmf::NO_FIRE) != 0 && ps.weapon != 0;
    // Asking for a gun you do not hold counts as no change (and drops the gun in hand if that one is gone too).
    let invalid = wanted != 0 && !ps.has_weapon(wanted);

    if env.using_vehicle_weapon {
        if ps.weapon == 0 && wanted == 0 {
            return;
        }
        if !core(facts, wanted).can_use_in_vehicle {
            if 0 != ps.weapon {
                begin_change(ps, cmd, env, facts, 0, false, out);
            }
            return;
        }
        if ps.weapon == wanted
            || ps.gun_phase == state::DEPLOYING
            || frozen
            || env.secondary_state_bits & OF_VEHICLE_NO_SWITCH != 0
            || invalid
        {
            return;
        }
        begin_change(
            ps,
            cmd,
            env,
            facts,
            wanted,
            env.ledge_climb_bits & MANTLE_QUICK != 0,
            out,
        );
        return;
    }

    let in_offhand = state::is_secondary_gear(ps.gun_phase);
    let offhand_hands_over = ps.off_hand_weapon != 0
        && core(facts, ps.off_hand_weapon).offhand_slot == offhand_slot::EQUIPMENT
        && cmd.held(OFFHAND_SWITCH_BUTTON);
    if ps.weapon == wanted
        || ps.gun_phase == state::DEPLOYING
        || frozen
        || invalid
        || (in_offhand && !offhand_hands_over)
    {
        if ps.weapon == wanted && state::is_dropping(ps.gun_phase) {
            // Asked for the gun already in hand while it goes down: cancel the drop on the spot.
            idle(ps, env);
            start_anim(ps, env, anim::FORCE_IDLE, false);
        } else if ps.weapon != 0 && !ps.has_weapon(ps.weapon) {
            begin_change(ps, cmd, env, facts, 0, false, out);
        }
        return;
    }

    match ps.gun_phase {
        state::DEPLOYING | state::BREAKING_DOWN => {}
        state::DEPLOYED => {
            if ps.weapon_state_bits & wf::DEPLOY_SWITCH_PENDING == 0 {
                out.clear_deployed = true;
            }
        }
        _ if env.ent_bits & EF_DEPLOYED != 0 => {
            ps.weapon_state_bits |= wf::DEPLOY_SWITCH_PENDING;
            out.push_id(EV_WEAPON_BREAKING_DOWN);
        }
        s => {
            let quick = env.ledge_climb_bits & MANTLE_QUICK != 0
                || ps.weapon_state_bits & wf::FORCE_QUICK_SWITCH != 0
                || is_quick_switch(facts, ps.weapon, wanted, s);
            begin_change(ps, cmd, env, facts, wanted, quick, out);
        }
    }
}

// ---------------------------------------------------------------------------------------------------------------
// Start of the drop (SWITCH.md section 4)
// ---------------------------------------------------------------------------------------------------------------

/// Starts putting the gun in hand away. Does nothing while already dropping: a new pick during a drop only changes
/// which gun comes up at the end (it is read from the command when the drop ends).
pub fn begin_change(
    player: &mut T6WeaponPs,
    _cmd: &T6Cmd,
    env: &T6Env,
    facts: &FactsTable,
    new: u32,
    quick: bool,
    out: &mut SwitchOut,
) {
    if (new != 0 && !player.has_weapon(new)) || state::is_dropping(player.gun_phase) {
        return;
    }
    let was = player.gun_phase;
    if state::is_reload_family(was) {
        out.add_predictable(EV_STOP_WEAPON_SOUND, was as u32);
    }
    player.weapon_state_bits &= !wf::FORCE_QUICK_SWITCH;
    player.weapon_ready_wait = 0;
    let old = if state::is_secondary_gear(was) {
        player.off_hand_weapon
    } else {
        player.weapon
    };
    let o = core(facts, old);

    if new != 0 && core(facts, new).gun_family_code == gun_family_code::RIOTSHIELD {
        player.weapon_state_bits |= wf::RIOTSHIELD_SWITCH;
    } else {
        player.weapon_state_bits &= !wf::RIOTSHIELD_SWITCH;
    }
    if o.fire_type == crate::facts::fire_type::CHARGESHOT {
        player.weapon_state_bits &= !wf::CHARGE_SHOT;
        player.charge_shot_timer = 0;
        player.charge_shot_level = 0;
    }
    if o.can_use_in_vehicle && core(facts, new).can_use_in_vehicle {
        return;
    }

    let drop_state = if quick {
        state::DROPPING_QUICK
    } else {
        state::DROPPING
    };
    if o.offhand_slot != offhand_slot::EQUIPMENT
        && (old == 0 || !player.has_weapon(old) || player.cook_remaining > 0)
    {
        // Nothing (or nothing real) in hand: no drop anim, the raise starts next tick.
        player.weapon_state_bits &= !wf::USING_OFFHAND;
        player.gun_timer_ms = Default::default();
        player.cook_remaining = 0;
        player.gun_phase = drop_state;
        out.prone_override = true;
        return;
    }
    player.weapon_state_bits &= !wf::USING_OFFHAND;

    player.ads_zoom_select = 0;
    player.ads_zoom_latch_time = 0;
    let empty = clip_empty(player, facts);
    let alt = is_alt_switch(player, facts, old, new);
    player.cook_remaining = 0;
    if player.weapon_state_bits & wf::WEAPONS_DISABLED == 0 {
        out.add_predictable(EV_WEAPON_SWITCH_STARTED, player.weapon);
    }

    let t = times(facts, old);
    let thrown_grenade = was == state::FIRING
        && o.gun_family_code == gun_family_code::GRENADE
        && o.offhand_slot == offhand_slot::SPECIFIC_USE;
    if alt {
        out.push_id(EV_WEAPON_ALT);
        start_anim(player, env, anim::ALTSWITCHFROM, false);
    } else if !thrown_grenade {
        out.push_id(EV_PUTAWAY_WEAPON);
        if !o.no_drops_or_raises {
            let a = if !empty || (quick && !t.no_quick_drop_when_empty) {
                if quick { anim::FAST_PUT_AWAY } else { anim::PUT_AWAY }
            } else {
                anim::EMPTY_PUT_AWAY
            };
            start_anim(player, env, a, o.dual_wield);
        }
    }

    if env.dvars.tu_enforce_rechamber_on_switch
        && o.manual_cycle
        && was == state::RELOAD_START_RECHAMBER
        && let Some(h) = player.held_mut(old)
    {
        h.needs_rechamber = true;
    }

    player.gun_phase = if alt {
        state::DROPPING_ALTSWITCH
    } else {
        drop_state
    };
    player.weaponstate_left = player.gun_phase;
    out.prone_override = true;
    player.gun_timer_ms = if o.no_drops_or_raises {
        0
    } else if alt {
        t.alt_drop
    } else if quick {
        t.quick_drop
    } else if empty {
        t.empty_drop
    } else {
        t.drop
    };
    if env.gun_stowed_while_climbing && env.ledge_climb_bits & MANTLE_CAP_DROP != 0 {
        player.gun_timer_ms = player.gun_timer_ms.min(env.mantle_time_left);
    }

    if !alt
        && !matches!(
            env.movement_mode,
            movement_mode::LASTSTAND | movement_mode::REVIVEE | movement_mode::LASTSTAND_TRANSITION
        )
        && env.view_height_settled
        && env.movement_state_bits & PMF_NO_DROP_SCRIPT == 0
        && core(facts, new).carry_slot_kind != inventory::OFFHAND
        && o.offhand_slot != offhand_slot::EQUIPMENT
    {
        out.scripts.push(AnimScript::DropWeapon);
    }
}

// ---------------------------------------------------------------------------------------------------------------
// End of the drop, start of the raise (SWITCH.md section 5)
// ---------------------------------------------------------------------------------------------------------------

/// Puts the raise on the clock: state RAISING (or RAISING_ALTSWITCH), the anim on both hands.
pub fn begin_raise(ps: &mut T6WeaponPs, env: &T6Env, now: i32, spread: f32, alt: bool, a: i32) {
    ps.gun_timer_ms = now;
    ps.sight_spread_factor = spread;
    ps.gun_phase = if alt {
        state::RAISING_ALTSWITCH
    } else {
        state::RAISING
    };
    start_anim(ps, env, a, true);
}

/// The drop timer ran out: the new gun goes in hand and starts coming up.
pub fn finish_change(
    ps: &mut T6WeaponPs,
    cmd: &T6Cmd,
    env: &T6Env,
    facts: &FactsTable,
    out: &mut SwitchOut,
) {
    if !state::is_dropping(ps.gun_phase) {
        return;
    }
    let drop_state = ps.gun_phase;
    let allowed = !env.gun_stowed_while_climbing
        && ps.weapon_state_bits & wf::WEAPONS_DISABLED == 0
        && env.movement_state_bits & pmf::LADDER == 0
        && ps.has_weapon(cmd.weapon)
        && ps.weapon_state_bits & wf::DISABLE_SWITCH_TO_NONE == 0
        && (!env.using_vehicle_weapon || core(facts, cmd.weapon).can_use_in_vehicle);
    let mut new = if allowed { cmd.weapon } else { 0 };
    if !ps.has_weapon(new) {
        new = 0;
    }
    let old = ps.weapon;
    ps.weapon = new;
    ps.last_weapon_alt_mode_switch = if new != 0 && ps.has_weapon(cmd.last_weapon_alt_mode_switch) {
        cmd.last_weapon_alt_mode_switch
    } else {
        0
    };
    if core(facts, ps.weapon).carry_slot_kind == inventory::ALTMODE
        && ps.last_weapon_alt_mode_switch == 0
    {
        ps.weapon = Default::default();
    }
    let new = ps.weapon;
    let n = core(facts, new);
    let t = times(facts, new);
    ps.ads_zoom_select = 0;
    ps.ads_zoom_latch_time = 0;

    if ps.weapon_state_bits & wf::WEAPONS_DISABLED != 0 {
        ps.gun_phase = state::OFFHAND_INIT;
        return;
    }
    if old == new {
        ps.gun_phase = state::READY;
        ps.weaponstate_left = state::READY;
        start_anim(ps, env, anim::REST, n.dual_wield);
        return;
    }

    let first = !ps.ever_had_weapon(new)
        && !matches!(
            env.movement_mode,
            movement_mode::LASTSTAND_TRANSITION | movement_mode::LASTSTAND
        );
    ps.set_ever_had(new, true);
    let alt = is_alt_switch(ps, facts, old, new);
    out.empty_raise_flag = Some(false);

    let (a, now, spread) = if alt {
        let spread = ps.sight_spread_factor.max(SPREAD_ALT_MIN);
        if !cmd.held(crate::cmd::button::FIRE_BIT) {
            ps.weapon_shot_count = Default::default();
        }
        (anim::ALTSWITCHTO, t.alt_raise, spread)
    } else {
        let empty = clip_empty(ps, facts);
        if empty {
            out.empty_raise_flag = Some(true);
        }
        let quick = is_quick_switch(facts, old, new, drop_state);
        if n.no_drops_or_raises {
            (anim::REST, 0, SPREAD_RAISE)
        } else if empty {
            (anim::EMPTY_TAKE_OUT, t.empty_raise, SPREAD_RAISE)
        } else if first {
            ps.mark_all_owned(true);
            (anim::FIRST_TAKE_OUT, t.first_raise, SPREAD_RAISE)
        } else if quick {
            (anim::FAST_TAKE_OUT, t.quick_raise, SPREAD_RAISE)
        } else {
            (anim::RAISE, t.raise, SPREAD_RAISE)
        }
    };

    if n.no_drops_or_raises {
        // BO2 stores the anim number as the state here: READY for a plain change (the alt case is the same quirk).
        ps.gun_phase = a;
        ps.gun_timer_ms = now;
        ps.sight_spread_factor = spread;
        out.prone_override = true;
        start_anim(ps, env, anim::REST, n.dual_wield);
    } else if !is_ads_only_change(facts, old, new) {
        begin_raise(ps, env, now, spread, alt, a);
        out.prone_override = true;
    }
    // ADS-only pairs get no raise: the state stays dropping at 0 ms and the next change check snaps it to READY.

    if n.gun_family_code == gun_family_code::RIOTSHIELD {
        out.reset_riotshield_model = true;
    }

    take_clip_only_if_empty(ps, facts, old);

    if old == 0 || alt {
        out.push_id(EV_RAISE_PICKUP_WEAPON);
        if !first {
            return;
        }
    } else {
        out.push_id(if first {
            EV_FIRST_RAISE_WEAPON
        } else {
            EV_RAISE_WEAPON
        });
        if matches!(
            env.movement_mode,
            movement_mode::LASTSTAND | movement_mode::REVIVEE | movement_mode::LASTSTAND_TRANSITION
        ) || env.movement_state_bits & PMF_NO_RAISE_SCRIPT != 0
            || !env.view_height_settled
        {
            return;
        }
        if !first {
            out.scripts.push(AnimScript::RaiseWeapon);
            return;
        }
    }
    out.scripts.push(AnimScript::FirstRaiseWeapon);
}

/// BO2 removes a clip-only gun that is fully empty when it is put away (not one with a detonator, not a plantable,
/// not the player's inventory gun).
fn take_clip_only_if_empty(ps: &mut T6WeaponPs, facts: &FactsTable, old: u32) {
    if old == 0 || !ps.has_weapon(old) || ps.inventory_weapon == old {
        return;
    }
    let c = core(facts, old);
    if c.motion_clip_exclusive
        && ps.ammo_in_clip(facts, old) == 0
        && ps.ammo_not_in_clip(facts, old) == 0
        && !c.has_detonator
        && !c.plantable
    {
        ps.take(old);
    }
}

/// The raise timer ran out: READY on both hands, anim IDLE.
pub fn finish_raise(ps: &mut T6WeaponPs, env: &T6Env) {
    if !state::is_raising(ps.gun_phase) {
        return;
    }
    ps.gun_phase = state::READY;
    ps.weaponstate_left = state::READY;
    start_anim(ps, env, anim::REST, true);
}

/// End-of-tick step: with both timers at 0, a raise ends or a drop ends (and the next raise starts) on this tick.
/// Returns true when it did either; the frame then runs nothing else this tick.
pub fn finish_if_due(
    ps: &mut T6WeaponPs,
    cmd: &T6Cmd,
    env: &T6Env,
    facts: &FactsTable,
    out: &mut SwitchOut,
) -> bool {
    let (time, delay) = ps.hand_time_delay();
    if time != 0 || delay != 0 {
        return false;
    }
    if state::is_raising(ps.gun_phase) {
        finish_raise(ps, env);
        true
    } else if state::is_dropping(ps.gun_phase) {
        finish_change(ps, cmd, env, facts, out);
        true
    } else {
        false
    }
}

// ---------------------------------------------------------------------------------------------------------------
// Out of ammo (SWITCH.md section 6, last test)
// ---------------------------------------------------------------------------------------------------------------

/// Server: the gun in hand has nothing left at all → `EV_NOAMMO`, which makes the client switch away.
/// Detonator and wire/TV-guided guns stay in hand.
pub fn switch_if_empty(player: &T6WeaponPs, facts: &FactsTable, out: &mut SwitchOut) {
    let c = core(facts, player.weapon);
    if c.unlimited_ammo
        || player.weapon_state_bits & wf::NO_AMMO_SWITCH != 0
        || c.has_detonator
        || matches!(c.guided_missile_type, GUIDED_WIRE | GUIDED_TV)
    {
        return;
    }
    if player.ammo_in_clip(facts, player.weapon) != 0 || player.ammo_not_in_clip(facts, player.weapon) != 0 {
        return;
    }
    if c.dual_wield && player.ammo_in_clip(facts, c.dual_wield_weapon) != 0 {
        return;
    }
    out.push_id(EV_NOAMMO);
}

// ---------------------------------------------------------------------------------------------------------------
// Client side (SWITCH.md section 7)
// ---------------------------------------------------------------------------------------------------------------

/// The client's switch memory: what it asks for and what it remembers for the alt toggle.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ClientSwitch {
    /// The gun put in the command's weapon field.
    pub selected: u32,
    /// The last real primary picked (what leaving an item or killstreak goes back to).
    pub previous_main_weapon: u32,
    /// For each held slot: the mode that gun was last used in (base gun or its alt), 0 = base.
    pub last_mode: [u32; MAX_HELD],
    /// Ms since the last cycle (for `cg_weaponCycleDelay`, 0 in BO2 so it never blocks).
    pub since_cycle_ms: i32,
}

fn cycle_candidate(
    ps: &T6WeaponPs,
    facts: &FactsTable,
    w: u32,
    skip: u32,
    need_ammo: bool,
) -> bool {
    if w == 0 || w == skip {
        return false;
    }
    let c = core(facts, w);
    // BO2's out-of-ammo cycle also takes a dual-wield gun whose left clip still has rounds.
    let has_ammo = !need_ammo
        || c.unlimited_ammo
        || ps.ammo_in_clip(facts, w) != 0
        || ps.ammo_not_in_clip(facts, w) != 0
        || c.dual_wield && ps.ammo_in_clip(facts, c.dual_wield_weapon) != 0;
    has_ammo
        && !matches!(
            c.carry_slot_kind,
            inventory::ALTMODE
                | inventory::DWLEFTHAND
                | inventory::ITEM
                | inventory::OFFHAND
                | inventory::CLOSE_COMBAT
        )
        && c.weap_class != gun_kind_ids::KILLSTREAK_ALT_STORED_WEAPON
        // bo2mp, his 10-20 ask: the knife the melee key swings is never walked to. BO2's held
        // Combat Knife (use-as-melee) is a real gun pick and stays.
        && !(c.gun_family_code == crate::facts::gun_family_code::CLOSE_COMBAT && !facts.get(w).melee.use_as_melee)
}

impl ClientSwitch {
    /// Pick a gun (CG_SelectWeaponIndex): it goes in the command. Returns whether ADS must drop (any pick that is
    /// not an attachment switch).
    pub fn select(&mut self, ps: &T6WeaponPs, facts: &FactsTable, w: u32) -> bool {
        let alt = is_alt_switch(ps, facts, self.selected, w);
        // A pick that is not an alt mode forgets that gun's last mode (BO2's CG_UpdateLastWeaponForAlt).
        if let Some(i) = ps.held_slot(w) {
            let base = core(facts, w);
            if base.carry_slot_kind != inventory::ALTMODE {
                self.last_mode[i] = 0;
            }
            if base.carry_slot_kind == inventory::PRIMARY {
                self.previous_main_weapon = w;
            }
        }
        // An alt-mode pick is remembered on its base gun's slot.
        if core(facts, w).carry_slot_kind == inventory::ALTMODE
            && let Some(i) = ps.held_slot(core(facts, w).alt_weapon)
        {
            self.last_mode[i] = w;
        }
        self.selected = w;
        !alt
    }

    /// Next / previous weapon (`weapnext`, `weapprev`, scroll, Y). `need_ammo` is the out-of-ammo variant.
    /// Returns the gun picked, if any.
    pub fn cycle(
        &mut self,
        ps: &T6WeaponPs,
        env: &T6Env,
        facts: &FactsTable,
        forward: bool,
        need_ammo: bool,
    ) -> Option<u32> {
        if env.movement_mode > movement_mode::LASTSTAND_TRANSITION || env.e_flags2 & EF2_WEAPON_LOCKED != 0 {
            return None;
        }
        let cur = self.selected;
        let c = core(facts, cur);
        let mut skip = 0;
        if c.carry_slot_kind == inventory::ALTMODE {
            // In the other mode: the first press goes back to the base gun, unless the mode disables switching
            // (select fire, Hybrid): then the cycle moves on and the gun keeps its mode for next time.
            let base = c.alt_weapon;
            if !c.alt_weapon_disable_switching && ps.has_weapon(base) {
                self.select(ps, facts, base);
                return Some(base);
            }
            skip = base;
        }
        if c.carry_slot_kind != inventory::PRIMARY
            && c.carry_slot_kind != inventory::ALTMODE
            && self.previous_main_weapon != 0
            && ps.has_weapon(self.previous_main_weapon)
        {
            let w = self.previous_main_weapon;
            self.select(ps, facts, w);
            return Some(w);
        }
        let start = ps.held_slot(cur).unwrap_or(0);
        let mut i = start;
        loop {
            i = if forward {
                (i + 1) % MAX_HELD
            } else {
                (i + MAX_HELD - 1) % MAX_HELD
            };
            if i == start {
                return None;
            }
            let w = ps.held[i].weapon;
            if cycle_candidate(ps, facts, w, skip, need_ammo) {
                let mode = self.last_mode[i];
                let pick = if core(facts, w).alt_weapon != 0 && mode != 0 && ps.has_weapon(mode) {
                    mode
                } else {
                    w
                };
                self.select(ps, facts, pick);
                self.since_cycle_ms = 0;
                return Some(pick);
            }
        }
    }

    /// Attachment key: go to the gun's other mode. An ADS-only alt needs `ads_full`.
    pub fn toggle_alt(
        &mut self,
        ps: &T6WeaponPs,
        facts: &FactsTable,
        ads_full: bool,
    ) -> Option<u32> {
        let c = core(facts, self.selected);
        let other = c.alt_weapon;
        if other == 0
            || !ps.has_weapon(other) && core(facts, other).carry_slot_kind != inventory::ALTMODE
        {
            return None;
        }
        if c.alt_weapon_ads_only && !ads_full {
            return None;
        }
        self.select(ps, facts, other);
        Some(other)
    }

    /// The command's alt-mode memory for `weapon` (what the server keeps so an alt-mode gun knows its base gun):
    /// a base gun sends its alt mode; an alt mode sends the held gun it belongs to, else what the server last kept.
    pub fn cmd_alt_memory(ps: &T6WeaponPs, facts: &FactsTable, gun: u32) -> u32 {
        if gun == 0 {
            return 0;
        }
        let c = core(facts, gun);
        if c.carry_slot_kind != inventory::ALTMODE {
            return c.alt_weapon;
        }
        ps.held
            .iter()
            .map(|h| h.weapon)
            .find(|&w| w != 0 && w != gun && core(facts, w).alt_weapon == gun)
            .unwrap_or(ps.last_weapon_alt_mode_switch)
    }

    /// Client reaction to the local player's `EV_NOAMMO` / `EV_NOAMMO_LEFT` (BO2's CG_EntityEvent hand-off and
    /// CG_OutOfAmmoChange). `left` = the left-gun event. Returns the gun picked; `Some(0)` = no weapon (an
    /// alt mode whose base gun is gone).
    ///
    /// - A dual-wield pair switches only when both clips are dry (the event fires per hand).
    /// - Nothing in hand: back to the last primary.
    /// - `NO_AMMO_SWITCH` or `cancelAutoHolsterWhenEmpty` (judged on the gun in hand): stay.
    /// - An alt mode: its base gun if that has any ammo (clip or stock).
    /// - Else the next primary with ammo (the out-of-ammo cycle; from an alt mode that cycle goes back to the
    ///   base gun even when it is dry, as BO2's does).
    /// - Nothing found from an alt mode that is not ADS-only: its base gun, unless the alt mode fires from
    ///   the base gun's own magazine (Select Fire), which is just as dry.
    ///
    /// A clip-only gun still switches: clip-only only mutes the dry-fire click.
    pub fn on_no_ammo(
        &mut self,
        ps: &T6WeaponPs,
        env: &T6Env,
        facts: &FactsTable,
        left: bool,
    ) -> Option<u32> {
        if env.movement_mode > movement_mode::LASTSTAND_TRANSITION {
            return None;
        }
        let held = ps.weapon;
        let c = core(facts, held);
        // The right-hand event goes this way only for a dual-wield gun; the left one always does.
        if (left || c.dual_wield)
            && (ps.ammo_in_clip(facts, held) != 0 || ps.ammo_in_clip(facts, c.dual_wield_weapon) != 0)
        {
            return None;
        }
        if held == 0 {
            let w = self.previous_main_weapon;
            if w != 0 && ps.has_weapon(w) {
                self.select(ps, facts, w);
                return Some(w);
            }
        }
        if ps.weapon_state_bits & wf::NO_AMMO_SWITCH != 0
            || times(facts, held).cancel_auto_holster_when_empty
        {
            return None;
        }
        let alt_mode = c.carry_slot_kind == inventory::ALTMODE;
        if alt_mode {
            let base = c.alt_weapon;
            if !ps.has_weapon(base) {
                // BO2 prints "holding alt-mode weapon but does not possess its original" and drops to none.
                self.select(ps, facts, 0);
                return Some(0);
            }
            if ps.ammo_in_clip(facts, base) != 0 || ps.ammo_not_in_clip(facts, base) != 0 {
                self.select(ps, facts, base);
                return Some(base);
            }
        }
        if let Some(w) = self.cycle(ps, env, facts, true, true) {
            return Some(w);
        }
        // Nothing else to cycle to: go back to the gun, unless the alt mode fires from the gun's
        // own magazine (Select Fire, the Hybrid's other optic), which is just as empty.
        if alt_mode
            && !c.alt_weapon_ads_only
            && facts.ammo_clip_weapon(c.alt_weapon) != facts.ammo_clip_weapon(held)
        {
            let base = c.alt_weapon;
            self.select(ps, facts, base);
            return Some(base);
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::facts::T6WeaponFacts;

    const AN94: u32 = 1;
    const MP7: u32 = 2;
    const FIVESEVEN: u32 = 3;
    const DSR: u32 = 4;
    const GL_AN94: u32 = 5;
    const OPTIC_A: u32 = 6;
    const OPTIC_B: u32 = 7;
    const SF_MP7: u32 = 8;

    fn gun(class: u8, sw: SwitchFacts) -> T6WeaponFacts {
        let mut f = T6WeaponFacts::default();
        f.core.weap_class = class;
        f.core.magazine_capacity = 30;
        f.switch = sw;
        f
    }

    fn sw(
        drop: i32,
        raise: i32,
        quick_drop: i32,
        quick_raise: i32,
        first: i32,
        empty_raise: i32,
        alt_raise: i32,
    ) -> SwitchFacts {
        SwitchFacts {
            drop,
            raise,
            quick_drop,
            quick_raise,
            first_raise: first,
            empty_raise,
            empty_drop: drop,
            alt_raise,
            ..SwitchFacts::default()
        }
    }

    /// Real patch-zone numbers (SWITCH.md section 8).
    fn table() -> FactsTable {
        let mut t = FactsTable::new();
        let mut an94 = gun(gun_kind_ids::RIFLE, sw(400, 600, 250, 400, 1600, 850, 700));
        an94.core.alt_weapon = GL_AN94;
        // an94_mp's reload numbers (window 4's dump), for the mid-reload trace.
        an94.reload.reload = 2475;
        an94.reload.reload_empty = 3105;
        an94.reload.reload_add = 1350;
        t.set(AN94, "an94_mp", an94);
        t.set(
            MP7,
            "mp7_mp",
            gun(gun_kind_ids::SMG, sw(350, 500, 250, 300, 1000, 800, 0)),
        );
        t.set(
            FIVESEVEN,
            "fiveseven_mp",
            gun(gun_kind_ids::PISTOL, sw(250, 250, 250, 250, 850, 250, 0)),
        );
        t.set(
            DSR,
            "dsr50_mp",
            gun(gun_kind_ids::RIFLE, sw(750, 750, 250, 500, 1750, 1000, 0)),
        );
        let mut gl = gun(gun_kind_ids::RIFLE, sw(600, 550, 250, 550, 1400, 550, 700));
        gl.core.carry_slot_kind = inventory::ALTMODE;
        gl.core.alt_weapon = AN94;
        // gl_ files ignore attachments, so the launcher keeps its own rounds.
        gl.core.ignore_attachments = true;
        t.set(GL_AN94, "gl_an94_mp", gl);
        for (w, other) in [(OPTIC_A, OPTIC_B), (OPTIC_B, OPTIC_A)] {
            let mut f = gun(gun_kind_ids::RIFLE, sw(400, 600, 250, 400, 1600, 850, 0));
            f.core.alt_weapon = other;
            f.core.alt_weapon_ads_only = true;
            if w == OPTIC_B {
                f.core.carry_slot_kind = inventory::ALTMODE;
            }
            t.set(w, if w == OPTIC_A { "optic_a" } else { "optic_b" }, f);
        }
        t
    }

    fn env() -> T6Env {
        T6Env {
            view_height_settled: true,
            ..T6Env::default()
        }
    }

    fn player(guns: &[u32], in_hand: u32) -> T6WeaponPs {
        let mut ps = T6WeaponPs::default();
        for &g in guns {
            ps.give(g);
            ps.set_clip_slot(g, 10);
            ps.set_pool_slot(g, 90);
            ps.set_ever_had(g, true);
        }
        ps.weapon = in_hand;
        ps
    }

    /// One weapon tick of the parts this file owns, in BO2's order. `other_states` lets a test stand in for a
    /// mechanic owned by another file (it just counts that state's timer down).
    fn tick(ps: &mut T6WeaponPs, cmd: &T6Cmd, env: &T6Env, t: &FactsTable, out: &mut SwitchOut) {
        let step = timer_step(ps, env, t, cmd.frame_ms);
        ps.gun_timer_ms = (ps.gun_timer_ms - step).max(0);
        ps.weapon_ready_wait = (ps.weapon_ready_wait - step).max(0);
        check_for_change(ps, cmd, env, t, out);
        finish_if_due(ps, cmd, env, t, out);
    }

    /// The press tick: the change check, then the end-of-tick step (a zero-length drop ends on this same tick).
    fn press(ps: &mut T6WeaponPs, cmd: &T6Cmd, env: &T6Env, t: &FactsTable, out: &mut SwitchOut) {
        check_for_change(ps, cmd, env, t, out);
        finish_if_due(ps, cmd, env, t, out);
    }

    /// Runs ticks until READY with `want` in hand; returns (ms when the raise started, ms when READY).
    fn run(
        ps: &mut T6WeaponPs,
        cmd: &T6Cmd,
        env: &T6Env,
        t: &FactsTable,
        out: &mut SwitchOut,
    ) -> (i32, i32) {
        let mut ms = 0;
        let mut raise_at = -1;
        for _ in 0..400 {
            let before = ps.gun_phase;
            tick(ps, cmd, env, t, out);
            ms += cmd.frame_ms;
            if state::is_dropping(before) && state::is_raising(ps.gun_phase) {
                raise_at = ms;
            }
            if ps.gun_phase == state::READY && ps.weapon == cmd.weapon {
                return (raise_at, ms);
            }
        }
        panic!("never READY: state {}", ps.gun_phase);
    }

    fn cmd(weapon: u32, msec: i32) -> T6Cmd {
        T6Cmd {
            frame_ms: msec,
            weapon,
            ..T6Cmd::default()
        }
    }

    fn ids(out: &SwitchOut) -> Vec<u16> {
        out.events.iter().map(|e| e.id).collect()
    }

    #[test]
    fn a_plain_swap_drop_then_raise() {
        let t = table();
        let mut ps = player(&[AN94, MP7], AN94);
        let mut out = SwitchOut::default();
        let c = cmd(MP7, 50);
        // Tick 0 starts the drop: 400 ms on the clock, DROP anim.
        check_for_change(&mut ps, &c, &env(), &t, &mut out);
        assert_eq!(ps.gun_phase, state::DROPPING);
        assert_eq!(ps.gun_timer_ms, 400);
        assert_eq!(anim::number(ps.gun_clip), anim::PUT_AWAY);
        let (raise_at, ready_at) = run(&mut ps, &c, &env(), &t, &mut out);
        assert_eq!((raise_at, ready_at), (400, 900));
        assert_eq!(
            ids(&out),
            vec![
                EV_WEAPON_SWITCH_STARTED,
                EV_PUTAWAY_WEAPON,
                EV_RAISE_WEAPON
            ]
        );
        assert_eq!(
            out.scripts,
            vec![AnimScript::DropWeapon, AnimScript::RaiseWeapon]
        );
    }

    #[test]
    fn a_fast_hands_halves_both_halves() {
        let t = table();
        let mut ps = player(&[AN94, MP7], AN94);
        let mut out = SwitchOut::default();
        let mut e = env();
        e.perks = 1 << perk::FASTSWITCH;
        e.dvars = crate::env::T6Dvars::default();
        let c = cmd(MP7, 50);
        check_for_change(&mut ps, &c, &e, &t, &mut out);
        assert_eq!(run(&mut ps, &c, &e, &t, &mut out), (200, 450));
    }

    #[test]
    fn a_to_pistol_is_quick_both_ways_and_back_is_normal() {
        let t = table();
        let mut ps = player(&[AN94, FIVESEVEN], AN94);
        let mut out = SwitchOut::default();
        let c = cmd(FIVESEVEN, 50);
        check_for_change(&mut ps, &c, &env(), &t, &mut out);
        assert_eq!(ps.gun_phase, state::DROPPING_QUICK);
        assert_eq!(anim::number(ps.gun_clip), anim::FAST_PUT_AWAY);
        // AN-94 quick drop 250, Five-seven quick raise 250.
        assert_eq!(run(&mut ps, &c, &env(), &t, &mut out), (250, 500));
        assert_eq!(anim::number(ps.gun_clip), anim::REST);
        let back = cmd(AN94, 50);
        check_for_change(&mut ps, &back, &env(), &t, &mut out);
        assert_eq!(ps.gun_phase, state::DROPPING);
        // Five-seven drop 250, AN-94 raise 600.
        assert_eq!(run(&mut ps, &back, &env(), &t, &mut out), (250, 850));
    }

    #[test]
    fn leftover_ms_is_lost_at_each_phase() {
        let t = table();
        let mut ps = player(&[AN94, MP7], AN94);
        let mut out = SwitchOut::default();
        // 60 ms ticks: drop 400 → 7 ticks (420), raise 500 → 9 ticks (540): READY at 960, not 900.
        let c = cmd(MP7, 60);
        check_for_change(&mut ps, &c, &env(), &t, &mut out);
        assert_eq!(run(&mut ps, &c, &env(), &t, &mut out), (420, 960));
    }

    #[test]
    fn b_mid_reload_switch_goes_at_once_and_stops_the_reload_sound() {
        let t = table();
        let mut ps = player(&[AN94, MP7], AN94);
        ps.gun_phase = state::RELOADING;
        ps.gun_timer_ms = 1500;
        let mut out = SwitchOut::default();
        let c = cmd(MP7, 50);
        check_for_change(&mut ps, &c, &env(), &t, &mut out);
        assert_eq!(ps.gun_phase, state::DROPPING);
        assert_eq!(ps.gun_timer_ms, 400);
        assert_eq!(
            out.events[0],
            Event {
                id: EV_STOP_WEAPON_SOUND,
                parm: state::RELOADING as u32,
                predictable: true
            }
        );
    }

    #[test]
    fn c_mid_fire_switch_waits_for_the_fire_time() {
        let t = table();
        let mut ps = player(&[AN94, MP7], AN94);
        ps.gun_phase = state::FIRING;
        ps.gun_timer_ms = 96;
        let mut out = SwitchOut::default();
        let c = cmd(MP7, 16);
        let mut ms = 0;
        // The fire file would drop FIRING back to READY when its time is up; stand in for it here.
        while ps.gun_phase == state::FIRING {
            ps.gun_timer_ms = (ps.gun_timer_ms - c.frame_ms).max(0);
            check_for_change(&mut ps, &c, &env(), &t, &mut out);
            if ps.gun_phase == state::FIRING && ps.gun_timer_ms == 0 {
                ps.gun_phase = state::READY;
                check_for_change(&mut ps, &c, &env(), &t, &mut out);
            }
            ms += c.frame_ms;
        }
        assert_eq!(ps.gun_phase, state::DROPPING);
        assert_eq!(ms, 96);
    }

    #[test]
    fn d_first_raise_of_a_picked_up_gun_marks_every_gun() {
        let t = table();
        let mut ps = player(&[AN94, MP7], AN94);
        ps.give(DSR);
        ps.set_clip_slot(DSR, 5);
        ps.set_ever_had(MP7, false);
        let mut out = SwitchOut::default();
        let c = cmd(DSR, 50);
        check_for_change(&mut ps, &c, &env(), &t, &mut out);
        assert_eq!(run(&mut ps, &c, &env(), &t, &mut out), (400, 2150));
        assert!(ids(&out).contains(&EV_FIRST_RAISE_WEAPON));
        assert_eq!(out.scripts.last(), Some(&AnimScript::FirstRaiseWeapon));
        assert!(
            ps.ever_had_weapon(MP7),
            "a first raise marks every held gun"
        );
    }

    #[test]
    fn empty_clip_beats_first_raise() {
        let t = table();
        let mut ps = player(&[AN94], AN94);
        ps.give(DSR);
        let mut out = SwitchOut::default();
        let c = cmd(DSR, 50);
        check_for_change(&mut ps, &c, &env(), &t, &mut out);
        run(&mut ps, &c, &env(), &t, &mut out);
        assert_eq!(out.empty_raise_flag, Some(true));
        assert!(!ps.held(AN94).unwrap().held_before || ps.ever_had_weapon(DSR));
    }

    #[test]
    fn e_alt_switch_uses_alt_anims_and_times() {
        let t = table();
        let mut ps = player(&[AN94, GL_AN94], AN94);
        ps.sight_spread_factor = 40.0;
        let mut out = SwitchOut::default();
        let mut c = cmd(GL_AN94, 50);
        c.last_weapon_alt_mode_switch = AN94;
        check_for_change(&mut ps, &c, &env(), &t, &mut out);
        assert_eq!(ps.gun_phase, state::DROPPING_ALTSWITCH);
        assert_eq!(anim::number(ps.gun_clip), anim::ALTSWITCHFROM);
        assert_eq!(ps.gun_timer_ms, 0, "base iAltDropTime is 0 on these guns");
        // The zero-length drop ends on the press tick: the launcher comes up on its own alt raise time.
        finish_if_due(&mut ps, &c, &env(), &t, &mut out);
        assert_eq!(ps.weapon, GL_AN94);
        assert_eq!(ps.gun_phase, state::RAISING_ALTSWITCH);
        assert_eq!(anim::number(ps.gun_clip), anim::ALTSWITCHTO);
        assert_eq!(ps.gun_timer_ms, 700);
        let (_, ready_at) = run(&mut ps, &c, &env(), &t, &mut out);
        assert_eq!(ready_at, 700);
        assert_eq!(ps.sight_spread_factor, 128.0);
        assert!(ids(&out).contains(&EV_WEAPON_ALT));
        assert!(ids(&out).contains(&EV_RAISE_PICKUP_WEAPON));
        assert!(!out.scripts.contains(&AnimScript::DropWeapon));
    }

    #[test]
    fn f_tap_twice_cancels_the_drop() {
        let t = table();
        let mut ps = player(&[AN94, MP7], AN94);
        let mut out = SwitchOut::default();
        check_for_change(&mut ps, &cmd(MP7, 50), &env(), &t, &mut out);
        tick(&mut ps, &cmd(MP7, 50), &env(), &t, &mut out);
        tick(&mut ps, &cmd(AN94, 50), &env(), &t, &mut out);
        assert_eq!(ps.gun_phase, state::READY);
        assert_eq!(ps.gun_timer_ms, 0);
        assert_eq!(anim::number(ps.gun_clip), anim::FORCE_IDLE);
    }

    #[test]
    fn f_third_gun_mid_drop_keeps_the_drop() {
        let t = table();
        let mut ps = player(&[AN94, MP7, FIVESEVEN], AN94);
        let mut out = SwitchOut::default();
        check_for_change(&mut ps, &cmd(MP7, 50), &env(), &t, &mut out);
        tick(&mut ps, &cmd(MP7, 50), &env(), &t, &mut out);
        assert_eq!(ps.gun_timer_ms, 350);
        tick(&mut ps, &cmd(FIVESEVEN, 50), &env(), &t, &mut out);
        assert_eq!(ps.gun_phase, state::DROPPING, "the drop is not restarted");
        assert_eq!(ps.gun_timer_ms, 300);
        let c = cmd(FIVESEVEN, 50);
        let (raise_at, ready_at) = run(&mut ps, &c, &env(), &t, &mut out);
        // The Five-seven comes up quick (pistol), from the normal drop's end at 400.
        assert_eq!((raise_at, ready_at), (300, 550));
    }

    #[test]
    fn f_switch_during_raise_starts_a_new_drop() {
        let t = table();
        let mut ps = player(&[AN94, MP7], MP7);
        ps.gun_phase = state::RAISING;
        ps.gun_timer_ms = 300;
        let mut out = SwitchOut::default();
        check_for_change(&mut ps, &cmd(AN94, 50), &env(), &t, &mut out);
        assert_eq!(ps.gun_phase, state::DROPPING);
        assert_eq!(ps.gun_timer_ms, 350);
    }

    #[test]
    fn ads_only_pair_snaps_without_a_raise() {
        let t = table();
        let mut ps = player(&[OPTIC_A, OPTIC_B], OPTIC_A);
        let mut out = SwitchOut::default();
        let mut c = cmd(OPTIC_B, 50);
        c.last_weapon_alt_mode_switch = OPTIC_A;
        press(&mut ps, &c, &env(), &t, &mut out);
        assert_eq!(ps.weapon, OPTIC_B);
        assert_eq!(
            ps.gun_phase,
            state::DROPPING_ALTSWITCH,
            "no raise for an ADS-only pair"
        );
        tick(&mut ps, &c, &env(), &t, &mut out);
        assert_eq!(ps.gun_phase, state::READY);
        assert_eq!(anim::number(ps.gun_clip), anim::FORCE_IDLE);
    }

    #[test]
    fn a_drop_and_a_raise_each_ask_for_the_prone_override() {
        let t = table();
        let e = env();
        let mut ps = player(&[AN94, MP7], AN94);
        let mut out = SwitchOut::default();
        begin_change(&mut ps, &cmd(MP7, 50), &e, &t, MP7, false, &mut out);
        assert!(out.prone_override, "the drop");
        let mut out = SwitchOut::default();
        ps.gun_timer_ms = Default::default();
        finish_change(&mut ps, &cmd(MP7, 50), &e, &t, &mut out);
        assert_eq!(ps.gun_phase, state::RAISING);
        assert!(out.prone_override, "the raise");
        // Nothing in hand: the drop is skipped but still asks.
        let mut ps = player(&[MP7], 0);
        let mut out = SwitchOut::default();
        begin_change(&mut ps, &cmd(MP7, 50), &e, &t, MP7, false, &mut out);
        assert!(out.prone_override);
    }

    #[test]
    fn knife_and_dive_block_the_switch() {
        let t = table();
        for s in [state::MELEE_INIT, 50, 31] {
            let mut ps = player(&[AN94, MP7], AN94);
            ps.gun_phase = s;
            let mut out = SwitchOut::default();
            check_for_change(&mut ps, &cmd(MP7, 50), &env(), &t, &mut out);
            assert_eq!(ps.gun_phase, s);
        }
    }

    #[test]
    fn out_of_ammo_event_and_client_cycle() {
        let t = table();
        let mut ps = player(&[AN94, MP7, FIVESEVEN], MP7);
        ps.set_clip_slot(MP7, 0);
        ps.set_pool_slot(MP7, 0);
        let mut out = SwitchOut::default();
        switch_if_empty(&ps, &t, &mut out);
        assert_eq!(ids(&out), vec![EV_NOAMMO]);
        let mut cl = ClientSwitch {
            selected: MP7,
            ..ClientSwitch::default()
        };
        assert_eq!(cl.on_no_ammo(&ps, &env(), &t, false), Some(FIVESEVEN));
    }

    fn dry(ps: &mut T6WeaponPs, w: u32) {
        ps.set_clip_slot(w, 0);
        ps.set_pool_slot(w, 0);
    }

    fn client(selected: u32) -> ClientSwitch {
        ClientSwitch {
            selected,
            ..ClientSwitch::default()
        }
    }

    #[test]
    fn f_dry_rifle_goes_to_the_next_gun_not_its_launcher() {
        let t = table();
        let mut ps = player(&[AN94, GL_AN94, MP7], AN94);
        dry(&mut ps, AN94);
        assert_eq!(client(AN94).on_no_ammo(&ps, &env(), &t, false), Some(MP7));
    }

    #[test]
    fn f_dry_launcher_goes_back_to_its_rifle() {
        let t = table();
        let mut ps = player(&[AN94, GL_AN94, MP7], GL_AN94);
        dry(&mut ps, GL_AN94);
        assert_eq!(
            client(GL_AN94).on_no_ammo(&ps, &env(), &t, false),
            Some(AN94)
        );
        // The rifle dry too: BO2's cycle from an alt mode still goes back to the rifle first.
        dry(&mut ps, AN94);
        assert_eq!(
            client(GL_AN94).on_no_ammo(&ps, &env(), &t, false),
            Some(AN94)
        );
    }

    #[test]
    fn f_dry_select_fire_stays_when_nothing_else_has_ammo() {
        let mut t = table();
        let mut mp7 = t.get(MP7);
        mp7.core.alt_weapon = SF_MP7;
        t.set(MP7, "mp7_mp", mp7);
        let mut sf = gun(gun_kind_ids::SMG, sw(350, 500, 250, 300, 1000, 800, 0));
        sf.core.carry_slot_kind = inventory::ALTMODE;
        sf.core.alt_weapon = MP7;
        sf.core.alt_weapon_disable_switching = true;
        t.set(SF_MP7, "sf_mp7_mp", sf);
        let mut ps = player(&[MP7, SF_MP7, AN94], SF_MP7);
        dry(&mut ps, MP7);
        dry(&mut ps, AN94);
        // Select Fire fires from the MP7's magazine: dry together, and BO2 does not swap back.
        assert_eq!(ps.ammo_in_clip(&t, SF_MP7), 0);
        assert_eq!(client(SF_MP7).on_no_ammo(&ps, &env(), &t, false), None);
        ps.set_ammo_not_in_clip(&t, SF_MP7, 30);
        assert_eq!(ps.ammo_not_in_clip(&t, MP7), 30);
        assert_eq!(client(SF_MP7).on_no_ammo(&ps, &env(), &t, false), Some(MP7));
    }

    #[test]
    fn f_launcher_without_its_rifle_drops_to_none() {
        let t = table();
        let mut ps = player(&[GL_AN94, MP7], GL_AN94);
        dry(&mut ps, GL_AN94);
        assert_eq!(client(GL_AN94).on_no_ammo(&ps, &env(), &t, false), Some(0));
    }

    #[test]
    fn f_stays_up_when_empty_or_flagged() {
        let mut t = table();
        let mut f = t.get(MP7);
        f.switch.cancel_auto_holster_when_empty = true;
        t.set(MP7, "mp7_mp", f);
        let mut ps = player(&[AN94, MP7], MP7);
        dry(&mut ps, MP7);
        assert_eq!(client(MP7).on_no_ammo(&ps, &env(), &t, false), None);
        let t = table();
        ps.weapon_state_bits |= wf::NO_AMMO_SWITCH;
        assert_eq!(client(MP7).on_no_ammo(&ps, &env(), &t, false), None);
    }

    #[test]
    fn f_clip_only_gun_still_switches() {
        let mut t = table();
        let mut f = t.get(MP7);
        f.core.motion_clip_exclusive = true;
        t.set(MP7, "mp7_mp", f);
        let mut ps = player(&[AN94, MP7], MP7);
        dry(&mut ps, MP7);
        assert_eq!(client(MP7).on_no_ammo(&ps, &env(), &t, false), Some(AN94));
    }

    #[test]
    fn f_dual_wield_switches_only_when_both_clips_are_dry() {
        const LEFT: u32 = 20;
        let mut t = table();
        let mut f = t.get(FIVESEVEN);
        f.core.dual_wield = true;
        f.core.dual_wield_weapon = LEFT;
        t.set(FIVESEVEN, "fiveseven_dw_mp", f);
        let mut ps = player(&[AN94, FIVESEVEN], FIVESEVEN);
        dry(&mut ps, FIVESEVEN);
        ps.set_clip_slot(LEFT, 3);
        for left in [false, true] {
            assert_eq!(client(FIVESEVEN).on_no_ammo(&ps, &env(), &t, left), None);
        }
        ps.set_clip_slot(LEFT, 0);
        assert_eq!(
            client(FIVESEVEN).on_no_ammo(&ps, &env(), &t, true),
            Some(AN94)
        );
        // A plain gun's left event (never sent in play) also checks both clips.
        let mut ps = player(&[AN94, MP7], MP7);
        dry(&mut ps, MP7);
        assert_eq!(client(MP7).on_no_ammo(&ps, &env(), &t, true), Some(AN94));
    }

    #[test]
    fn f_nothing_in_hand_goes_back_to_the_last_primary() {
        let t = table();
        let ps = player(&[AN94, MP7], 0);
        let mut cl = ClientSwitch {
            previous_main_weapon: MP7,
            ..ClientSwitch::default()
        };
        assert_eq!(cl.on_no_ammo(&ps, &env(), &t, false), Some(MP7));
    }

    #[test]
    fn f_no_gun_with_ammo_stays() {
        let t = table();
        let mut ps = player(&[AN94, MP7], MP7);
        dry(&mut ps, MP7);
        dry(&mut ps, AN94);
        assert_eq!(client(MP7).on_no_ammo(&ps, &env(), &t, false), None);
    }

    #[test]
    fn client_cycle_wraps_and_skips_the_alt_mode_copy() {
        let t = table();
        let ps = player(&[AN94, GL_AN94, MP7], AN94);
        let mut cl = ClientSwitch {
            selected: AN94,
            ..ClientSwitch::default()
        };
        assert_eq!(cl.cycle(&ps, &env(), &t, true, false), Some(MP7));
        assert_eq!(cl.cycle(&ps, &env(), &t, true, false), Some(AN94));
        // On the launcher, the first press goes back to the rifle.
        assert_eq!(cl.toggle_alt(&ps, &t, false), Some(GL_AN94));
        assert_eq!(cl.cycle(&ps, &env(), &t, true, false), Some(AN94));
        // Back on the rifle, the launcher is forgotten: the next lap comes back to the rifle.
        assert_eq!(cl.cycle(&ps, &env(), &t, true, false), Some(MP7));
        assert_eq!(cl.cycle(&ps, &env(), &t, true, false), Some(AN94));
    }

    #[test]
    fn client_cycle_remembers_a_mode_that_disables_switching() {
        // Select fire / Hybrid: the cycle leaves the mode for the next gun and the gun comes back in it.
        let mut t = table();
        let mut f = t.get(GL_AN94);
        f.core.alt_weapon_disable_switching = true;
        t.set(GL_AN94, "gl_an94_mp+gl", f);
        let ps = player(&[AN94, GL_AN94, MP7], AN94);
        let mut cl = client(AN94);
        assert_eq!(cl.toggle_alt(&ps, &t, false), Some(GL_AN94));
        assert_eq!(cl.cycle(&ps, &env(), &t, true, false), Some(MP7));
        assert_eq!(cl.cycle(&ps, &env(), &t, true, false), Some(GL_AN94));
        // The attachment key back to the base mode forgets it.
        assert_eq!(cl.toggle_alt(&ps, &t, false), Some(AN94));
        assert_eq!(cl.cycle(&ps, &env(), &t, true, false), Some(MP7));
        assert_eq!(cl.cycle(&ps, &env(), &t, true, false), Some(AN94));
    }

    #[test]
    fn mantle_caps_the_drop_at_the_time_it_has_left() {
        let t = table();
        let mut ps = player(&[AN94, MP7], AN94);
        let mut out = SwitchOut::default();
        let mut e = env();
        e.gun_stowed_while_climbing = true;
        e.ledge_climb_bits = MANTLE_CAP_DROP;
        e.mantle_time_left = 120;
        begin_change(&mut ps, &cmd(MP7, 50), &e, &t, MP7, false, &mut out);
        assert_eq!(ps.gun_phase, state::DROPPING);
        assert_eq!(ps.gun_timer_ms, 120);
        // More mantle left than the drop: the drop keeps its own 400.
        let mut ps = player(&[AN94, MP7], AN94);
        e.mantle_time_left = 900;
        begin_change(&mut ps, &cmd(MP7, 50), &e, &t, MP7, false, &mut out);
        assert_eq!(ps.gun_timer_ms, 400);
    }

    #[test]
    fn empty_clip_only_gun_is_taken_when_put_away() {
        let mut t = table();
        let mut f = t.get(DSR);
        f.core.motion_clip_exclusive = true;
        t.set(DSR, "dsr50_mp", f);
        let mut ps = player(&[AN94, DSR], DSR);
        ps.set_clip_slot(DSR, 0);
        ps.set_pool_slot(DSR, 0);
        let mut out = SwitchOut::default();
        let c = cmd(AN94, 50);
        press(&mut ps, &c, &env(), &t, &mut out);
        run(&mut ps, &c, &env(), &t, &mut out);
        assert!(!ps.has_weapon(DSR));
        // A plantable one stays.
        f.core.plantable = true;
        t.set(DSR, "dsr50_mp", f);
        let mut ps = player(&[AN94, DSR], DSR);
        ps.set_clip_slot(DSR, 0);
        ps.set_pool_slot(DSR, 0);
        press(&mut ps, &c, &env(), &t, &mut out);
        run(&mut ps, &c, &env(), &t, &mut out);
        assert!(ps.has_weapon(DSR));
        // So does the player's inventory gun.
        f.core.plantable = false;
        t.set(DSR, "dsr50_mp", f);
        let mut ps = player(&[AN94, DSR], DSR);
        ps.inventory_weapon = DSR;
        ps.set_clip_slot(DSR, 0);
        ps.set_pool_slot(DSR, 0);
        press(&mut ps, &c, &env(), &t, &mut out);
        run(&mut ps, &c, &env(), &t, &mut out);
        assert!(ps.has_weapon(DSR));
    }

    #[test]
    fn switching_to_a_riot_shield_resets_its_model() {
        let mut t = table();
        let mut f = t.get(MP7);
        f.core.gun_family_code = gun_family_code::RIOTSHIELD;
        t.set(MP7, "riotshield_mp", f);
        let mut ps = player(&[AN94, MP7], AN94);
        let mut out = SwitchOut::default();
        let c = cmd(MP7, 50);
        press(&mut ps, &c, &env(), &t, &mut out);
        assert!(!out.reset_riotshield_model, "not at the drop");
        run(&mut ps, &c, &env(), &t, &mut out);
        assert!(out.reset_riotshield_model, "at the change");
        // Putting the shield away does not.
        let mut out = SwitchOut::default();
        let c = cmd(AN94, 50);
        press(&mut ps, &c, &env(), &t, &mut out);
        run(&mut ps, &c, &env(), &t, &mut out);
        assert!(!out.reset_riotshield_model);
    }

    #[test]
    fn guided_and_detonator_guns_never_ask_to_switch_when_empty() {
        let mut t = table();
        let mut ps = player(&[AN94, MP7], AN94);
        ps.set_clip_slot(AN94, 0);
        ps.set_pool_slot(AN94, 0);
        let mut out = SwitchOut::default();
        switch_if_empty(&ps, &t, &mut out);
        assert_eq!(ids(&out), vec![EV_NOAMMO]);
        for guided in [GUIDED_WIRE, GUIDED_TV] {
            let mut f = t.get(AN94);
            f.core.guided_missile_type = guided;
            t.set(AN94, "an94_mp", f);
            let mut out = SwitchOut::default();
            switch_if_empty(&ps, &t, &mut out);
            assert!(out.events.is_empty());
        }
        let mut f = t.get(AN94);
        f.core.guided_missile_type = 0;
        f.core.has_detonator = true;
        t.set(AN94, "an94_mp", f);
        let mut out = SwitchOut::default();
        switch_if_empty(&ps, &t, &mut out);
        assert!(out.events.is_empty());
        ps.weapon_state_bits |= wf::NO_AMMO_SWITCH;
        f.core.has_detonator = false;
        t.set(AN94, "an94_mp", f);
        switch_if_empty(&ps, &t, &mut out);
        assert!(out.events.is_empty());
    }

    /// The switch trace scripts (docs/gun/switch-traces) through the whole weapon tick, on the real numbers above.
    /// SWITCH_TRACE_OUT=<dir> also writes each printed trace there.
    #[test]
    fn trace_scripts_pass_on_the_real_numbers() {
        let t = table();
        let scripts = [
            (
                "a_plain",
                include_str!("../../../docs/gun/switch-traces/a_plain.txt"),
            ),
            (
                "a_fast_hands",
                include_str!("../../../docs/gun/switch-traces/a_fast_hands.txt"),
            ),
            (
                "b_mid_reload",
                include_str!("../../../docs/gun/switch-traces/b_mid_reload.txt"),
            ),
            (
                "d_first_raise",
                include_str!("../../../docs/gun/switch-traces/d_first_raise.txt"),
            ),
            (
                "e_alt",
                include_str!("../../../docs/gun/switch-traces/e_alt.txt"),
            ),
        ];
        let out_dir = std::env::var("SWITCH_TRACE_OUT").ok();
        let mut failed = Vec::<_>::new();
        for (name, script) in scripts {
            let mut runner = crate::trace::ScenarioDriver::new(&t);
            // The runner's `select` does not fill the alt-mode memory yet: the launcher belongs to the AN-94.
            runner.cmd.last_weapon_alt_mode_switch = AN94;
            let r = runner.run_script(script).expect(name);
            let text = r.log_rows.join(
                "
",
            );
            if let Some(d) = &out_dir {
                std::fs::write(format!("{d}/{name}.trace.txt"), &text).expect("write trace");
            }
            if r.failures != 0 {
                failed.push(format!(
                    "{name}:
{text}"
                ));
            }
        }
        assert!(
            failed.is_empty(),
            "{}",
            failed.join(
                "

"
            )
        );
    }

    #[test]
    fn cmd_alt_memory_points_both_ways() {
        let t = table();
        let ps = player(&[AN94, MP7], AN94);
        assert_eq!(ClientSwitch::cmd_alt_memory(&ps, &t, AN94), GL_AN94);
        assert_eq!(ClientSwitch::cmd_alt_memory(&ps, &t, GL_AN94), AN94);
        assert_eq!(ClientSwitch::cmd_alt_memory(&ps, &t, MP7), 0);
    }
}
