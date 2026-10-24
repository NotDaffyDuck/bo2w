//! Window 6: Melee — the knife swing, the knife lunge and use-as-melee weapons.
//!
//! The spec with every number is docs/gun/MELEE.md. This file holds the swing as the weapon
//! tick runs it: the press check, the start (normal swing or lunge), the hit and the end.
//! [`anims`] picks the first-person animation a melee anim number plays; [`lunge`] is the
//! lunge's target pick and movement (run by movement, not by the weapon tick).

pub mod anims;
pub mod lunge;

use super::Pm;
use crate::anim;
use crate::base;
use crate::cmd::button;
use crate::env::{T6Dvars, ef, ef2, pm_type, pmf};
use crate::events::{AnimScript, EV_FIRE_MELEE, EV_MELEE_SWIPE};
use crate::facts::weap_type;
use crate::player::{Hand, wf};
use crate::state;
use fastfile_t6::layout::{WeaponDef as D, WeaponVariantDef as V};

/// Hit reach without a bayonet (`player_meleeRange` default; the swing reads the dvar).
pub const MELEE_RANGE: f32 = 55.0;

/// `weaponOverlayReticle_t` NONE: no scope overlay.
const OVERLAY_NONE: i32 = 0;
/// The melee numbers of one weapon.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct MeleeFacts {
    pub damage: i32,
    /// Swing length and hit delay (ms).
    pub time: i32,
    pub delay: i32,
    /// Lunge swing length and hit delay (ms).
    pub charge_time: i32,
    pub charge_delay: i32,
    /// `meleeChargeRange`: auto-melee reach of a melee weapon (0 = the default reach).
    pub charge_range: f32,
    pub has_bayonet: bool,
    /// BG_IsUseAsMeleeWeapon from the weapon itself: a bayonet or `bUseAsMelee`. An
    /// attachment's own `bUseAsMelee` is not in our weapon ids yet.
    pub use_as_melee: bool,
    /// `overlayReticle`: a scope overlay.
    pub overlay_reticle: i32,
    /// BG_UsingVariableZoomWeapon: the three ADS zoom fields differ.
    pub variable_zoom: bool,
}

impl MeleeFacts {
    pub fn read(variant: &[u8], def: &[u8]) -> Self {
        let has_bayonet = rd_u8(def, D::bHasBayonet) != 0;
        let fov = |at| f32::from_bits(rd_i32(variant, at) as u32);
        let (z1, z2, z3) = (fov(V::fAdsZoomFov1), fov(V::fAdsZoomFov2), fov(V::fAdsZoomFov3));
        Self {
            damage: rd_i32(def, D::iMeleeDamage),
            time: rd_i32(def, D::iMeleeTime),
            delay: rd_i32(def, D::iMeleeDelay),
            charge_time: rd_i32(def, D::meleeChargeTime),
            charge_delay: rd_i32(def, D::meleeChargeDelay),
            charge_range: f32::from_bits(rd_i32(def, D::meleeChargeRange) as u32),
            has_bayonet,
            use_as_melee: has_bayonet || rd_u8(def, D::bUseAsMelee) != 0,
            overlay_reticle: rd_i32(def, D::overlayReticle),
            variable_zoom: z1 != z2 || z1 != z3,
        }
    }

    /// The hit reach of this weapon's swing (`player_meleeRange`, or `player_bayonetRange`).
    pub fn reach(&self, d: &T6Dvars) -> f32 {
        if self.has_bayonet { d.player_bayonet_range } else { d.player_melee_range }
    }
}

fn rd_i32(b: &[u8], at: usize) -> i32 {
    b.get(at..at + 4)
        .map(|s| i32::from_le_bytes([s[0], s[1], s[2], s[3]]))
        .unwrap_or(0)
}

fn rd_u8(b: &[u8], at: usize) -> u8 {
    b.get(at).copied().unwrap_or(0)
}

