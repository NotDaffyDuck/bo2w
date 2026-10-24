//! Window 11: the riot shield (raise to plant, the placement preview, plant or lower) and
//! mountable-weapon deploy / breakdown. Timing spec: `docs/gun/SHIELD.md`.
//!
//! How BO2 plants a shield, in tick-switch order:
//! 1. READY, fire newly pressed: PREPARE, hold-fire time (`check_for_deploy_riotshield`).
//! 2. PREPARE runs out: HOLD, no timers (`riotshield_hold`).
//! 3. HOLD each tick: the placement test paints the shield red or normal; while fire stays held
//!    the shield waits there. On release: a good spot goes to START (fire time + fire delay),
//!    a bad one lowers the shield (END, reload time) (`riotshield_start`).
//! 4. START runs out: the plant event, the one round spent, END (`riotshield`).
//! 5. END runs out: idle (window 1's switch).

use super::Pm;
use crate::env::{ef, ef2, pmf};
use crate::events::{AnimScript, EV_DEPLOY_RIOTSHIELD, EV_LOWER_RIOTSHIELD, EV_PREP_RIOTSHIELD};
use crate::facts::weap_type;
use crate::player::wf;
use crate::{anim, base, state};
use fastfile_t6::layout::{WeaponDef as D, WeaponVariantDef as V};

/// weapFlags bits that block raising a shield.
const WF_NO_SHIELD: u64 = wf::DISABLE_SWITCH_TO_NONE | wf::GATE_100;
/// eFlags: on a turret / deployed.
const EF_DEPLOYED: u32 = 0x300;
/// Water level from which the shield can't come up.
const UNDERWATER: i32 = 4;

/// Window 11's per-weapon facts (read from `fastfile_t6::layout` offsets).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ShieldFacts {
    /// `iHoldFireTime`: how long the shield takes to come up before the preview.
    pub hold_fire_time: i32,
    /// `iFireTime`: the plant swing.
    pub fire_time: i32,
    /// `iFireDelay`: when in the plant swing the shield goes down.
    pub fire_delay: i32,
    /// `iReloadTime` (variant): lowering the shield from a bad spot.
    pub reload_time: i32,
    /// `mountableWeapon`.
    pub mountable: bool,
    pub deploy_time: i32,
    pub breakdown_time: i32,
}

fn rd_i32(b: &[u8], at: usize) -> i32 {
    b.get(at..at + 4)
        .map_or(0, |s| i32::from_le_bytes([s[0], s[1], s[2], s[3]]))
}

impl ShieldFacts {
    pub fn read(variant: &[u8], def: &[u8]) -> Self {
        Self {
            hold_fire_time: rd_i32(def, D::iHoldFireTime),
            fire_time: rd_i32(def, D::iFireTime),
            fire_delay: rd_i32(def, D::iFireDelay),
            reload_time: rd_i32(variant, V::iReloadTime),
            mountable: def.get(D::mountableWeapon).is_some_and(|&b| b != 0),
            deploy_time: rd_i32(def, D::deployTime),
            breakdown_time: rd_i32(def, D::breakdownTime),
        }
    }
}

// Inputs from window 1's shared files, in one place so a rename there is one line here.
fn held_now(pm: &Pm) -> bool {
    pm.cmd.buttons & crate::cmd::button::ATTACK != 0
}
fn held_before(pm: &Pm) -> bool {
    pm.cmd.old_buttons & crate::cmd::button::ATTACK != 0
}
fn can_place(pm: &Pm) -> bool {
    !pm.env.riotshield_blocked
}

/// BO2's weapon parm on shield events (`weaponData`).
fn weapon_parm(pm: &Pm) -> u32 {
    pm.ps.weapon
}

fn is_shield(pm: &Pm) -> bool {
    pm.ps.weapon != 0 && pm.weapon_facts().core.weap_type == weap_type::RIOTSHIELD
}

