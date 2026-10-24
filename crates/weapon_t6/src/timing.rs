//! BO2's PM_Weapon_WeaponTimeAdjust: counts the hand's weapon timers down for one tick, with
//! the perk speed-ups, and ends timed-out states. Returns 1 on the tick the delay runs out.

use crate::base;
use crate::env::perk;
use crate::facts::{fire_type, offhand_slot};
use crate::player::Hand;
use crate::pm::{Pm, fire};
use crate::{anim, state};

/// BO2 nudges the scaled time up by 2^-30 before rounding.
const ROUND_NUDGE: f64 = 9.313226e-10;

fn scaled(msec: i32, mult: f32) -> i32 {
    ((msec as f32 / mult) as f64 + ROUND_NUDGE).round() as i32
}

/// The ms this tick takes off the hand's timers (BO2's `local_8`).
fn step_ms(pm: &Pm, h: &Hand) -> i32 {
    let msec = pm.cmd.msec;
    let env = &*pm.env;
    let d = &env.dvars;
    let st = h.state;
    let hand = pm.hand_facts().core;
    let off = pm.facts.get(pm.ps.off_hand_weapon).core;
    // A zero reload/rate multiplier means "finish now": take the whole remaining time.
    let or_all = |mult: f32| {
        if mult == 0.0 {
            h.time.max(h.delay)
        } else {
            scaled(msec, mult)
        }
    };
    if (state::RELOADING..=state::RELOAD_QUICK_EMPTY).contains(&st)
        && env.has_perk(perk::FASTRELOAD)
    {
        or_all(d.perk_weap_reload_multiplier)
    } else if matches!(
        st,
        state::FIRING | state::RECHAMBER_START | state::RECHAMBERING | state::CONT_FIRE_LOOP
    ) && env.has_perk(perk::RATEOFFIRE)
    {
        or_all(d.perk_weap_rate_multiplier)
    } else if env.has_perk(perk::FASTSWITCH)
        && (state::RAISING..=state::DROPPING_ALTSWITCH).contains(&st)
        && hand.offhand_slot != offhand_slot::SPECIFIC_USE
    {
        debug_assert!(d.perk_weap_switch_multiplier > 0.0);
        scaled(msec, d.perk_weap_switch_multiplier)
    } else if st == state::MELEE_END && env.has_perk(perk::FASTMELEE) && !env.zombies {
        debug_assert!(d.perk_weap_melee_multiplier > 0.0);
        scaled(msec, d.perk_weap_melee_multiplier)
    } else if (state::OFFHAND_INIT..=state::OFFHAND_END).contains(&st)
        && env.has_perk(perk::FASTTOSS)
        && off.offhand_slot != offhand_slot::EQUIPMENT
    {
        debug_assert!(d.perk_weap_toss_multiplier > 0.0);
        scaled(msec, d.perk_weap_toss_multiplier)
    } else if (state::OFFHAND_INIT..=state::OFFHAND_END).contains(&st)
        && env.has_perk(perk::FASTEQUIPMENTUSE)
        && off.offhand_slot == offhand_slot::EQUIPMENT
    {
        debug_assert!(d.perk_weap_equipment_use_multiplier > 0.0);
        scaled(msec, d.perk_weap_equipment_use_multiplier)
    } else {
        msec
    }
}

/// PM_Weapon_WeaponTimeAdjust for the hand being run.
pub fn time_adjust(pm: &mut Pm) -> i32 {
    let r = time_adjust_inner(pm);
    pm.delay_fired = r != 0;
    r
}

fn time_adjust_inner(pm: &mut Pm) -> i32 {
    let left = pm.left();
    let msec = pm.cmd.msec;
    if pm.ps.weapon_restrict_kick_time > 0 {
        pm.ps.weapon_restrict_kick_time = (pm.ps.weapon_restrict_kick_time - msec).max(0);
    }
    let mut h = pm.hand();
    let r = step_ms(pm, &h);

    if h.time != 0 {
        h.time -= r;
        let stacked = pm.weapon_facts().core.fire_type == fire_type::STACKED;
        let ends = if stacked && fire::burst_fire_pending(pm, left) {
            h.time = 0;
            true
        } else {
            h.time <= 0
        };
        if ends {
            let hw = pm.hand_weapon();
            let hand = pm.hand_facts().core;
            if matches!(h.state, state::FIRING | state::CONT_FIRE_LOOP)
                && hw != 0
                && fire_type::uses_burst(hand.fire_type)
                && !fire::burst_fire_pending(pm, left)
            {
                // A burst finished: wait out the burst delay back in READY.
                h.time = if hand.burst_delay_time == 0 {
                    1
                } else {
                    hand.burst_delay_time
                };
                h.state = state::READY;
                pm.set_hand(h);
                base::continue_anim(pm, anim::IDLE);
                return 0;
            }
            if fire::burst_waiting_for_trigger_release(pm) {
                h.time = 1;
                match h.state {
                    s if state::is_reload_family(s) => {
                        pm.server_print("end weapon (timeout)");
                        h.time = 0;
                        h.shot_count = 0;
                    }
                    state::RECHAMBERING => {
                        pm.set_hand(h);
                        fire::finish_rechamber(pm);
                        h = pm.hand();
                    }
                    state::FIRING
                    | state::RECHAMBER_START
                    | state::CONT_FIRE_LOOP
                    | state::MELEE_INIT
                    | state::MELEE_FIRE
                    | state::MELEE_END => {
                        h.state = state::READY;
                        pm.set_hand(h);
                        base::continue_anim(pm, anim::IDLE);
                    }
                    _ => {}
                }
            } else {
                pm.server_print("end weapon (timeout)");
                let held = pm.cmd.held(fire::fire_button(pm, left));
                if (!held || fire::trigger_pull_pending(pm, left))
                    && !fire::burst_fire_pending(pm, left)
                {
                    h.shot_count = 0;
                }
                h.time = 0;
            }
        }
    }
    pm.set_hand(h);

    // Crawling: both hands' shot counts reset once their triggers are let go.
    if (state::CRAWL_FORWARD..=state::CRAWL_LEFT).contains(&pm.ps.weaponstate) {
        for hand_left in [false, true] {
            let held = pm.cmd.held(fire::fire_button(pm, hand_left));
            if (!held || fire::trigger_pull_pending(pm, hand_left))
                && !fire::burst_fire_pending(pm, hand_left)
            {
                if hand_left {
                    pm.ps.weapon_shot_count_left = 0;
                } else {
                    pm.ps.weapon_shot_count = 0;
                }
            }
        }
    }

    let mut h = pm.hand();
    if h.delay != 0 {
        h.delay -= r;
        if h.delay < 1 {
            h.delay = 0;
            pm.set_hand(h);
            return 1;
        }
        pm.set_hand(h);
    }
    0
}

#[cfg(test)]
mod tests {
    use super::scaled;

    #[test]
    fn perk_scaling_rounds_like_bo2() {
        assert_eq!(scaled(16, 0.5), 32);
        assert_eq!(scaled(16, 0.75), 21);
        assert_eq!(scaled(16, 0.6), 27);
        assert_eq!(scaled(50, 0.75), 67);
    }
}
