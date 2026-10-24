//! Window 8: Dual wield (the left hand). docs/gun/DUAL.md has the full timing spec.
//!
//! A dual-wield gun is two weapons: the right one the player holds and a left one (its
//! `dualWieldWeaponIndex`, the `*_lh` weapon file) with its own clip, fire and reload numbers.
//! [`crate::tick::run`] runs the weapon tick twice, right hand then left; the left pass reads
//! and writes the left slots and the left weapon's facts ([`Pm::hand_weapon`]). This file owns
//! the rules only dual wield has: when the left acts on its own, which button fires which
//! hand, and the two-clip ammo helpers.

use super::Pm;
use crate::cmd::{T6Cmd, button};
use crate::facts::{FactsTable, weap_type};
use crate::player::T6WeaponPs;
use crate::state;

/// The perk that lets the left hand act in every state (Zombies only).
pub const PERK_UNIVERSAL_DW: u32 = 42;

/// Window 8's per-weapon facts. Nothing extra to read: the left hand's numbers live in the
/// left weapon's own row of the facts table (its fire, clip and reload times), and the link
/// between the two is [`crate::facts::CoreFacts::dual_wield_weapon`].
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct DualFacts {}

impl DualFacts {
    pub fn read(variant: &[u8], def: &[u8]) -> Self {
        let _ = (variant, def);
        Self {}
    }
}

/// `BG_IsDualWield`.
pub fn is_dual_wield(facts: &FactsTable, weapon: u32) -> bool {
    weapon != 0 && facts.get(weapon).core.dual_wield
}

/// `BG_GetDualWieldWeapon`: the left-hand weapon (0 = none).
pub fn dual_wield_weapon(facts: &FactsTable, weapon: u32) -> u32 {
    if weapon == 0 {
        0
    } else {
        facts.get(weapon).core.dual_wield_weapon
    }
}

/// The right-hand states the left hand only mirrors (it copies the state and shows marker 88).
/// Raise/drop, segmented reload, grenade and equipment, detonate, sprint, continuous fire,
/// dive and crawl. Every other state lets the left act on its own.
fn left_follows(right_state: i32) -> bool {
    matches!(
        right_state,
        state::RAISING..=state::DROPPING_ALTSWITCH
            | state::RELOADING_INTERUPT..=state::RELOAD_END
            | state::OFFHAND_PREPARE
            | state::OFFHAND_HOLD
            | state::OFFHAND_START
            | state::OFFHAND
            | state::DETONATING..=state::CONT_FIRE_OUT
            | state::DTP_IN..=state::DTP_OUT
            | state::CRAWL_IN..=state::CRAWL_OUT
    )
}

/// `BG_CheckForUniversalActionDW`: the left gun runs its own actions this tick. `false` makes
/// the left hand mirror the right ([`crate::tick`] copies the state and sets marker 88).
pub fn universal_action_dw(pm: &Pm) -> bool {
    // Player perk or one the weapon in the arms grants (BG_HasPerk).
    if pm.env.zombies && pm.has_perk(PERK_UNIVERSAL_DW) {
        return true;
    }
    let right = pm.ps.weaponstate;
    if (state::MELEE_INIT..=state::MELEE_END).contains(&right) {
        // Two-handed melee weapons (fists) knife with each hand; a gun's left hand follows the knife.
        let w = pm.weapon_facts().core;
        return w.weap_type == weap_type::MELEE && w.dual_wield;
    }
    !left_follows(right)
}

/// `PM_GetWeaponFireButton` for a dual-wield weapon: each trigger fires the gun on its own side.
/// Pad: right trigger (attack) = right gun, left trigger (the aim button) = left gun.
/// Mouse + keyboard: left mouse (attack) = left gun, right mouse (the aim button) = right gun.
/// Not dual wield: attack.
pub fn fire_button(dual: bool, left: bool, pad: bool) -> u32 {
    if dual && left == pad {
        button::ADS
    } else {
        button::ATTACK
    }
}

/// The last input came from a gamepad (BO2 `cmd.lastInput == LAST_INPUT_GAMEPAD`).
pub fn last_input_was_pad(cmd: &T6Cmd) -> bool {
    cmd.last_input == crate::cmd::last_input::GAMEPAD
}

/// `BG_CheckIfDualWieldEmpty`, by what it returns: `true` while either hand has a round in its
/// clip. A one-gun weapon checks its one clip.
pub fn any_clip_loaded(ps: &T6WeaponPs, facts: &FactsTable) -> bool {
    if ps.ammo_in_clip(facts, ps.weapon) != 0 {
        return true;
    }
    let left = dual_wield_weapon(facts, ps.weapon);
    left != 0 && ps.ammo_in_clip(facts, left) != 0
}