/// BG_SetRiotshieldModel: the held shield's red "can't place" model on or off. In zombies it
/// only ever turns on (it stays red until something else clears it).
fn set_model(pm: &mut Pm, red: bool) {
    if !is_shield(pm) {
        return;
    }
    let zombies = pm.env.zombies;
    let w = pm.ps.weapon;
    if let Some(h) = pm.ps.held_mut(w) {
        if !zombies {
            h.model = u8::from(red);
        } else if red {
            h.model = 1;
        }
    }
}

/// UpdateRiotShieldSwitchingFlag: the shield-switch flag only lives through a switch.
pub fn update_switching_flag(pm: &mut Pm) {
    if !(state::RAISING..=state::DROPPING_ALTSWITCH).contains(&pm.ps.weaponstate) {
        pm.ps.weap_flags &= !wf::RIOTSHIELD_SWITCH;
    }
}

/// Swimming with the shield up: the red model goes off.
pub fn model_off(pm: &mut Pm) {
    set_model(pm, false);
}

/// PM_Weapon_CheckForDeploy: mountable weapons go down on a deploy request and come back up
/// when it ends, on a reload or on an empty clip. No MP weapon is mountable.
pub fn check_for_deploy(pm: &mut Pm) {
    if !pm.weapon_facts().shield.mountable {
        return;
    }
    let s = pm.ps.weaponstate;
    let busy = (state::OFFHAND_INIT..=state::OFFHAND_END).contains(&s)
        || (state::RAISING..=state::DROPPING_ALTSWITCH).contains(&s)
        || (state::DTP_IN..=state::DTP_OUT).contains(&s)
        || (state::RIOTSHIELD_PREPARE..=state::RIOTSHIELD_END).contains(&s);
    if busy {
        return;
    }
    let reloading = (state::RELOADING..=state::RELOAD_QUICK_EMPTY).contains(&s);
    if pm.ps.weap_flags & wf::DEPLOY_REQUEST == 0
        || reloading
        || pm.ps.ammo_in_clip(pm.ps.weapon) < 1
    {
        if pm.ps.weap_flags & wf::MOUNTED != 0 {
            pm.ps.weap_flags &= !wf::MOUNTED;
            deploy_state(pm, state::BREAKING_DOWN);
        }
        return;
    }
    pm.ps.weap_flags &= !wf::DEPLOY_REQUEST;
    if pm.env.on_ground && pm.env.velocity_z >= 0.0 {
        deploy_state(pm, state::DEPLOYING);
    }
}

/// Deploy_State_Deploy / Deploy_State_Breakdown.
fn deploy_state(pm: &mut Pm, to: i32) {
    let f = pm.weapon_facts().shield;
    let (time, a) = if to == state::DEPLOYING {
        (f.deploy_time, anim::DEPLOYING)
    } else {
        (f.breakdown_time, anim::BREAKING_DOWN)
    };
    pm.ps.weaponstate = to;
    pm.ps.weapon_time = time;
    pm.ps.weapon_delay = 0;
    base::start_anim(pm, a, false);
}

/// PM_Weapon_IsHoldingGrenade: a press-to-throw grenade or mine with fire held.
fn holding_grenade(pm: &Pm) -> bool {
    if pm.ps.weapon == 0 {
        return false;
    }
    let core = pm.weapon_facts().core;
    (core.weap_type == weap_type::GRENADE || core.weap_type == weap_type::MINE)
        && !core.hold_button_to_throw
        && pm.cmd.buttons & super::fire::fire_button(pm, false) != 0
}

/// PM_Weapon_CheckForDeployRiotshield: a fresh fire press with the shield out starts raising it.
pub fn check_for_deploy_riotshield(pm: &mut Pm) {
    if !is_shield(pm)
        || pm.env.waterlevel >= UNDERWATER
        || pm.env.e_flags & EF_DEPLOYED != 0
        || pm.ps.weap_flags & WF_NO_SHIELD != 0
        || pm.env.using_vehicle_weapon
        || pm.env.e_flags & ef::GATE_4000 != 0
        || pm.env.pm_flags & pmf::SPRINT != 0
        || holding_grenade(pm)
    {
        return;
    }
    let s = pm.ps.weaponstate;
    // RIOTSHIELD_END (34) is allowed: a fresh press while lowering raises it again.
    let busy = (state::DROPPING..=state::DROPPING_ALTSWITCH).contains(&s)
        || (state::RIOTSHIELD_PREPARE..state::RIOTSHIELD_END).contains(&s)
        || (state::DTP_IN..=state::DTP_OUT).contains(&s)
        || (state::LOWREADY_RAISE..=state::LOWREADY_DROP).contains(&s)
        || (state::OFFHAND_INIT..=state::OFFHAND_END).contains(&s);
    if busy
        || pm.env.e_flags2 & ef2::WEAPON_LOCKED != 0
        || pm.ps.weap_flags & wf::CHANGE_LOCKED != 0
        || !held_now(pm)
        || held_before(pm)
    {
        return;
    }
    prepare(pm);
}

