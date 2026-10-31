//! bo2mp gun brain, window 13 (gun/script): the gun builtins and notifies on BO2's gun state.
//! Bound after natives_player / natives_mp / bots, so these replace the
//! MW2-state versions. A builtin first does what the one it replaces did to the game's own state
//! (held list, ammo rows, HUD, the select sent to the client), then applies BO2's rule to the T6
//! player state (`weapon_t6::script`). Mod-menu trickshot binds (canswap, canzoom, illusion,
//! always canswap ...) are nothing special here: they fall out of these rules.

use bevy_ecs::prelude::{Resource, World};
use gsc_t6::{Native, ScriptValue, Vm};
use std::collections::HashMap;
use std::sync::OnceLock;
use weapon_t6::facts::FactsTable;
use weapon_t6::player::T6WeaponPs;
use weapon_t6::script::{self, Effects, NotifyWatch, ScriptError};

use super::{Zm, client, frame, int, text, weapon, weapon_text, with_vm};
use crate::client_script;
use crate::gun_brain;
use crate::sim_parts::ClientId;

/// The builtins bound before this file's (each one here calls the one it replaces first).
static PREV: OnceLock<HashMap<&'static str, Native<World>>> = OnceLock::new();

const WRAPPED: [&str; 22] = [
    "giveweapon",
    "takeweapon",
    "takeallweapons",
    "switchtoweapon",
    "switchtoweaponimmediate",
    "setspawnweapon",
    "initialweaponraise",
    "seteverhadweaponall",
    "setweaponammoclip",
    "getweaponammoclip",
    "disableweapons",
    "enableweapons",
    "disableweaponcycling",
    "enableweaponcycling",
    "isswitchingweapons",
    "isreloading",
    "isthrowinggrenade",
    "isfiring",
    "ismeleeing",
    "isusingoffhand",
    "hasagrenadepulledback",
    "hasperk",
];

fn prev(vm: &mut Vm<World>, game: &mut World, st: &ScriptValue, lhs: &[ScriptValue], label: &str) -> Result<ScriptValue, String> {
    match PREV.get().and_then(|m| m.get(label)) {
        Some(f) => f(vm, game, st, lhs),
        None => Ok(ScriptValue::Undefined),
    }
}

fn is_bot(game: &World, cid: ClientId) -> bool {
    game.resource::<Zm>().bots.contains_key(&cid.0)
}

fn bo2_error(world: &mut World, e: ScriptError) -> String {
    match e {
        ScriptError::NotHeld(w) => format!("player does not have weapon {}", weapon_text(world, w)),
        ScriptError::NotPlayerWeapon(w) => {
            format!("weapon {} is not a player weapon", weapon_text(world, w))
        }
        ScriptError::NoFreeSlot(w) => format!("no free weapon slot for {}", weapon_text(world, w)),
    }
}

/// The weapons in the T6 held slots.
fn held_weapons(ps: &T6WeaponPs) -> Vec<u32> {
    ps.held.iter().map(|h| h.weapon).filter(|&w| w != 0).collect()
}

/// A builtin's rule on the T6 state.
fn edit<R>(
    game: &mut World, cid: ClientId,
    f: impl FnOnce(&mut T6WeaponPs, &FactsTable, &mut u32) -> R,
) -> Option<R> {
    gun_brain::script_edit(&mut frame(game), cid, f)
}

/// What a builtin asks of the game outside the gun state. The weapon select (server -> client)
/// goes the way the game already sends one. The server keeps no copy of the client's last command
/// to zero, and the dive end and deploy flags belong to movement and the riot shield: those wait.
fn apply(game: &mut World, cid: ClientId, fx: &Effects) {
    if let Some(w) = fx.select.or(fx.bot_switch) {
        client_script::switch_to_weapon(&mut frame(game), cid, w);
    }
}

/// A state query: BO2's answer from the T6 state, the old answer before his first usercmd.
fn query(
    vm: &mut Vm<World>, game: &mut World,
    s: &ScriptValue,
    a: &[ScriptValue],
    name: &str,
    f: fn(&T6WeaponPs) -> bool,
) -> Result<ScriptValue, String> {
    let id = client(vm, game, s)?;
    match gun_brain::script_read(&frame(game), id, |ps, _| f(ps)) {
        Some(b) => Ok(ScriptValue::bool(b)),
        None => prev(vm, game, s, a, name),
    }
}

