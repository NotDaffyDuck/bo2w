//! Window 13: what the GSC builtins do to the gun brain, and the notifies the gun sends the script.
//!
//! Every function here is one BO2 builtin acting on [`T6WeaponPs`], with BO2's order and checks
//! (docs/gun/SCRIPT.md section 2). Nothing here special-cases a trickshot bind: canswap, canzoom,
//! illusion, instashoot and the rest fall out of these rules plus the switch rules in
//! [`crate::switch`]. The game side calls these from its natives and applies [`Effects`].
//!
//! Frame order in BO2: a server frame runs the scripts, then [`NotifyWatch::frame`] for every
//! client. The client's usercmds (the weapon tick, where [`event_notifies`] fire) are run between
//! server frames, as their packets arrive. So a takeweapon is seen by the same frame's notifies,
//! before the client's next usercmd raises anything.

use crate::events::{self, Event};
use crate::facts::{FactsTable, fire_type, inventory, weap_class, weap_type};
use crate::player::{HeldWeapon, T6WeaponPs, wf};
use crate::state;

/// The `pm_flags` bit `initialweaponraise` clears.
pub const PMF_INITIAL_RAISE_CLEARS: u32 = 0x400;

/// What a builtin asks the game to do outside the weapon state.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Effects {
    /// The server's weapon-select message: the client asks for this gun from its next usercmd on.
    pub select: Option<u32>,
    /// The server's stored copy of the client's last command had its weapon zeroed. The client
    /// itself is not told, so its next usercmd still asks for the old gun.
    pub zero_server_cmd_weapon: bool,
    /// A bot switches to this gun through its own path instead.
    pub bot_switch: Option<u32>,
    /// The "you got a weapon" message (giveweapon), one per gun handed over.
    pub gave: Vec<u32>,
    /// Deploy flags on the entity clear (a take while deploying / deployed / breaking down).
    pub clear_deployed: bool,
    /// `pm_flags` bits to clear.
    pub clear_pm_flags: u32,
    /// A dive in the air ends at once (disableweapons mid-dive): flag off, velocity and jump
    /// time zeroed, the dive's end time set to now, sprint needs no fresh press. Movement applies it.
    pub end_dive: bool,
}

/// The script errors BO2 raises (the script stops at the call).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ScriptError {
    /// "player does not have weapon" / "Trying to give player ammo for weapon .. not holding".
    NotHeld(u32),
    /// A weapon players cannot hold (turret, non-player class).
    NotPlayerWeapon(u32),
    /// Every held slot is taken.
    NoFreeSlot(u32),
}

fn facts_of(facts: &FactsTable, w: u32) -> crate::facts::CoreFacts {
    facts.get(w).core
}

fn clip_size(facts: &FactsTable, w: u32) -> i32 {
    facts_of(facts, w).clip_size.max(0)
}

/// A gun's alternate-mode chain (grenade launcher, second optic ...), without the gun itself.
fn alt_chain(facts: &FactsTable, w: u32) -> Vec<u32> {
    let mut v = Vec::new();
    let mut a = facts_of(facts, w).alt_weapon;
    while a != 0 && a != w && !v.contains(&a) && v.len() < 4 {
        v.push(a);
        a = facts_of(facts, a).alt_weapon;
    }
    v
}

/// Put one gun in a free slot the way a script give does: raised-before cleared (so it first-raises),
/// rechamber cleared, model kept.
fn hold_new(ps: &mut T6WeaponPs, w: u32, model: u8) -> Result<(), ScriptError> {
    if !ps.give(w) {
        return Err(ScriptError::NoFreeSlot(w));
    }
    if let Some(h) = ps.held_mut(w) {
        *h = HeldWeapon {
            weapon: w,
            model,
            ..HeldWeapon::default()
        };
    }
    Ok(())
}

fn has_ammo(ps: &T6WeaponPs, w: u32) -> bool {
    ps.ammo_in_clip(w) > 0 || ps.ammo_not_in_clip(w) > 0
}

/// giveweapon(name [, model] [, renderOptions]). `start_stock` is the start ammo (stock) for a
/// weapon id; the ammo pools belong to window 4.
pub fn give_weapon(
    ps: &mut T6WeaponPs,
    facts: &FactsTable,
    w: u32,
    model: u8,
    start_stock: &dyn Fn(u32) -> i32,
) -> Result<Effects, ScriptError> {
    let mut fx = Effects::default();
    if w == 0 || ps.has_weapon(w) {
        // Already held: nothing at all happens.
        return Ok(fx);
    }
    let c = facts_of(facts, w);
    if c.weap_class == weap_class::TURRET || c.weap_class == weap_class::NON_PLAYER {
        return Err(ScriptError::NotPlayerWeapon(w));
    }
    hold_new(ps, w, model)?;
    if c.weap_class == weap_class::ITEM {
        return Ok(fx);
    }
    if c.inventory_type == inventory::MELEE {
        ps.melee_weapon = w;
    }

    let mut given = vec![w];
    for a in alt_chain(facts, w) {
        if facts_of(facts, a).fire_type == fire_type::CHARGESHOT || ps.has_weapon(a) {
            continue;
        }
        hold_new(ps, a, model)?;
        given.push(a);
    }
    if c.dual_wield && c.dual_wield_weapon != 0 && !ps.has_weapon(c.dual_wield_weapon) {
        hold_new(ps, c.dual_wield_weapon, model)?;
        given.push(c.dual_wield_weapon);
    }

    fx.gave.push(w);
    for &g in &given {
        ps.set_ammo_not_in_clip(g, start_stock(g));
        ps.set_ammo_in_clip(g, clip_size(facts, g));
    }

    if c.inventory_type == inventory::OFFHAND {
        let equipped = ps.off_hand_weapon;
        if equipped == 0 || !ps.has_weapon(equipped) {
            ps.off_hand_weapon = w;
        } else if !has_ammo(ps, equipped) {
            let pick = ps.held.iter().map(|h| h.weapon).find(|&h| {
                h != 0 && facts_of(facts, h).inventory_type == inventory::OFFHAND && has_ammo(ps, h)
            });
            if let Some(p) = pick {
                ps.off_hand_weapon = p;
            }
        }
    }
    Ok(fx)
}

