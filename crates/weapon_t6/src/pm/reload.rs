//! Window 4: reloading, quick reload (Fast Mag), one-by-one reloads and every reload cancel.
//!
//! The timeline itself is [`crate::reload`] (one hand, BO2's rules; docs/gun/RELOAD.md). These
//! hooks copy the hand being run out of the player state, run that code, and write the result
//! back: state, timers, anim, ammo, the rechamber / Fast Mag flags, charge shot, events.
//!
//! Known gaps: the vehicle-seat reload path. Fuel-tank auto reload belongs to window 3.

use super::Pm;
use crate::cmd::button;
use crate::env::movement_mode;
use crate::events::AnimScript;
use crate::facts::{fire_type, gun_family_code};
use crate::player::wf;
use crate::reload::{self, Event, Hand, Input, ReloadDef};
use fastfile_t6::layout::{WeaponDef, WeaponVariantDef};

/// movement_state_bits 0x4: no reload check at all.
const PMF_NO_RELOAD_CHECK: u32 = 0x4;
/// movement_state_bits 0x800 / otherFlags 0x400: reload requests are ignored.
const PMF_REQUEST_LOCKED: u32 = 0x800;
const OF_REQUEST_LOCKED: u32 = 0x400;
/// eFlags 0x300: on a turret or using an object.
const EF_USING_OBJECT: u32 = 0x300;
/// eFlags2 0x10000000: no reload may start.
const EF2_NO_RELOAD: u32 = 0x1000_0000;
/// Deep enough that `player_disableWeaponsInWater` stops the check.
const WATER_DEEP: i32 = 4;

/// Window 4's per-weapon facts: every reload time and rule from the weapon file. `read` gives the
/// base values; a `<gun>+<attachment>` row has them scaled by its attachments
/// ([`ReloadFacts::scale_by_attachments`]) and Fast Mag's `dual_mag` set.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ReloadFacts {
    pub reload: i32,
    pub reload_empty: i32,
    pub reload_quick: i32,
    pub reload_quick_empty: i32,
    pub reload_add: i32,
    pub reload_empty_add: i32,
    pub reload_quick_add: i32,
    pub reload_quick_empty_add: i32,
    pub reload_start: i32,
    pub reload_begin_bonus: i32,
    pub reload_end: i32,
    pub rechamber_bolt_time: i32,
    pub forbid_topup_reload: bool,
    pub segmented: bool,
    pub no_ads_auto_reload: bool,
    /// Rounds per step of a one-by-one reload (0 = fill).
    pub ammo_add: i32,
    /// Rounds the start anim loads.
    pub start_add: i32,
    pub shots_before_rechamber: i32,
    pub use_as_melee: bool,
    /// Fast Mag: quick and normal reloads alternate.
    pub dual_mag: bool,
}

fn rd_i32(b: &[u8], at: usize) -> i32 {
    b.get(at..at + 4)
        .map_or(0, |x| i32::from_le_bytes([x[0], x[1], x[2], x[3]]))
}

fn rd_bool(b: &[u8], off: usize) -> bool {
    b.get(off).is_some_and(|&x| x != 0)
}

impl ReloadFacts {
    /// BG_GetReloadTime, BG_GetReloadEmptyTime, BG_GetReloadAddTime and the three quick ones: the
    /// file's time times the attachments' scale, cut to whole ms. The other times are the file's.
    pub fn scale_by_attachments(&mut self, s: &crate::facts::AttachmentScales) {
        let scaled = |ms: i32, k: f32| (ms as f32 * k) as i32;
        self.reload = scaled(self.reload, s.reload);
        self.reload_empty = scaled(self.reload_empty, s.reload_empty);
        self.reload_add = scaled(self.reload_add, s.reload_add);
        self.reload_quick = scaled(self.reload_quick, s.reload_quick);
        self.reload_quick_empty = scaled(self.reload_quick_empty, s.reload_quick_empty);
        self.reload_quick_add = scaled(self.reload_quick_add, s.reload_quick_add);
    }