/// `SetWeaponAmmoClipDualWield`: the script call that fills the left gun's clip. Returns
/// `false` (BO2: a script error) when the player does not hold the left weapon.
pub fn fill_left_clip(ps: &mut T6WeaponPs, facts: &FactsTable) -> bool {
    let left = dual_wield_weapon(facts, ps.weapon);
    if left == 0 {
        return true;
    }
    if !ps.has_weapon(left) {
        return false;
    }
    ps.set_ammo_in_clip(facts, left, facts.get(left).core.clip_size);
    true
}

/// A left-hand gun: its weapon file is the left-hand kind. (BO2 also counts a dual-wield gun
/// whose third-person anim type is 1; no MP or Zombies gun needs that rule.)
pub fn is_left_hand_weapon(facts: &FactsTable, weapon: u32) -> bool {
    facts.get(weapon).core.inventory_type == crate::facts::inventory::DWLEFTHAND
}

/// The weapon whose reserve `weapon` draws from ([`FactsTable::ammo_pool_weapon`]): a
/// dual-wield left-hand gun uses its right-hand partner's pool, so both guns share one reserve.
pub fn ammo_pool_weapon(facts: &FactsTable, weapon: u32) -> u32 {
    facts.ammo_pool_weapon(weapon)
}

/// The left-hand viewmodel anims, by their slot in the weapon file's anim list.
pub mod left_anim {
    pub const FIRE: usize = 78;
    pub const LASTSHOT: usize = 79;
    pub const FINALSHOT: usize = 80;
    pub const IDLE: usize = 81;
    pub const EMPTY_IDLE: usize = 82;
    pub const RELOAD_EMPTY: usize = 83;
    pub const RELOAD: usize = 84;
}

/// Which left-hand viewmodel anim the left anim number asks for (the viewmodel plays it on the
/// left gun over the right hand's anim). `None` = the left has no anim of its own (the follow
/// marker 88, raises, knife ...): the right hand's anim moves both guns. Idle waits for the
/// left fire/reload anim to finish before it starts; FINALSHOT is loaded and never asked for.
pub fn left_view_anim(weap_anim_left: i32, left_clip: i32) -> Option<usize> {
    match crate::anim::number(weap_anim_left) {
        crate::anim::IDLE | crate::anim::FORCE_IDLE if left_clip == 0 => Some(left_anim::EMPTY_IDLE),
        crate::anim::IDLE | crate::anim::FORCE_IDLE => Some(left_anim::IDLE),
        crate::anim::ATTACK => Some(left_anim::FIRE),
        crate::anim::ATTACK_LASTSHOT => Some(left_anim::LASTSHOT),
        crate::anim::RELOAD => Some(left_anim::RELOAD),
        crate::anim::RELOAD_EMPTY => Some(left_anim::RELOAD_EMPTY),
        _ => None,
    }
}

/// HUD: digits of the left clip counter (one under 10 rounds, else two). The main counter
/// shows the right clip; the left one uses the same low-ammo colour and fade and hides in a
/// vehicle. Reload hints count the gun as loaded while [`any_clip_loaded`].
pub fn left_counter_digits(left_clip_size: i32) -> u8 {
    if left_clip_size < 10 { 1 } else { 2 }
}