/// PM_Weapon_RiotshieldPrepare: the shield starts coming up.
fn prepare(pm: &mut Pm) {
    if pm.env.waterlevel >= UNDERWATER {
        return;
    }
    pm.ps.weaponstate = state::RIOTSHIELD_PREPARE;
    pm.ps.weapon_time = pm.weapon_facts().shield.hold_fire_time;
    pm.ps.weapon_delay = 0;
    let parm = weapon_parm(pm);
    pm.add_predictable_event(EV_PREP_RIOTSHIELD, parm);
    base::start_anim(pm, anim::HOLD_FIRE, false);
    base::start_anim(pm, anim::HOLD_FIRE, true);
    base::set_prone_override(pm);
}

/// PM_Weapon_RiotshieldHold (runs when PREPARE ends): the shield is up, the preview starts.
pub fn riotshield_hold(pm: &mut Pm) {
    pm.server_print("end weapon (riotshield hold)");
    pm.ps.weaponstate = state::RIOTSHIELD_HOLD;
    pm.ps.weapon_time = 0;
    pm.ps.weapon_delay = 0;
}

/// PM_Weapon_Riotshield (runs when START ends): the shield goes into the ground.
pub fn riotshield(pm: &mut Pm) {
    let parm = weapon_parm(pm);
    pm.add_predictable_event(EV_DEPLOY_RIOTSHIELD, parm);
    // PM_WeaponUseAmmo(1): nothing spent while player_sustainAmmo is on.
    if !pm.env.dvars.player_sustain_ammo {
        let w = pm.ps.weapon;
        let clip = pm.ps.ammo_in_clip(w);
        pm.ps.set_ammo_in_clip(w, clip - clip.min(1));
    }
    pm.ps.weaponstate = state::RIOTSHIELD_END;
}

/// PM_Weapon_RiotshieldStart (runs each tick in HOLD): preview the spot while fire is held,
/// then plant (good spot) or lower the shield (bad spot) on release.
pub fn riotshield_start(pm: &mut Pm) {
    let f = pm.weapon_facts().shield;
    if is_shield(pm) {
        let ok = can_place(pm);
        set_model(pm, !ok);
        if ok {
            pm.ps.weap_flags &= !wf::RIOTSHIELD_CANT_PLACE;
        } else {
            pm.ps.weap_flags |= wf::RIOTSHIELD_CANT_PLACE;
        }
        if held_before(pm) && held_now(pm) {
            pm.ps.weapon_delay = 1;
            return;
        }
        set_model(pm, false);
        if ok {
            pm.ps.weaponstate = state::RIOTSHIELD_START;
            pm.ps.weapon_time = f.fire_time;
            pm.ps.weapon_delay = f.fire_delay;
            base::start_anim(pm, anim::ATTACK, false);
            pm.out.scripts.push(AnimScript::FireWeapon);
            return;
        }
    }
    // A bad spot, or the weapon is no longer a shield: lower it.
    pm.ps.weaponstate = state::RIOTSHIELD_END;
    pm.ps.weapon_time = f.reload_time;
    pm.ps.weapon_delay = 0;
    base::start_anim(pm, anim::RELOAD, false);
    let parm = weapon_parm(pm);
    pm.add_predictable_event(EV_LOWER_RIOTSHIELD, parm);
}