    pub fn read(variant: &[u8], def: &[u8]) -> Self {
        Self {
            reload: rd_i32(variant, WeaponVariantDef::iReloadTime),
            reload_empty: rd_i32(variant, WeaponVariantDef::iReloadEmptyTime),
            reload_quick: rd_i32(variant, WeaponVariantDef::iReloadQuickTime),
            reload_quick_empty: rd_i32(variant, WeaponVariantDef::iReloadQuickEmptyTime),
            reload_add: rd_i32(def, WeaponDef::iReloadAddTime),
            reload_empty_add: rd_i32(def, WeaponDef::iReloadEmptyAddTime),
            reload_quick_add: rd_i32(def, WeaponDef::iReloadQuickAddTime),
            reload_quick_empty_add: rd_i32(def, WeaponDef::iReloadQuickEmptyAddTime),
            reload_start: rd_i32(def, WeaponDef::iReloadStartTime),
            reload_begin_bonus: rd_i32(def, WeaponDef::iReloadStartAddTime),
            reload_end: rd_i32(def, WeaponDef::iReloadEndTime),
            rechamber_bolt_time: rd_i32(def, WeaponDef::iRechamberBoltTime),
            forbid_topup_reload: rd_bool(def, WeaponDef::bNoPartialReload),
            segmented: rd_bool(def, WeaponDef::bSegmentedReload),
            no_ads_auto_reload: rd_bool(def, WeaponDef::bNoADSAutoReload),
            ammo_add: rd_i32(def, WeaponDef::iReloadAmmoAdd),
            start_add: rd_i32(def, WeaponDef::iReloadStartAdd),
            shots_before_rechamber: rd_i32(def, WeaponDef::iShotsBeforeRechamber),
            use_as_melee: rd_bool(def, WeaponDef::bUseAsMelee),
            dual_mag: rd_bool(variant, WeaponVariantDef::bDualMag),
        }
    }
}

/// The reload numbers of the weapon in the hand being run.
fn def(pm: &Pm) -> ReloadDef {
    let f = pm.hand_facts();
    let (c, rf) = (f.core, f.reload);
    ReloadDef {
        bullet: c.gun_family_code == gun_family_code::BULLET,
        melee_type: c.gun_family_code == gun_family_code::CLOSE_COMBAT,
        dual_wield: c.dual_wield,
        use_as_melee: rf.use_as_melee,
        magazine_capacity: c.magazine_capacity,
        reload: rf.reload,
        reload_empty: rf.reload_empty,
        reload_quick: rf.reload_quick,
        reload_quick_empty: rf.reload_quick_empty,
        reload_add: rf.reload_add,
        reload_empty_add: rf.reload_empty_add,
        reload_quick_add: rf.reload_quick_add,
        reload_quick_empty_add: rf.reload_quick_empty_add,
        reload_start: rf.reload_start,
        reload_begin_bonus: rf.reload_begin_bonus,
        reload_end: rf.reload_end,
        rechamber_bolt_time: rf.rechamber_bolt_time,
        empty_raise_time: f.switch.empty_raise,
        forbid_topup_reload: rf.forbid_topup_reload,
        segmented: rf.segmented,
        no_ads_auto_reload: rf.no_ads_auto_reload,
        ammo_add: rf.ammo_add,
        start_add: rf.start_add,
        manual_cycle: c.manual_cycle,
        shots_before_rechamber: rf.shots_before_rechamber,
        unlimited_ammo: c.unlimited_ammo,
        dual_mag: rf.dual_mag,
        charge_shot: c.fire_type == fire_type::CHARGESHOT,
        motion_clip_exclusive: c.motion_clip_exclusive,
    }
}

/// Charge value handed to the timeline; it comes back 0 when a reload reset the charge shot.
const CHARGE_SET: i32 = 1;

/// The hand being run, in the timeline's terms.
fn hand(pm: &Pm) -> Hand {
    let left = pm.left();
    let hw = pm.hand_weapon();
    let ph = pm.hand();
    let held = pm.ps.held(hw);
    Hand {
        phase: ph.phase,
        timer_ms: ph.time,
        weapon_ready_wait: ph.delay,
        anim: if left {
            pm.ps.weap_anim_left
        } else {
            pm.ps.gun_clip
        },
        mag_rounds: pm.ps.ammo_in_clip(pm.facts, hw),
        spare_rounds: pm.ps.ammo_not_in_clip(pm.facts, hw),
        rounds_fired: ph.rounds_fired as i32,
        shots_from_chamber: pm.ps.shots_fired_from_chamber,
        needs_rechamber: held.is_some_and(|h| h.needs_rechamber),
        quick_reload: held.is_some_and(|h| h.quick_reload),
        charge: CHARGE_SET,
    }
}