/// Free one gun's slot and its clip.
fn drop_slot(ps: &mut T6WeaponPs, w: u32) {
    if let Some(c) = ps.ammo_in_clip.iter_mut().find(|c| c.weapon == w) {
        *c = Default::default();
    }
    ps.take(w);
}

/// takeweapon(name). The gun in hand goes to none on the server, and the server's copy of the
/// command weapon too; the client keeps asking for it (that is canswap).
pub fn take_weapon(ps: &mut T6WeaponPs, facts: &FactsTable, w: u32) -> Effects {
    let mut fx = Effects::default();
    if w != 0 && ps.weapon == w {
        ps.weapon = 0;
        fx.zero_server_cmd_weapon = true;
    }
    if !ps.has_weapon(w) {
        return fx;
    }
    if ps.melee_weapon == w {
        ps.melee_weapon = 0;
    }
    for a in alt_chain(facts, w) {
        if ps.has_weapon(a) {
            ps.set_ammo_in_clip(a, 0);
            ps.set_ammo_not_in_clip(a, 0);
            drop_slot(ps, a);
        }
    }
    let c = facts_of(facts, w);
    if c.dual_wield && c.dual_wield_weapon != 0 {
        drop_slot(ps, c.dual_wield_weapon);
    }
    if (state::DEPLOYING..=state::BREAKING_DOWN).contains(&ps.weaponstate) {
        fx.clear_deployed = true;
        ps.weaponstate = state::READY;
        ps.weaponstate_left = state::READY;
    }
    drop_slot(ps, w);
    fx
}

/// takeallweapons().
pub fn take_all_weapons(ps: &mut T6WeaponPs, facts: &FactsTable) -> Effects {
    let mut fx = Effects::default();
    ps.weapon = 0;
    fx.zero_server_cmd_weapon = true;
    let held: Vec<u32> = ps
        .held
        .iter()
        .map(|h| h.weapon)
        .filter(|&w| w != 0)
        .collect();
    for w in held {
        let f = take_weapon(ps, facts, w);
        fx.clear_deployed |= f.clear_deployed;
    }
    fx
}

/// switchtoweapon(name) / switchtoweaponimmediate(name). True when the gun is held. Nothing
/// changes on the server now except, for the immediate form, the gun in hand going to none (so
/// the next usercmd raises the new gun with no drop).
pub fn switch_to_weapon(
    ps: &mut T6WeaponPs,
    w: u32,
    immediate: bool,
    bot: bool,
) -> (bool, Effects) {
    let mut fx = Effects::default();
    if !ps.has_weapon(w) {
        return (false, fx);
    }
    if bot {
        fx.bot_switch = Some(w);
        return (true, fx);
    }
    if immediate {
        ps.weapon = 0;
    }
    fx.select = Some(w);
    (true, fx)
}

/// setspawnweapon(name): the gun in hand at once, state ready with no raise, its raised-before
/// flag set. The timers and the anim are left as they are (a cut raise keeps playing its anim).
pub fn set_spawn_weapon(
    ps: &mut T6WeaponPs,
    facts: &FactsTable,
    w: u32,
    bot: bool,
) -> Result<Effects, ScriptError> {
    if !ps.has_weapon(w) {
        return Err(ScriptError::NotHeld(w));
    }
    let mut fx = Effects::default();
    ps.weapon = w;
    ps.weaponstate = state::READY;
    ps.set_ever_had(w, true);
    if facts_of(facts, w).inventory_type == inventory::ALTMODE {
        let base = facts_of(facts, w).alt_weapon;
        ps.set_ever_had(base, true);
        ps.last_weapon_alt_mode_switch = base;
    }
    fx.select = Some(w);
    if bot {
        fx.bot_switch = Some(w);
    }
    Ok(fx)
}

/// initialweaponraise(name): clear the gun's raised-before flag, switch to it at once, clear the
/// movement flag. Returns the switch's true/false.
pub fn initial_weapon_raise(ps: &mut T6WeaponPs, w: u32, bot: bool) -> (bool, Effects) {
    ps.set_ever_had(w, false);
    let (ok, mut fx) = switch_to_weapon(ps, w, true, bot);
    fx.clear_pm_flags |= PMF_INITIAL_RAISE_CLEARS;
    (ok, fx)
}

/// seteverhadweaponall(bool): false = every held gun first-raises again.
pub fn set_ever_had_weapon_all(ps: &mut T6WeaponPs, value: bool) {
    ps.set_ever_had_all(value);
}

/// isswitchingweapons(): raising or dropping.
pub fn is_switching_weapons(ps: &T6WeaponPs) -> bool {
    (state::RAISING..=state::DROPPING_ALTSWITCH).contains(&ps.weaponstate)
}

/// isreloading(): the reload family 11-18 (not a dual wield's per-hand 9/10).
pub fn is_reloading(ps: &T6WeaponPs) -> bool {
    (state::RELOADING..=state::RELOAD_QUICK_EMPTY).contains(&ps.weaponstate)
}

/// isthrowinggrenade(): the offhand family 22-29.
pub fn is_throwing_grenade(ps: &T6WeaponPs) -> bool {
    (state::OFFHAND_INIT..=state::OFFHAND_END).contains(&ps.weaponstate)
}