/// A dual-pistol table for tests and the trace: `fiveseven_dw_mp` and its left gun
/// `fiveseven_lh_mp` with BO2's own clip and switch times (from the weapon files, read with
/// `t6dw`; each file names the other as its dual-wield partner), plus a one-gun SMG
/// (`demo_smg`, round test times) to swap to. Only the fields the frame reads so far are filled.
pub fn demo_facts() -> FactsTable {
    use crate::facts::{T6WeaponFacts, fire_type, weap_class};
    let mut t = FactsTable::new();
    // drop, raise, quick drop, quick raise, first raise, empty raise, empty drop.
    let gun = |class: u8, clip: i32, partner: u32, sw: [i32; 7]| {
        let mut f = T6WeaponFacts::default();
        f.core.weap_class = class;
        f.core.fire_type = fire_type::SINGLESHOT;
        f.core.clip_size = clip;
        f.core.dual_wield = partner != 0;
        f.core.dual_wield_weapon = partner;
        let s = &mut f.switch;
        [s.drop, s.raise, s.quick_drop, s.quick_raise, s.first_raise, s.empty_raise, s.empty_drop] = sw;
        s.alt_raise = s.raise;
        s.alt_drop = s.drop;
        f
    };
    t.set(1, "fiveseven_dw_mp", gun(weap_class::PISTOL, 20, 2, [300, 300, 250, 300, 900, 300, 250]));
    let mut lh = gun(weap_class::PISTOL, 20, 1, [300, 300, 300, 300, 1400, 300, 300]);
    lh.core.inventory_type = crate::facts::inventory::DWLEFTHAND;
    t.set(2, "fiveseven_lh_mp", lh);
    let mut smg = gun(weap_class::SMG, 30, 0, [400, 500, 250, 300, 1000, 500, 400]);
    smg.core.fire_type = fire_type::FULLAUTO;
    t.set(3, "demo_smg", smg);
    t
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::trace::Runner;

    #[test]
    fn follow_list_is_bo2s() {
        let own = [0, 6, 7, 8, 9, 10, 11, 17, 18, 22, 25, 28, 29, 30, 34, 42, 48, 52, 53, 54, 58];
        let follow = [
            1, 2, 3, 4, 5, 12, 13, 14, 15, 16, 23, 24, 26, 27, 35, 36, 37, 38, 39, 40, 41, 49, 50,
            51, 59, 60, 61, 62, 63, 64,
        ];
        for s in own {
            assert!(!left_follows(s), "state {s} acts on its own");
        }
        for s in follow {
            assert!(left_follows(s), "state {s} follows");
        }
    }

    #[test]
    fn each_trigger_fires_its_own_side() {
        // Pad: RT right, LT (aim) left. Mouse: right mouse (aim) right, left mouse left.
        assert_eq!(fire_button(true, false, true), button::ATTACK);
        assert_eq!(fire_button(true, true, true), button::ADS);
        assert_eq!(fire_button(true, false, false), button::ADS);
        assert_eq!(fire_button(true, true, false), button::ATTACK);
        assert_eq!(fire_button(false, false, false), button::ATTACK);
        assert_eq!(fire_button(false, false, true), button::ATTACK);
    }

    #[test]
    fn left_slot_picks_the_left_anims() {
        use crate::anim;
        assert_eq!(left_view_anim(anim::TOGGLE | anim::ATTACK, 5), Some(left_anim::FIRE));
        assert_eq!(left_view_anim(anim::ATTACK_LASTSHOT, 0), Some(left_anim::LASTSHOT));
        assert_eq!(left_view_anim(anim::RELOAD, 3), Some(left_anim::RELOAD));
        assert_eq!(left_view_anim(anim::RELOAD_EMPTY, 0), Some(left_anim::RELOAD_EMPTY));
        assert_eq!(left_view_anim(anim::IDLE, 4), Some(left_anim::IDLE));
        assert_eq!(left_view_anim(anim::TOGGLE, 0), Some(left_anim::EMPTY_IDLE));
        assert_eq!(left_view_anim(anim::LEFT_NONE, 4), None);
        assert_eq!(left_view_anim(anim::QUICK_RAISE, 4), None);
        assert_eq!(left_counter_digits(5), 1);
        assert_eq!(left_counter_digits(20), 2);
    }

    #[test]
    fn left_gun_draws_from_the_right_guns_reserve() {
        let f = demo_facts();
        assert_eq!(ammo_pool_weapon(&f, 2), 1);
        assert_eq!(ammo_pool_weapon(&f, 1), 1);
        assert_eq!(ammo_pool_weapon(&f, 3), 3);
        assert!(is_left_hand_weapon(&f, 2) && !is_left_hand_weapon(&f, 1));
    }

    #[test]
    fn either_clip_counts_as_loaded() {
        let facts = demo_facts();
        let mut ps = T6WeaponPs::default();
        ps.give(1);
        ps.give(2);
        ps.weapon = 1;
        ps.set_clip_slot(1, 0);
        ps.set_clip_slot(2, 3);
        assert!(any_clip_loaded(&ps, &facts));
        ps.set_clip_slot(2, 0);
        assert!(!any_clip_loaded(&ps, &facts));
        assert!(fill_left_clip(&mut ps, &facts));
        assert_eq!(ps.clip_slot(2), 20);
    }

    /// Swap away from the dual pistols and back: the left mirrors every drop/raise frame with
    /// marker 88, then starts fresh (READY, idle with the restart toggle) once the right is READY.
    #[test]
    fn left_follows_a_swap_then_starts_fresh() {
        let facts = demo_facts();
        let script = "
            msec 16
            spawn fiveseven_dw_mp
            give fiveseven_lh_mp
            give demo_smg
            run 1
            expect state READY
            expect left READY
            select demo_smg
            run 1               # drop 300 = 19 frames
            expect state DROPPING
            expect left DROPPING
            expect leftanim NONE
            run 18
            expect state DROPPING
            expect leftanim NONE
            run 1
            expect weapon demo_smg
            expect state RAISING
            run 63              # first raise of the SMG, 1000
            expect state READY
            select fiveseven_dw_mp
            run 1               # SMG to pistol: quick drop 250 = 16 frames
            expect state DROPPING_QUICK
            run 15
            expect weapon demo_smg
            run 1               # quick raise 300 = 19 frames, the left mirrors it
            expect weapon fiveseven_dw_mp
            expect state RAISING
            expect left RAISING
            expect leftanim NONE
            run 18
            expect left RAISING
            expect leftanim NONE
            run 1               # the right is READY: the left starts fresh the same frame
            expect state READY
            expect left READY
            expect leftanim IDLE
        ";
        let r = Runner::new(&facts).run_script(script).expect("script runs");
        let text = r.lines.join("\n");
        assert_eq!(r.failures, 0, "{text}");
    }
}