/// This tick's input. `script_reload` is the script flag for this hand (0x1, plus 0x20000 on the
/// dual-wield left pass).
fn input(pm: &Pm) -> Input {
    let env = &pm.env;
    let wf_bits = pm.ps.weapon_state_bits;
    Input {
        frame_ms: pm.cmd.frame_ms,
        reload: pm.cmd.held(button::REFILL_BIT),
        script_reload: wf_bits & wf::SCRIPT_RELOAD_REQUEST != 0
            || (pm.left() && wf_bits & wf::LEFT_RELOAD_REQUEST != 0),
        fire: pm.cmd.held(button::FIRE_BIT),
        final_round_fired: pm.cmd.was_held(button::FIRE_BIT),
        fast_reload: pm.hand_has_perk(crate::env::perk::FASTRELOAD),
        reload_mult: env.dvars.quick_hands_factor,
        fully_ads: pm.ps.weapon_raise_fraction == 1.0,
        request_locked: env.movement_state_bits & PMF_REQUEST_LOCKED != 0
            || env.secondary_state_bits & OF_REQUEST_LOCKED != 0,
        check_blocked: env.movement_state_bits & PMF_NO_RELOAD_CHECK != 0
            || (env.waterlevel >= WATER_DEEP && env.dvars.player_disable_weapons_in_water),
        using_object: env.ent_bits & EF_USING_OBJECT != 0,
        no_reload: env.e_flags2 & EF2_NO_RELOAD != 0,
        mantling: env.mantle_weapon_equipped,
        prone_change: env.prone_change,
        enforce_rechamber: env.dvars.tu_enforce_rechamber_on_switch,
    }
}

/// Write the timeline's hand and events back into the player state.
fn write_back(pm: &mut Pm, before: &Hand, h: &Hand, ev: &[Event]) {
    let left = pm.left();
    let hw = pm.hand_weapon();
    pm.set_hand(crate::player::Hand {
        phase: h.phase,
        time: h.timer_ms,
        delay: h.weapon_ready_wait,
        rounds_fired: h.rounds_fired.max(0) as u32,
    });
    // BO2 only writes the weapon anim while the player is alive.
    if h.anim != before.anim && pm.env.movement_mode < movement_mode::DEAD {
        if left {
            pm.ps.weap_anim_left = h.anim;
        } else {
            pm.ps.gun_clip = h.anim;
        }
    }
    if h.mag_rounds != before.mag_rounds {
        pm.ps.set_ammo_in_clip(pm.facts, hw, h.mag_rounds);
    }
    if h.spare_rounds != before.spare_rounds {
        pm.ps.set_ammo_not_in_clip(pm.facts, hw, h.spare_rounds);
    }
    pm.ps.shots_fired_from_chamber = h.shots_from_chamber;
    if let Some(held) = pm.ps.held_mut(hw) {
        held.needs_rechamber = h.needs_rechamber;
    }
    // Fast Mag: BO2 tests and toggles the selected weapon (ps->weapon), whichever hand reloaded.
    if h.quick_reload != before.quick_reload && pm.facts.get(pm.ps.weapon).reload.dual_mag {
        let w = pm.ps.weapon;
        if let Some(held) = pm.ps.held_mut(w) {
            held.quick_reload = !held.quick_reload;
        }
    }
    if h.charge != CHARGE_SET {
        pm.ps.weapon_state_bits &= !wf::CHARGE_SHOT;
        pm.ps.charge_shot_timer = 0;
        pm.ps.charge_shot_level = 0;
    }
    for &e in ev {
        if e == Event::ThirdPersonReload {
            // Dual wield: only when both hands are reloading with over half to go (the
            // hand state is written above, so this reads this tick's start).
            if !pm.hand_facts().core.dual_wield
                || crate::third::dual_reload_shows_ps(&*pm.ps, pm.facts)
            {
                pm.out.scripts.push(AnimScript::Reload);
            }
        } else {
            if e == Event::ReloadAddAmmo {
                // PM_ReloadClip: ammo went in, so the clip is no longer used up (the launcher's tube).
                pm.env.e_flags2 &= !super::fire::EF2_CLIP_USED_UP;
            }
            pm.add_event(e as i32 as u16);
        }
    }
}