pub(super) fn bind(vm: &mut Vm<World>) {
    let mut old = HashMap::new();
    for name in WRAPPED {
        if let Some(f) = vm.bound(name, true) {
            old.insert(name, f);
        }
    }
    let _ = PREV.set(old);

    macro_rules! m {
        ($name:literal, $body:expr) => {
            vm.bind($name, true, $body);
        };
    }

    // giveweapon: a gun already held is left as it is (BO2 gives nothing, refills nothing).
    // The game's give lists the gun, so the T6 state takes it in (and what comes with it) on
    // the next sync; BO2's give then runs on the state as it was before the game's give, so
    // its magazines, model and offhand pick stand (the start reserves are the game's).
    m!("giveweapon", |vm, world, s, a| {
        let id = client(vm, world, s)?;
        let name = text(vm, a, 0);
        let w = weapon(world, &name)?;
        let held = gun_brain::script_read(&frame(world), id, |pstate, _| pstate.has_weapon(w));
        if w == 0 || held == Some(true) {
            return Ok(ScriptValue::Undefined);
        }
        let model = int(a, 1).unwrap_or(0).clamp(0, 255) as u8;
        // BO2 refuses a turret or a non-player weapon before anything is given.
        let f = frame(world);
        let check = gun_brain::synced_copy(&f, id)
            .map(|mut c| script::give_weapon(&mut c, f.gun_facts(), w, model, &|_| 0));
        drop(f);
        if let Some(Err(e)) = check {
            return Err(bo2_error(world, e));
        }
        let before = edit(world, id, |pstate, _, _| held_weapons(pstate)).unwrap_or_default();
        prev(vm, world, s, a, "giveweapon")?;
        let gave = edit(world, id, |pstate, facts, _| {
            let new: Vec<u32> = held_weapons(pstate).into_iter().filter(|g| !before.contains(g)).collect();
            let stock: Vec<(u32, i32)> = new.iter().map(|&g| (g, pstate.ammo_not_in_clip(facts, g))).collect();
            for &g in &new {
                pstate.take(g);
            }
            let start = |g: u32| stock.iter().find(|(h, _)| *h == g).map_or(0, |(_, n)| *n);
            script::give_weapon(pstate, facts, w, model, &start)
        });
        match gave {
            Some(Err(e)) => Err(bo2_error(world, e)),
            _ => Ok(ScriptValue::Undefined),
        }
    });
    m!("takeweapon", |vm, world, s, a| {
        let id = client(vm, world, s)?;
        let w = weapon(world, &text(vm, a, 0))?;
        let fx = edit(world, id, |pstate, facts, _| script::take_weapon(pstate, facts, w));
        prev(vm, world, s, a, "takeweapon")?;
        // The game's take may pick another gun for the hand: BO2 leaves none there.
        if let Some(effects) = fx {
            edit(world, id, |_, _, _| ());
            apply(world, id, &effects);
        }
        Ok(ScriptValue::Undefined)
    });
    m!("takeallweapons", |vm, world, s, a| {
        let id = client(vm, world, s)?;
        let fx = edit(world, id, |pstate, facts, _| script::take_all_weapons(pstate, facts));
        prev(vm, world, s, a, "takeallweapons")?;
        if fx.is_some() {
            edit(world, id, |_, _, _| ());
        }
        Ok(ScriptValue::Undefined)
    });
    // hasperk: BG_HasPerk, so the perks the gun in the arms grants count too (FMJ's
    // bullet penetration plays the "deepimpact" chatter in _globallogic_player).
    m!("hasperk", |vm, world, s, a| {
        let had = prev(vm, world, s, a, "hasperk")?;
        if gsc_t6::truthy(&had) {
            return Ok(had);
        }
        let id = client(vm, world, s)?;
        let Some(p) = weapon_t6::env::perk::by_name(&text(vm, a, 0)) else {
            return Ok(had);
        };
        let granted = gun_brain::script_read(&frame(world), id, |pstate, facts| script::weapon_grants_perk(pstate, facts, p));
        Ok(if granted == Some(true) { ScriptValue::bool(true) } else { had })
    });
    // True when the gun is held.
    m!("switchtoweapon", |vm, world, s, a| switch(vm, world, s, a, false));
    m!("switchtoweaponimmediate", |vm, world, s, a| switch(vm, world, s, a, true));
    m!("setspawnweapon", |vm, world, s, a| {
        let id = client(vm, world, s)?;
        let w = weapon(world, &text(vm, a, 0))?;
        let bot = is_bot(world, id);
        match edit(world, id, |pstate, facts, _| script::set_spawn_weapon(pstate, facts, w, bot)) {
            Some(Ok(fx)) => apply(world, id, &fx),
            Some(Err(e)) => return Err(bo2_error(world, e)),
            None => {
                prev(vm, world, s, a, "setspawnweapon")?;
            }
        }
        Ok(ScriptValue::Undefined)
    });
    m!("initialweaponraise", |vm, world, s, a| {
        let id = client(vm, world, s)?;
        let w = weapon(world, &text(vm, a, 0))?;
        let bot = is_bot(world, id);
        let r = edit(world, id, |pstate, _, movement_state_bits| {
            let (ok, fx) = script::initial_weapon_raise(pstate, w, bot);
            *movement_state_bits &= !fx.clear_pm_flags;
            (ok, fx)
        });
        if let Some(fx) = r.as_ref().map(|done| &done.1) {
            apply(world, id, fx);
        }
        Ok(ScriptValue::bool(r.is_some_and(|(ok, _)| ok)))
    });
    // bo2mp: forceoffhandend() (BO2's `PlayerCmd_forceOffhandEnd`): the
    // offhand in hand goes straight to its end (the hacker tool's, when its
    // hack is done or given up).
    m!("forceoffhandend", |vm, world, s, _| {
        let id = client(vm, world, s)?;
        edit(world, id, |pstate, _, _| {
            pstate.weapon_state_bits |= weapon_t6::player::wf::USING_OFFHAND;
            pstate.gun_phase = weapon_t6::state::OFFHAND_END;
            pstate.gun_timer_ms = 0;
            pstate.weapon_ready_wait = 0;
        });
        Ok(ScriptValue::Undefined)
    });
    m!("seteverhadweaponall", |vm, world, s, a| {
        let id = client(vm, world, s)?;
        let v = int(a, 0).unwrap_or(0) != 0;
        edit(world, id, |pstate, _, _| script::set_ever_had_weapon_all(pstate, v));
        Ok(ScriptValue::Undefined)
    });
    m!("setweaponammoclip", |vm, world, s, a| {
        let id = client(vm, world, s)?;
        let w = weapon(world, &text(vm, a, 0))?;
        let n = int(a, 1)?;
        match edit(world, id, |pstate, facts, _| script::set_weapon_ammo_clip(pstate, facts, w, n)) {
            Some(Err(e)) => Err(bo2_error(world, e)),
            Some(Ok(())) => Ok(ScriptValue::Undefined),
            None => prev(vm, world, s, a, "setweaponammoclip"),
        }
    });
    m!("getweaponammoclip", |vm, world, s, a| {
        let id = client(vm, world, s)?;
        let w = weapon(world, &text(vm, a, 0))?;
        match gun_brain::script_read(&frame(world), id, |pstate, facts| script::get_weapon_ammo_clip(pstate, facts, w)) {
            Some(n) => Ok(ScriptValue::Int(n)),
            None => prev(vm, world, s, a, "getweaponammoclip"),
        }
    });
    m!("disableweapons", |vm, world, s, a| {
        prev(vm, world, s, a, "disableweapons")?;
        let id = client(vm, world, s)?;
        edit(world, id, |pstate, _, movement_state_bits| script::disable_weapons(pstate, *movement_state_bits));
        Ok(ScriptValue::Undefined)
    });
    m!("enableweapons", |vm, world, s, a| {
        prev(vm, world, s, a, "enableweapons")?;
        let id = client(vm, world, s)?;
        edit(world, id, |pstate, _, _| script::enable_weapons(pstate));
        Ok(ScriptValue::Undefined)
    });
    m!("disableweaponcycling", |vm, world, s, a| {
        prev(vm, world, s, a, "disableweaponcycling")?;
        let id = client(vm, world, s)?;
        edit(world, id, |pstate, _, _| script::set_weapon_cycling(pstate, false));
        Ok(ScriptValue::Undefined)
    });
    m!("enableweaponcycling", |vm, world, s, a| {
        prev(vm, world, s, a, "enableweaponcycling")?;
        let id = client(vm, world, s)?;
        edit(world, id, |pstate, _, _| script::set_weapon_cycling(pstate, true));
        Ok(ScriptValue::Undefined)
    });
    m!("isswitchingweapons", |vm, world, s, a| query(
        vm,
        world,
        s,
        a,
        "isswitchingweapons",
        script::is_switching_weapons
    ));
    m!("isreloading", |vm, world, s, a| query(vm, world, s, a, "isreloading", script::is_reloading));
    m!("isthrowinggrenade", |vm, world, s, a| query(
        vm,
        world,
        s,
        a,
        "isthrowinggrenade",
        script::is_throwing_grenade
    ));
    m!("isfiring", |vm, world, s, a| query(vm, world, s, a, "isfiring", script::is_firing));
    m!("ismeleeing", |vm, world, s, a| query(vm, world, s, a, "ismeleeing", script::is_meleeing));
    m!("isusingoffhand", |vm, world, s, a| query(
        vm,
        world,
        s,
        a,
        "isusingoffhand",
        script::is_using_offhand
    ));
    m!("hasagrenadepulledback", |vm, world, s, a| query(
        vm,
        world,
        s,
        a,
        "hasagrenadepulledback",
        script::has_a_grenade_pulled_back
    ));
}