#[cfg(test)]
mod tests {
    use super::ShieldFacts;
    use crate::cmd::{T6Cmd, button};
    use crate::env::T6Env;
    use crate::events::{EV_DEPLOY_RIOTSHIELD, EV_LOWER_RIOTSHIELD, EV_PREP_RIOTSHIELD};
    use crate::facts::{FactsTable, T6WeaponFacts, weap_type};
    use crate::player::{T6WeaponPs, wf};
    use crate::{anim, state, tick};

    const SHIELD: u32 = 1;
    const MOUNT: u32 = 2;

    /// riotshield_mp's numbers (SHIELD.md section 2) and a made-up mountable gun.
    fn facts() -> FactsTable {
        let mut t = FactsTable::new();
        let mut s = T6WeaponFacts::default();
        s.core.weap_type = weap_type::RIOTSHIELD;
        s.core.clip_size = 1;
        s.shield = ShieldFacts {
            hold_fire_time: 125,
            fire_time: 500,
            fire_delay: 400,
            reload_time: 300,
            ..ShieldFacts::default()
        };
        t.set(SHIELD, "riotshield_mp", s);
        let mut m = T6WeaponFacts::default();
        m.core.clip_size = 30;
        m.shield = ShieldFacts {
            mountable: true,
            deploy_time: 600,
            breakdown_time: 450,
            ..ShieldFacts::default()
        };
        t.set(MOUNT, "mount_test", m);
        t
    }

    /// A spawned player holding `w`, ticked like the trace runner (last tick's buttons kept).
    struct Sim {
        ps: T6WeaponPs,
        cmd: T6Cmd,
        env: T6Env,
        facts: FactsTable,
        /// Each tick: the state it ended in and the events it raised.
        log: Vec<(i32, Vec<u16>)>,
    }

    impl Sim {
        fn new(w: u32, msec: i32) -> Self {
            let facts = facts();
            let mut ps = T6WeaponPs::default();
            ps.give(w);
            ps.set_ammo_in_clip(w, facts.get(w).core.clip_size);
            ps.weapon = w;
            let cmd = T6Cmd {
                msec,
                weapon: w,
                ..T6Cmd::default()
            };
            let env = T6Env {
                is_server: true,
                view_height_settled: true,
                ..T6Env::default()
            };
            Self {
                ps,
                cmd,
                env,
                facts,
                log: Vec::new(),
            }
        }

        fn hold(&mut self, on: bool) {
            if on {
                self.cmd.buttons |= button::ATTACK;
            } else {
                self.cmd.buttons &= !button::ATTACK;
            }
        }

        fn run(&mut self, n: usize) {
            for _ in 0..n {
                self.cmd.server_time += self.cmd.msec;
                let out = tick::run(&mut self.ps, &self.cmd, &mut self.env, &self.facts);
                self.cmd.old_buttons = self.cmd.buttons;
                let evs = out.events.iter().map(|e| e.id).collect();
                self.log.push((self.ps.weaponstate, evs));
            }
        }

        /// First tick from `after` on that ended in state `s`. Tick 0 is the first tick run
        /// (the press, as in SHIELD.md section 10).
        fn first_state_after(&self, after: usize, s: i32) -> usize {
            self.log[after..]
                .iter()
                .position(|(st, _)| *st == s)
                .unwrap_or_else(|| panic!("state {s} never reached: {:?}", self.log))
                + after
        }

        fn first_state(&self, s: i32) -> usize {
            self.first_state_after(0, s)
        }

        fn first_event(&self, ev: u16) -> usize {
            self.log
                .iter()
                .position(|(_, e)| e.contains(&ev))
                .unwrap_or_else(|| panic!("event {ev} never raised: {:?}", self.log))
        }

        fn model(&self) -> u8 {
            self.ps.held(self.ps.weapon).map_or(0, |h| h.model)
        }
    }

    /// A tap: fire pressed for one tick, then released.
    fn tap(msec: i32, blocked: bool) -> Sim {
        let mut s = Sim::new(SHIELD, msec);
        s.env.riotshield_blocked = blocked;
        s.hold(true);
        s.run(1);
        s.hold(false);
        s.run(60);
        s
    }