/// PM_Weapon_CheckForReload: the reload button / script request, the one-by-one fire
/// interrupt, and the empty-clip auto reload.
pub fn check_for_reload(pm: &mut Pm) {
    if !pm.facts.get(pm.ps.weapon).core.dual_wield {
        pm.ps.weapon_state_bits &= !wf::LEFT_RELOAD_REQUEST;
    }
    let d = def(pm);
    let inp = input(pm);
    let before = hand(pm);
    let mut h = before;
    let mut ev = Vec::new();
    if reload::check(&d, &mut h, &inp, &mut ev) {
        pm.ps.weapon_state_bits &= !wf::SCRIPT_RELOAD_REQUEST;
        if pm.left() {
            pm.ps.weapon_state_bits &= !wf::LEFT_RELOAD_REQUEST;
        }
    }
    write_back(pm, &before, &h, &ev);
}

/// PM_BeginWeaponReload: start a reload now (CheckFiringAmmo calls it when the clip runs short).
pub fn begin_weapon_reload(pm: &mut Pm) {
    let d = def(pm);
    let before = hand(pm);
    let inp = input(pm);
    let mut h = before;
    let mut ev = Vec::new();
    reload::begin(&d, &mut h, &inp, &mut ev);
    write_back(pm, &before, &h, &ev);
}

/// PM_Weapon_FinishReload (states 11/12).
pub fn finish_reload(pm: &mut Pm) {
    let delayed = pm.delay_fired;
    let d = def(pm);
    let inp = input(pm);
    let before = hand(pm);
    let mut h = before;
    let mut ev = Vec::new();
    reload::finish_reload(&d, &mut h, &inp, delayed, &mut ev);
    write_back(pm, &before, &h, &ev);
}

/// PM_Weapon_FinishReloadStart (states 13/14/15).
pub fn finish_reload_start(pm: &mut Pm) {
    let delayed = pm.delay_fired;
    let d = def(pm);
    let inp = input(pm);
    let before = hand(pm);
    let mut h = before;
    let mut ev = Vec::new();
    reload::finish_start(&d, &mut h, &inp, delayed, &mut ev);
    write_back(pm, &before, &h, &ev);
}

/// PM_Weapon_FinishReloadEnd (state 16).
pub fn finish_reload_end(pm: &mut Pm) {
    let before = hand(pm);
    let mut h = before;
    reload::finish_end(&mut h);
    write_back(pm, &before, &h, &[]);
}

#[cfg(test)]
mod tests {
    //! Whole-tick tests on BO2's own MP numbers (mp7_mp, an94_mp, 870mcs_mp weapon files):
    //! when the ammo goes in, how much, and what a cancel keeps.
    use crate::cmd::button;
    use crate::facts::{inventory, FactsTable, T6WeaponFacts};
    use crate::state;
    use crate::trace::ScenarioDriver;

    const MP7: u32 = 1;
    const AN94: u32 = 2;
    const MCS: u32 = 3;
    const FS_DW: u32 = 4;
    const FS_LH: u32 = 5;

    fn table() -> FactsTable {
        let mut t = FactsTable::new();
        let base = |clip: i32| {
            let mut f = T6WeaponFacts::default();
            f.core.magazine_capacity = clip;
            f.switch.drop = 500;
            f.switch.raise = 600;
            f
        };
        let mut mp7 = base(40);
        mp7.reload.reload = 1800;
        mp7.reload.reload_empty = 2400;
        mp7.reload.reload_add = 1300;
        t.set(MP7, "mp7_mp", mp7);
        let mut an94 = base(30);
        an94.reload.reload = 2475;
        an94.reload.reload_empty = 3105;
        an94.reload.reload_add = 1350;
        t.set(AN94, "an94_mp", an94);
        let mut mcs = base(8);
        mcs.core.manual_cycle = true;
        mcs.reload.reload = 550;
        mcs.reload.reload_empty = 550;
        mcs.reload.reload_add = 200;
        mcs.reload.reload_empty_add = 400;
        mcs.reload.reload_start = 1000;
        mcs.reload.reload_begin_bonus = 700;
        mcs.reload.reload_end = 850;
        mcs.reload.segmented = true;
        mcs.reload.ammo_add = 1;
        mcs.reload.start_add = 1;
        t.set(MCS, "870mcs_mp", mcs);
        // fiveseven_dw_mp + fiveseven_lh_mp: each names the other as its dual-wield partner.
        let fs = |partner: u32, carry_slot_kind: u8| {
            let mut f = base(20);
            f.core.dual_wield = true;
            f.core.dual_wield_weapon = partner;
            f.core.carry_slot_kind = carry_slot_kind;
            f.reload.reload = 1900;
            f.reload.reload_empty = 2400;
            f.reload.reload_add = 1300;
            f.reload.reload_empty_add = 1300;
            f
        };
        t.set(FS_DW, "fiveseven_dw_mp", fs(FS_LH, inventory::PRIMARY));
        t.set(FS_LH, "fiveseven_lh_mp", fs(FS_DW, inventory::DWLEFTHAND));
        t
    }