fn switch(
    vm: &mut Vm<World>, game: &mut World,
    s: &ScriptValue,
    a: &[ScriptValue],
    immediate: bool,
) -> Result<ScriptValue, String> {
    let id = client(vm, game, s)?;
    let w = weapon(game, &text(vm, a, 0))?;
    let bot = is_bot(game, id);
    match edit(game, id, |ps, _, _| script::switch_to_weapon(ps, w, immediate, bot)) {
        Some((ok, fx)) => {
            apply(game, id, &fx);
            // BO2MP_SWITCHLOG=1: each scripted switch, who asked for which gun (test aid).
            if std::env::var_os("BO2MP_SWITCHLOG").is_some() {
                diag::info!(Sim, "bo2mp switchtoweapon client {} {} immediate {immediate} ok {ok}", id.0, text(vm, a, 0));
            }
            Ok(ScriptValue::bool(ok))
        }
        None => prev(
            vm,
            game,
            s,
            a,
            if immediate { "switchtoweaponimmediate" } else { "switchtoweapon" },
        ),
    }
}

/// Each player's memory for the per-frame notifies.
#[derive(Resource, Default)]
pub(super) struct Watches(HashMap<u32, NotifyWatch>);

/// bo2mp: BO2's `ClientSpawn` clears this memory with the rest of his
/// client, so a spawned player's first frame tells the scripts
/// `weapon_change` (new, "none") even with the gun he died holding.
pub(super) fn forget_watch(world: &mut World, client: u32) {
    if let Some(mut watches) = world.get_resource_mut::<Watches>() {
        watches.0.remove(&client);
    }
}