    #[test]
    fn tap_plants_at_16ms() {
        let s = tap(16, false);
        assert_eq!(
            s.log[0],
            (state::RIOTSHIELD_PREPARE, vec![EV_PREP_RIOTSHIELD])
        );
        assert_eq!(s.first_state(state::RIOTSHIELD_HOLD), 8);
        assert_eq!(s.first_state(state::RIOTSHIELD_START), 9);
        assert_eq!(s.first_event(EV_DEPLOY_RIOTSHIELD), 34);
        assert_eq!(s.first_state(state::RIOTSHIELD_END), 34);
        assert_eq!(s.first_state_after(34, state::READY), 41);
        assert_eq!(s.ps.ammo_in_clip(SHIELD), 0, "the plant spends the round");
    }

    #[test]
    fn tap_plants_at_50ms() {
        let s = tap(50, false);
        assert_eq!(s.first_state(state::RIOTSHIELD_HOLD), 3);
        assert_eq!(s.first_state(state::RIOTSHIELD_START), 4);
        assert_eq!(s.first_event(EV_DEPLOY_RIOTSHIELD), 12);
        assert_eq!(s.first_state_after(12, state::READY), 14);
    }

    #[test]
    fn sustain_ammo_keeps_the_round_on_a_plant() {
        let mut s = Sim::new(SHIELD, 50);
        s.env.dvars.player_sustain_ammo = true;
        s.hold(true);
        s.run(1);
        s.hold(false);
        s.run(20);
        assert_eq!(s.first_event(EV_DEPLOY_RIOTSHIELD), 12);
        assert_eq!(s.ps.ammo_in_clip(SHIELD), 1);
    }

    #[test]
    fn bad_spot_lowers_and_keeps_the_round() {
        for (msec, ticks) in [(16, 19), (50, 6)] {
            let s = tap(msec, true);
            let end = s.first_state(state::RIOTSHIELD_END);
            assert_eq!(s.first_event(EV_LOWER_RIOTSHIELD), end);
            assert_eq!(
                s.first_state_after(end, state::READY) - end,
                ticks,
                "{msec} ms"
            );
            assert!(
                s.log
                    .iter()
                    .all(|(_, e)| !e.contains(&EV_DEPLOY_RIOTSHIELD))
            );
            assert_eq!(s.ps.ammo_in_clip(SHIELD), 1);
        }
    }

    #[test]
    fn holding_fire_waits_in_the_preview_red_over_a_bad_spot() {
        let mut s = Sim::new(SHIELD, 50);
        s.env.riotshield_blocked = true;
        s.hold(true);
        s.run(20);
        assert_eq!(s.ps.weaponstate, state::RIOTSHIELD_HOLD);
        assert_eq!(s.ps.weapon_delay, 1);
        assert_eq!(s.model(), 1, "red over a bad spot");
        assert_ne!(s.ps.weap_flags & wf::RIOTSHIELD_CANT_PLACE, 0);
        // The spot turns good while still held: normal model, flag off, still waiting.
        s.env.riotshield_blocked = false;
        s.run(1);
        assert_eq!(
            (s.model(), s.ps.weap_flags & wf::RIOTSHIELD_CANT_PLACE),
            (0, 0)
        );
        assert_eq!(s.ps.weaponstate, state::RIOTSHIELD_HOLD);
        s.hold(false);
        s.run(1);
        assert_eq!(s.ps.weaponstate, state::RIOTSHIELD_START);
        assert_eq!(anim::number(s.ps.weap_anim), anim::ATTACK);
    }

    #[test]
    fn release_over_a_bad_spot_clears_the_red_model() {
        let mut s = Sim::new(SHIELD, 50);
        s.env.riotshield_blocked = true;
        s.hold(true);
        s.run(5);
        assert_eq!(s.model(), 1);
        s.hold(false);
        s.run(1);
        assert_eq!(s.ps.weaponstate, state::RIOTSHIELD_END);
        assert_eq!(anim::number(s.ps.weap_anim), anim::RELOAD);
        assert_eq!(s.model(), 0, "MP: red off on release");
    }