    fn spawn(r: &mut ScenarioDriver, w: u32, clip: i32, spare: i32) {
        r.ps.give(w);
        r.ps.weapon = w;
        r.cmd.weapon = w;
        r.ps.gun_phase = state::READY;
        r.ps.set_clip_slot(w, clip);
        r.ps.set_pool_slot(w, spare);
        r.step();
    }

    /// Tap reload; returns the time of the first reload tick.
    fn tap_reload(r: &mut ScenarioDriver) -> i32 {
        r.cmd.buttons |= button::REFILL_BIT;
        r.step();
        r.cmd.buttons &= !button::REFILL_BIT;
        r.now
    }

    /// Step until the gun is READY again (or `limit` ms); every clip change as (ms after
    /// `t0`, clip).
    fn clip_changes(r: &mut ScenarioDriver, w: u32, t0: i32, limit: i32) -> (Vec<(i32, i32)>, i32) {
        let mut log = Vec::new();
        let mut last = r.ps.clip_slot(w);
        while r.now - t0 < limit {
            r.step();
            let c = r.ps.clip_slot(w);
            if c != last {
                log.push((r.now - t0, c));
                last = c;
            }
            if r.ps.gun_phase == state::READY {
                return (log, r.now - t0);
            }
        }
        (log, -1)
    }

    #[test]
    fn mp7_tactical_ammo_at_add_time() {
        let t = table();
        let mut r = ScenarioDriver::new(&t);
        spawn(&mut r, MP7, 10, 120);
        let t0 = tap_reload(&mut r);
        assert_eq!(r.ps.gun_phase, state::RELOADING);
        let (changes, ready) = clip_changes(&mut r, MP7, t0, 5000);
        assert_eq!(changes, vec![(1300, 40)]);
        assert_eq!(ready, 1800);
        assert_eq!(r.ps.reserve_index(MP7), 90);
    }

    #[test]
    fn mp7_empty_auto_reload_uses_empty_time_and_normal_add() {
        let t = table();
        let mut r = ScenarioDriver::new(&t);
        spawn(&mut r, MP7, 0, 120);
        assert_eq!(r.ps.gun_phase, state::RELOADING);
        let t0 = r.now;
        let (changes, ready) = clip_changes(&mut r, MP7, t0, 5000);
        assert_eq!(changes, vec![(1300, 40)]);
        assert_eq!(ready, 2400);
    }

    #[test]
    fn ammo_going_in_clears_the_clip_used_up_mark() {
        let t = table();
        let mut r = ScenarioDriver::new(&t);
        r.env.e_flags2 |= super::super::fire::EF2_CLIP_USED_UP;
        spawn(&mut r, MP7, 0, 120);
        // Still marked while the reload runs, before the ammo goes in.
        assert_ne!(r.env.e_flags2 & super::super::fire::EF2_CLIP_USED_UP, 0);
        let t0 = r.now;
        let (changes, _) = clip_changes(&mut r, MP7, t0, 5000);
        assert_eq!(changes, vec![(1300, 40)]);
        assert_eq!(r.env.e_flags2 & super::super::fire::EF2_CLIP_USED_UP, 0);
    }

    #[test]
    fn short_stock_loads_what_is_left() {
        let t = table();
        let mut r = ScenarioDriver::new(&t);
        spawn(&mut r, MP7, 10, 5);
        let t0 = tap_reload(&mut r);
        let (changes, _) = clip_changes(&mut r, MP7, t0, 5000);
        assert_eq!(changes, vec![(1300, 15)]);
        assert_eq!(r.ps.reserve_index(MP7), 0);
    }

