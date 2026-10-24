//! BO2's PM_Weapon: one weapon tick for one hand, in BO2's order. [`run`] does what Pmove does:
//! the right hand, then the left hand (which returns at once unless the gun is dual wield).
//!
//! The order of every check and hook here is the order in BO2. Mechanic windows fill the hooks
//! in `crate::pm::*`; switching is window 2's `crate::switch`, wired in here.

use crate::cmd::T6Cmd;
use crate::env::{T6Env, pm_type, pmf};
use crate::events::{self, AnimScript, Event};
use crate::facts::FactsTable;
use crate::player::{T6WeaponPs, wf};
use crate::pm::{FrameOut, Pm, ads, dual, fire, melee, moves, offhand, reload, shield, spread};
use crate::switch::{self, SwitchOut};
use crate::{base, state, timing};

/// `eFlags` bits for a deployed (mounted) weapon.
const EF_DEPLOYED: u32 = 0x300;
/// `eFlags2`: weapon locked (vehicle / script).
const EF2_WEAPON_LOCKED: u32 = 0x1000_0000;
/// `linkFlags`: linked but still allowed a weapon.
const LF_WEAPON_ALLOWED: u32 = 0x4;
/// `otherFlags`: no firing.
const OF_NO_FIRE: u32 = 0x400;
/// The left hand's anim number while it only mirrors the right hand.
const LEFT_FOLLOWS_RIGHT: i32 = 0x58;
/// Water depth at which weapons go away (when `player_disableWeaponsInWater`).
const WATER_UNDER: i32 = 4;
/// Swim-out (and swim back in during a swim-out) time, ms.
const SWIM_TIME: i32 = 1000;
const SPREAD_MAX: f32 = 255.0;

/// What PmoveSingle runs before the sprint update and the weapon passes: the ADS flag.
pub fn pre_weapon(
    ps: &mut T6WeaponPs,
    cmd: &T6Cmd,
    env: &mut T6Env,
    facts: &FactsTable,
) -> FrameOut {
    let mut out = FrameOut::default();
    let mut pm = Pm {
        ps,
        cmd,
        env,
        facts,
        out: &mut out,
        delay_fired: false,
    };
    ads::update_ads_flag(&mut pm);
    out
}

/// One whole weapon tick: right hand, then left hand. Returns what it produced.
pub fn run(ps: &mut T6WeaponPs, cmd: &T6Cmd, env: &mut T6Env, facts: &FactsTable) -> FrameOut {
    let mut out = FrameOut::default();
    let mut pm = Pm {
        ps,
        cmd,
        env,
        facts,
        out: &mut out,
        delay_fired: false,
    };
    for left in [false, true] {
        pm.ps.b_run_left_gun = left;
        pm_weapon(&mut pm);
    }
    pm.ps.b_run_left_gun = false;
    out
}

/// Copy what a switch call produced into the frame and apply its entity-flag changes.
fn apply_switch(pm: &mut Pm, so: SwitchOut) {
    pm.out.events.extend(so.events);
    pm.out.scripts.extend(so.scripts);
    match so.empty_raise_flag {
        Some(true) => pm.env.e_flags2 |= switch::EF2_EMPTY_RAISE,
        Some(false) => pm.env.e_flags2 &= !switch::EF2_EMPTY_RAISE,
        None => {}
    }
    if so.clear_deployed {
        pm.env.e_flags &= !EF_DEPLOYED;
    }
    if so.reset_riotshield_model {
        shield::model_off(pm);
    }
}

/// BG_SwitchWeaponsIfEmpty for code that holds a [`Pm`] (FireWeapon's last call).
pub fn switch_if_empty(pm: &mut Pm) {
    let mut so = SwitchOut::default();
    switch::switch_if_empty(pm.ps, pm.facts, &mut so);
    apply_switch(pm, so);
}

fn change_weapon(pm: &mut Pm) {
    let mut so = SwitchOut::default();
    switch::check_for_change(pm.ps, pm.cmd, pm.env, pm.facts, &mut so);
    apply_switch(pm, so);
}

fn finish_change(pm: &mut Pm) {
    let mut so = SwitchOut::default();
    switch::finish_change(pm.ps, pm.cmd, pm.env, pm.facts, &mut so);
    apply_switch(pm, so);
}