/// isfiring(): firing, the two rechamber states, the continuous-fire loop, or a melee swing.
pub fn is_firing(ps: &T6WeaponPs) -> bool {
    matches!(
        ps.weaponstate,
        state::FIRING | state::RECHAMBER_START | state::RECHAMBERING | state::CONT_FIRE_LOOP
    ) || is_meleeing(ps)
}

/// ismeleeing(): the three melee states.
pub fn is_meleeing(ps: &T6WeaponPs) -> bool {
    (state::MELEE_INIT..=state::MELEE_END).contains(&ps.weaponstate)
}

/// hasagrenadepulledback(): offhand init, prepare, hold, start, or the throw itself.
pub fn has_a_grenade_pulled_back(ps: &T6WeaponPs) -> bool {
    matches!(
        ps.weaponstate,
        state::OFFHAND_INIT
            | state::OFFHAND_PREPARE
            | state::OFFHAND_HOLD
            | state::OFFHAND_START
            | state::OFFHAND
    )
}

/// isusingoffhand(): either offhand-in-use flag on the weapon flags.
pub fn is_using_offhand(ps: &T6WeaponPs) -> bool {
    ps.weap_flags & (wf::USING_OFFHAND | wf::WEAPONS_DISABLED) != 0
}

/// disableweapons(): the next switch goes to no weapon. A dive still in the air is ended.
pub fn disable_weapons(ps: &mut T6WeaponPs, pm_flags: u32) -> Effects {
    ps.weap_flags |= wf::DISABLE_SWITCH_TO_NONE;
    let mut fx = Effects::default();
    if pm_flags & crate::env::pmf::DTP_AIR != 0 {
        fx.clear_pm_flags = crate::env::pmf::DTP_AIR;
        fx.end_dive = true;
    }
    fx
}

/// enableweapons().
pub fn enable_weapons(ps: &mut T6WeaponPs) {
    ps.weap_flags &= !wf::DISABLE_SWITCH_TO_NONE;
}

/// disableweaponcycling() / enableweaponcycling(): the flag that keeps the gun in hand when it
/// runs dry.
pub fn set_weapon_cycling(ps: &mut T6WeaponPs, enabled: bool) {
    if enabled {
        ps.weap_flags &= !wf::NO_AMMO_SWITCH;
    } else {
        ps.weap_flags |= wf::NO_AMMO_SWITCH;
    }
}

/// setweaponammoclip(name, count). Clamped to 0..clip size. A dual wield gun then fills the left
/// partner of the gun **in hand** to a full clip (BO2's second step does not use the count).
/// A left-hand gun's own file is flagged dual wield too, so a count set on the left pistol
/// sticks only while another gun is in hand; with the pistols up BO2 refills it at once.
pub fn set_weapon_ammo_clip(
    ps: &mut T6WeaponPs,
    facts: &FactsTable,
    w: u32,
    count: i32,
) -> Result<(), ScriptError> {
    if w == 0 {
        return Ok(());
    }
    if !ps.has_weapon(w) {
        return Err(ScriptError::NotHeld(w));
    }
    let n = count.clamp(0, clip_size(facts, w));
    ps.set_ammo_in_clip(w, n);
    if facts_of(facts, w).dual_wield {
        let partner = facts_of(facts, ps.weapon).dual_wield_weapon;
        if partner != 0 {
            if !ps.has_weapon(partner) {
                return Err(ScriptError::NotHeld(partner));
            }
            ps.set_ammo_in_clip(partner, clip_size(facts, partner));
        }
    }
    Ok(())
}

/// getweaponammoclip(name).
pub fn get_weapon_ammo_clip(ps: &T6WeaponPs, w: u32) -> i32 {
    ps.ammo_in_clip(w)
}

// ---------------------------------------------------------------------------------------------------------------
// Notifies
// ---------------------------------------------------------------------------------------------------------------

/// One notify on the player entity. `weapons` are its arguments in GSC order (0 = "none");
/// `flag` is a bool argument instead (`deploy_riotshield`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Notify {
    pub name: &'static str,
    pub weapons: Vec<u32>,
    pub flag: Option<bool>,
}

impl Notify {
    fn new(name: &'static str, weapons: &[u32]) -> Self {
        Self {
            name,
            weapons: weapons.to_vec(),
            flag: None,
        }
    }

    fn with_flag(name: &'static str, flag: bool) -> Self {
        Self {
            name,
            weapons: Vec::new(),
            flag: Some(flag),
        }
    }

    pub fn describe(&self, facts: &FactsTable) -> String {
        if let Some(b) = self.flag {
            return format!("{}({})", self.name, u8::from(b));
        }
        if self.weapons.is_empty() {
            return self.name.to_owned();
        }
        let args: Vec<String> = self
            .weapons
            .iter()
            .map(|&w| {
                if w == 0 {
                    "none".to_owned()
                } else {
                    facts.name(w).to_owned()
                }
            })
            .collect();
        format!("{}({})", self.name, args.join(", "))
    }
}