/// The third-person melee anim: a riot shield bashes; a swing without a lunge (or with no gun
/// model) is a knife swing, a bare melee attack with no gun model, or the left-hand swing;
/// a lunge with a gun model is the knife charge.
pub fn pick_script(melee_weap_type: u8, has_gun_model: bool, lunge: bool, left: bool) -> AnimScript {
    if melee_weap_type == weap_type::RIOTSHIELD {
        AnimScript::MeleeAttack
    } else if has_gun_model && lunge {
        AnimScript::KnifeMeleeCharge
    } else if left {
        AnimScript::MeleeLeft
    } else if has_gun_model {
        AnimScript::KnifeMelee
    } else {
        AnimScript::MeleeAttack
    }
}

/// The weapon whose melee numbers time the swing: the held weapon when it is use-as-melee,
/// else the player's melee weapon (the knife).
pub fn melee_weapon(pm: &Pm) -> u32 {
    if pm.weapon_facts().melee.use_as_melee {
        pm.ps.weapon
    } else {
        pm.ps.melee_weapon
    }
}

/// PM_WeaponClipEmpty: the hand being run has an empty clip (never for unlimited ammo).
pub fn clip_empty(pm: &Pm) -> bool {
    !pm.weapon_facts().core.unlimited_ammo && pm.ps.ammo_in_clip(pm.facts, pm.hand_weapon()) == 0
}

fn is_melee_state(s: i32) -> bool {
    (state::MELEE_INIT..=state::MELEE_END).contains(&s)
}

/// Weapon states a melee press never cuts into: offhands, sprint, the riot shield, deploys,
/// and the like (BO2 states 22-34, 42, 43, 49-51).
fn blocks_melee(s: i32) -> bool {
    (22..=34).contains(&s) || s == 42 || s == 43 || (49..=51).contains(&s)
}

/// Firing and reload states a melee press may cut into while their delay still runs.
fn melee_cuts_in(s: i32) -> bool {
    (11..=18).contains(&s)
}

/// PM_Weapon_CheckForMelee: take a melee press on this tick. `r` is the timer step's result;
/// a press is never taken on the tick a hand's delay ran out.
pub fn check_for_melee(pm: &mut Pm, r: i32) {
    let held = pm.weapon_facts();
    let dual = held.core.dual_wield;
    let now = pm.cmd.buttons;
    let old = pm.cmd.old_buttons;
    // The left trigger only exists with two guns out (window 1's dual-wield mapping).
    let left_trig = crate::pm::fire::fire_button(pm, true);
    let left_held = dual && now & left_trig != 0;
    let melee_held = now & button::MELEE != 0;
    let attack_held = now & button::ATTACK != 0;
    let mut right = melee_held || attack_held;
    let mut left = left_held;

    if pm.env.pm_flags & (pmf::LADDER | pmf::GATE_2000) != 0
        || pm.ps.weap_flags & wf::DISABLE_SWITCH_TO_NONE != 0
    {
        return;
    }
    let is_melee_type = held.core.weap_type == weap_type::MELEE;
    let mw = melee_weapon(pm);
    let mdef = pm.facts.get(mw);
    let s = pm.ps.weaponstate;
    if pm.env.e_flags & ef::GATE_4000 != 0
        || pm.env.in_vehicle
        || blocks_melee(s)
        || pm.env.pm_flags & pmf::MANTLE != 0
    {
        return;
    }
    // A left trigger swing needs a melee weapon in the left hand.
    let partner = pm.facts.get(held.core.dual_wield_weapon);
    let left_ok = !left_held || partner.core.weap_type == weap_type::MELEE;
    let nothing_else = (!is_melee_type || !attack_held) && !melee_held && !left_held;
    if !left_ok || (!nothing_else && crate::pm::fire::check_friendly_fire(pm)) {
        return;
    }
    // A hand already swinging does not start again.
    if is_melee_state(s) && (attack_held || melee_held) {
        right = false;
    }
    if is_melee_state(pm.ps.weaponstate_left) && left_held {
        left = false;
    }

    let new_press = (melee_held && old & button::MELEE == 0)
        || (left_held && old & left_trig == 0)
        || (is_melee_type && attack_held && old & button::ATTACK == 0);
    let scoped = pm.ps.f_weapon_pos_frac > 0.0
        && (held.melee.overlay_reticle != OVERLAY_NONE || held.melee.variable_zoom);
    if mdef.melee.damage == 0
        || r != 0
        || !(pm.ps.weapon_delay == 0 || melee_cuts_in(s))
        || !new_press
        || scoped
        || !anims::has(mdef.core.anim_slots, anims::file::MELEE)
        || pm.env.e_flags2 & ef2::WEAPON_LOCKED != 0
        || (1..=5).contains(&s)
        || !(right || left)
    {
        return;
    }
    charge_start(pm);
    melee_init(pm, right, left);
}