/// The water part of PM_Weapon. `false` = the tick ends here.
fn water(pm: &mut Pm) -> bool {
    if pm.env.waterlevel < WATER_UNDER {
        if pm.ps.weaponstate == state::SWIM_IN {
            pm.ps.weaponstate = state::SWIM_OUT;
            pm.ps.weapon_time = SWIM_TIME;
        }
        return true;
    }
    if !pm.env.dvars.player_disable_weapons_in_water {
        return true;
    }
    if (state::RIOTSHIELD_PREPARE..=state::RIOTSHIELD_END).contains(&pm.ps.weaponstate) {
        shield::model_off(pm);
    }
    let r = timing::time_adjust(pm);
    let left = pm.left();
    if !fire::burst_fire_pending(pm, left) {
        melee::check_for_melee(pm, r);
    }
    let s = pm.ps.weaponstate;
    if s == state::SWIM_IN {
        if pm.ps.weap_flags & wf::SWIM_LOWERED == 0 {
            return false;
        }
        pm.ps.weap_flags &= !wf::SWIM_LOWERED;
    } else if !(state::OFFHAND_INIT..=state::OFFHAND_HOLDUSE).contains(&s)
        && !(state::MELEE_INIT..=state::MELEE_FIRE).contains(&s)
    {
        pm.ps.weap_flags |= wf::SWIM_LOWERED;
        pm.ps.weaponstate = state::SWIM_IN;
        pm.ps.weapon_delay = 0;
        pm.ps.weapon_time = if s == state::SWIM_OUT { SWIM_TIME } else { 0 };
        ads::exit_ads(pm);
        pm.ps.f_weapon_pos_frac = 0.0;
        pm.ps.ads_delay_time = 0;
    }
    true
}

