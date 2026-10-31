//! BO2's PM_Weapon_WeaponTimeAdjust: counts the hand's weapon timers down for one tick, with
//! the perk speed-ups, and ends timed-out states. Returns 1 on the tick the delay runs out.

use crate::base;
use crate::env::perk;
use crate::facts::{fire_type, offhand_slot};
use crate::player::Hand;
use crate::pm::{Pm, fire};
use {crate::anim, crate::state};

/// BO2 nudges the scaled time up by 2^-30 before rounding.
const ROUND_NUDGE: f64 = 9.313226e-10;

fn scaled(msec: i32, mult: f32) -> i32 {
    ((msec as f32 / mult) as f64 + ROUND_NUDGE).round() as i32
}

/// The ms this tick takes off the hand's timers (BO2's `local_8`).
fn step_ms(pm: &Pm, h: &Hand) -> i32 {
    let msec = pm.cmd.frame_ms;
    let env = &*pm.env;
    let d = &env.dvars;
    let st = h.phase;
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
    // BG_GetPerks: the player's perks plus the ones this hand's gun grants.
    let has_perk = |p| pm.hand_has_perk(p);
    if (state::RELOADING..=state::RELOAD_QUICK_EMPTY).contains(&st)
        && has_perk(perk::FASTRELOAD)
    {
        or_all(d.quick_hands_factor)
    } else if matches!(
        st,
        state::FIRING | state::RECHAMBER_START | state::RECHAMBERING | state::CONT_FIRE_LOOP
    ) && has_perk(perk::RATEOFFIRE)
    {
        or_all(d.perk_weap_rate_multiplier)
    } else if has_perk(perk::FASTSWITCH)
        && (state::RAISING..=state::DROPPING_ALTSWITCH).contains(&st)
        && hand.offhand_slot != offhand_slot::SPECIFIC_USE
    {
        debug_assert!(d.perk_weap_switch_multiplier > 0.0);
        scaled(msec, d.perk_weap_switch_multiplier)
    } else if st == state::MELEE_END && has_perk(perk::FASTMELEE) && !env.zombies {
        debug_assert!(d.perk_weap_melee_multiplier > 0.0);
        scaled(msec, d.perk_weap_melee_multiplier)
    } else if (state::OFFHAND_INIT..=state::OFFHAND_END).contains(&st)
        && has_perk(perk::FASTTOSS)
        && off.offhand_slot != offhand_slot::EQUIPMENT
    {
        debug_assert!(d.perk_weap_toss_multiplier > 0.0);
        scaled(msec, d.perk_weap_toss_multiplier)
    } else if (state::OFFHAND_INIT..=state::OFFHAND_END).contains(&st)
        && has_perk(perk::FASTEQUIPMENTUSE)
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
    let msec = pm.cmd.frame_ms;
    if pm.ps.gun_lockout_kick_stamp > 0 {
        pm.ps.gun_lockout_kick_stamp = (pm.ps.gun_lockout_kick_stamp - msec).max(0);
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
            if matches!(h.phase, state::FIRING | state::CONT_FIRE_LOOP)
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
                h.phase = state::READY;
                pm.set_hand(h);
                base::continue_anim(pm, anim::REST);
                return 0;
            }
            if fire::burst_waiting_for_trigger_release(pm) {
                h.time = 1;
                match h.phase {
                    s if state::is_reload_family(s) => {
                        pm.server_print("end weapon (timeout)");
                        h.time = 0;
                        h.rounds_fired = 0;
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
                        h.phase = state::READY;
                        pm.set_hand(h);
                        base::continue_anim(pm, anim::REST);
                    }
                    _ => {}
                }
            } else {
                pm.server_print("end weapon (timeout)");
                let held = pm.cmd.held(fire::fire_button(pm, left));
                if (!held || fire::trigger_pull_pending(pm, left))
                    && !fire::burst_fire_pending(pm, left)
                {
                    h.rounds_fired = 0;
                }
                h.time = 0;
            }
        }
    }
    pm.set_hand(h);

    // Crawling: both hands' shot counts reset once their triggers are let go.
    if (state::CRAWL_FORWARD..=state::CRAWL_LEFT).contains(&pm.ps.gun_phase) {
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
    use self::super::{scaled, step_ms};
    use crate::cmd::T6Cmd;
    use crate::env::{T6Env, perk};
    use crate::facts::{FactsTable, T6WeaponFacts};
    use crate::player::{Hand, T6WeaponPs, wf};
    use crate::pm::{FrameOut, Pm, viewmodel_weapon};
    use crate::state;

    /// Rapid Fire's perk sits on the attachment (the `gun+rf` row), not on the player.
    fn table() -> FactsTable {
        let mut rf = T6WeaponFacts::default();
        rf.spread.perks = 1 << perk::RATEOFFIRE;
        let mut dual = T6WeaponFacts::default();
        dual.core.dual_wield_weapon = 4;
        let mut t = FactsTable::new();
        t.set(1, "gun_mp", T6WeaponFacts::default());
        t.set(2, "gun_mp+rf", rf);
        t.set(3, "pistol_dw_mp", dual);
        t.set(4, "pistol_dwlh_mp", rf);
        t
    }

    fn step(ps: &mut T6WeaponPs, env: &mut T6Env, facts: &FactsTable) -> i32 {
        let cmd = T6Cmd {
            frame_ms: 50,
            ..T6Cmd::default()
        };
        let mut out = FrameOut::default();
        let pm = Pm {
            ps,
            cmd: &cmd,
            env,
            facts,
            out: &mut out,
            delay_fired: false,
        };
        let h = pm.hand();
        step_ms(&pm, &h)
    }

    #[test]
    fn the_hand_guns_own_perks_speed_its_timers() {
        // PM_Weapon_WeaponTimeAdjust: BG_GetPerks ORs the hand slot's perks into the player's.
        let facts = table();
        let mut env = T6Env::default();
        env.dvars.perk_weap_rate_multiplier = 0.75;
        for (slot, want) in [(1, 50), (2, 67)] {
            let mut ps = T6WeaponPs {
                weapon: slot,
                gun_phase: state::FIRING,
                ..T6WeaponPs::default()
            };
            assert_eq!(step(&mut ps, &mut env, &facts), want, "weapon {slot}");
        }
        // The player's own perk still counts on a plain gun.
        env.perks = 1 << perk::RATEOFFIRE;
        let mut ps = T6WeaponPs {
            weapon: 1,
            gun_phase: state::FIRING,
            ..T6WeaponPs::default()
        };
        assert_eq!(step(&mut ps, &mut env, &facts), 67);
    }

    #[test]
    fn the_left_pass_reads_the_left_guns_perks() {
        let facts = table();
        let mut env = T6Env::default();
        env.dvars.perk_weap_rate_multiplier = 0.75;
        let mut ps = T6WeaponPs {
            weapon: 3,
            ..T6WeaponPs::default()
        };
        let firing = Hand {
            phase: state::FIRING,
            ..Hand::default()
        };
        ps.set_hand(false, firing);
        ps.set_hand(true, firing);
        assert_eq!(step(&mut ps, &mut env, &facts), 50, "right hand: no perk");
        ps.b_run_left_gun = true;
        assert_eq!(step(&mut ps, &mut env, &facts), 67, "left hand: its gun grants it");
    }

    #[test]
    fn the_arms_hold_the_offhand_or_the_knife_in_their_states() {
        // BG_GetViewmodelWeaponIndex.
        let facts = table();
        let mut ps = T6WeaponPs {
            weapon: 1,
            off_hand_weapon: 7,
            melee_weapon: 8,
            ..T6WeaponPs::default()
        };
        for (st, offhand_flag, want) in [
            (state::READY, true, 1),
            (state::OFFHAND_INIT, false, 1),
            (state::OFFHAND_INIT, true, 7),
            (state::OFFHAND_END, true, 7),
            (state::MELEE_INIT, false, 8),
            (state::MELEE_FIRE, false, 8),
            (state::MELEE_END, false, 1),
        ] {
            ps.gun_phase = st;
            ps.weapon_state_bits = if offhand_flag { wf::USING_OFFHAND } else { 0 };
            assert_eq!(viewmodel_weapon(&ps, &facts), want, "state {st} flag {offhand_flag}");
        }
        // A gun that is its own melee weapon (a bayonet) keeps the gun in the swing.
        let mut facts = facts;
        let mut bayonet = T6WeaponFacts::default();
        bayonet.melee.use_as_melee = true;
        facts.set(1, "gun_mp", bayonet);
        ps.gun_phase = state::MELEE_FIRE;
        ps.weapon_state_bits = 0;
        assert_eq!(viewmodel_weapon(&ps, &facts), 1);
    }

    #[test]
    fn perk_scaling_rounds_like_bo2() {
        assert_eq!(scaled(16, 0.5), 32);
        assert_eq!(scaled(16, 0.75), 21);
        assert_eq!(scaled(16, 0.6), 27);
        assert_eq!(scaled(50, 0.75), 67);
    }
}