/// PM_MeleeChargeStart: start a lunge when the command names a valid target, else clear one.
/// The lunge time is zeroed so the next move frame launches.
pub fn charge_start(pm: &mut Pm) {
    if !charge_valid(pm) {
        charge_clear(pm);
        return;
    }
    pm.env.pm_flags |= pmf::MELEE_CHARGE;
    pm.ps.melee_charge_time = 0;
    pm.ps.melee_charge_ent = pm.cmd.melee_charge_ent;
    pm.ps.melee_charge_dist = u32::from(pm.cmd.melee_charge_dist);
}

/// PM_MeleeChargeIsValid: no lunge running, the command names a target, a normal player on
/// the ground, not mantling or on a ladder. One more player-state gate (bits 0x300 of an
/// unidentified field) is taken as clear.
fn charge_valid(pm: &Pm) -> bool {
    pm.env.pm_flags & (pmf::MELEE_CHARGE | pmf::MANTLE | pmf::LADDER) == 0
        && pm.cmd.melee_charge_ent.is_some()
        && pm.env.pm_type == pm_type::NORMAL
        && pm.env.on_ground
}

/// PM_MeleeChargeClear, the weapon side: no lunge, no target. Movement also caps the
/// upward speed ([`lunge::Lunge::clear`]).
pub fn charge_clear(pm: &mut Pm) {
    pm.env.pm_flags &= !pmf::MELEE_CHARGE;
    pm.ps.melee_charge_time = 0;
    pm.ps.melee_charge_dist = 0;
    pm.ps.melee_charge_ent = None;
}

/// PM_Weapon_MeleeInit: start the swing on the hands given. A lunge started past the
/// weapon's reach uses the lunge timers and anim; otherwise each hand swings normally.
pub fn melee_init(pm: &mut Pm, right: bool, left: bool) {
    let mw = melee_weapon(pm);
    let mdef = pm.facts.get(mw);
    let m = mdef.melee;
    let lunge = pm.env.pm_flags & pmf::MELEE_CHARGE != 0 && pm.ps.melee_charge_dist as f32 > m.reach(&pm.env.dvars);
    let empty = clip_empty(pm);
    let alive = pm.env.pm_type < pm_type::DEAD;
    if lunge {
        let timers = (m.charge_time, m.charge_delay);
        if left {
            (pm.ps.weapon_time_left, pm.ps.weapon_delay_left) = timers;
        } else {
            (pm.ps.weapon_time, pm.ps.weapon_delay) = timers;
        }
        if alive {
            let a = if empty { anim::MELEE_CHARGE_EMPTY } else { anim::MELEE_CHARGE };
            pm.ps.weap_anim = anim::start(pm.ps.weap_anim, a);
        }
    } else {
        if left {
            (pm.ps.weapon_time_left, pm.ps.weapon_delay_left) = (m.time, m.delay);
            if alive {
                pm.ps.weap_anim_left = anim::start(pm.ps.weap_anim_left, anim::ATTACK);
            }
        }
        if right {
            (pm.ps.weapon_time, pm.ps.weapon_delay) = (m.time, m.delay);
            if alive {
                let a = if empty { anim::MELEE_ATTACK_EMPTY } else { anim::MELEE_ATTACK };
                pm.ps.weap_anim = anim::start(pm.ps.weap_anim, a);
            }
        }
    }
    let script = pick_script(mdef.core.weap_type, mdef.core.has_gun_model, lunge, left);
    pm.out.scripts.push(script);
    if left {
        pm.ps.weaponstate_left = state::MELEE_INIT;
    }
    if right {
        pm.ps.weaponstate = state::MELEE_INIT;
    }
    pm.add_event(EV_MELEE_SWIPE);
    base::set_prone_override(pm);
}