/// BO2's PM_Weapon for the hand `pm.ps.b_run_left_gun` names.
pub fn pm_weapon(pm: &mut Pm) {
    let left = pm.left();
    if left && !pm.weapon_facts().core.dual_wield {
        return;
    }
    debug_assert!(
        pm.ps.weap_flags & wf::USING_OFFHAND == 0
            || (state::RAISING..=state::DROPPING_ALTSWITCH).contains(&pm.ps.weaponstate)
            || (state::OFFHAND_PREPARE..=state::OFFHAND_HOLDUSE).contains(&pm.ps.weaponstate),
        "offhand flag outside an offhand/raise/drop state"
    );
    let env = *pm.env;
    if env.pm_type > pm_type::LASTSTAND_TRANSITION
        && env.link_flags & LF_WEAPON_ALLOWED == 0
        && env.e_flags2 & EF2_WEAPON_LOCKED == 0
    {
        pm.ps.weapon = 0;
        return;
    }
    if env.pm_flags & pmf::FROZEN != 0 {
        return;
    }
    if env.e_flags & EF_DEPLOYED != 0
        && (env.e_flags2 & EF2_WEAPON_LOCKED == 0 || pm.ps.weaponstate == state::READY)
    {
        return;
    }
    if !water(pm) {
        return;
    }

    // Every-tick updates.
    shield::update_switching_flag(pm);
    let prev_ads_frac = pm.ps.f_weapon_pos_frac;
    ads::update_ads_lerp(pm);
    fire::update_spin(pm);
    ads::update_scope_zoom(pm);
    ads::update_hold_breath(pm);
    spread::update_sway_prediction(pm);
    ads::update_quickscope(pm, prev_ads_frac);
    fire::update_charge_shot(pm);
    ads::update_airburst_lase(pm);

    // The left hand only mirrors the right unless the gun runs its own left-hand actions.
    if left {
        if !dual::universal_action_dw(pm) {
            pm.ps.weaponstate_left = pm.ps.weaponstate;
            pm.ps.weapon_delay_left = 0;
            pm.ps.weapon_time_left = 0;
            pm.ps.weap_anim_left = LEFT_FOLLOWS_RIGHT;
            return;
        }
        if pm.ps.weap_anim_left == LEFT_FOLLOWS_RIGHT {
            pm.ps.weaponstate_left = state::READY;
            pm.ps.weapon_delay_left = 0;
            pm.ps.weapon_time_left = 0;
            if pm.env.pm_type > pm_type::LASTSTAND_TRANSITION {
                return;
            }
            pm.ps.weap_anim_left = crate::anim::TOGGLE;
            return;
        }
    }

    if offhand::update_grenade_throw(pm) {
        return;
    }
    fire::update_pending_trigger_pull(pm);
    let r = timing::time_adjust(pm);

    if !fire::burst_fire_pending(pm, left) {
        moves::check_for_night_vision(pm);
        moves::check_for_sprint(pm);
        moves::check_for_dtp(pm);
        moves::check_for_slide(pm);
        shield::check_for_deploy(pm);
        offhand::check_for_offhand(pm);
        shield::check_for_deploy_riotshield(pm);
        change_weapon(pm);
        reload::check_for_reload(pm);
        fire::check_for_fuel_time(pm);
        melee::check_for_melee(pm, r);
        offhand::check_for_detonation(pm);
        offhand::check_for_grenade_throw_cancel(pm);
        moves::check_for_mantle(pm);
        fire::check_for_continuous_fire(pm, r);
        moves::check_for_prone(pm);
        ads::check_for_airburst_mark(pm);
    }
    if fire::check_for_rechamber(pm, r) {
        return;
    }

    let moving = pm.cmd.forwardmove != 0 || pm.cmd.rightmove != 0;
    if (pm.env.pm_flags & pmf::PRONE != 0 && moving && pm.ps.f_weapon_pos_frac != 1.0)
        || (state::MELEE_INIT..=state::MELEE_END).contains(&pm.ps.weaponstate)
    {
        pm.ps.aim_spread_scale = SPREAD_MAX;
    }
    debug_assert!(pm.ps.weapon_time >= 0 && pm.ps.weapon_delay >= 0);

    let h = pm.hand();
    if r == 0 && (h.time != 0 || h.delay != 0) {
        return;
    }

    if !left {
        match pm.ps.weaponstate {
            state::RAISING | state::RAISING_ALTSWITCH => {
                return switch::finish_raise(pm.ps, pm.env);
            }
            state::DROPPING | state::DROPPING_QUICK | state::DROPPING_ALTSWITCH => {
                return finish_change(pm);
            }
            state::RELOADING | state::RELOADING_INTERUPT => return reload::finish_reload(pm),
            state::RELOAD_START | state::RELOAD_START_RECHAMBER | state::RELOAD_START_INTERUPT => {
                return reload::finish_reload_start(pm);
            }
            state::RELOAD_END => return reload::finish_reload_end(pm),
            state::MELEE_INIT => return melee::melee_fire(pm, false),
            state::MELEE_FIRE => return melee::melee_end(pm, false),
            state::MELEE_END
            | state::OFFHAND_END
            | state::RIOTSHIELD_END
            | state::SPRINT_DROP
            | state::CONT_FIRE_OUT
            | state::DTP_OUT
            | state::SLIDE_IN
            | state::JAMMED
            | state::CRAWL_OUT => return base::idle(pm),
            state::OFFHAND_INIT => return offhand::offhand_prepare(pm),
            state::OFFHAND_PREPARE => return offhand::offhand_hold(pm),
            state::OFFHAND_HOLD => return offhand::offhand(pm),
            state::OFFHAND_DETONATE | state::DETONATING => return offhand::detonate(pm),
            state::OFFHAND_START => return offhand::offhand_start(pm),
            state::OFFHAND => return offhand::offhand_end(pm),
            state::OFFHAND_HOLDUSE => return offhand::offhand_hold_use(pm),
            state::RIOTSHIELD_PREPARE => return shield::riotshield_hold(pm),
            state::RIOTSHIELD_START => return shield::riotshield(pm),
            state::RIOTSHIELD_HOLD => return shield::riotshield_start(pm),
            state::SPRINT_RAISE => return moves::sprint_loop(pm),
            state::SPRINT_LOOP | state::DEPLOYED => return,
            state::NIGHTVISION_WEAR => return moves::finish_night_vision_wear(pm),
            state::NIGHTVISION_REMOVE => return moves::finish_night_vision_remove(pm),
            state::DEPLOYING => return pm.add_event(events::EV_WEAPON_FINISH_DEPLOYING),
            state::BREAKING_DOWN => {
                base::idle(pm);
                return pm.add_event(events::EV_WEAPON_FINISH_BREAKING_DOWN);
            }
            state::SWIM_IN => return moves::swim_in(pm),
            state::SWIM_OUT => return moves::swim_out(pm),
            state::DTP_IN => return moves::dtp_loop(pm),
            state::DTP_LOOP => {
                if moves::dtp_is_sliding(pm) || pm.env.pm_flags & pmf::DTP_AIR == 0 {
                    moves::dtp_out(pm);
                }
                return;
            }
            state::CRAWL_IN => return moves::crawl_set_direction(pm),
            state::CRAWL_FORWARD..=state::CRAWL_LEFT => return moves::crawl_continue_loop(pm),
            // CONT_FIRE_IN runs its loop, then carries on like the firing states.
            state::CONT_FIRE_IN => fire::continuous_fire_loop(pm),
            _ => {}
        }
    }

    // Firing states, READY, and every left-hand pass.
    if left {
        match pm.ps.weaponstate_left {
            state::RELOADING | state::RELOADING_INTERUPT => return reload::finish_reload(pm),
            state::MELEE_INIT => return melee::melee_fire(pm, true),
            state::MELEE_FIRE => return melee::melee_end(pm, true),
            _ => {}
        }
    }
    if pm.ps.weapon != 0
        && fire::should_be_firing(pm, false)
        && !fire::check_friendly_fire(pm)
        && !offhand::check_grenade_hold(pm, r)
        && pm.env.pm_flags & pmf::NO_FIRE == 0
        && pm.ps.weaponstate != state::DEPLOYING
        && pm.env.other_flags & OF_NO_FIRE == 0
    {
        fire::fire_weapon(pm);
        debug_assert!(pm.ps.weapon_time >= 0 && pm.ps.weapon_delay >= 0);
    }
}

/// Events and anim scripts in one list, for traces.
pub fn describe(out: &FrameOut) -> Vec<String> {
    let mut v: Vec<String> = out
        .events
        .iter()
        .map(|e: &Event| format!("ev {}({})", events::name(e.id), e.parm))
        .collect();
    v.extend(
        out.scripts
            .iter()
            .map(|s: &AnimScript| format!("script {s:?}")),
    );
    v.extend(out.prints.iter().map(|p| format!("print {p}")));
    v
}