    #[test]
    fn zombies_red_model_sticks() {
        let mut s = Sim::new(SHIELD, 50);
        s.env.zombies = true;
        s.env.riotshield_blocked = true;
        s.hold(true);
        s.run(5);
        s.hold(false);
        s.run(1);
        assert_eq!(s.model(), 1, "zombies only ever turns the red model on");
    }

    #[test]
    fn only_a_fresh_press_raises_it() {
        let mut s = Sim::new(SHIELD, 50);
        s.cmd.old_buttons = button::ATTACK;
        s.hold(true);
        s.run(1);
        assert_eq!(s.ps.weaponstate, state::READY, "fire held from before");
        let mut s = Sim::new(SHIELD, 50);
        s.env.waterlevel = 4;
        s.hold(true);
        s.run(1);
        assert_ne!(s.ps.weaponstate, state::RIOTSHIELD_PREPARE, "under water");
        let mut s = Sim::new(SHIELD, 50);
        s.ps.weap_flags |= wf::CHANGE_LOCKED;
        s.hold(true);
        s.run(1);
        assert_eq!(s.ps.weaponstate, state::READY, "weapon change locked");
        let mut s = Sim::new(MOUNT, 50);
        s.hold(true);
        s.run(1);
        assert_ne!(s.ps.weaponstate, state::RIOTSHIELD_PREPARE, "not a shield");
    }

    #[test]
    fn a_new_press_while_lowering_raises_it_again() {
        let mut s = Sim::new(SHIELD, 50);
        s.env.riotshield_blocked = true;
        s.hold(true);
        s.run(1);
        s.hold(false);
        s.run(4);
        assert_eq!(s.ps.weaponstate, state::RIOTSHIELD_END);
        s.hold(true);
        s.run(1);
        assert_eq!(s.ps.weaponstate, state::RIOTSHIELD_PREPARE);
    }

    #[test]
    fn mountable_deploys_on_ground_and_breaks_down_when_the_request_ends() {
        let mut s = Sim::new(MOUNT, 50);
        s.ps.weap_flags |= wf::DEPLOY_REQUEST;
        s.run(1);
        assert_eq!(
            s.ps.weaponstate,
            state::READY,
            "in the air: request spent, no deploy"
        );
        assert_eq!(s.ps.weap_flags & wf::DEPLOY_REQUEST, 0);
        s.env.on_ground = true;
        s.ps.weap_flags |= wf::DEPLOY_REQUEST;
        s.run(1);
        assert_eq!(s.ps.weaponstate, state::DEPLOYING);
        assert_eq!(s.ps.weapon_time, 600);
        assert_eq!(anim::number(s.ps.weap_anim), anim::DEPLOYING);
        // Once deployed the game marks it mounted; with no request left it breaks down.
        s.run(11);
        s.ps.weap_flags |= wf::MOUNTED;
        s.run(1);
        assert_eq!(s.ps.weaponstate, state::BREAKING_DOWN);
        assert_eq!(anim::number(s.ps.weap_anim), anim::BREAKING_DOWN);
        assert_eq!(s.ps.weap_flags & wf::MOUNTED, 0);
    }

    #[test]
    fn switching_flag_clears_outside_a_switch() {
        let mut s = Sim::new(SHIELD, 50);
        s.ps.weap_flags |= wf::RIOTSHIELD_SWITCH;
        s.run(1);
        assert_eq!(s.ps.weap_flags & wf::RIOTSHIELD_SWITCH, 0);
    }

    #[test]
    fn trace_script_shows_the_plant_and_the_lower() {
        let f = facts();
        let script = "
            msec 16
            spawn riotshield_mp
            hold attack
            run 1
            expect state RIOTSHIELD_PREPARE
            release attack
            run 8 # ticks 1-8: PREPARE's 125 ms
            expect state RIOTSHIELD_HOLD
            run 33 # START, the plant at 34, END, READY at 41
            expect state READY
            blocked 1
            hold attack
            run 1
            release attack
            run 9 # PREPARE, HOLD, then the bad spot lowers it
            expect state RIOTSHIELD_END
        ";
        let r = crate::trace::Runner::new(&f)
            .run_script(script)
            .expect("script runs");
        assert_eq!(r.failures, 0, "{}", r.lines.join("\n"));
    }
}