/// PM_Weapon_MeleeFire: the hit delay ran out (the server's hit test runs off this event).
pub fn melee_fire(pm: &mut Pm, left: bool) {
    if left {
        pm.ps.weaponstate_left = state::MELEE_FIRE;
    } else {
        pm.ps.weaponstate = state::MELEE_FIRE;
    }
    pm.add_event(EV_FIRE_MELEE);
    base::set_prone_override(pm);
}

/// PM_Weapon_MeleeEnd: the swing ran out. A gun comes back with its quick raise; a held
/// melee weapon goes straight back to idle.
pub fn melee_end(pm: &mut Pm, left: bool) {
    let held = pm.weapon_facts();
    if pm.ps.weapon != melee_weapon(pm) {
        let h = Hand {
            state: state::MELEE_END,
            time: held.switch.quick_raise,
            delay: 0,
            shot_count: pm.ps.hand(left).shot_count,
        };
        pm.ps.set_hand(left, h);
        if pm.env.waterlevel < 4 && pm.env.pm_type < pm_type::DEAD {
            pm.ps.weap_anim = anim::start(pm.ps.weap_anim, anim::QUICK_RAISE);
        }
        base::set_prone_override(pm);
        return;
    }
    if held.core.dual_wield {
        let h = Hand {
            state: state::READY,
            time: 0,
            delay: 0,
            shot_count: pm.ps.hand(left).shot_count,
        };
        pm.ps.set_hand(left, h);
        base::start_anim(pm, anim::IDLE, left);
        return;
    }
    base::idle(pm);
}


#[cfg(test)]
mod tests {
    use super::*;
    use crate::facts::{FactsTable, T6WeaponFacts};
    use crate::trace::Runner;

    const GUN: u32 = 1;
    const KNIFE: u32 = 2;
    const SMG: u32 = 3;
    const HELD_KNIFE: u32 = 4;

    fn gun(clip: i32, drop: i32, raise: i32, quick_raise: i32, first_raise: i32) -> T6WeaponFacts {
        let mut g = T6WeaponFacts::default();
        g.core.clip_size = clip;
        g.switch.drop = drop;
        g.switch.raise = raise;
        g.switch.quick_raise = quick_raise;
        g.switch.first_raise = first_raise;
        g
    }

    fn knife(m: MeleeFacts, quick_raise: i32) -> T6WeaponFacts {
        let mut k = T6WeaponFacts::default();
        k.core.weap_type = weap_type::MELEE;
        k.core.clip_size = 1;
        k.core.unlimited_ammo = true;
        k.switch.quick_raise = quick_raise;
        k.core.anim_slots = 1 << anims::file::MELEE | 1 << anims::file::MELEE_CHARGE;
        k.core.has_gun_model = true;
        k.melee = m;
        k
    }

    /// Real numbers (common_mp): knife_mp 150 damage, 800 ms swing hit at 125, lunge 1160 hit
    /// at 100; knife_held_mp 450 hit at 50, swings itself; an94_mp drop 400 / raise 600 /
    /// quick raise 400 / first raise 1600, reload 2475 (empty 3105, ammo in at 1350); mp7_mp 350 / 500 / 300 / 1000.
    fn table() -> FactsTable {
        let mut t = FactsTable::new();
        let k = MeleeFacts { damage: 150, time: 800, delay: 125, charge_time: 1160, charge_delay: 100, ..MeleeFacts::default() };
        let held = MeleeFacts { time: 450, delay: 50, charge_range: 116.0, use_as_melee: true, ..k };
        let mut an94 = gun(30, 400, 600, 400, 1600);
        an94.reload.reload = 2475;
        an94.reload.reload_empty = 3105;
        an94.reload.reload_add = 1350;
        t.set(GUN, "an94_mp", an94);
        t.set(KNIFE, "knife_mp", knife(k, 0));
        t.set(SMG, "mp7_mp", gun(40, 350, 500, 300, 1000));
        t.set(HELD_KNIFE, "knife_held_mp", knife(held, 300));
        t
    }