/// What a usercmd's client events tell the script, in event order. `grenade_fire`,
/// `grenade_launcher_fire` and `missile_fire` carry an entity and come from the game side.
pub fn event_notifies(ps: &T6WeaponPs, facts: &FactsTable, evs: &[Event], out: &mut Vec<Notify>) {
    for e in evs {
        match e.id {
            events::EV_RELOAD_START_NOTIFY => out.push(Notify::new("reload_start", &[])),
            events::EV_RELOAD_ADDAMMO => out.push(Notify::new("reload", &[])),
            // A held detonator went off (C4, the Time Bomb's remote): no arguments.
            events::EV_DETONATE => out.push(Notify::new("detonate", &[])),
            events::EV_WEAPON_SWITCH_STARTED => {
                out.push(Notify::new("weapon_switch_started", &[e.parm]))
            }
            events::EV_PULLBACK_WEAPON | events::EV_PREP_OFFHAND => {
                let w = if e.parm != 0 {
                    e.parm
                } else {
                    ps.off_hand_weapon
                };
                if w != 0 {
                    out.push(Notify::new("grenade_pullback", &[w]));
                }
            }
            events::EV_FIRE_WEAPON
            | events::EV_FIRE_WEAPON_LASTSHOT
            | events::EV_FIRE_WEAPON_LEFT
            | events::EV_FIRE_WEAPON_LASTSHOT_LEFT
            | events::EV_FIRE_WEAPON_MOUNTED_TURRET
            | events::EV_FIRE_BARREL_1
            | events::EV_FIRE_BARREL_2 => {
                // FireWeapon names the charged level's weapon.
                let w = crate::pm::fire::weapon_for_bullets(facts, e.parm, ps.weapon);
                out.push(Notify::new("weapon_fired", &[w]))
            }
            events::EV_FIRE_MELEE => {
                let c = facts_of(facts, ps.weapon);
                let w = if c.weap_type == weap_type::MELEE || c.inventory_type == inventory::MELEE {
                    ps.weapon
                } else {
                    ps.melee_weapon
                };
                out.push(Notify::new("weapon_melee", &[w]));
            }
            // ClientEvents: the shield coming up, planted (1), or lowered while still in hand (0).
            events::EV_PREP_RIOTSHIELD => out.push(Notify::new("raise_riotshield", &[])),
            events::EV_DEPLOY_RIOTSHIELD => out.push(Notify::with_flag("deploy_riotshield", true)),
            events::EV_LOWER_RIOTSHIELD => {
                if facts_of(facts, ps.weapon).weap_type == weap_type::RIOTSHIELD {
                    out.push(Notify::with_flag("deploy_riotshield", false));
                }
            }
            _ => {}
        }
    }
}

/// The per-client memory BO2 keeps for the per-frame notifies.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct NotifyWatch {
    pub last_weapon: u32,
    pub previously_began_raise: bool,
    pub previously_firing: bool,
}

impl NotifyWatch {
    /// Once per server frame, after the usercmds: weapon_change, weapon_raising,
    /// weapon_change_complete, begin_firing / end_firing.
    pub fn frame(&mut self, ps: &T6WeaponPs, pm_type: i32, out: &mut Vec<Notify>) {
        if ps.weapon != self.last_weapon {
            out.push(Notify::new("weapon_change", &[ps.weapon, self.last_weapon]));
            self.last_weapon = ps.weapon;
        }
        let raising = state::is_raising(ps.weaponstate);
        if raising {
            if !self.previously_began_raise {
                out.push(Notify::new("weapon_raising", &[ps.weapon]));
            }
            self.previously_began_raise = true;
        } else if self.previously_began_raise {
            out.push(Notify::new("weapon_change_complete", &[ps.weapon]));
            self.previously_began_raise = false;
        }
        let firing = matches!(ps.weaponstate, state::FIRING | state::CONT_FIRE_LOOP)
            && pm_type < crate::env::pm_type::DEAD;
        if firing != self.previously_firing {
            out.push(Notify::new(
                if firing { "begin_firing" } else { "end_firing" },
                &[],
            ));
            self.previously_firing = firing;
        }
    }
}

// ---------------------------------------------------------------------------------------------------------------
// Script trace: the frame trace with GSC calls and the client round trip
// ---------------------------------------------------------------------------------------------------------------

/// Runs a script of GSC calls against the gun brain, one server frame per step, in BO2's order:
/// the GSC calls written before the step, the per-frame notifies, then the client's usercmd.
///
/// Commands, one per line (`#` comment): `msec N`, `rtt N` (ticks before the client's usercmd
/// carries a server weapon-select, default 1), `spawn NAME` (held, in hand, ready, raised before),
/// `give NAME` / `take NAME` / `takeall` / `switch NAME` / `switchnow NAME` / `spawnweapon NAME` /
/// `initialraise NAME` / `everhadall 0|1` / `setclip NAME N` (the builtins), `select NAME` (the
/// player picks a gun), `hold B` / `release B` (attack ads reload melee), `run N`, `wait SECONDS`
/// (runs whole ticks), `disable` / `enable` (disableweapons / enableweapons),
/// `on NOTIFY COMMAND ...` (a menu thread: runs COMMAND inside that notify, before the client's
/// next usercmd, as a woken `waittill` does; `$new` is the notify's first weapon),
/// `expect state|weapon|anim|everhad|clip|notify|is ...` (`is` takes a query name:
/// switching reloading throwing firing meleeing pulledback offhand, then 0 or 1).
pub struct ScriptTrace<'f> {
    pub ps: T6WeaponPs,
    pub env: crate::env::T6Env,
    pub cmd: crate::cmd::T6Cmd,
    pub facts: &'f FactsTable,
    pub watch: NotifyWatch,
    pub now: i32,
    /// Ticks a weapon-select takes to reach the client's usercmd.
    pub rtt: i32,
    pending: Vec<(i32, u32)>,
    /// Notifies of the last tick (for `expect notify`).
    last_notifies: Vec<Notify>,
    /// GSC calls made since the last step, printed before its line.
    script_lines: Vec<String>,
    /// `on` handlers: notify name and the command it runs.
    handlers: Vec<(String, Vec<String>)>,
    pub lines: Vec<String>,
    pub failures: usize,
}

impl<'f> ScriptTrace<'f> {
    pub fn new(facts: &'f FactsTable) -> Self {
        Self {
            ps: T6WeaponPs::default(),
            env: crate::env::T6Env {
                is_server: true,
                view_height_settled: true,
                ..Default::default()
            },
            cmd: crate::cmd::T6Cmd {
                msec: 50,
                ..Default::default()
            },
            facts,
            watch: NotifyWatch::default(),
            now: 0,
            rtt: 1,
            pending: Vec::new(),
            last_notifies: Vec::new(),
            script_lines: Vec::new(),
            handlers: Vec::new(),
            lines: Vec::new(),
            failures: 0,
        }
    }

