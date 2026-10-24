//! bo2mp gun brain, window 13 (gun/script): the gun builtins and notifies on BO2's gun state.
//! Bound after natives_player / natives_mp / bots, so these replace the
//! MW2-state versions. A builtin first does what the one it replaces did to the game's own state
//! (held list, ammo rows, HUD, the select sent to the client), then applies BO2's rule to the T6
//! player state (`weapon_t6::script`). Mod-menu trickshot binds (canswap, canzoom, illusion,
//! always canswap ...) are nothing special here: they fall out of these rules.

use bevy_ecs::prelude::{Resource, World};
use gsc_t6::{Native, Value, Vm};
use std::collections::HashMap;
use std::sync::OnceLock;
use weapon_t6::facts::FactsTable;
use weapon_t6::player::T6WeaponPs;
use weapon_t6::script::{self as bo2, Effects, NotifyWatch, ScriptError};

use super::{Zm, client, frame, int, text, weapon, weapon_text, with_vm};
use crate::script_player;
use crate::t6_gun;
use crate::world::ClientId;

/// The builtins bound before this file's (each one here calls the one it replaces first).
static PREV: OnceLock<HashMap<&'static str, Native<World>>> = OnceLock::new();

const WRAPPED: [&str; 21] = [
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
];

fn prev(vm: &mut Vm<World>, world: &mut World, s: &Value, a: &[Value], name: &str) -> Result<Value, String> {
    match PREV.get().and_then(|m| m.get(name)) {
        Some(f) => f(vm, world, s, a),
        None => Ok(Value::Undefined),
    }
}