    /// The melee trace scripts (docs/gun/melee-traces) through the whole weapon tick, on the
    /// real numbers above. MELEE_TRACE_OUT=<dir> also writes each printed trace there.
    #[test]
    fn trace_scripts_pass_on_the_real_numbers() {
        let t = table();
        let scripts = [
            ("a_knife_on_gun", include_str!("../../../../docs/gun/melee-traces/a_knife_on_gun.txt")),
            ("b_fast_melee", include_str!("../../../../docs/gun/melee-traces/b_fast_melee.txt")),
            ("c_empty_gun", include_str!("../../../../docs/gun/melee-traces/c_empty_gun.txt")),
            ("d_mid_swap", include_str!("../../../../docs/gun/melee-traces/d_mid_swap.txt")),
            ("e_held_knife", include_str!("../../../../docs/gun/melee-traces/e_held_knife.txt")),
            ("f_mid_reload", include_str!("../../../../docs/gun/melee-traces/f_mid_reload.txt")),
            ("g_lunge", include_str!("../../../../docs/gun/melee-traces/g_lunge.txt")),
        ];
        let out_dir = std::env::var("MELEE_TRACE_OUT").ok();
        let mut failed = Vec::new();
        for (name, script) in scripts {
            let mut runner = Runner::new(&t);
            runner.ps.melee_weapon = KNIFE;
            let r = runner.run_script(script).expect(name);
            let text = r.lines.join("\n");
            if let Some(d) = &out_dir {
                std::fs::write(format!("{d}/{name}.trace.txt"), &text).expect("write trace");
            }
            if r.failures != 0 {
                failed.push(format!("{name}:\n{text}"));
            }
        }
        assert!(failed.is_empty(), "{}", failed.join("\n\n"));
    }

    /// Zombies numbers (zm_transit knife_zm, zm_highrise m1911_zm): the trace that sits next to
    /// the real BO2 clip in MELEE.md section 9.
    #[test]
    fn zombies_knife_trace_passes_on_the_real_numbers() {
        const ZM_GUN: u32 = 5;
        const ZM_KNIFE: u32 = 6;
        let mut t = FactsTable::new();
        let k = MeleeFacts { damage: 150, time: 500, delay: 50, charge_time: 1000, charge_delay: 150, charge_range: 128.0, ..MeleeFacts::default() };
        t.set(ZM_GUN, "m1911_zm", gun(8, 250, 250, 250, 250));
        t.set(ZM_KNIFE, "knife_zm", knife(k, 0));
        let mut runner = Runner::new(&t);
        runner.ps.melee_weapon = ZM_KNIFE;
        let r = runner.run_script(include_str!("../../../../docs/gun/melee-traces/h_zm_knife_m1911.txt")).unwrap();
        let text = r.lines.join("\n");
        if let Ok(d) = std::env::var("MELEE_TRACE_OUT") {
            std::fs::write(format!("{d}/h_zm_knife_m1911.trace.txt"), &text).expect("write trace");
        }
        assert_eq!(r.failures, 0, "{text}");
    }

    /// Runs `script` with the knife as the melee weapon (or none); panics with the trace on a
    /// failed `expect`.
    fn run(script: &str, knife: bool) -> String {
        let t = table();
        let mut r = Runner::new(&t);
        if knife {
            r.ps.melee_weapon = KNIFE;
        }
        let res = r.run_script(script).unwrap();
        let text = res.lines.join("\n");
        assert_eq!(res.failures, 0, "{text}");
        text
    }