    fn wname(&self, w: u32) -> &str {
        if w == 0 { "none" } else { self.facts.name(w) }
    }

    /// Runs the `on` handlers for these notifies (a woken script thread).
    fn wake(&mut self, notifies: &[Notify]) {
        let handlers = self.handlers.clone();
        for x in notifies {
            for (name, cmd) in handlers.iter().filter(|h| h.0 == x.name) {
                let new = self
                    .wname(x.weapons.first().copied().unwrap_or(0))
                    .to_owned();
                let words: Vec<String> = cmd
                    .iter()
                    .map(|c| if c == "$new" { new.clone() } else { c.clone() })
                    .collect();
                let refs: Vec<&str> = words.iter().map(String::as_str).collect();
                if let Err(e) = self.command(0, &refs) {
                    self.failures += 1;
                    self.script_lines.push(format!("FAIL on {name}: {e}"));
                }
            }
        }
    }

    fn id(&self, name: &str) -> Result<u32, String> {
        if name.eq_ignore_ascii_case("none") {
            return Ok(0);
        }
        self.facts
            .find(name)
            .ok_or_else(|| format!("no weapon named {name}"))
    }

    fn apply(&mut self, call: &str, fx: Effects) {
        if let Some(w) = fx.select {
            self.pending.push((self.rtt, w));
        }
        if fx.clear_pm_flags != 0 {
            self.env.pm_flags &= !fx.clear_pm_flags;
        }
        let mut note = format!("{:>6} gsc {call}", self.now);
        if fx.zero_server_cmd_weapon {
            note.push_str(" [server cmd weapon 0]");
        }
        self.script_lines.push(note);
    }

    /// One server frame (its scripts already ran), then one usercmd from the client.
    pub fn step(&mut self) {
        let mut n = Vec::new();
        self.watch.frame(&self.ps, self.env.pm_type, &mut n);
        self.wake(&n);
        let frame_notifies = n.len();
        self.now += self.cmd.msec;
        self.cmd.server_time = self.now;
        // The weapon-select message reaches the client; its usercmd carries the gun from then on.
        for p in &mut self.pending {
            p.0 -= 1;
        }
        while let Some(i) = self.pending.iter().position(|p| p.0 <= 0) {
            self.cmd.weapon = self.pending.remove(i).1;
        }
        let out = crate::tick::run(&mut self.ps, &self.cmd, &mut self.env, self.facts);
        event_notifies(&self.ps, self.facts, &out.events, &mut n);
        let from_events = n[frame_notifies..].to_vec();
        self.wake(&from_events);
        let ps = &self.ps;
        let ever = ps.ever_had_weapon(ps.weapon);
        let mut line = format!(
            "{:>6} {:<12} {:<18} t{:>5} {:<14} ever {} clip {:>3}",
            self.now,
            self.wname(ps.weapon),
            state::name(ps.weaponstate),
            ps.weapon_time,
            crate::anim::name(ps.weap_anim),
            u8::from(ever),
            ps.ammo_in_clip(ps.weapon),
        );
        for e in &out.events {
            line.push_str(&format!(" | ev {}", events::name(e.id)));
        }
        for x in &n {
            line.push_str(&format!(" | notify {}", x.describe(self.facts)));
        }
        self.lines.append(&mut self.script_lines);
        self.lines.push(line);
        self.last_notifies = n;
    }

    fn expect(&mut self, n: usize, words: &[&str]) -> Result<(), String> {
        let what = *words.get(1).ok_or("expect what?")?;
        let rest = words[2..].join(" ");
        let got = match what {
            "state" => state::name(self.ps.weaponstate).to_owned(),
            "weapon" => self.wname(self.ps.weapon).to_owned(),
            "anim" => crate::anim::name(self.ps.weap_anim).to_owned(),
            "everhad" => {
                let w = self.id(words.get(2).ok_or("expect everhad NAME 0|1")?)?;
                let got = u8::from(self.ps.ever_had_weapon(w)).to_string();
                return self.check(
                    n,
                    &format!("everhad {}", words[2]),
                    words.get(3).copied().unwrap_or(""),
                    &got,
                );
            }
            "clip" => {
                let w = self.id(words.get(2).ok_or("expect clip NAME N")?)?;
                let got = self.ps.ammo_in_clip(w).to_string();
                return self.check(
                    n,
                    &format!("clip {}", words[2]),
                    words.get(3).copied().unwrap_or(""),
                    &got,
                );
            }
            "is" => {
                let q = *words.get(2).ok_or("expect is QUERY 0|1")?;
                let ps = &self.ps;
                let v = match q {
                    "switching" => is_switching_weapons(ps),
                    "reloading" => is_reloading(ps),
                    "throwing" => is_throwing_grenade(ps),
                    "firing" => is_firing(ps),
                    "meleeing" => is_meleeing(ps),
                    "pulledback" => has_a_grenade_pulled_back(ps),
                    "offhand" => is_using_offhand(ps),
                    _ => return Err(format!("line {n}: query {q}?")),
                };
                let got = u8::from(v).to_string();
                return self.check(
                    n,
                    &format!("is {q}"),
                    words.get(3).copied().unwrap_or(""),
                    &got,
                );
            }
            "notify" => {
                let have: Vec<String> = self
                    .last_notifies
                    .iter()
                    .map(|x| x.describe(self.facts))
                    .collect();
                if have.contains(&rest) {
                    return Ok(());
                }
                return self.check(n, "notify", &rest, &have.join("; "));
            }
            _ => return Err(format!("line {n}: expect what? {what}")),
        };
        self.check(n, what, &rest, &got)
    }