/// bo2zm: Zombies has no bots.
fn is_bot(_world: &World, _id: ClientId) -> bool {
    false
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

/// A builtin's rule on the T6 state.
fn edit<R>(
    world: &mut World,
    id: ClientId,
    f: impl FnOnce(&mut T6WeaponPs, &FactsTable, &mut u32) -> R,
) -> Option<R> {
    t6_gun::script_edit(&mut frame(world), id, f)
}

/// What a builtin asks of the game outside the gun state. The weapon select (server -> client)
/// goes the way the game already sends one. The server keeps no copy of the client's last command
/// to zero, and the dive end and deploy flags belong to movement and the riot shield: those wait.
fn apply(world: &mut World, id: ClientId, fx: &Effects) {
    if let Some(w) = fx.select.or(fx.bot_switch) {
        script_player::switch_to_weapon(&mut frame(world), id, w);
    }
}

/// A state query: BO2's answer from the T6 state, the old answer before his first usercmd.
fn query(
    vm: &mut Vm<World>,
    world: &mut World,
    s: &Value,
    a: &[Value],
    name: &str,
    f: fn(&T6WeaponPs) -> bool,
) -> Result<Value, String> {
    let id = client(vm, world, s)?;
    match t6_gun::script_read(&frame(world), id, |ps, _| f(ps)) {
        Some(b) => Ok(Value::bool(b)),
        None => prev(vm, world, s, a, name),
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
    m!("giveweapon", |vm, world, s, a| {
        let id = client(vm, world, s)?;
        let w = weapon(world, &text(vm, a, 0))?;
        let held = t6_gun::script_read(&frame(world), id, |ps, _| ps.has_weapon(w));
        if w == 0 || held == Some(true) {
            return Ok(Value::Undefined);
        }
        prev(vm, world, s, a, "giveweapon")?;
        let model = int(a, 1).unwrap_or(0).clamp(0, 255) as u8;
        let stock: Vec<(u32, i32)> = frame(world)
            .player(id)
            .map_or_else(Vec::new, |ps| {
                ps.weapons.iter().filter(|&&g| g > 0).map(|&g| g as u32).collect::<Vec<_>>()
            })
            .into_iter()
            .map(|g| (g, script_player::ammo_stock(&frame(world), id, g)))
            .collect();
        let start = move |g: u32| stock.iter().find(|(h, _)| *h == g).map_or(0, |(_, n)| *n);
        match edit(world, id, |ps, facts, _| bo2::give_weapon(ps, facts, w, model, &start)) {
            Some(Err(e)) => Err(bo2_error(world, e)),
            _ => Ok(Value::Undefined),
        }
    });
    m!("takeweapon", |vm, world, s, a| {
        let id = client(vm, world, s)?;
        let w = weapon(world, &text(vm, a, 0))?;
        let fx = edit(world, id, |ps, facts, _| bo2::take_weapon(ps, facts, w));
        prev(vm, world, s, a, "takeweapon")?;
        // The game's take may pick another gun for the hand: BO2 leaves none there.
        if let Some(fx) = fx {
            edit(world, id, |_, _, _| ());
            apply(world, id, &fx);
        }
        Ok(Value::Undefined)
    });
    m!("takeallweapons", |vm, world, s, a| {
        let id = client(vm, world, s)?;
        let fx = edit(world, id, |ps, facts, _| bo2::take_all_weapons(ps, facts));
        prev(vm, world, s, a, "takeallweapons")?;
        if fx.is_some() {
            edit(world, id, |_, _, _| ());
        }
        Ok(Value::Undefined)
    });
    // True when the gun is held.
    m!("switchtoweapon", |vm, world, s, a| switch(vm, world, s, a, false));
    m!("switchtoweaponimmediate", |vm, world, s, a| switch(vm, world, s, a, true));
    m!("setspawnweapon", |vm, world, s, a| {
        let id = client(vm, world, s)?;
        let w = weapon(world, &text(vm, a, 0))?;
        let bot = is_bot(world, id);
        match edit(world, id, |ps, facts, _| bo2::set_spawn_weapon(ps, facts, w, bot)) {
            Some(Ok(fx)) => apply(world, id, &fx),
            Some(Err(e)) => return Err(bo2_error(world, e)),
            None => {
                prev(vm, world, s, a, "setspawnweapon")?;
            }
        }
        Ok(Value::Undefined)
    });
    m!("initialweaponraise", |vm, world, s, a| {
        let id = client(vm, world, s)?;
        let w = weapon(world, &text(vm, a, 0))?;
        let bot = is_bot(world, id);
        let r = edit(world, id, |ps, _, pm_flags| {
            let (ok, fx) = bo2::initial_weapon_raise(ps, w, bot);
            *pm_flags &= !fx.clear_pm_flags;
            (ok, fx)
        });
        if let Some((_, fx)) = &r {
            apply(world, id, fx);
        }
        Ok(Value::bool(r.is_some_and(|(ok, _)| ok)))
    });
    m!("seteverhadweaponall", |vm, world, s, a| {
        let id = client(vm, world, s)?;
        let v = int(a, 0).unwrap_or(0) != 0;
        edit(world, id, |ps, _, _| bo2::set_ever_had_weapon_all(ps, v));
        Ok(Value::Undefined)
    });
    m!("setweaponammoclip", |vm, world, s, a| {
        let id = client(vm, world, s)?;
        let w = weapon(world, &text(vm, a, 0))?;
        let n = int(a, 1)?;
        match edit(world, id, |ps, facts, _| bo2::set_weapon_ammo_clip(ps, facts, w, n)) {
            Some(Err(e)) => Err(bo2_error(world, e)),
            Some(Ok(())) => Ok(Value::Undefined),
            None => prev(vm, world, s, a, "setweaponammoclip"),
        }
    });
    m!("getweaponammoclip", |vm, world, s, a| {
        let id = client(vm, world, s)?;
        let w = weapon(world, &text(vm, a, 0))?;
        match t6_gun::script_read(&frame(world), id, |ps, _| bo2::get_weapon_ammo_clip(ps, w)) {
            Some(n) => Ok(Value::Int(n)),
            None => prev(vm, world, s, a, "getweaponammoclip"),
        }
    });
    m!("disableweapons", |vm, world, s, a| {
        prev(vm, world, s, a, "disableweapons")?;
        let id = client(vm, world, s)?;
        edit(world, id, |ps, _, pm_flags| bo2::disable_weapons(ps, *pm_flags));
        Ok(Value::Undefined)
    });
    m!("enableweapons", |vm, world, s, a| {
        prev(vm, world, s, a, "enableweapons")?;
        let id = client(vm, world, s)?;
        edit(world, id, |ps, _, _| bo2::enable_weapons(ps));
        Ok(Value::Undefined)
    });
    m!("disableweaponcycling", |vm, world, s, a| {
        prev(vm, world, s, a, "disableweaponcycling")?;
        let id = client(vm, world, s)?;
        edit(world, id, |ps, _, _| bo2::set_weapon_cycling(ps, false));
        Ok(Value::Undefined)
    });
    m!("enableweaponcycling", |vm, world, s, a| {
        prev(vm, world, s, a, "enableweaponcycling")?;
        let id = client(vm, world, s)?;
        edit(world, id, |ps, _, _| bo2::set_weapon_cycling(ps, true));
        Ok(Value::Undefined)
    });
    m!("isswitchingweapons", |vm, world, s, a| query(
        vm,
        world,
        s,
        a,
        "isswitchingweapons",
        bo2::is_switching_weapons
    ));
    m!("isreloading", |vm, world, s, a| query(vm, world, s, a, "isreloading", bo2::is_reloading));
    m!("isthrowinggrenade", |vm, world, s, a| query(
        vm,
        world,
        s,
        a,
        "isthrowinggrenade",
        bo2::is_throwing_grenade
    ));
    m!("isfiring", |vm, world, s, a| query(vm, world, s, a, "isfiring", bo2::is_firing));
    m!("ismeleeing", |vm, world, s, a| query(vm, world, s, a, "ismeleeing", bo2::is_meleeing));
    m!("isusingoffhand", |vm, world, s, a| query(
        vm,
        world,
        s,
        a,
        "isusingoffhand",
        bo2::is_using_offhand
    ));
    m!("hasagrenadepulledback", |vm, world, s, a| query(
        vm,
        world,
        s,
        a,
        "hasagrenadepulledback",
        bo2::has_a_grenade_pulled_back
    ));
}

fn switch(
    vm: &mut Vm<World>,
    world: &mut World,
    s: &Value,
    a: &[Value],
    immediate: bool,
) -> Result<Value, String> {
    let id = client(vm, world, s)?;
    let w = weapon(world, &text(vm, a, 0))?;
    let bot = is_bot(world, id);
    match edit(world, id, |ps, _, _| bo2::switch_to_weapon(ps, w, immediate, bot)) {
        Some((ok, fx)) => {
            apply(world, id, &fx);
            Ok(Value::bool(ok))
        }
        None => prev(
            vm,
            world,
            s,
            a,
            if immediate { "switchtoweaponimmediate" } else { "switchtoweapon" },
        ),
    }
}

/// Each player's memory for the per-frame notifies.
#[derive(Resource, Default)]
pub(super) struct Watches(HashMap<u32, NotifyWatch>);

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
        let evs = if watch_pass { Vec::new() } else { t6_gun::script_take_events(&mut frame(world), id) };
        let pm_type = frame(world).player(id).map_or(0, |ps| ps.pm_type);
        let mut watch = world.resource::<Watches>().0.get(&c).copied().unwrap_or(NotifyWatch::default());
        let Some((out, st, clip)) = t6_gun::script_read(&frame(world), id, |ps, facts| {
            let mut out = Vec::new();
            if watch_pass {
                watch.frame(ps, pm_type, &mut out);
            } else {
                bo2::event_notifies(ps, facts, &evs, &mut out);
            }
            (out, ps.weaponstate, ps.ammo_in_clip(ps.weapon))
        }) else {
            continue;
        };
        world.resource_mut::<Watches>().0.insert(c, watch);
        if out.is_empty() {
            continue;
        }
        let named: Vec<(&'static str, Vec<String>, Option<bool>)> = out
            .iter()
            .map(|n| {
                let args = n
                    .weapons
                    .iter()
                    .map(|&w| if w == 0 { "none".to_owned() } else { weapon_text(world, w) })
                    .collect();
                (n.name, args, n.flag)
            })
            .collect();
        // IW4L_T6_WEAPLOG: the same lines the MW2-state diff logged (players.rs), so the
        // specialty weapon proofs read the same on either path; =2 logs every notify.
        if let Ok(log) = std::env::var("IW4L_T6_WEAPLOG") {
            for (name, args, _) in &named {
                let w = args.first().map_or("none", String::as_str);
                match *name {
                    "weapon_change" => {
                        let det = out
                            .iter()
                            .find(|n| n.name == "weapon_change")
                            .and_then(|n| n.weapons.first().copied())
                            .is_some_and(|w| frame(world).equipment_facts_for(w).is_some_and(|e| e.has_detonator));
                        diag::info!(Sim, "bo2zm t6 weapon in hand: {w} clip {clip} state {st} detonator {det}");
                    }
                    "weapon_fired" => diag::info!(Sim, "bo2zm t6 weapon fired: {w} clip {clip}"),
                    "weapon_melee" => diag::info!(Sim, "bo2zm t6 weapon melee: client {c} {w}"),
                    _ if log == "2" => diag::info!(Sim, "bo2zm t6 notify: {name} {args:?} state {st}"),
                    _ => {}
                }
            }
        }
        with_vm(world, |vm, world| {
            for (name, args, flag) in named {
                let mut args: Vec<Value> = args.iter().map(|a| vm.string(a)).collect();
                args.extend(flag.map(Value::bool));
                vm.notify_str(world, obj, name, &args);
            }
        });
    }
}