    #[test]
    fn fast_reload_perk_halves_the_add_time() {
        let t = table();
        let mut r = ScenarioDriver::new(&t);
        r.env.perks |= 1 << crate::env::perk::FASTRELOAD;
        spawn(&mut r, MP7, 10, 120);
        let t0 = tap_reload(&mut r);
        let (changes, ready) = clip_changes(&mut r, MP7, t0, 5000);
        assert_eq!(changes, vec![(650, 40)]);
        assert_eq!(ready, 900);
    }

    #[test]
    fn switch_before_add_keeps_old_clip() {
        let t = table();
        let mut r = ScenarioDriver::new(&t);
        r.ps.give(MP7);
        spawn(&mut r, AN94, 10, 120);
        let t0 = tap_reload(&mut r);
        while r.now - t0 < 500 {
            r.step();
        }
        r.cmd.weapon = MP7;
        for _ in 0..40 {
            r.step();
        }
        assert_eq!(r.ps.weapon, MP7);
        assert_eq!(r.ps.clip_slot(AN94), 10);
        assert_eq!(r.ps.reserve_index(AN94), 120);
    }

    #[test]
    fn shotgun_loads_one_shell_per_step() {
        let t = table();
        let mut r = ScenarioDriver::new(&t);
        spawn(&mut r, MCS, 5, 32);
        let t0 = tap_reload(&mut r);
        assert_eq!(r.ps.gun_phase, state::RELOAD_START);
        let (changes, ready) = clip_changes(&mut r, MCS, t0, 10_000);
        // Start loads one at 700; each 550 step loads one at 200; end 850.
        assert_eq!(changes, vec![(700, 6), (1200, 7), (1750, 8)]);
        assert_eq!(ready, 1000 + 550 + 550 + 850);
        assert_eq!(r.ps.reserve_index(MCS), 29);
    }

    /// The left-hand pass on its own (the whole-tick left pass needs window 8's
    /// BG_CheckForUniversalActionDW; until it merges the tick's left hand mirrors the right).
    #[test]
    fn dual_wield_left_gun_reloads_from_the_partner_pool() {
        use crate::pm::{FrameOut, Pm};
        let t = table();
        let mut r = ScenarioDriver::new(&t);
        r.ps.give(FS_DW);
        r.ps.give(FS_LH);
        r.ps.weapon = FS_DW;
        r.ps.gun_phase = state::READY;
        r.ps.weaponstate_left = state::READY;
        r.ps.set_clip_slot(FS_DW, 12);
        r.ps.set_clip_slot(FS_LH, 0);
        r.ps.set_pool_slot(FS_DW, 25);
        r.ps.b_run_left_gun = true;
        let mut out = FrameOut::default();
        let mut pm = Pm {
            ps: &mut r.ps,
            cmd: &r.cmd,
            env: &mut r.env,
            facts: &t,
            out: &mut out,
            delay_fired: false,
        };
        // The empty left gun starts its own empty reload; the right hand is untouched.
        super::check_for_reload(&mut pm);
        assert_eq!(pm.ps.weaponstate_left, state::RELOADING);
        assert_eq!(pm.ps.weapon_time_left, 2400);
        assert_eq!(pm.ps.weapon_delay_left, 1300);
        assert_eq!(pm.ps.gun_phase, state::READY);
        // The add point: 20 rounds from fiveseven_dw_mp's pool; the left gun has no pool of its own.
        pm.ps.weapon_time_left = 1100;
        pm.ps.weapon_delay_left = 0;
        pm.delay_fired = true;
        super::finish_reload(&mut pm);
        assert_eq!(pm.ps.clip_slot(FS_LH), 20);
        assert_eq!(pm.ps.reserve_index(FS_DW), 5);
        assert_eq!(pm.ps.reserve_index(FS_LH), 0);
        assert_eq!(pm.ps.clip_slot(FS_DW), 12);
    }