    fn check(&mut self, n: usize, what: &str, want: &str, got: &str) -> Result<(), String> {
        if !want.eq_ignore_ascii_case(got) {
            self.failures += 1;
            self.lines
                .push(format!("FAIL line {n}: {what} want {want}, got {got}"));
        }
        Ok(())
    }

    fn command(&mut self, n: usize, w: &[&str]) -> Result<(), String> {
        let arg = |i: usize| {
            w.get(i)
                .copied()
                .ok_or(format!("line {n}: missing argument"))
        };
        let num = |i: usize| -> Result<f64, String> {
            arg(i)?
                .parse::<f64>()
                .map_err(|_| format!("line {n}: not a number"))
        };
        let script_err = |e: ScriptError| format!("line {n}: script error {e:?}");
        let no_ammo = |_: u32| 0;
        match w[0] {
            "msec" => self.cmd.msec = num(1)? as i32,
            "rtt" => self.rtt = num(1)? as i32,
            "spawn" => {
                let g = self.id(arg(1)?)?;
                give_weapon(&mut self.ps, self.facts, g, 0, &no_ammo).map_err(script_err)?;
                self.ps.weapon = g;
                self.ps.weaponstate = state::READY;
                self.ps.set_ever_had(g, true);
                self.cmd.weapon = g;
                self.watch.last_weapon = g;
            }
            "give" => {
                let g = self.id(arg(1)?)?;
                let fx =
                    give_weapon(&mut self.ps, self.facts, g, 0, &no_ammo).map_err(script_err)?;
                self.apply(&format!("giveweapon {}", arg(1)?), fx);
            }
            "take" => {
                let g = self.id(arg(1)?)?;
                let fx = take_weapon(&mut self.ps, self.facts, g);
                self.apply(&format!("takeweapon {}", arg(1)?), fx);
            }
            "takeall" => {
                let fx = take_all_weapons(&mut self.ps, self.facts);
                self.apply("takeallweapons", fx);
            }
            "switch" | "switchnow" => {
                let g = self.id(arg(1)?)?;
                let (ok, fx) = switch_to_weapon(&mut self.ps, g, w[0] == "switchnow", false);
                self.apply(&format!("{} {} -> {ok}", w[0], arg(1)?), fx);
            }
            "spawnweapon" => {
                let g = self.id(arg(1)?)?;
                let fx =
                    set_spawn_weapon(&mut self.ps, self.facts, g, false).map_err(script_err)?;
                self.apply(&format!("setspawnweapon {}", arg(1)?), fx);
            }
            "initialraise" => {
                let g = self.id(arg(1)?)?;
                let (ok, fx) = initial_weapon_raise(&mut self.ps, g, false);
                self.apply(&format!("initialweaponraise {} -> {ok}", arg(1)?), fx);
            }
            "everhadall" => {
                set_ever_had_weapon_all(&mut self.ps, num(1)? != 0.0);
                self.apply(
                    &format!("seteverhadweaponall {}", arg(1)?),
                    Effects::default(),
                );
            }
            "setclip" => {
                let g = self.id(arg(1)?)?;
                set_weapon_ammo_clip(&mut self.ps, self.facts, g, num(2)? as i32)
                    .map_err(script_err)?;
                self.apply(
                    &format!("setweaponammoclip {} {}", arg(1)?, arg(2)?),
                    Effects::default(),
                );
            }
            "disable" => {
                let fx = disable_weapons(&mut self.ps, self.env.pm_flags);
                self.apply("disableweapons", fx);
            }
            "enable" => {
                enable_weapons(&mut self.ps);
                self.apply("enableweapons", Effects::default());
            }
            "on" => {
                let name = arg(1)?.to_owned();
                if w.len() < 3 {
                    return Err(format!("line {n}: on NOTIFY COMMAND"));
                }
                let cmd = w[2..].iter().map(|x| (*x).to_owned()).collect();
                self.handlers.push((name, cmd));
            }
            "select" => self.cmd.weapon = self.id(arg(1)?)?,
            "hold" | "release" => {
                let bit = match arg(1)? {
                    "attack" => crate::cmd::button::ATTACK,
                    "ads" => crate::cmd::button::ADS,
                    "reload" => crate::cmd::button::RELOAD,
                    "melee" => crate::cmd::button::MELEE,
                    b => return Err(format!("line {n}: button {b}?")),
                };
                if w[0] == "hold" {
                    self.cmd.buttons |= bit;
                } else {
                    self.cmd.buttons &= !bit;
                }
            }
            "run" => {
                for _ in 0..num(1)? as i32 {
                    self.step();
                }
            }
            "wait" => {
                // GSC wait: whole server frames.
                let ticks = ((num(1)? * 1000.0) / f64::from(self.cmd.msec.max(1))).ceil() as i32;
                for _ in 0..ticks.max(1) {
                    self.step();
                }
            }
            "expect" => self.expect(n, w)?,
            "facts" | "link" => {} // read by load_facts
            c => return Err(format!("line {n}: unknown command {c}")),
        }
        Ok(())
    }