    #[test]
    fn reads_melee_facts_at_layout_offsets() {
        let mut def = vec![0u8; 4096];
        def[D::iMeleeDamage..D::iMeleeDamage + 4].copy_from_slice(&150i32.to_le_bytes());
        def[D::iMeleeTime..D::iMeleeTime + 4].copy_from_slice(&800i32.to_le_bytes());
        def[D::meleeChargeRange..D::meleeChargeRange + 4].copy_from_slice(&150f32.to_le_bytes());
        def[D::bUseAsMelee] = 1;
        let mut variant = vec![0u8; 1024];
        variant[V::fAdsZoomFov1..V::fAdsZoomFov1 + 4].copy_from_slice(&30f32.to_le_bytes());
        let f = MeleeFacts::read(&variant, &def);
        assert_eq!((f.damage, f.time, f.charge_range), (150, 800, 150.0));
        assert!(f.use_as_melee && !f.has_bayonet && f.variable_zoom);
        assert_eq!(f.reach(&T6Dvars::default()), MELEE_RANGE);
    }

    #[test]
    fn third_person_picks() {
        use AnimScript::*;
        assert_eq!(pick_script(weap_type::RIOTSHIELD, true, true, false), MeleeAttack);
        assert_eq!(pick_script(weap_type::MELEE, true, true, false), KnifeMeleeCharge);
        assert_eq!(pick_script(weap_type::MELEE, true, false, false), KnifeMelee);
        assert_eq!(pick_script(weap_type::MELEE, false, false, false), MeleeAttack);
        assert_eq!(pick_script(weap_type::MELEE, true, false, true), MeleeLeft);
    }

    /// 50 ms ticks: the press tick starts the swing; the 125 ms hit lands on the 3rd tick
    /// after; the 800 ms swing ends 16 ticks after the press; the 400 ms quick raise 8 later.
    #[test]
    fn knife_on_a_gun_hits_then_quick_raises() {
        let text = run(
            "msec 50\nspawn an94_mp\nhold melee\nrun 1\nexpect state MELEE_INIT\nexpect anim MELEE_ATTACK\n\
             release melee\nrun 2\nexpect state MELEE_INIT\nrun 1\nexpect state MELEE_FIRE\n\
             run 12\nexpect state MELEE_FIRE\nrun 1\nexpect state MELEE_END\nexpect anim QUICK_RAISE\n\
             run 7\nexpect state MELEE_END\nrun 1\nexpect state READY\n",
            true,
        );
        assert!(text.contains("MELEE_SWIPE") && text.contains("FIRE_MELEE"), "{text}");
    }

    #[test]
    fn fast_melee_halves_only_the_quick_raise() {
        run(
            "msec 50\nspawn an94_mp\nperk 19\nhold melee\nrun 1\nrelease melee\nrun 16\n\
             expect state MELEE_END\nrun 3\nexpect state MELEE_END\nrun 1\nexpect state READY\n",
            true,
        );
    }

    #[test]
    fn empty_gun_swings_the_empty_anim() {
        run("msec 50\nspawn an94_mp\nclip an94_mp 0\nhold melee\nrun 1\nexpect anim MELEE_ATTACK_EMPTY\n", true);
    }

    #[test]
    fn no_swing_without_a_knife_or_while_switching() {
        run("msec 50\nspawn an94_mp\nhold melee\nrun 1\nexpect state READY\n", false);
        run(
            "msec 50\nspawn an94_mp\ngive knife_mp\nselect knife_mp\nrun 1\nhold melee\nrun 1\n\
             expect state DROPPING\n",
            true,
        );
    }

    #[test]
    fn held_melee_weapon_goes_straight_to_idle() {
        run(
            "msec 50\nspawn knife_mp\nhold melee\nrun 1\nexpect state MELEE_INIT\nrelease melee\n\
             run 3\nexpect state MELEE_FIRE\nrun 13\nexpect state READY\n",
            true,
        );
    }

    #[test]
    fn a_swing_in_progress_is_not_restarted() {
        // Held through the swing: it runs its course, and only a fresh press swings again.
        run(
            "msec 50\nspawn an94_mp\nhold melee\nrun 4\nexpect state MELEE_FIRE\nrun 30\n\
             expect state READY\nrelease melee\nrun 1\nhold melee\nrun 1\nexpect state MELEE_INIT\n",
            true,
        );
    }
}