    /// Fast Mag on a dual gun: the left hand's reload flips the selected gun's flag when the
    /// selected gun is dual mag (BO2 tests ps->weapon, not the hand), and never otherwise.
    #[test]
    fn dual_wield_fast_mag_flip_reads_the_selected_gun() {
        use crate::pm::{FrameOut, Pm};
        for (dw_mag, lh_mag, flips) in [(true, false, true), (false, true, false)] {
            let mut t = table();
            let mut dw = t.get(FS_DW);
            dw.reload.dual_mag = dw_mag;
            t.set(FS_DW, "fiveseven_dw_mp", dw);
            let mut lh = t.get(FS_LH);
            lh.reload.dual_mag = lh_mag;
            t.set(FS_LH, "fiveseven_lh_mp", lh);
            let mut r = ScenarioDriver::new(&t);
            r.ps.give(FS_DW);
            r.ps.give(FS_LH);
            r.ps.weapon = FS_DW;
            r.ps.gun_phase = state::READY;
            r.ps.weaponstate_left = state::RELOADING;
            r.ps.weapon_time_left = 1100;
            r.ps.set_clip_slot(FS_DW, 12);
            r.ps.set_clip_slot(FS_LH, 0);
            r.ps.set_pool_slot(FS_DW, 25);
            r.ps.b_run_left_gun = true;
            let mut out = FrameOut::default();
            let mut pm = Pm {
                ps: &mut r.ps,
                cmd: &r.cmd,
                env: &mut r.env,
                facts: &t,
                out: &mut out,
                delay_fired: true,
            };
            super::finish_reload(&mut pm);
            assert_eq!(pm.ps.clip_slot(FS_LH), 20);
            assert_eq!(pm.ps.held(FS_DW).unwrap().quick_reload, flips, "dw {dw_mag} lh {lh_mag}");
            assert!(!pm.ps.held(FS_LH).unwrap().quick_reload);
        }
    }

    /// BG_PlayDWThirdPersonReload: the body reload plays when the second hand starts its
    /// reload while the first still has over half of its reload to go, and not otherwise.
    #[test]
    fn dual_wield_body_reload_needs_both_hands() {
        use crate::events::AnimScript;
        use crate::pm::{FrameOut, Pm};
        let t = table();
        let half_left = t.get(FS_LH).reload.reload >> 1;
        let shows = |left_state: i32, left_time: i32| {
            let mut r = ScenarioDriver::new(&t);
            r.ps.give(FS_DW);
            r.ps.give(FS_LH);
            r.ps.weapon = FS_DW;
            r.ps.gun_phase = state::READY;
            r.ps.weaponstate_left = left_state;
            r.ps.weapon_time_left = left_time;
            r.ps.set_clip_slot(FS_DW, 0);
            r.ps.set_clip_slot(FS_LH, 0);
            r.ps.set_pool_slot(FS_DW, 25);
            let mut out = FrameOut::default();
            let mut pm = Pm {
                ps: &mut r.ps,
                cmd: &r.cmd,
                env: &mut r.env,
                facts: &t,
                out: &mut out,
                delay_fired: false,
            };
            super::check_for_reload(&mut pm);
            assert_eq!(pm.ps.gun_phase, state::RELOADING);
            out.scripts.contains(&AnimScript::Reload)
        };
        // Left already reloading with over half to go: both arms reload on the body.
        assert!(shows(state::RELOADING, half_left + 1));
        // Left past its half, or not reloading: no body reload.
        assert!(!shows(state::RELOADING, half_left));
        assert!(!shows(state::READY, 0));
    }

    #[test]
    fn going_prone_mid_reload_restarts_the_body_reload() {
        use crate::events::AnimScript;
        use crate::pm::{FrameOut, Pm};
        let t = table();
        let shows = |prone_change: bool| {
            let mut r = ScenarioDriver::new(&t);
            r.ps.give(MCS);
            r.ps.weapon = MCS;
            r.ps.gun_phase = state::RELOAD_START;
            r.ps.gun_timer_ms = 200;
            r.ps.set_clip_slot(MCS, 0);
            r.ps.set_pool_slot(MCS, 25);
            r.env.prone_change = prone_change;
            let mut out = FrameOut::default();
            let mut pm = Pm {
                ps: &mut r.ps,
                cmd: &r.cmd,
                env: &mut r.env,
                facts: &t,
                out: &mut out,
                delay_fired: false,
            };
            super::check_for_reload(&mut pm);
            out.scripts.contains(&AnimScript::Reload)
        };
        assert!(shows(true));
        assert!(!shows(false));
    }
}