    /// The facts a trace file asks for: `facts PATH` (a facts dump; line N = weapon N) and
    /// `link dual|alt NAME OTHER` (a dump keeps BO2's own weapon index for the partner, so the
    /// trace names it). None when the file names no dump.
    pub fn load_facts(script: &str) -> Result<Option<FactsTable>, String> {
        let mut table = None;
        for raw in script.lines() {
            let w: Vec<&str> = raw
                .split('#')
                .next()
                .unwrap_or("")
                .split_whitespace()
                .collect();
            match w.as_slice() {
                ["facts", path] => {
                    let text = std::fs::read_to_string(path).map_err(|e| format!("{path}: {e}"))?;
                    table = Some(FactsTable::from_dump(&text));
                }
                ["link", kind, a, b] => {
                    let t = table.as_mut().ok_or("link before facts")?;
                    let ia = t.find(a).ok_or(format!("no weapon {a}"))?;
                    let ib = t.find(b).ok_or(format!("no weapon {b}"))?;
                    let mut f = t.get(ia);
                    match *kind {
                        "dual" => f.core.dual_wield_weapon = ib,
                        "alt" => f.core.alt_weapon = ib,
                        k => return Err(format!("link {k}?")),
                    }
                    let name = t.name(ia).to_owned();
                    t.set(ia, &name, f);
                }
                _ => {}
            }
        }
        Ok(table)
    }

    pub fn run_script(&mut self, script: &str) -> Result<(), String> {
        for (i, raw) in script.lines().enumerate() {
            let line = raw.split('#').next().unwrap_or("").trim();
            if line.is_empty() {
                continue;
            }
            let words: Vec<&str> = line.split_whitespace().collect();
            self.command(i + 1, &words)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::trace::demo_facts;

    fn run(script: &str) -> ScriptTrace<'static> {
        let facts: &'static FactsTable = Box::leak(Box::new(demo_facts()));
        let mut t = ScriptTrace::new(facts);
        t.run_script(script).unwrap();
        for l in &t.lines {
            println!("{l}");
        }
        t
    }

    /// Runs docs/gun/traces/*.trace against the BO2 facts dump each names (skipped when the dump
    /// is not on this PC). `GUN_SCRIPT_TRACE_OUT=1` writes each trace to NAME.out beside it.
    #[test]
    fn bind_trace_files() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/gun/traces");
        let Ok(entries) = std::fs::read_dir(&dir) else {
            return;
        };
        let mut ran = 0;
        for e in entries.flatten() {
            let path = e.path();
            if path.extension().is_none_or(|x| x != "trace") {
                continue;
            }
            let script = std::fs::read_to_string(&path).unwrap();
            let facts = match ScriptTrace::load_facts(&script) {
                Ok(Some(f)) => f,
                // No facts line: a gun_trace file (window 3's fire traces), not a bind trace.
                Ok(None) => continue,
                Err(e) => {
                    println!("skip {}: {e}", path.display());
                    continue;
                }
            };
            let mut t = ScriptTrace::new(&facts);
            t.run_script(&script)
                .unwrap_or_else(|e| panic!("{}: {e}", path.display()));
            if std::env::var("GUN_SCRIPT_TRACE_OUT").is_ok() {
                std::fs::write(path.with_extension("out"), t.lines.join("\n") + "\n").unwrap();
            }
            assert_eq!(t.failures, 0, "{}:\n{}", path.display(), t.lines.join("\n"));
            ran += 1;
        }
        println!("{ran} trace files");
    }