/// In place of the MW2-state diff, before the script threads: the client events the usercmds
/// made (reload_start, reload, weapon_switch_started, grenade_pullback, weapon_fired,
/// weapon_melee), as BO2 sends them while it runs the usercmd. "none" stands for no gun.
pub(super) fn frame_notifies(world: &mut World) {
    notifies(world, false);
}

/// After the script threads, as BO2's server frame does: the per-frame notifies
/// (weapon_change(new, old), weapon_raising, weapon_change_complete, begin_firing /
/// end_firing). A take and give in a thread shows here as weapon_change(none, old) the same
/// frame, before the next usercmd raises the gun again (canswap).
pub(super) fn frame_watch(world: &mut World) {
    notifies(world, true);
}

fn notifies(world: &mut World, watch_pass: bool) {
    if !world.contains_resource::<Watches>() {
        world.insert_resource(Watches::default());
    }
    let clients: Vec<(u32, gsc_t6::ObjRef)> =
        world.resource::<Zm>().players.iter().map(|(c, p)| (*c, p.obj)).collect();
    for (c, obj) in clients {
        let id = ClientId(c);
        let evs = if watch_pass { Vec::new() } else { gun_brain::script_take_events(&mut frame(world), id) };
        let taps = if watch_pass { Vec::new() } else { gun_brain::script_take_double_taps(&mut frame(world), id) };
        let movement_mode = frame(world).avatar(id).map_or(0, |ps| ps.movement_mode);
        let mut watch = world.resource::<Watches>().0.get(&c).copied().unwrap_or(NotifyWatch::default());
        let Some(out_list) = gun_brain::script_read(&frame(world), id, |ps, facts| {
            let mut out_list = Vec::new();
            if watch_pass {
                watch.frame(ps, movement_mode, &mut out_list);
            } else {
                // Each usercmd's double tap goes before that command's events.
                let mut consumed = 0;
                for &at in &taps {
                    let at = at.clamp(consumed, evs.len());
                    script::event_notifies(ps, facts, &evs[consumed..at], &mut out_list);
                    out_list.push(script::double_tap_notify());
                    consumed = at;
                }
                script::event_notifies(ps, facts, &evs[consumed..], &mut out_list);
            }
            out_list
        }) else {
            continue;
        };
        world.resource_mut::<Watches>().0.insert(c, watch);
        if out_list.is_empty() {
            continue;
        }
        // Each notify's weapons by name, in the order of `out_list`.
        let gun_names: Vec<Vec<String>> = out_list
            .iter()
            .map(|n| {
                n.weapons
                    .iter()
                    .map(|&w| if w == 0 { "none".to_owned() } else { weapon_text(world, w) })
                    .collect()
            })
            .collect();
        with_vm(world, |vm, world| {
            for (n, guns) in out_list.iter().zip(gun_names) {
                let mut args: Vec<ScriptValue> = guns.iter().map(|a| vm.string(a)).collect();
                args.extend(n.flag.map(ScriptValue::bool));
                vm.notify_str(world, obj, n.name, &args);
            }
        });
    }
}