    #[test]
    fn give_held_gun_does_nothing_and_new_gun_first_raises() {
        let t = run("
            spawn demo_smg
            setclip demo_smg 3
            give demo_smg          # already held: no ammo, no flag change
            expect clip demo_smg 3
            expect everhad demo_smg 1
            give demo_pistol
            expect everhad demo_pistol 0
            expect clip demo_pistol 30
        ");
        assert_eq!(t.failures, 0);
    }

    #[test]
    fn canswap_take_and_give_first_raises_with_no_drop() {
        let t = run("
            spawn demo_smg
            run 2
            take demo_smg
            give demo_smg
            run 1   # this frame sees none; the client still asks for the gun: raise from none
            expect notify weapon_change(none, demo_smg)
            expect weapon demo_smg
            expect state RAISING
            expect anim FIRST_RAISE
            run 1
            expect notify weapon_change(demo_smg, none)
            expect notify weapon_raising(demo_smg)
            run 20
            expect state READY
            expect notify weapon_change_complete(demo_smg)
        ");
        assert_eq!(t.failures, 0);
        assert!(
            !t.lines.iter().any(|l| l.contains("DROPPING ")),
            "no drop on canswap"
        );
    }

    #[test]
    fn canzoom_spawnweapon_cuts_the_first_raise_to_ready() {
        let t = run("
            spawn demo_smg
            run 2
            take demo_smg
            give demo_smg
            run 2
            expect anim FIRST_RAISE
            wait 0.08
            spawnweapon demo_smg
            expect state READY
            expect everhad demo_smg 1
            run 1
            expect state READY
        ");
        assert_eq!(t.failures, 0);
    }

    #[test]
    fn switch_immediate_raises_from_none_with_no_drop() {
        let t = run("
            spawn demo_smg
            give demo_pistol
            everhadall 1
            run 1
            switchnow demo_pistol
            run 1
            expect notify weapon_change(none, demo_smg)
            expect weapon demo_pistol
            expect state RAISING
            expect anim RAISE
            run 1
            expect notify weapon_change(demo_pistol, none)
        ");
        assert_eq!(t.failures, 0);
        assert!(!t.lines.iter().any(|l| l.contains("DROPPING ")));
    }

    #[test]
    fn always_canswap_every_switch_first_raises() {
        let t = run("
            spawn demo_smg
            give demo_pistol
            everhadall 1
            everhadall 0
            switch demo_pistol
            run 12
            expect weapon demo_pistol
            expect anim FIRST_RAISE
        ");
        assert_eq!(t.failures, 0);
    }

    #[test]
    fn illusion_empty_one_frame_then_spawnweapon_back_to_ready() {
        let t = run("
            spawn demo_smg
            run 2
            setclip demo_smg 0
            run 1
            setclip demo_smg 30
            spawnweapon demo_smg
            expect state READY
            run 1
            expect state READY
            expect clip demo_smg 30
            expect is reloading 0
        ");
        assert_eq!(t.failures, 0);
    }

    #[test]
    fn instashoot_spawnweapon_on_weapon_change_skips_the_raise() {
        let t = run("
            spawn demo_smg
            give demo_pistol
            everhadall 1
            on weapon_change spawnweapon $new   # the menu's thread
            select demo_pistol
            run 9   # the smg drops (400 ms); the pistol starts to raise
            expect state RAISING
            run 1   # weapon_change wakes the thread before the next usercmd
            expect notify weapon_change(demo_pistol, demo_smg)
            expect state READY
            expect is switching 0
            run 1
            expect state READY
        ");
        assert_eq!(t.failures, 0);
        // Holding fire on the next usercmd fires once window 3's fire hook is merged.
    }

    #[test]
    fn nac_take_one_frame_give_back_first_raises() {
        let t = run("
            spawn demo_smg
            run 2
            take demo_smg
            run 1
            expect notify weapon_change(none, demo_smg)
            give demo_smg
            setclip demo_smg 12
            run 1
            expect weapon demo_smg
            expect anim FIRST_RAISE
            expect clip demo_smg 12
        ");
        assert_eq!(t.failures, 0);
    }

    #[test]
    fn disableweapons_then_switch_goes_to_none() {
        let t = run("
            spawn demo_smg
            give demo_pistol
            run 1
            disable
            select demo_pistol
            run 12
            expect weapon none
            enable
            run 1
            expect weapon demo_pistol
        ");
        assert_eq!(t.failures, 0);
    }

    #[test]
    fn state_queries_use_bo2_numbers() {
        let mut ps = T6WeaponPs::default();
        for (s, sw, rl, gr) in [
            (state::DROPPING, true, false, false),
            (state::RELOADING_RIGHT, false, false, false),
            (state::RELOAD_QUICK_EMPTY, false, true, false),
            (state::OFFHAND_END, false, false, true),
            (state::RIOTSHIELD_PREPARE, false, false, false),
            (state::RELOADING_LEFT, false, false, false),
        ] {
            ps.weaponstate = s;
            assert_eq!(
                (
                    is_switching_weapons(&ps),
                    is_reloading(&ps),
                    is_throwing_grenade(&ps)
                ),
                (sw, rl, gr),
                "state {s}"
            );
        }
        for (s, firing, melee, pulled) in [
            (state::FIRING, true, false, false),
            (state::RECHAMBER_START, true, false, false),
            (state::CONT_FIRE_LOOP, true, false, false),
            (state::MELEE_FIRE, true, true, false),
            (state::OFFHAND_HOLD, false, false, true),
            (state::OFFHAND_DETONATE, false, false, false),
            (state::OFFHAND, false, false, true),
            (state::OFFHAND_HOLDUSE, false, false, false),
        ] {
            ps.weaponstate = s;
            assert_eq!(
                (
                    is_firing(&ps),
                    is_meleeing(&ps),
                    has_a_grenade_pulled_back(&ps)
                ),
                (firing, melee, pulled),
                "state {s}"
            );
        }
        ps.weap_flags = wf::WEAPONS_DISABLED;
        assert!(is_using_offhand(&ps));
    }

    #[test]
    fn spawnweapon_and_clip_on_a_gun_not_held_are_script_errors() {
        let facts = demo_facts();
        let mut ps = T6WeaponPs::default();
        assert_eq!(
            set_spawn_weapon(&mut ps, &facts, 1, false),
            Err(ScriptError::NotHeld(1))
        );
        assert_eq!(
            set_weapon_ammo_clip(&mut ps, &facts, 1, 5),
            Err(ScriptError::NotHeld(1))
        );
        let (ok, fx) = switch_to_weapon(&mut ps, 1, false, false);
        assert!(!ok && fx.select.is_none());
    }

    /// setweaponammoclip / getweaponammoclip on both pistols. Setting the right never touches its
    /// count from the left; the left keeps a set count only while another gun is in hand, and
    /// both counts survive the swap back.
    #[test]
    fn left_pistol_clip_sticks_only_with_another_gun_in_hand() {
        let facts: &'static FactsTable = Box::leak(Box::new(crate::pm::dual::demo_facts()));
        let mut t = ScriptTrace::new(facts);
        t.run_script("
            spawn fiveseven_dw_mp
            give fiveseven_lh_mp
            give demo_smg
            run 1
            setclip fiveseven_dw_mp 5
            expect clip fiveseven_dw_mp 5
            expect clip fiveseven_lh_mp 20
            setclip fiveseven_lh_mp 3      # pistols up: written, then refilled
            expect clip fiveseven_lh_mp 20
            expect clip fiveseven_dw_mp 5
            select demo_smg
            run 120
            expect weapon demo_smg
            setclip fiveseven_lh_mp 3      # SMG up: the left keeps 3
            expect clip fiveseven_lh_mp 3
            expect clip fiveseven_dw_mp 5
            setclip fiveseven_dw_mp 7      # the SMG has no left partner: the left stays 3
            expect clip fiveseven_dw_mp 7
            expect clip fiveseven_lh_mp 3
            select fiveseven_dw_mp
            run 120
            expect weapon fiveseven_dw_mp
            expect clip fiveseven_dw_mp 7
            expect clip fiveseven_lh_mp 3
        ")
        .unwrap();
        for l in &t.lines {
            println!("{l}");
        }
        assert_eq!(t.failures, 0);
        assert_eq!(get_weapon_ammo_clip(&t.ps, 2), 3);
        assert_eq!(get_weapon_ammo_clip(&t.ps, 1), 7);
    }

    #[test]
    fn setclip_clamps() {
        let t = run("
            spawn demo_smg
            setclip demo_smg 99
            expect clip demo_smg 30
            setclip demo_smg -4
            expect clip demo_smg 0
        ");
        assert_eq!(t.failures, 0);
    }
}
