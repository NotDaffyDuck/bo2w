//! bo2mp lane D: the engine half of Black Ops II's scorestreaks.
//!
//! BO2's own scripts (`maps/mp/killstreaks/*`) run every scorestreak: score
//! fills a player's momentum, `_globallogic_score` hands him the streak's
//! weapon (`radar_mp`, `supplydrop_mp`, ...) with one in the clip, `_class`
//! puts his three streak weapons on the d-pad (`setactionslot`, cheapest on
//! the right), and raising one (`weapon_change`) calls it in
//! (`_killstreaks::killstreakwaiter`). The engine keeps what BO2's exe kept:
//! the teams' radar state (spy plane, satellite), the player's inventory
//! slot (a care package's streak), the d-pad slots his keys switch to, and
//! which entities lock-on launchers may target.

use std::collections::{BTreeMap, BTreeSet};

use bevy_ecs::prelude::{Resource, World};
use gsc_t6::{ObjRef, Value, Vm};

use super::{Zm, arg, entnum, frame, list, text};
use crate::world::ClientId;

mod bullet;
mod can_damage;
mod craft;
mod heli_lock;
mod lock_on;
mod ride;
mod turret;
mod vehicle_damage;
mod view_link;

/// Scorestreak state the engine keeps for the scripts.
#[derive(Resource, Default)]
pub(crate) struct Streaks {
    /// Radar by team: the spy plane's mode (0 off, 1 sweep, 2 fast sweep,
    /// `setteamspyplane`) and the satellite (`setteamsatellite`).
    pub spyplane: BTreeMap<String, i32>,
    pub satellite: BTreeMap<String, i32>,
    /// Each player's inventory weapon (`setinventoryweapon`): a streak a
    /// care package gave him, used from its own slot.
    pub inventory: BTreeMap<u32, String>,
    /// Entities launchers may lock on to (`target_set`), with the offset on
    /// each that a lock aims at.
    pub targets: BTreeMap<u32, [f32; 3]>,
    /// Projectiles handed to the scripts, by projectile id.
    pub thrown: BTreeMap<u32, Thrown>,
    /// Entities falling after `physicslaunch` (a dropped crate): velocity.
    pub falling: BTreeMap<u32, [f32; 3]>,
    /// Landed crates lying still before `stationary`: ms left.
    pub settling: BTreeMap<u32, u32>,
    /// Entities a player can use (`makeusable`: a care package).
    pub usable: BTreeSet<u32>,
    /// Helicopters and planes the scripts fly (`spawnhelicopter`).
    pub craft: BTreeMap<u32, craft::Craft>,
    /// Turrets (`spawnturret`).
    pub turrets: BTreeMap<u32, turret::Turret>,
    /// Players riding or driving a vehicle (`usevehicle`), by client.
    pub rides: BTreeMap<u32, ride::Ride>,
    /// Players whose view sits on an entity's tag
    /// (`playerlinkweaponviewtodelta`: the Lodestar), by client.
    pub view_links: BTreeMap<u32, view_link::ViewLink>,
    /// Lock-on targets of missiles (`missile_settarget`): missile ent ->
    /// (target ent, the offset on it the scripts aim at).
    pub homing: BTreeMap<u32, (u32, [f32; 3])>,
    /// What each missile was launched at (the Hunter Killer's goal):
    /// missile ent -> target ent. Homing on it again = no lock.
    pub launched_at: BTreeMap<u32, u32>,
    /// Each player's launcher lock (`weaponlockfinalize`): client -> target.
    pub locks: BTreeMap<u32, u32>,
    /// Test aid (BO2MP_KSLOG): when the hand log last ran.
    pub log_at: i64,
    /// Entities below this number have their `birthtime`.
    pub born_upto: u32,
    /// Vehicles' own weapons from his zones: type -> (turret weapon,
    /// gunner weapons).
    pub vehicle_weapons: BTreeMap<String, (String, Vec<String>)>,
    /// Shots its craft fired (each one's own tracer).
    pub shots: u32,
    /// How each vehicle type drives and how its driver sees it.
    pub vehicle_drive: BTreeMap<String, super::T6VehicleDrive>,
    /// The minimap's frame (`setminimap`): upper left, north, world size.
    pub minimap: Option<hud_t6::compass::CompassMapBounds>,
    /// bo2mp vehicle screens: each player's server vision by client
    /// (`useservervisionset` on, `setvisionsetforplayer` name).
    pub server_vision: BTreeMap<u32, (bool, String)>,
    /// Missiles that went off while a player still rode them: their script
    /// objects live until the ride's own script has finished (see
    /// `projectiles`), by entity -> level time it is freed at the latest.
    pub late_free: BTreeMap<u32, i32>,
    /// When each player last pressed each d-pad slot's key (server ms), by
    /// client: `actionslot*buttonpressed`.
    pub slot_presses: BTreeMap<u32, [i64; 4]>,
    /// Client flags the scripts set on entities (`setclientflag`), by
    /// entity: bit n is flag n. A counter-UAV jams its enemies' minimaps
    /// while it carries flag 11.
    pub client_flags: BTreeMap<u32, u32>,
    /// Players an enemy EMP jams (`setempjammed`).
    pub emp_jammed: BTreeSet<u32>,
    /// How many times each player was shown his enemies for a moment
    /// (`g_compassShowEnemies`); his HUD pings them all when it changes.
    pub radar_pulse: BTreeMap<u32, u32>,
    /// The map's helicopter height lock ceilings, read from its entities
    /// when a script first asks; a restart reads them afresh, all on.
    pub heli_patches: Option<Vec<heli_lock::Patch>>,
    /// Hellstorms whose riders have heard `missile_boost`, by projectile id.
    pub boost_told: BTreeSet<u32>,
    /// Missiles flying on unsteered (`setmissilecoasting`), by entity: one
    /// goes quietly, with `death` and no `explode`.
    pub coasting: BTreeSet<u32>,
}

/// His radar shows his enemies once, now: BO2 turns the flag on for 0.2 s
/// and every enemy it shows is pinged, fading like a sweep's ping.
pub(super) fn radar_pulse(world: &mut World, client: u32) {
    let mut st = streaks(world);
    let n = st.radar_pulse.entry(client).or_default();
    *n += 1;
    let n = *n;
    diag::info!(Sim, "bo2mp radar: pulse for player {client} (count {n})");
}

/// The client flag BO2's counter-UAV plane carries while it jams
/// (`_spyplane`'s callcounteruav; its client script scrambles the enemies'
/// minimap while it is set).
const FLAG_COUNTERUAV: u32 = 11;

/// How long a slot key's press reads as held (the key sends one press, BO2's
/// scripts poll it every 50 ms).
const SLOT_PRESS_MS: i64 = 150;

/// He pressed a d-pad slot's key (0-based slot).
pub(super) fn action_slot_pressed(world: &mut World, client: u32, slot: u8) {
    let now = world.resource::<Zm>().now_ms;
    if let Some(at) = streaks(world)
        .slot_presses
        .entry(client)
        .or_insert([i64::MIN; 4])
        .get_mut(usize::from(slot))
    {
        *at = now;
    }
}

/// Whether his slot key (0-based) was pressed just now.
fn slot_pressed(world: &mut World, client: u32, slot: usize) -> bool {
    let now = world.resource::<Zm>().now_ms;
    streaks(world)
        .slot_presses
        .get(&client)
        .is_some_and(|p| now.saturating_sub(p[slot]) <= SLOT_PRESS_MS)
}

/// His client's vision dvar (`bo2mp_vision`): the server's vision for him
/// while `useservervisionset` is on, "" for the map's own.
fn publish_vision(world: &mut World, client: u32) {
    let shown = streaks(world)
        .server_vision
        .get(&client)
        .filter(|(on, name)| *on && !name.is_empty())
        .map(|(_, name)| name.clone())
        .unwrap_or_default();
    super::set_client_dvar(world, client, "bo2mp_vision", &shown);
}

/// A new map: fresh scorestreak state, the vehicles' weapons from his zones.
pub(super) fn install(world: &mut World, vehicles: Vec<super::T6Vehicle>) {
    let vehicle_drive = vehicles
        .iter()
        .map(|v| (v.name.to_ascii_lowercase(), v.drive))
        .collect();
    let vehicle_weapons = vehicles
        .into_iter()
        .map(|v| (v.name.to_ascii_lowercase(), (v.turret, v.gunners)))
        .collect::<BTreeMap<String, (String, Vec<String>)>>();
    let armed = vehicle_weapons
        .iter()
        .filter(|(_, (t, g))| !t.is_empty() || !g.is_empty())
        .map(|(n, (t, g))| {
            format!(
                "{n}={t}{}",
                if g.is_empty() {
                    String::new()
                } else {
                    format!("+{}", g.join("+"))
                }
            )
        })
        .collect::<Vec<_>>();
    if !vehicle_weapons.is_empty() {
        diag::info!(
            Sim,
            "bo2mp streaks: {} vehicles, armed: {}",
            vehicle_weapons.len(),
            armed.join(" ")
        );
    }
    world.insert_resource(Streaks {
        vehicle_weapons,
        vehicle_drive,
        ..Default::default()
    });
}

/// `missile_create{attractor,repulsor}{ent,origin}( at, strength, maxDist )`:
/// a decoy that draws (or pushes away) the wobbling rockets near it, on an
/// entity or at a point. It answers the slot it took.
fn pull(
    vm: &mut Vm<World>,
    world: &mut World,
    a: &[Value],
    attractor: bool,
    on_ent: bool,
) -> Result<Value, String> {
    let slot = crate::missile_decoy::free_slot(world)?;
    let (entnum, origin) = if on_ent {
        (entnum(vm, arg(a, 0)).ok_or("not an entity")?, [0.0; 3])
    } else {
        (crate::missile_decoy::AT_POINT, super::vec3(a, 0)?)
    };
    let pull = crate::missile_decoy::Pull {
        attractor,
        entnum,
        origin,
        strength: super::num(a, 1)?,
        max_dist: super::num(a, 2)?,
    };
    crate::missile_decoy::place(world, slot, pull)?;
    Ok(Value::Int(slot as i32))
}

/// The level restarts (`map_restart`): every ride and view link ends, each
/// player's server vision goes, the rest starts fresh; his zones' vehicles
/// and the minimap stay.
pub(super) fn restart(world: &mut World) {
    crate::missile_decoy::restart(world);
    let rides: Vec<u32> = streaks(world).rides.keys().copied().collect();
    for client in rides {
        ride::end_ride(world, client);
    }
    let links: Vec<u32> = streaks(world).view_links.keys().copied().collect();
    for client in links {
        view_link::end(world, client);
    }
    let old = std::mem::take(&mut *streaks(world));
    let seen: Vec<u32> = old.server_vision.keys().copied().collect();
    world.insert_resource(Streaks {
        vehicle_weapons: old.vehicle_weapons,
        vehicle_drive: old.vehicle_drive,
        minimap: old.minimap,
        ..Default::default()
    });
    for client in seen {
        publish_vision(world, client);
    }
}

/// bo2mp: the flying projectile a script entity stands for (the Hunter
/// Killer's drone), as its clients see it: its number, model, facing and
/// place. A client script's effect on it rides the projectile.
pub(super) fn thrown_shown(world: &mut World, n: u32) -> Option<(i32, String, [f32; 3], [f32; 3])> {
    let id = streaks(world)
        .thrown
        .iter()
        .find(|(_, t)| t.ent == n)
        .map(|(id, _)| *id)?;
    let p = crate::frame::collect_projectiles(world)
        .into_iter()
        .find(|p| p.live && p.id.0 == id)?;
    let model = frame(world).weapon_projectile_model(p.weapon).to_owned();
    Some((p.entnum, model, p.apos.tr_base, p.origin))
}

/// bo2mp: the script entity standing for live projectile `id`, if the scripts were told of it.
pub(super) fn thrown_ent(world: &mut World, id: u32) -> Option<u32> {
    streaks(world).thrown.get(&id).map(|t| t.ent)
}

/// bo2mp: who drives vehicle `n` (sits in its seat 0), if anyone.
pub(super) fn driver_of(world: &World, n: u32) -> Option<u32> {
    world
        .get_resource::<Streaks>()?
        .rides
        .iter()
        .find(|(_, r)| r.vehicle == n && r.seat == 0)
        .map(|(c, _)| *c)
}

fn streaks(world: &mut World) -> bevy_ecs::prelude::Mut<'_, Streaks> {
    if !world.contains_resource::<Streaks>() {
        world.insert_resource(Streaks::default());
    }
    world.resource_mut::<Streaks>()
}

/// A scorestreak's own entity that takes damage from weapons (a plane, a
/// helicopter, a drone or vehicle, a turret, a crate): BO2's scripts watch
/// its `damage`.
pub(super) fn damage_target(world: &World, n: u32) -> bool {
    let zm = world.resource::<Zm>();
    zm.mp
        && (world
            .get_resource::<Streaks>()
            .is_some_and(|s| s.craft.contains_key(&n) || s.turrets.contains_key(&n))
            || zm.ents.get(&n).is_some_and(|e| e.can_damage))
}

/// A turret `spawnturret` made (its classname is the scripts' own).
pub(super) fn is_turret(world: &World, n: u32) -> bool {
    world
        .get_resource::<Streaks>()
        .is_some_and(|s| s.turrets.contains_key(&n))
}

/// A vehicle on wheels (the RC-XD): its middle is low, not a flyer's.
pub(super) fn ground_vehicle(world: &World, n: u32) -> bool {
    world
        .get_resource::<Streaks>()
        .is_some_and(|s| s.craft.get(&n).is_some_and(|c| c.ground))
}

/// A driven vehicle's speed in the world (units a second), for bots that
/// lead it with their aim.
pub(super) fn driven_velocity(world: &World, n: u32) -> Option<[f32; 3]> {
    ride::velocity_of(world, n)
}

/// A weapon's hit on such an entity: its `damage` (amount, attacker,
/// direction, point, means, model, tag, part, weapon, flags), as BO2 sends
/// it.
#[allow(clippy::too_many_arguments)]
pub(super) fn damage_entity(
    world: &mut World,
    n: u32,
    amount: i32,
    attacker: Option<ClientId>,
    means: &str,
    weapon: &str,
    point: [f32; 3],
    dir: [f32; 3],
    flags: i32,
) {
    let Some(obj) = world.resource::<Zm>().ents.get(&n).and_then(|e| e.obj) else {
        return;
    };
    super::with_vm(world, |vm, world| {
        let attacker = attacker
            .and_then(|a| world.resource::<Zm>().players.get(&a.0).map(|p| Value::Object(p.obj)))
            .unwrap_or(Value::Undefined);
        // A vehicle's damage goes through the scripts' vehicle callback.
        if vehicle_damage::is_vehicle(world, n) {
            vehicle_damage::hit(
                vm,
                world,
                n,
                attacker.clone(),
                attacker,
                amount,
                flags,
                means,
                weapon,
                point,
                dir,
            );
            return;
        }
        let (m, w, none) = (vm.string(means), vm.string(weapon), vm.string(""));
        let args = [
            Value::Int(amount),
            attacker,
            Value::Vec3(gsc_t6::math::normalize(dir)),
            Value::Vec3(point),
            m,
            none.clone(),
            none.clone(),
            none,
            w,
            Value::Int(flags),
        ];
        if vm.alive(obj) {
            vm.notify_str(world, obj, "damage", &args);
        }
    });
}

/// A scorestreak vehicle (not a plane): BO2 hands its damage to the
/// scripts' vehicle callbacks.
pub(super) fn is_vehicle(world: &World, n: u32) -> bool {
    vehicle_damage::is_vehicle(world, n)
}

/// A scorestreak vehicle's `health` and `maxhealth`, as the scripts read
/// them.
pub(super) fn vehicle_health(vm: &mut Vm<World>, obj: ObjRef) -> (i32, i32) {
    vehicle_damage::vehicle_health(vm, obj)
}

/// A hit on vehicle `n` from a script (`dodamage`), as BO2's
/// `G_DamageVehicle` takes it.
#[allow(clippy::too_many_arguments)]
pub(super) fn vehicle_hit(
    vm: &mut Vm<World>,
    world: &mut World,
    n: u32,
    inflictor: Value,
    attacker: Value,
    amount: i32,
    dflags: i32,
    means: &str,
    weapon: &str,
    point: [f32; 3],
    dir: [f32; 3],
) {
    vehicle_damage::hit(vm, world, n, inflictor, attacker, amount, dflags, means, weapon, point, dir);
}

/// A weapon's blast on vehicle `n` (a grenade, a rocket), as BO2's
/// `G_RadiusDamage` hands it to the vehicle radius callback.
#[allow(clippy::too_many_arguments)]
pub(super) fn vehicle_blast(
    world: &mut World,
    n: u32,
    origin: [f32; 3],
    radius: f32,
    inner: f32,
    outer: f32,
    cone: Option<([f32; 3], f32)>,
    means: &str,
    attacker: Option<ClientId>,
    weapon: &str,
) {
    super::with_vm(world, |vm, world| {
        let attacker = attacker
            .and_then(|a| world.resource::<Zm>().players.get(&a.0).map(|p| Value::Object(p.obj)))
            .unwrap_or(Value::Undefined);
        vehicle_damage::blast(
            vm,
            world,
            n,
            attacker.clone(),
            attacker,
            origin,
            radius,
            inner,
            outer,
            cone,
            means,
            weapon,
        );
    });
}

/// His HUD's vehicle screen (`bo2mp_vehicle`) goes, if it is still the one
/// `kind` (its first word) put up: a screen another of his streaks put up
/// since stays.
pub(super) fn clear_vehicle_hud(world: &mut World, client: u32, kind: &str) {
    let shown = frame(world)
        .client_meta(ClientId(client))
        .and_then(|m| {
            m.client_dvars
                .iter()
                .find(|(k, _)| k == "bo2mp_vehicle")
                .map(|(_, v)| v.split_whitespace().next().unwrap_or("").to_owned())
        })
        .unwrap_or_default();
    if shown == kind {
        super::set_client_dvar(world, client, "bo2mp_vehicle", "");
    }
}

/// He is in a scorestreak vehicle (BO2's eFlags 0x4000, EF_IN_VEHICLE).
pub(super) fn riding(world: &World, client: u32) -> bool {
    world
        .get_resource::<Streaks>()
        .is_some_and(|s| s.rides.contains_key(&client))
}

/// The entity his view is locked to (`viewlockedentity`): the vehicle he
/// rides or drives, or the one his view sits on (the Lodestar).
pub(super) fn view_locked(world: &World, client: u32) -> Option<u32> {
    let s = world.get_resource::<Streaks>()?;
    s.rides
        .get(&client)
        .map(|r| r.vehicle)
        .or_else(|| s.view_links.get(&client).map(|l| l.ent))
}

/// He steers a scorestreak from where he stands: drives or rides one of its
/// vehicles, or his view sits on one (the Lodestar).
pub(super) fn remote_control(world: &World, client: u32) -> bool {
    world
        .get_resource::<Streaks>()
        .is_some_and(|s| s.rides.contains_key(&client) || s.view_links.contains_key(&client))
}

/// He sits in a vehicle whose def is driven from afar (the RC-XD's): BO2's
/// `VEH_LinkPlayer` gives him `EF2_REMOTE_CONTROL` for it, in any seat.
pub(super) fn in_remote_vehicle(world: &World, client: u32) -> bool {
    world.get_resource::<Streaks>().is_some_and(|s| {
        s.rides
            .get(&client)
            .and_then(|r| s.craft.get(&r.vehicle))
            .is_some_and(|c| c.remote_control)
    })
}

/// bo2mp: the scorestreak vehicles a flash reaches (BO2's FlashbangBlastEnt:
/// one driven from afar with someone in it), each with its driver (seat 0).
pub(super) fn remote_driven(world: &World) -> Vec<(u32, Option<u32>)> {
    let Some(s) = world.get_resource::<Streaks>() else {
        return Vec::new();
    };
    s.craft
        .iter()
        .filter(|(n, c)| c.remote_control && s.rides.values().any(|r| r.vehicle == **n))
        .map(|(n, _)| {
            let driver = s
                .rides
                .iter()
                .find(|(_, r)| r.vehicle == *n && r.seat == 0)
                .map(|(c, _)| *c);
            (*n, driver)
        })
        .collect()
}

pub(super) fn bind(vm: &mut Vm<World>) {
    macro_rules! f {
        ($name:literal, $body:expr) => {
            vm.bind($name, false, $body);
        };
    }
    macro_rules! m {
        ($name:literal, $body:expr) => {
            vm.bind($name, true, $body);
        };
    }
    // Scorestreaks, not killstreaks: BO2's exe sets scr_scorestreaks (score
    // earns them, each kept until used) and lets one of each be held.
    // PLACEHOLDER: the exe's own defaults are not in his data files; these
    // are the values BO2's public matches play by. player_useRadius: how
    // near a player must be to use something (the engine's 128; bots
    // capture a care package from inside it).
    for (k, v) in [
        ("scr_scorestreaks", "1"),
        ("scr_scorestreaks_maxstacking", "1"),
        ("player_useradius", "128"),
    ] {
        vm.dvars.entry(k.to_owned()).or_insert_with(|| v.to_owned());
    }
    // bo2mp: Black Ops II's earthquake (its GScr_Earthquake; the shared list
    // ignores it, so multiplayer's is bound here): the Hellstorm's launch,
    // a car or barrel going up, a gunner's rocket. A player named fifth
    // alone feels it.
    f!("earthquake", |vm, world, _, a| {
        let quake = earthquake(
            super::num(a, 0)?,
            super::num(a, 1)?,
            arg(a, 2).as_vec3().ok_or("earthquake's origin is not a vector")?,
            super::num(a, 3)?,
        )?;
        let audience = match entnum(vm, arg(a, 4)) {
            Some(n) if world.resource::<Zm>().players.contains_key(&n) => {
                crate::EventAudience::Client(ClientId(n))
            }
            _ => crate::EventAudience::All,
        };
        send_earthquake(world, audience, quake);
        Ok(Value::Undefined)
    });
    // Radar: a team's spy plane (UAV: 1 sweeps, 2 sweeps fast with two up)
    // and satellite (Orbital VSAT: enemies shown all the time).
    f!("getteamspyplane", |vm, world, _, a| {
        let team = text(vm, a, 0);
        Ok(Value::Int(
            streaks(world).spyplane.get(&team).copied().unwrap_or(0),
        ))
    });
    f!("setteamspyplane", |vm, world, _, a| {
        let team = text(vm, a, 0);
        let v = arg(a, 1).as_int().unwrap_or(0);
        streaks(world).spyplane.insert(team, v);
        Ok(Value::Undefined)
    });
    f!("getteamsatellite", |vm, world, _, a| {
        let team = text(vm, a, 0);
        Ok(Value::Int(
            streaks(world).satellite.get(&team).copied().unwrap_or(0),
        ))
    });
    f!("setteamsatellite", |vm, world, _, a| {
        let team = text(vm, a, 0);
        let v = arg(a, 1).as_int().unwrap_or(0);
        streaks(world).satellite.insert(team, v);
        Ok(Value::Undefined)
    });
    // The inventory slot: a streak from a care package ("" empties it;
    // "none" when empty, as the scripts compare it with weapon names). The
    // engine's own field carries it to the client, which switches to it when
    // the switch-weapon key is held.
    m!("setinventoryweapon", |vm, world, s, a| {
        let Some(n) = entnum(vm, s) else {
            return Ok(Value::Undefined);
        };
        let w = text(vm, a, 0);
        let empty = w.is_empty() || w == "none";
        let index = if empty {
            0
        } else {
            super::weapon(world, &w).unwrap_or(0) as i32
        };
        diag::info!(Sim, "bo2mp streaks: player {n} inventory weapon `{w}` ({index})");
        {
            let mut st = streaks(world);
            if empty {
                st.inventory.remove(&n);
            } else {
                st.inventory.insert(n, w);
            }
        }
        if let Ok(id) = super::client(vm, world, s)
            && let Some(ps) = frame(world).player_mut(id)
        {
            ps.inventory_weapon = index;
        }
        Ok(Value::Undefined)
    });
    m!("getinventoryweapon", |vm, world, s, _| {
        let n = entnum(vm, s).unwrap_or(u32::MAX);
        let w = streaks(world).inventory.get(&n).cloned();
        Ok(vm.string(w.as_deref().unwrap_or("none")))
    });
    // A d-pad slot (1 up, 2 down, 3 left, 4 right): BO2's HUD draws what it
    // holds, and pressing it switches to a weapon it holds (the scorestreak
    // weapons, cheapest on the right). The engine's own slots carry it to
    // the client, which raises the weapon as the key goes down.
    m!("setactionslot", |vm, world, s, a| {
        let id = super::client(vm, world, s)?;
        let slot = arg(a, 0).as_int().unwrap_or(0);
        let kind = text(vm, a, 1).to_ascii_lowercase();
        let held = if kind == "weapon" {
            text(vm, a, 2)
        } else {
            kind.clone()
        };
        {
            let mut zm = world.resource_mut::<Zm>();
            let slots = zm.action_slots.entry(id.0).or_default();
            slots.retain(|(n, _)| *n != slot);
            if !held.is_empty() {
                slots.push((slot, held.clone()));
                slots.sort();
            }
        }
        let (ty, param) = match kind.as_str() {
            "weapon" => (1, super::weapon(world, &held).unwrap_or(0) as i32),
            "altmode" => (2, 0),
            "nightvision" => (3, 0),
            _ => (0, 0),
        };
        let (ty, param) = if ty == 1 && param == 0 {
            (0, 0)
        } else {
            (ty, param)
        };
        if let Ok(i) = usize::try_from(slot - 1)
            && i < 4
            && let Some(ps) = frame(world).player_mut(id)
        {
            ps.action_slot_type[i] = ty;
            ps.action_slot_param[i] = param;
        }
        Ok(Value::Undefined)
    });
    // His inventory button: BO2 tells a streak used from the inventory slot
    // from one on the d-pad by it. His client sets it on each command while
    // it switches to the inventory weapon (switch-weapon held).
    m!("inventorybuttonpressed", |vm, world, s, _| {
        let id = super::client(vm, world, s)?;
        let held = crate::script_player::buttons(&mut frame(world), id);
        Ok(Value::bool(held & playerstate_t6::game::buttons::INVENTORY != 0))
    });
    // A use trigger only he may use (`player clientclaimtrigger(trigger)`:
    // his turret's pick-up), or only a team (`trigger
    // setteamfortrigger(team)`); the scripts read them back as
    // `claimedby` and `triggerteam`.
    m!("clientclaimtrigger", |vm, _, s, a| {
        if let Value::Object(t) = arg(a, 0) {
            let f = vm.intern("claimedby");
            vm.set_raw_field(*t, f, s.clone());
        }
        Ok(Value::Undefined)
    });
    m!("clientreleasetrigger", |vm, _, _, a| {
        if let Value::Object(t) = arg(a, 0) {
            let f = vm.intern("claimedby");
            vm.set_raw_field(*t, f, Value::Undefined);
        }
        Ok(Value::Undefined)
    });
    m!("releaseclaimedtrigger", |vm, _, s, _| {
        if let Value::Object(t) = s {
            let f = vm.intern("claimedby");
            vm.set_raw_field(*t, f, Value::Undefined);
        }
        Ok(Value::Undefined)
    });
    m!("setteamfortrigger", |vm, _, s, a| {
        if let Value::Object(t) = s {
            let f = vm.intern("triggerteam");
            let team = arg(a, 0).clone();
            vm.set_raw_field(*t, f, team);
        }
        Ok(Value::Undefined)
    });
    // His d-pad slot keys (a placed-to-be Sentry or Guardian goes back to
    // his streaks on the fourth: `watchreturnturrettoinventory`).
    m!("actionslotonebuttonpressed", |vm, world, s, _| {
        let id = super::client(vm, world, s)?;
        Ok(Value::bool(slot_pressed(world, id.0, 0)))
    });
    m!("actionslottwobuttonpressed", |vm, world, s, _| {
        let id = super::client(vm, world, s)?;
        Ok(Value::bool(slot_pressed(world, id.0, 1)))
    });
    m!("actionslotthreebuttonpressed", |vm, world, s, _| {
        let id = super::client(vm, world, s)?;
        Ok(Value::bool(slot_pressed(world, id.0, 2)))
    });
    m!("actionslotfourbuttonpressed", |vm, world, s, _| {
        let id = super::client(vm, world, s)?;
        Ok(Value::bool(slot_pressed(world, id.0, 3)))
    });
    // Lock-on targets (spy planes, helicopters, drones): what launchers and
    // the missile drone may home on.
    f!("target_set", |vm, world, _, a| {
        if let Some(n) = entnum(vm, arg(a, 0)) {
            let offset = arg(a, 1).as_vec3().unwrap_or([0.0; 3]);
            streaks(world).targets.insert(n, offset);
        }
        Ok(Value::Undefined)
    });
    f!("target_remove", |vm, world, _, a| {
        if let Some(n) = entnum(vm, arg(a, 0)) {
            streaks(world).targets.remove(&n);
        }
        Ok(Value::Undefined)
    });
    f!("target_istarget", |vm, world, _, a| {
        let n = entnum(vm, arg(a, 0)).unwrap_or(u32::MAX);
        Ok(Value::bool(streaks(world).targets.contains_key(&n)))
    });
    f!("target_getarray", |_, world, _, _| {
        let ns: Vec<u32> = streaks(world).targets.keys().copied().collect();
        let zm = world.resource::<Zm>();
        let v = ns
            .into_iter()
            .filter_map(|n| zm.ents.get(&n).and_then(|e| e.obj).map(Value::Object))
            .collect();
        Ok(list(v))
    });
    f!("target_getoffset", |vm, world, _, a| {
        let n = entnum(vm, arg(a, 0)).unwrap_or(u32::MAX);
        let offset = streaks(world).targets.get(&n).copied();
        Ok(Value::Vec3(offset.unwrap_or([0.0; 3])))
    });
    f!("target_setturretaquire", |_, _, _, _| Ok(Value::Undefined));
    // `entity getentnum()`: its entity number (the dogs' debug prints).
    m!("getentnum", |vm, _, s, _| {
        let n = entnum(vm, s).ok_or("not an entity")?;
        Ok(Value::Int(n as i32))
    });
    // `entity showpart(tag)`: the AGR shows its first-person part again
    // when he gets out. Nothing here ever hides a part (no MP script calls
    // `hidepart`), so every part already shows.
    m!("showpart", |vm, _, s, _| {
        entnum(vm, s).ok_or("not an entity")?;
        Ok(Value::Undefined)
    });
    // Stats and the match recorder (no online services), thermal outlines.
    for name in [
        "recordkillstreakevent",
        "recordkillstreakendevent",
        "sendkillstreakdamageevent",
    ] {
        vm.bind(name, true, |_, _, _, _| Ok(Value::Undefined));
        vm.bind(name, false, |_, _, _, _| Ok(Value::Undefined));
    }
    for name in [
        "setdrawinfrared",
        "setforcenocull",
        "mayapplyscreeneffect",
    ] {
        vm.bind(name, true, |_, _, _, _| Ok(Value::Undefined));
    }
    // bo2mp vehicle screens: a player's own vision from the server
    // (_helicopter_gunner.gsc visionswitch: `useservervisionset(1)` then
    // `setvisionsetforplayer(level.chopper_enhanced_vision, 1)`, the
    // remote_mortar_enhanced vision; `useservervisionset(0)` when he leaves).
    // His client grades its frame with it (its fade time is not run).
    m!("useservervisionset", |vm, world, s, a| {
        let id = super::client(vm, world, s)?;
        let on = arg(a, 0).as_float().unwrap_or(0.0) != 0.0;
        streaks(world).server_vision.entry(id.0).or_default().0 = on;
        publish_vision(world, id.0);
        Ok(Value::Undefined)
    });
    // His vehicle screen's infrared (`setinfraredvision`: the Lodestar and
    // the VTOL Warship turn it on when he rides, off on his enhanced view
    // and when he leaves): BO2's HUD lights its FLIR box while it is on,
    // OPT while it is off (`bo2mp_infrared`).
    m!("setinfraredvision", |vm, world, s, a| {
        let id = super::client(vm, world, s)?;
        let on = arg(a, 0).as_float().unwrap_or(0.0) != 0.0;
        super::set_client_dvar(world, id.0, "bo2mp_infrared", if on { "1" } else { "" });
        Ok(Value::Undefined)
    });
    m!("setvisionsetforplayer", |vm, world, s, a| {
        let id = super::client(vm, world, s)?;
        let name = text(vm, a, 0);
        diag::info!(Sim, "bo2mp: setvisionsetforplayer {name} for player {}", id.0);
        streaks(world).server_vision.entry(id.0).or_default().1 = name;
        publish_vision(world, id.0);
        Ok(Value::Undefined)
    });
    // The gunner's FLIR toggle (_helicopter_gunner.gsc visionswitch polls his
    // change seat button, the HUD's `[{+weapnext_inventory}]` prompt, then
    // waits for it to be let go). His client puts the wheel-up bind on the
    // command's CHANGE_SEAT button.
    m!("changeseatbuttonpressed", |vm, world, s, _| {
        let id = super::client(vm, world, s)?;
        let held = crate::script_player::buttons(&mut frame(world), id);
        Ok(Value::bool(
            held & playerstate_t6::game::buttons::CHANGE_SEAT != 0,
        ))
    });
    // A dropped crate falls (`physicslaunch`): straight down under gravity
    // from where it was let go, then `stationary` where it lands.
    m!("physicslaunch", |vm, world, s, a| {
        if let Some(n) = entnum(vm, s) {
            let v = arg(a, 1).as_vec3().unwrap_or([0.0; 3]);
            streaks(world).settling.remove(&n);
            streaks(world).falling.insert(n, [0.0, 0.0, v[2]]);
        }
        Ok(Value::Undefined)
    });
    // A model a player can use (the care package): looking near it and
    // pressing use fires its `trigger`, as a use trigger does.
    m!("makeusable", |vm, world, s, _| {
        if let Some(n) = entnum(vm, s) {
            streaks(world).usable.insert(n);
        }
        Ok(Value::Undefined)
    });
    m!("makeunusable", |vm, world, s, _| {
        if let Some(n) = entnum(vm, s) {
            streaks(world).usable.remove(&n);
        }
        Ok(Value::Undefined)
    });
    // Splash damage the scripts deal (a crate landing on someone, a
    // turret's explosion, the EMP's blast): everyone in the radius the
    // blast can see, full at the centre down to the minimum at the edge.
    f!("radiusdamage", |vm, world, _, a| {
        radius_damage(vm, world, None, a)
    });
    m!("radiusdamage", |vm, world, s, a| {
        let inflictor = s.clone();
        radius_damage(vm, world, Some(inflictor), a)
    });
    // A blast for the glass alone (the airstrikes' bombs, Takeoff's
    // shutters): BO2's GScr_GlassRadiusDamage, MOD_EXPLOSIVE unless the
    // fifth argument names the kind.
    f!("glassradiusdamage", |vm, world, _, a| {
        let origin = super::vec3(a, 0)?;
        let radius = super::num(a, 1)?;
        let (max, min) = (super::num(a, 2)?, super::num(a, 3)?);
        let means = match arg(a, 4) {
            Value::Undefined => "MOD_EXPLOSIVE".to_owned(),
            v => vm.to_text(v),
        };
        super::glass::script_blast(world, origin, radius, max, min, &means);
        Ok(Value::Undefined)
    });
    // A projectile the scripts fire themselves (a missile, the Hellstorm,
    // the Swarm): the engine's own projectile, owned by the player.
    f!("magicbullet", |vm, world, _, a| {
        let name = text(vm, a, 0);
        let (start, end) = (super::vec3(a, 1)?, super::vec3(a, 2)?);
        let Ok(owner) = super::client(vm, world, arg(a, 3)) else {
            return Ok(Value::Undefined);
        };
        let w = super::weapon(world, &name)?;
        if w == 0 {
            return Ok(Value::Undefined);
        }
        let t = super::tick(world);
        let p = match crate::missile::magic_bullet(&mut frame(world), t, owner, w, start, end) {
            Ok(p) => p,
            Err(e) => {
                diag::warn!(Sim, "bo2mp streaks: magicbullet {name}: {e}");
                return Ok(Value::Undefined);
            }
        };
        let classname = if name.contains("grenade") {
            "grenade"
        } else {
            "rocket"
        };
        let (ent, obj) = adopt(vm, world, p.origin, classname);
        streaks(world).thrown.insert(
            p.id.0,
            Thrown {
                ent,
                owner: owner.0,
                origin: p.origin,
                stationary: false,
                stuck: false,
            },
        );
        if let Some(target) = entnum(vm, arg(a, 4)) {
            streaks(world).homing.insert(ent, (target, [0.0; 3]));
            streaks(world).launched_at.insert(ent, target);
        }
        Ok(Value::Object(obj))
    });
    // A plane drops a bomb (`plane launchbomb(weapon, position, velocity)`:
    // the Lightning Strike): the weapon's projectile from there along the
    // velocity, owned by the plane's owner.
    m!("launchbomb", |vm, world, s, a| {
        let name = text(vm, a, 0);
        let from = super::vec3(a, 1)?;
        let v = arg(a, 2).as_vec3().unwrap_or([0.0, 0.0, -5000.0]);
        // Called on the player (the Lightning Strike's dropbomb) or on his
        // plane (its owner).
        let owner = super::client(vm, world, s).ok().or_else(|| match s {
            Value::Object(o) => {
                let f = vm.intern("owner");
                let who = vm.raw_field(*o, f);
                super::client(vm, world, &who).ok()
            }
            _ => None,
        });
        let (Some(owner), Ok(w)) = (owner, super::weapon(world, &name)) else {
            return Ok(Value::Undefined);
        };
        let to = [from[0] + v[0], from[1] + v[1], from[2] + v[2]];
        let t = super::tick(world);
        let p = match crate::missile::magic_bullet(&mut frame(world), t, owner, w, from, to) {
            Ok(p) => p,
            Err(e) => {
                diag::warn!(Sim, "bo2mp streaks: launchbomb {name}: {e}");
                return Ok(Value::Undefined);
            }
        };
        // A bomb leaves at the velocity given, not at the weapon's throw
        // speeds (the planemortar bomb's upward launch sent it skyward).
        if let Some(b) = frame(world).projectile_mut_by_number(p.entnum) {
            b.velocity = v;
            b.pos.tr_base = from;
            b.pos.tr_delta = v;
            b.pos.tr_time = crate::level_time_ms(t);
        }
        let (ent, obj) = adopt(vm, world, p.origin, "rocket");
        streaks(world).thrown.insert(
            p.id.0,
            Thrown {
                ent,
                owner: owner.0,
                origin: p.origin,
                stationary: false,
                stuck: false,
            },
        );
        Ok(Value::Object(obj))
    });
    // The Hellstorm: the player rides the missile (`linktomissile(missile,
    // armed)`) and steers it with his view until it hits or he lets go - the
    // engine's own guided missile, as MW2's Predator. BO2's armed missile
    // boosts once by itself; fire drops its bomblets (the scripts'
    // missile_deploy_watch).
    m!("linktomissile", |vm, world, s, a| {
        let id = super::client(vm, world, s)?;
        let Some(n) = entnum(vm, arg(a, 0)) else {
            return Ok(Value::Undefined);
        };
        let Some(pid) = streaks(world)
            .thrown
            .iter()
            .find(|(_, t)| t.ent == n)
            .map(|(id, _)| *id)
        else {
            return Ok(Value::Undefined);
        };
        let Some(p) = crate::frame::collect_projectiles(world)
            .into_iter()
            .find(|p| p.id.0 == pid)
        else {
            return Ok(Value::Undefined);
        };
        let mut f = frame(world);
        if f.client_meta(id).is_some() {
            f.client_meta_mut(id).remote_missile = Some(crate::RemoteMissile {
                projectile: p.id,
                entnum: p.entnum,
                angles: aim_angles(p.velocity),
                armed: gsc_t6::truthy(arg(a, 1)),
                t6: true,
                ..Default::default()
            });
        }
        // BO2's HUD hears the ride the way it hears the drones' (`bo2mp_vehicle`
        // = "<weapon> <seat>"): its Hellstorm screen is PredatorHUD.
        super::set_client_dvar(world, id.0, "bo2mp_vehicle", "remote_missile_mp 0");
        diag::info!(Sim, "bo2mp hellstorm: player {} rides the missile (projectile {pid})", id.0);
        Ok(Value::Undefined)
    });
    m!("unlinkfrommissile", |vm, world, s, _| {
        let id = super::client(vm, world, s)?;
        let mut f = frame(world);
        if f.client_meta(id).is_some() {
            f.client_meta_mut(id).remote_missile = None;
        }
        clear_vehicle_hud(world, id.0, "remote_missile_mp");
        diag::info!(Sim, "bo2mp hellstorm: player {} lets go of the missile", id.0);
        Ok(Value::Undefined)
    });
    // A missile flies on unsteered (`missile setmissilecoasting(1)`: the
    // Hellstorm, hidden once its bomblets are out, until it goes off).
    m!("setmissilecoasting", |vm, world, s, a| {
        let Some(n) = entnum(vm, s) else {
            return Ok(Value::Undefined);
        };
        let on = arg(a, 0).as_float().unwrap_or(1.0) != 0.0;
        let Some(pid) = streaks(world)
            .thrown
            .iter()
            .find(|(_, t)| t.ent == n)
            .map(|(id, _)| *id)
        else {
            return Ok(Value::Undefined);
        };
        let mut f = frame(world);
        crate::equipment::set_missile_coasting(&mut f, pid, on);
        for c in f.client_ids_sorted() {
            if let Some(mut link) = f.client_meta(c).and_then(|m| m.remote_missile)
                && link.projectile.0 == pid
            {
                link.coasting = on;
                f.client_meta_mut(c).remote_missile = Some(link);
            }
        }
        drop(f);
        let mut st = streaks(world);
        if on {
            st.coasting.insert(n);
        } else {
            st.coasting.remove(&n);
        }
        Ok(Value::Undefined)
    });
    // A missile homes on what the scripts name (the Hunter Killer's car).
    m!("missile_settarget", |vm, world, s, a| {
        if let Some(n) = entnum(vm, s) {
            // BO2's offset is in the target's own axes; none is zero.
            let offset = arg(a, 1).as_vec3().unwrap_or([0.0; 3]);
            match entnum(vm, arg(a, 0)) {
                Some(t) => streaks(world).homing.insert(n, (t, offset)),
                None => streaks(world).homing.remove(&n),
            };
        }
        Ok(Value::Undefined)
    });
    // A projectile goes off now (`detonate`).
    m!("detonate", |vm, world, s, _| {
        let Some(n) = entnum(vm, s) else {
            return Ok(Value::Undefined);
        };
        let id = streaks(world)
            .thrown
            .iter()
            .find(|(_, t)| t.ent == n)
            .map(|(id, _)| *id);
        let now = super::tick(world);
        let now_ms = crate::level_time_ms(now);
        if let Some(id) = id {
            let mut f = frame(world);
            let number = crate::frame::collect_projectiles(f.ecs())
                .into_iter()
                .find(|p| p.id.0 == id)
                .map(|p| p.entnum);
            if let Some(number) = number
                && let Some(p) = f.projectile_mut_by_number(number)
            {
                p.detonate_at_ms = Some(now_ms);
                p.grounded = true;
                crate::equipment::order_script_detonation(f.ecs(), crate::identities::ProjectileId(id));
            }
        }
        Ok(Value::Undefined)
    });
    // A grenade's fuse (the care package marker's smoke waits for it).
    // PLACEHOLDER: the weapon's fuse time is not carried by the engine's
    // weapon facts yet.
    f!("getweaponfusetime", |_, _, _, _| Ok(Value::Int(1000)));
    f!("getweaponprojexplosionsound", |vm, _, _, _| Ok(
        vm.string("")
    ));
    // A weapon with only what is in its clip (the Death Machine): its
    // reserve holds no more than one clip.
    f!("isweaponcliponly", |vm, world, _, a| {
        let w = super::weapon(world, &text(vm, a, 0)).unwrap_or(0);
        let only = frame(world)
            .combat_facts_for(w)
            .is_some_and(|f| f.max_ammo <= f.clip_size);
        Ok(Value::bool(only))
    });
    // Picking a spot on the map (Lightning Strike, Lodestar, Stealth
    // Chopper): BO2's HUD shows the full map with the selector (its
    // material and radius); the pick comes back as `confirm_location`
    // (a bot's own scripts send it themselves), backing out as
    // `cancel_location`.
    fn begin_selection(
        vm: &mut Vm<World>,
        world: &mut World,
        s: &Value,
        a: &[Value],
    ) -> Result<Value, String> {
        let id = super::client(vm, world, s)?;
        let material = text(vm, a, 0);
        let radius = arg(a, 1).as_float().unwrap_or(0.0);
        let mut f = frame(world);
        if f.client_meta(id).is_some() {
            f.client_meta_mut(id).location_selection = Some(crate::LocationSelection {
                material: material.clone(),
                choose_direction: false,
                radius,
            });
        }
        super::set_client_dvar(world, id.0, "bo2mp_locsel", &format!("{material} {radius}"));
        Ok(Value::Undefined)
    }
    m!("beginlocationselection", |vm, world, s, a| begin_selection(
        vm, world, s, a
    ));
    m!("beginlocationmortarselection", |vm, world, s, a| {
        begin_selection(vm, world, s, a)
    });
    m!("beginlocationcomlinkselection", |vm, world, s, a| {
        begin_selection(vm, world, s, a)
    });
    m!("beginlocationairstrikeselection", |vm, world, s, a| {
        begin_selection(vm, world, s, a)
    });
    m!("endlocationselection", |vm, world, s, _| {
        let id = super::client(vm, world, s)?;
        let mut f = frame(world);
        if f.client_meta(id).is_some() {
            f.client_meta_mut(id).location_selection = None;
        }
        super::set_client_dvar(world, id.0, "bo2mp_locsel", "");
        Ok(Value::Undefined)
    });
    // Path nodes a turret or a dropped crate covers (bots keep off them):
    // the engine's bot pathing reads no danger marks yet.
    for name in [
        "setblockweaponpickup",
        "cleardangerous",
        "sethintstringforperk",
        "rotatevelocity",
    ] {
        vm.bind(name, true, |_, _, _, _| Ok(Value::Undefined));
    }
    // Where to aim at someone (`getshootatpos`: a dog's head tracks it):
    // a player's eye, else a body's middle.
    m!("getshootatpos", |vm, world, s, _| {
        if let Ok(id) = super::client(vm, world, s) {
            let f = frame(world);
            let (o, h) = f
                .player(id)
                .map_or(([0.0; 3], 60.0), |ps| (ps.origin, ps.view_height_current));
            return Ok(Value::Vec3([o[0], o[1], o[2] + h]));
        }
        let o = super::origin_of(vm, world, s).unwrap_or([0.0; 3]);
        Ok(Value::Vec3([o[0], o[1], o[2] + 32.0]))
    });
    // A body's eye (`geteye` on an actor: a dog's head tracks its enemy
    // from it): a player's own (on the Lodestar's drone while his view
    // rides it), else a dog's head height over it.
    m!("geteye", |vm, world, s, _| {
        if let Ok(id) = super::client(vm, world, s) {
            if let Some(eye) = view_link::eye(world, id.0) {
                return Ok(Value::Vec3(eye));
            }
            let f = frame(world);
            let (o, h) = f
                .player(id)
                .map_or(([0.0; 3], 60.0), |ps| (ps.origin, ps.view_height_current));
            return Ok(Value::Vec3([o[0], o[1], o[2] + h]));
        }
        let o = super::origin_of(vm, world, s).unwrap_or([0.0; 3]);
        Ok(Value::Vec3([o[0], o[1], o[2] + 24.0]))
    });
    // Which voices it answers (`settalktospecies`): no battle chatter.
    m!("settalktospecies", |_, _, _, _| Ok(Value::Undefined));
    // Whose an actor is (`setentityowner`: a K9 Unit dog never goes for
    // the player who called it).
    m!("setentityowner", |vm, world, s, a| {
        let n = entnum(vm, s).unwrap_or(u32::MAX);
        if let Ok(id) = super::client(vm, world, arg(a, 0))
            && let Some(x) = world.resource_mut::<Zm>().actors.get_mut(&n)
        {
            x.owner = Some(id.0);
        }
        Ok(Value::Undefined)
    });
    // Whose a missile is (`getmissileowner(missile)`: the player who
    // fired it, the same one its `missile_fire` went to), and handing it
    // to someone else (`missile setmissileowner(player)`).
    f!("getmissileowner", |vm, world, _, a| {
        let n = entnum(vm, arg(a, 0)).unwrap_or(u32::MAX);
        let owner = streaks(world)
            .thrown
            .values()
            .find(|t| t.ent == n)
            .map(|t| t.owner);
        Ok(owner.map_or(Value::Undefined, |o| craft::world_obj(world, o)))
    });
    m!("setmissileowner", |vm, world, s, a| {
        let n = entnum(vm, s).unwrap_or(u32::MAX);
        if let Ok(id) = super::client(vm, world, arg(a, 0))
            && let Some(t) = streaks(world).thrown.values_mut().find(|t| t.ent == n)
        {
            t.owner = id.0;
        }
        Ok(Value::Undefined)
    });
    // The K9 Unit's dogs (actors): put somewhere at once (`teleport`), an
    // animation state of their own (`setanimstate`: jumping down a ledge),
    // no forced target.
    m!("teleport", |vm, world, s, a| {
        let Some(n) = entnum(vm, s) else {
            return Ok(Value::Undefined);
        };
        let p = super::vec3(a, 0)?;
        let ang = arg(a, 1).as_vec3();
        if let Some(e) = world.resource_mut::<Zm>().ents.get_mut(&n) {
            e.origin = p;
            if let Some(ang) = ang {
                e.angles = ang;
            }
        }
        Ok(Value::Undefined)
    });
    m!("setanimstate", |vm, world, s, a| {
        let Some(n) = entnum(vm, s).filter(|n| world.resource::<Zm>().actors.contains_key(n))
        else {
            return Ok(Value::Undefined);
        };
        let state = text(vm, a, 0);
        super::actors::set_state(vm, world, n, &state, &Value::Undefined);
        Ok(Value::Undefined)
    });
    m!("clearentitytarget", |_, _, _, _| Ok(Value::Undefined));
    // Client flags on an entity: its client scripts' switches (a
    // counter-UAV's jam, a turret's open or closed, a Hellstorm rider's
    // screen).
    m!("setclientflag", |vm, world, s, a| {
        if let (Some(n), Some(bit)) = (entnum(vm, s), arg(a, 0).as_int().filter(|b| (0..32).contains(b))) {
            let was = {
                let mut st = streaks(world);
                let f = st.client_flags.entry(n).or_default();
                let was = *f & (1 << bit) != 0;
                *f |= 1 << bit;
                was
            };
            // Its client script's flag callback runs when the bit changes.
            if !was {
                super::clientfields::client_flag(world, n, bit, true);
            }
        }
        Ok(Value::Undefined)
    });
    m!("clearclientflag", |vm, world, s, a| {
        if let (Some(n), Some(bit)) = (entnum(vm, s), arg(a, 0).as_int().filter(|b| (0..32).contains(b))) {
            let was = {
                let mut st = streaks(world);
                st.client_flags.get_mut(&n).is_some_and(|f| {
                    let was = *f & (1 << bit) != 0;
                    *f &= !(1 << bit);
                    was
                })
            };
            if was {
                super::clientfields::client_flag(world, n, bit, false);
            }
        }
        Ok(Value::Undefined)
    });
    m!("getclientflag", |vm, world, s, a| {
        let n = entnum(vm, s).unwrap_or(u32::MAX);
        let bit = arg(a, 0).as_int().filter(|b| (0..32).contains(b));
        let flags = streaks(world).client_flags.get(&n).copied().unwrap_or(0);
        Ok(Value::bool(bit.is_some_and(|b| flags & (1 << b) != 0)))
    });
    // An enemy EMP jams him (`setempjammed`, `_emp`'s team tracker): no
    // scorestreaks, no remote rides, a dead minimap. BO2 also sets his
    // EMP-jammed bit, which the client reads.
    m!("setempjammed", |vm, world, s, a| {
        let id = super::client(vm, world, s)?;
        let on = arg(a, 0).as_float().unwrap_or(0.0) != 0.0;
        let mut st = streaks(world);
        if on {
            st.emp_jammed.insert(id.0);
        } else {
            st.emp_jammed.remove(&id.0);
        }
        drop(st);
        if let Some(ps) = frame(world).player_mut(id) {
            let bit = playerstate_t6::game::other_flags::EMP_JAMMED;
            ps.other_flags = if on { ps.other_flags | bit } else { ps.other_flags & !bit };
        }
        Ok(Value::Undefined)
    });
    m!("isempjammed", |vm, world, s, _| {
        let n = entnum(vm, s).unwrap_or(u32::MAX);
        Ok(Value::bool(streaks(world).emp_jammed.contains(&n)))
    });
    // A weapon kept out of a match's first moments (none in a local match).
    f!("isweapondisallowedatmatchstart", |_, _, _, _| Ok(
        Value::Int(0)
    ));
    // A box swept through the world (`physicstrace(start, end, mins, maxs,
    // ignore, mask)`: where an RC-XD or an AGR can be set down): BO2 hands
    // back the trace's facts (fraction, position, normal), not a point.
    f!("physicstrace", |vm, world, _, a| {
        let (s, e) = (super::vec3(a, 0)?, super::vec3(a, 1)?);
        let mins = arg(a, 2).as_vec3().unwrap_or([0.0; 3]);
        let maxs = arg(a, 3).as_vec3().unwrap_or([0.0; 3]);
        let t =
            frame(world).trace_static_world(s, e, mins, maxs, crate::bullet_collision::MASK_SHOT);
        let f = if t.startsolid {
            0.0
        } else {
            t.fraction.clamp(0.0, 1.0)
        };
        let pos: [f32; 3] = std::array::from_fn(|i| s[i] + (e[i] - s[i]) * f);
        let mut arr = gsc_t6::Array::new();
        let k = |vm: &mut Vm<World>, s: &str| gsc_t6::Key::Str(vm.intern(s));
        let surface = vm.string(if f < 1.0 { "default" } else { "none" });
        arr.set(k(vm, "fraction"), Value::Float(f));
        arr.set(k(vm, "position"), Value::Vec3(pos));
        arr.set(
            k(vm, "normal"),
            Value::Vec3(if f < 1.0 { t.normal } else { [0.0; 3] }),
        );
        arr.set(k(vm, "surfacetype"), surface);
        Ok(Value::array(arr))
    });
    // Locking a launcher on a scorestreak (FHJ-18, Stinger): how long a
    // lock takes and how wide its circle is on screen (PLACEHOLDER: the
    // weapon's own lockOnSpeed / lockOnRadius are not in the engine's
    // weapon facts yet), and whether a target sits inside the circle
    // (`target_isincircle(target, player, fov, radius)`: within the angle
    // that radius covers on a 640-wide screen at that field of view).
    m!("getlockonspeed", |_, _, _, _| Ok(Value::Int(1000)));
    m!("getlockonradius", |_, _, _, _| Ok(Value::Int(100)));
    f!("target_isincircle", |vm, world, _, a| {
        let target = arg(a, 0).clone();
        let Ok(id) = super::client(vm, world, arg(a, 1)) else {
            return Ok(Value::Int(0));
        };
        let fov = arg(a, 2).as_float().unwrap_or(65.0);
        let radius = arg(a, 3).as_float().unwrap_or(100.0);
        let (Some(to), Some(ps)) = (
            super::origin_of(vm, world, &target),
            frame(world).player(id).copied(),
        ) else {
            return Ok(Value::Int(0));
        };
        let eye = [
            ps.origin[0],
            ps.origin[1],
            ps.origin[2] + ps.view_height_current,
        ];
        let dir = gsc_t6::math::normalize(gsc_t6::math::sub(to, eye));
        let fwd = gsc_t6::math::angle_vectors(ps.viewangles).0;
        let limit = (radius * (fov.to_radians() * 0.5).tan() / 320.0).atan();
        let angle = gsc_t6::math::dot(dir, fwd).clamp(-1.0, 1.0).acos();
        Ok(Value::bool(angle <= limit))
    });
    // A point turned by angles (`rotatepoint(point, angles)`: the RC-XD's
    // wheels around its spawn, the care package's rear hatch).
    f!("rotatepoint", |_, _, _, a| {
        let p = super::vec3(a, 0)?;
        let ang = arg(a, 1).as_vec3().unwrap_or([0.0; 3]);
        let (f, r, u) = gsc_t6::math::angle_vectors(ang);
        Ok(Value::Vec3(std::array::from_fn(|i| {
            p[0] * f[i] - p[1] * r[i] + p[2] * u[i]
        })))
    });
    // Whether an entity is a vehicle (scorestreak craft count).
    f!("isvehicle", |vm, world, _, a| {
        let n = entnum(vm, arg(a, 0)).unwrap_or(u32::MAX);
        Ok(Value::bool(
            world
                .resource::<Zm>()
                .ents
                .get(&n)
                .is_some_and(|e| e.classname == "script_vehicle"),
        ))
    });
    // Collision contents (a drone that must not block), trigger filters (a
    // turret's hack prompt ignores its turret, wants Hacker), a shield's
    // model, a player's view kick: kept by the scripts' own entities.
    for name in [
        "setcontents",
        "setignoreentfortrigger",
        "setperkfortrigger",
        "viewkick",
        "setweapon",
    ] {
        vm.bind(name, true, |_, _, _, _| Ok(Value::Undefined));
    }
    // A missile decoy (the Escort Drone draws locking missiles to itself)
    // and a drone missile's map mark: kept by the scripts' own entities.
    f!("missile_createattractorent", |vm, world, _, a| pull(vm, world, a, true, true));
    f!("missile_createattractororigin", |vm, world, _, a| pull(vm, world, a, true, false));
    f!("missile_createrepulsorent", |vm, world, _, a| pull(vm, world, a, false, true));
    f!("missile_createrepulsororigin", |vm, world, _, a| pull(vm, world, a, false, false));
    f!("missile_deleteattractor", |_, world, _, a| {
        crate::missile_decoy::delete(world, super::int(a, 0)?)?;
        Ok(Value::Undefined)
    });
    // The rockets this player fires from now fly straight, or wobble again.
    m!("setstablemissile", |vm, world, s, a| {
        let n = entnum(vm, s)
            .filter(|n| world.resource::<Zm>().players.contains_key(n))
            .ok_or("Type should be a player")?;
        crate::missile_decoy::set_stable(world, n, super::int(a, 0)? != 0);
        Ok(Value::Undefined)
    });
    m!("missile_dronesetvisible", |_, _, _, _| Ok(Value::Undefined));
    craft::bind(vm);
    can_damage::bind(vm);
    heli_lock::bind(vm);
    lock_on::bind(vm);
    turret::bind(vm);
    ride::bind(vm);
    view_link::bind(vm);
    vehicle_damage::bind(vm);
}

/// The minimap's frame from BO2's own _compass (`setminimap(material,
/// northwest x, y, southeast x, y)` with the map's north): where a spot
/// picked on the map is.
pub(super) fn set_minimap(
    world: &mut World,
    northwest: [f32; 2],
    southeast: [f32; 2],
    north_yaw: f32,
) {
    streaks(world).minimap =
        Some(hud_t6::compass::mini_map_changed(northwest, southeast, north_yaw));
}

/// A player picked a spot on the map (or backed out): the cursor's place on
/// the minimap (0..255 across, down; direction in 256ths of a turn) as a
/// world position for BO2's scripts, as the engine's own selector reports
/// it.
pub(super) fn location_pick(world: &mut World, client: u32, picked: [u8; 3], confirm: bool) {
    let Some(obj) = world.resource::<Zm>().players.get(&client).map(|p| p.obj) else {
        return;
    };
    if !confirm {
        super::with_vm(world, |vm, world| {
            vm.notify_str(world, obj, "cancel_location", &[])
        });
        return;
    }
    let Some(map) = streaks(world).minimap else {
        diag::warn!(Sim, "bo2mp location: player {client}'s pick dropped: no minimap bounds");
        return;
    };
    let at = |byte: u8| (f32::from(byte as i8) + 128.0) / 255.0;
    let x = at(picked[0]) * map.world_size[0];
    let y = at(picked[1]) * map.world_size[1];
    let location = [
        x * map.north[1] + map.upper_left[0] - y * map.north[0],
        map.upper_left[1] - x * map.north[0] - y * map.north[1],
        0.0,
    ];
    let north_yaw = map.north[1].atan2(map.north[0]).to_degrees();
    let yaw = (f32::from(picked[2]) * (360.0 / 256.0) + north_yaw).rem_euclid(360.0);
    super::with_vm(world, |vm, world| {
        vm.notify_str(
            world,
            obj,
            "confirm_location",
            &[Value::Vec3(location), Value::Float(yaw)],
        )
    });
}

/// The console's `give killstreak <menu name>`: BO2's own givekillstreak
/// with the streak's type (`level.menureferenceforkillstreak`), so it lands
/// in the inventory slot as a care package's streak does.
pub(super) fn give(world: &mut World, client: u32, menu_name: &str) {
    let Some(obj) = world.resource::<Zm>().players.get(&client).map(|p| p.obj) else {
        return;
    };
    super::with_vm(world, |vm, world| {
        let (types, menu) = (vm.intern("menureferenceforkillstreak"), vm.intern(menu_name));
        let kind = match vm.raw_field(vm.level, types) {
            Value::Array(a) => a.get(&gsc_t6::Key::Str(menu)),
            _ => None,
        };
        let Some(kind) = kind else {
            diag::warn!(Sim, "bo2mp streaks: give {menu_name}: no such scorestreak");
            return;
        };
        let started = vm
            .spawn_named(world, "maps/mp/killstreaks/_killstreaks", "givekillstreak", Value::Object(obj), vec![kind])
            .is_some();
        diag::info!(Sim, "bo2mp streaks: give player {client} {menu_name} (started {started})");
    });
}

/// A thrown or fired projectile the scripts hold (`grenade_fire`,
/// `missile_fire`, `magicbullet`): its entity, whose it is, where it was,
/// whether it came to rest.
#[derive(Clone, Debug)]
pub(crate) struct Thrown {
    ent: u32,
    owner: u32,
    origin: [f32; 3],
    stationary: bool,
    /// It has told the scripts it stuck to a player.
    stuck: bool,
}

/// The vehicle a missile passes (BO2's `G_RunMissileInternal`): the one
/// its owner rides, so a gunner's rocket leaves from inside it; none on a
/// tank, or when he rides nothing. `owner` is asked only while someone
/// rides.
pub(crate) fn missile_passes(
    world: &World,
    owner: impl FnOnce() -> Option<u32>,
) -> Option<crate::ScriptModelId> {
    let rides = &world.get_resource::<Streaks>()?.rides;
    if rides.is_empty() {
        return None;
    }
    let ride = rides.get(&owner()?)?;
    if !craft::passes_own_vehicle(ride.info.vehicle_type) {
        return None;
    }
    Some(world.get_resource::<Zm>()?.presences.by_ent.get(&ride.vehicle)?.id)
}

/// BO2's `grenade launch(vel [, avel])` on a projectile the scripts hold
/// (the decoy's hops, a stuck item dropped off a player): it flies again
/// from where it is, at `vel` under gravity, turning at `avel`, lets go of
/// whatever it stuck to, and comes to rest (`stationary`) anew. False when
/// `ent` is none of them.
pub(crate) fn relaunch(world: &mut World, ent: u32, vel: [f32; 3], avel: Option<[f32; 3]>) -> bool {
    let Some(id) = streaks(world)
        .thrown
        .iter()
        .find(|(_, t)| t.ent == ent)
        .map(|(id, _)| *id)
    else {
        return false;
    };
    let now_ms = crate::level_time_ms(super::tick(world));
    let mut f = frame(world);
    let number = crate::frame::collect_projectiles(f.ecs())
        .into_iter()
        .find(|p| p.id.0 == id)
        .map(|p| p.entnum);
    let Some(p) = number.and_then(|n| f.projectile_mut_by_number(n)) else {
        return false;
    };
    p.velocity = vel;
    p.pos = entity_t6::Trajectory {
        tr_type: entity_t6::trajectory::tr_type::GRAVITY,
        tr_time: now_ms,
        tr_duration: 0,
        tr_base: p.origin,
        tr_delta: vel,
    };
    if let Some(avel) = avel {
        p.apos = entity_t6::Trajectory {
            tr_type: entity_t6::trajectory::tr_type::LINEAR,
            tr_time: now_ms,
            tr_duration: 0,
            tr_base: entity_t6::evaluate_trajectory(&p.apos, now_ms),
            tr_delta: avel,
        };
    }
    p.grounded = false;
    p.rider = None;
    p.stuck_pane = None;
    if let Some(t) = streaks(world).thrown.get_mut(&id) {
        t.stationary = false;
        t.stuck = false;
    }
    true
}

/// A script entity standing for something the engine draws itself (a
/// projectile): hidden, not solid.
fn adopt(
    vm: &mut Vm<World>,
    world: &mut World,
    origin: [f32; 3],
    classname: &str,
) -> (u32, gsc_t6::ObjRef) {
    let ent = world.resource_mut::<Zm>().alloc_entnum();
    let obj = vm.alloc_object(gsc_t6::ObjKind::Entity(ent));
    world.resource_mut::<Zm>().ents.insert(
        ent,
        super::Ent {
            obj: Some(obj),
            classname: classname.to_owned(),
            origin,
            hidden: true,
            ..Default::default()
        },
    );
    (ent, obj)
}

/// `radiusdamage(origin, range, max, min, attacker, means, weapon)`.
fn radius_damage(
    vm: &mut Vm<World>,
    world: &mut World,
    inflictor: Option<Value>,
    a: &[Value],
) -> Result<Value, String> {
    let origin = super::vec3(a, 0)?;
    let range = super::num(a, 1)?.max(1.0);
    let (max, min) = (
        super::num(a, 2).unwrap_or(0.0),
        super::num(a, 3).unwrap_or(0.0),
    );
    let attacker = arg(a, 4).clone();
    let means = match arg(a, 5) {
        Value::Undefined => "MOD_EXPLOSIVE".to_owned(),
        v => vm.to_text(v),
    };
    let weapon = match arg(a, 6) {
        Value::Undefined => "none".to_owned(),
        v => vm.to_text(v),
    };
    let inflictor = inflictor.unwrap_or_else(|| attacker.clone());
    let sees = |world: &mut World, to: [f32; 3]| {
        frame(world)
            .trace_static_world(
                origin,
                to,
                [0.0; 3],
                [0.0; 3],
                crate::bullet_collision::MASK_SHOT,
            )
            .fraction
            >= 1.0
    };
    let amount = |d: f32| (max - (max - min) * (d / range)).round() as i32;
    let dist = |p: [f32; 3]| {
        ((p[0] - origin[0]).powi(2) + (p[1] - origin[1]).powi(2) + (p[2] - origin[2]).powi(2))
            .sqrt()
    };
    // Players: aimed at their middle.
    let players: Vec<u32> = world.resource::<Zm>().players.keys().copied().collect();
    for n in players {
        let Some(ps) = frame(world).player(ClientId(n)).copied() else {
            continue;
        };
        if ps.health <= 0 {
            continue;
        }
        let mid = [ps.origin[0], ps.origin[1], ps.origin[2] + 32.0];
        let d = dist(mid);
        if d > range || !sees(world, mid) {
            continue;
        }
        let dir = gsc_t6::math::normalize(gsc_t6::math::sub(mid, origin));
        // The riot shield he bears takes its share (BO2's radius damage).
        let shield = frame(world).shield_blast_protection(ClientId(n), &ps, origin);
        if shield > 0.0 {
            let t = super::tick(world);
            crate::damage::shield_splash_event(&mut frame(world), t, ClientId(n), ps.origin, amount(d));
        }
        let hurt = if shield > 0.0 {
            (amount(d) as f32 * (1.0 - shield)) as i32
        } else {
            amount(d)
        };
        if hurt <= 0 {
            continue;
        }
        if std::env::var_os("BO2MP_SHIELDLOG").is_some() && shield > 0.0 {
            diag::info!(Sim, "bo2mp shield: blast {} at {origin:?} on {n}: shield stops {shield}, {} of {}", means, hurt, amount(d));
        }
        super::natives_ai::player_damage(
            vm,
            world,
            n,
            inflictor.clone(),
            attacker.clone(),
            hurt,
            0,
            &means,
            &weapon,
            origin,
            dir,
            "none",
        );
    }
    // Actors (dogs) and entities that take damage (turrets, crates).
    let ents: Vec<(u32, [f32; 3], bool, Option<gsc_t6::ObjRef>)> = {
        let zm = world.resource::<Zm>();
        zm.ents
            .iter()
            .filter(|(n, e)| e.can_damage || zm.actors.contains_key(n))
            .map(|(n, e)| (*n, e.origin, zm.actors.contains_key(n), e.obj))
            .collect()
    };
    for (n, at, actor, obj) in ents {
        let d = dist(at);
        if d > range || inflictor.as_obj() == obj {
            continue;
        }
        if actor {
            super::actors::damage(
                vm,
                world,
                n,
                inflictor.clone(),
                attacker.clone(),
                amount(d),
                0,
                &means,
                &weapon,
                origin,
                [0.0; 3],
                "none",
            );
        } else if vehicle_damage::is_vehicle(world, n) {
            vehicle_damage::blast(
                vm,
                world,
                n,
                inflictor.clone(),
                attacker.clone(),
                origin,
                range,
                max,
                min,
                None,
                &means,
                &weapon,
            );
        } else if let Some(o) = obj {
            let (m, w, none) = (vm.string(&means), vm.string(&weapon), vm.string(""));
            let args = [
                Value::Int(amount(d)),
                attacker.clone(),
                Value::Vec3(gsc_t6::math::normalize(gsc_t6::math::sub(at, origin))),
                Value::Vec3(origin),
                m,
                none.clone(),
                none.clone(),
                none,
                w,
                Value::Int(0),
            ];
            vm.notify_str(world, o, "damage", &args);
        }
    }
    // bo2mp: the map's shoot-to-use triggers (an ad, a drum).
    super::damage_triggers::blast_in(
        vm,
        world,
        origin,
        range,
        max,
        min,
        &attacker,
        &means,
        &weapon,
        inflictor.as_obj(),
        &|_| true,
    );
    // codmp: and the map's breakables (a car's blast sets off the barrel
    // beside it), by the share unrounded as BO2's G_RadiusDamage has it.
    let blast_at = |d: f32| max - (max - min) * (d / range);
    super::destructible::script_blast(
        world, origin, range, blast_at, &means, &inflictor, &attacker, &weapon,
    );
    // And the panes (BO2's G_RadiusDamage ends with GlassSv_RadiusDamage).
    super::glass::script_blast(world, origin, range, max, min, &means);
    // BO2's breakables hear it before the caller's next line.
    super::destructible::flush_now(vm, world);
    Ok(Value::Undefined)
}

/// Each authority frame (before the scripts run): projectiles players threw
/// or fired become script entities (BO2's engine hands them to the scripts:
/// the care package marker, a launcher's rocket) that tell the scripts when
/// they come to rest (`stationary`) and go off (`explode`, `death`);
/// homing missiles steer; dropped crates fall; usable models are used;
/// helicopters fly; the radar state goes to each player's HUD.
pub(super) fn advance(world: &mut World) {
    birth(world);
    crate::missile_decoy::sweep(world);
    lock_on::advance(world);
    projectiles(world);
    falling(world);
    use_models(world);
    ride::advance(world);
    view_link::advance(world);
    missile_hud_end(world);
    missile_boost(world);
    craft::advance(world);
    turret::advance(world);
    publish_radar(world);
    hand_log(world);
}

/// The Hellstorm screen closes when the ride is over, however the engine
/// let go of the missile (it hit, it was shot, the rider died): the HUD
/// hears `bo2mp_vehicle` go empty, as `player_missile_end` ends the screen.
fn missile_hud_end(world: &mut World) {
    let over: Vec<u32> = {
        let f = frame(world);
        f.client_ids_sorted()
            .into_iter()
            .filter(|&id| {
                f.client_meta(id).is_some_and(|m| {
                    m.client_dvars
                        .iter()
                        .any(|(k, v)| k == "bo2mp_vehicle" && v.starts_with("remote_missile_mp"))
                        && !m.remote_missile.is_some_and(|l| l.unlink_at_ms.is_none())
                })
            })
            .map(|id| id.0)
            .collect()
    };
    for id in over {
        clear_vehicle_hud(world, id, "remote_missile_mp");
    }
}

/// The angles that look along `dir`, BO2's way (its vectoangles): a
/// dead-vertical one looks straight up or straight down. The shared MW2 math
/// turns those over, so a gun or missile aimed straight down would look up.
pub(super) fn aim_angles(dir: [f32; 3]) -> [f32; 3] {
    if dir[0] == 0.0 && dir[1] == 0.0 {
        return [if dir[2] > 0.0 { 270.0 } else { 90.0 }, 0.0, 0.0];
    }
    game_math::vect_to_angles(dir)
}

/// A Hellstorm's rider hears `missile_boost` once it boosts (its scripts'
/// missile_sound_boost plays the roar).
fn missile_boost(world: &mut World) {
    let links: Vec<(u32, u32, bool)> = {
        let f = frame(world);
        f.client_ids_sorted()
            .into_iter()
            .filter_map(|c| {
                let l = f.client_meta(c)?.remote_missile?;
                l.t6.then_some((c.0, l.projectile.0, l.boosted))
            })
            .collect()
    };
    let mut events = Vec::new();
    let mut st = streaks(world);
    st.boost_told.retain(|p| links.iter().any(|(_, q, _)| q == p));
    for (client, projectile, boosted) in links {
        if boosted && st.boost_told.insert(projectile) {
            events.push((client, "missile_boost", Vec::new()));
        }
    }
    drop(st);
    notify_all(world, events);
}

/// A new entity's `birthtime`: the server time it came (the spy plane
/// scripts compare it with its owner's spawn time).
fn birth(world: &mut World) {
    let from = streaks(world).born_upto;
    let (to, now) = {
        let zm = world.resource::<Zm>();
        (zm.next_entnum, zm.now_ms)
    };
    if to <= from {
        return;
    }
    let objs: Vec<gsc_t6::ObjRef> = world
        .resource::<Zm>()
        .ents
        .range(from..to)
        .filter_map(|(_, e)| e.obj)
        .collect();
    super::with_vm(world, |vm, _| {
        let f = vm.intern("birthtime");
        for o in objs {
            if matches!(vm.raw_field(o, f), Value::Undefined) {
                vm.set_raw_field(o, f, Value::Int(now as i32));
            }
        }
    });
    streaks(world).born_upto = to;
}

/// Notify players or entities (by entity number) in one go.
fn notify_all(world: &mut World, events: Vec<(u32, &'static str, Vec<Value>)>) {
    if events.is_empty() {
        return;
    }
    super::with_vm(world, |vm, world| {
        for (ent, name, args) in events {
            let zm = world.resource::<Zm>();
            let obj = zm
                .players
                .get(&ent)
                .map(|p| p.obj)
                .or_else(|| zm.ents.get(&ent).and_then(|e| e.obj));
            if let Some(o) = obj
                && vm.alive(o)
            {
                vm.notify_str(world, o, name, &args);
            }
        }
    });
}

/// The longest a ridden missile's object waits for its ride's script to
/// end (it needs a quarter second of fade): a safety, not a rule.
const RIDE_END_MS: i32 = 3000;

fn projectiles(world: &mut World) {
    let live: Vec<crate::ProjectileState> = crate::frame::collect_projectiles(world)
        .into_iter()
        .filter(|p| p.live)
        .collect();
    let mut known: BTreeMap<u32, Thrown> = std::mem::take(&mut streaks(world).thrown);
    let mut keep: BTreeMap<u32, Thrown> = BTreeMap::new();
    // Ballistic knife blades that landed: they go with no blast.
    let mut quiet: Vec<u32> = Vec::new();
    let mut events: Vec<(u32, &'static str, Vec<Value>)> = Vec::new();
    for p in &live {
        let id = p.id.0;
        if let Some(mut t) = known.get(&id).cloned() {
            // The scripts deleted it (the Hunter Killer's thrown case): it
            // is gone from the world too.
            if !world.resource::<Zm>().ents.contains_key(&t.ent) {
                frame(world).remove_projectile_by_number(p.entnum);
                continue;
            }
            t.origin = p.origin;
            if let Some(e) = world.resource_mut::<Zm>().ents.get_mut(&t.ent) {
                e.origin = p.origin;
                // Its facing (the Hunter Killer looks for targets ahead of it).
                e.angles = p.apos.tr_base;
            }
            // BO2 notifies `stationary` once it is at rest, not at its first
            // touch of the floor (BounceMissile).
            if !t.stationary && p.pos.tr_type == entity_t6::trajectory::tr_type::STATIONARY {
                // A ballistic knife's blade tells where and what it struck,
                // then goes quietly (BO2's createRetrieveableProjectile).
                match super::knife::landed(world, id, t.owner) {
                    super::knife::Landed::Now(args) => {
                        events.push((t.ent, "stationary", args));
                        frame(world).remove_projectile_by_number(p.entnum);
                        known.remove(&id);
                        quiet.push(t.ent);
                        continue;
                    }
                    super::knife::Landed::Wait => {}
                    super::knife::Landed::No => {
                        t.stationary = true;
                        events.push((t.ent, "stationary", Vec::new()));
                    }
                }
            }
            // Stuck to a player (BO2's StickMissile): `stuck_to_player` with
            // him, and on his shield `stuck_to_shield` with him and its owner
            // too (_weapons.gsc checkstucktoplayer, checkstucktoshield).
            if !t.stuck
                && let Some(r) = p.rider
            {
                t.stuck = true;
                let zm = world.resource::<Zm>();
                let him = zm.players.get(&r.client.0).map(|q| Value::Object(q.obj));
                let owner = zm.players.get(&t.owner).map(|q| Value::Object(q.obj));
                if let Some(him) = him {
                    events.push((t.ent, "stuck_to_player", vec![him.clone()]));
                    if r.shield {
                        events.push((t.ent, "stuck_to_shield", vec![him, owner.unwrap_or(Value::Undefined)]));
                    }
                }
            }
            keep.insert(id, t);
            continue;
        }
        let kind = frame(world)
            .combat_facts_for(p.weapon)
            .map_or(-1, |f| f.weap_type);
        let (classname, notify) = match u8::try_from(kind) {
            Ok(weapon_t6::facts::weap_type::GRENADE) => ("grenade", "grenade_fire"),
            Ok(weapon_t6::facts::weap_type::PROJECTILE) => ("rocket", "missile_fire"),
            _ => continue,
        };
        if !world.resource::<Zm>().players.contains_key(&p.owner.0) {
            continue;
        }
        let name = super::weapon_text(world, p.weapon);
        let made = super::with_vm(world, |vm, world| {
            let (ent, obj) = adopt(vm, world, p.origin, classname);
            let w = vm.string(&name);
            (ent, obj, w)
        });
        if let Some((ent, obj, w)) = made {
            keep.insert(
                id,
                Thrown {
                    ent,
                    owner: p.owner.0,
                    origin: p.origin,
                    stationary: false,
                    stuck: false,
                },
            );
            // A missile fired on a lock homes on its target (BO2's
            // FireWeapon), at the offset the target was set with.
            if classname == "rocket"
                && let Some(target) = lock_on::locked_target(world, p.owner.0, p.weapon)
            {
                let offset = streaks(world).targets.get(&target).copied().unwrap_or([0.0; 3]);
                streaks(world).homing.insert(ent, (target, offset));
            }
            events.push((p.owner.0, notify, vec![Value::Object(obj), w]));
        }
    }
    // Gone: it went off where it last was.
    let mut gone = quiet;
    for (id, t) in &known {
        if keep.contains_key(id) {
            continue;
        }
        diag::info!(Sim, "bo2mp streaks: projectile {id} (ent {}) went off at {:?}", t.ent, t.origin);
        // A coasting missile goes quietly: BO2 sends it only `death`.
        if !streaks(world).coasting.remove(&t.ent) {
            events.push((t.ent, "explode", vec![Value::Vec3(t.origin)]));
        }
        events.push((t.ent, "death", Vec::new()));
        super::clientfields::map_icon_died(world, t.ent);
        // A missile someone rides stays an object until his ride's script
        // is done with it: `watch_missile_death` ends on `deleted`, yet it is
        // `death` that starts `player_missile_end` (the fade, `hide`,
        // `unlinkfrommissile`), which waits a quarter second (_remotemissile.gsc).
        let ridden = {
            let f = frame(world);
            f.client_ids_sorted().into_iter().any(|c| {
                f.client_meta(c)
                    .and_then(|m| m.remote_missile)
                    .is_some_and(|l| l.projectile.0 == *id && l.unlink_at_ms.is_none())
            })
        };
        if ridden {
            let until = crate::level_time_ms(super::tick(world)) + RIDE_END_MS;
            streaks(world).late_free.insert(t.ent, until);
        } else {
            gone.push(t.ent);
        }
    }
    // The ridden ones go once their rider has let go (or time is up).
    {
        let now = crate::level_time_ms(super::tick(world));
        let held: Vec<u32> = streaks(world).late_free.keys().copied().collect();
        for ent in held {
            let riding = {
                let f = frame(world);
                f.client_ids_sorted().into_iter().any(|c| {
                    f.client_meta(c)
                        .and_then(|m| m.remote_missile)
                        .is_some_and(|l| l.unlink_at_ms.is_none())
                })
            };
            let until = streaks(world).late_free.get(&ent).copied().unwrap_or(0);
            if !riding || now >= until {
                streaks(world).late_free.remove(&ent);
                gone.push(ent);
            }
        }
    }
    streaks(world).thrown = keep;
    // Homing missiles turn toward their target (BO2's GetTargetPosition:
    // its origin plus the offset in its own axes).
    let homing: Vec<(u32, (u32, [f32; 3]))> = streaks(world)
        .homing
        .iter()
        .map(|(m, t)| (*m, *t))
        .collect();
    for (missile, (target, offset)) in homing {
        let id = streaks(world)
            .thrown
            .iter()
            .find(|(_, t)| t.ent == missile)
            .map(|(id, _)| *id);
        let Some(id) = id else {
            streaks(world).homing.remove(&missile);
            streaks(world).launched_at.remove(&missile);
            continue;
        };
        let Some(to) = aim_point(world, target, offset) else {
            continue;
        };
        let number = live.iter().find(|p| p.id.0 == id).map(|p| p.entnum);
        let searching = streaks(world).launched_at.get(&missile) == Some(&target);
        if let Some(number) = number
            && let Some(p) = frame(world).projectile_mut_by_number(number)
        {
            p.guide.target = Some(crate::MissileTarget::Point(to));
            if searching {
                p.guide.home = Some(crate::MissileTarget::Point(to));
            }
            p.guide.offset = offset;
        }
    }
    notify_all(world, events);
    for ent in gone {
        let obj = world
            .resource_mut::<Zm>()
            .ents
            .remove(&ent)
            .and_then(|e| e.obj);
        streaks(world).homing.remove(&ent);
        streaks(world).launched_at.remove(&ent);
        if let Some(o) = obj {
            super::with_vm(world, |vm, world| vm.free_object(world, o));
        }
    }
}

/// Where a missile homing on an entity aims (BO2's GetTargetPosition): its
/// origin plus the offset in its own axes (a player's: his facing alone).
fn aim_point(world: &mut World, target: u32, offset: [f32; 3]) -> Option<[f32; 3]> {
    let at = if world.resource::<Zm>().players.contains_key(&target) {
        frame(world)
            .player(ClientId(target))
            .map(|ps| (ps.origin, [0.0, ps.viewangles[1], 0.0]))
    } else {
        world
            .resource::<Zm>()
            .ents
            .get(&target)
            .map(|e| (e.origin, e.angles))
    };
    let (origin, angles) = at?;
    let axis = game_math::angles_to_axis(angles);
    Some(core::array::from_fn(|i| {
        origin[i] + offset[0] * axis[0][i] + offset[1] * axis[1][i] + offset[2] * axis[2][i]
    }))
}

/// The box a falling crate traces with: its model's bounds (lying flat),
/// else 32 x 32 x 16 standing on its origin.
fn fall_box(world: &mut World, n: u32) -> ([f32; 3], [f32; 3]) {
    let model = world
        .resource::<Zm>()
        .ents
        .get(&n)
        .map(|e| e.model.clone())
        .unwrap_or_default();
    match frame(world)
        .model_capability(&model)
        .flatten()
        .and_then(|c| c.bounds)
    {
        Some((mid, half)) => (
            std::array::from_fn(|i| mid[i] - half[i]),
            std::array::from_fn(|i| mid[i] + half[i]),
        ),
        None => ([-16.0, -16.0, 0.0], [16.0, 16.0, 16.0]),
    }
}

/// Dropped crates fall at BO2's gravity (800 units/s²) until they hit the
/// world, then settle upright: `stationary`.
fn falling(world: &mut World) {
    let dt = crate::MATCH_TICK_MS as f32 / 1000.0;
    let mut events = Vec::new();
    // BO2's physics says `stationary` once the body has gone to sleep, after
    // it has lain still a while, not on the frame it lands: the script's
    // update_crate_velocity (every 0.05 s, ended by `stationary`) must read
    // it still, or cratekill keeps its fall speed and crushes whoever walks
    // up to the crate (bots died taking it, 10-15).
    let settling: Vec<(u32, u32)> = streaks(world)
        .settling
        .iter()
        .map(|(n, t)| (*n, *t))
        .collect();
    for (n, left) in settling {
        if left <= crate::MATCH_TICK_MS {
            streaks(world).settling.remove(&n);
            events.push((n, "stationary", Vec::new()));
        } else {
            streaks(world).settling.insert(n, left - crate::MATCH_TICK_MS);
        }
    }
    let list: Vec<(u32, [f32; 3])> = streaks(world)
        .falling
        .iter()
        .map(|(n, v)| (*n, *v))
        .collect();
    for (n, mut v) in list {
        let Some(pos) = world.resource::<Zm>().ents.get(&n).map(|e| e.origin) else {
            streaks(world).falling.remove(&n);
            continue;
        };
        v[2] -= 800.0 * dt;
        let to: [f32; 3] = std::array::from_fn(|i| pos[i] + v[i] * dt);
        // Its own model's box: the crate's origin is its middle, so a box
        // from the origin up left its lower half in the ground.
        let (lo, hi) = fall_box(world, n);
        let t = frame(world).trace_static_world(
            pos,
            to,
            lo,
            hi,
            crate::bullet_collision::MASK_SHOT,
        );
        let f = t.fraction.clamp(0.0, 1.0);
        let mut at: [f32; 3] = std::array::from_fn(|i| pos[i] + (to[i] - pos[i]) * f);
        // Caught on something thinner than itself (a lamp post, a sign): it
        // tips off the side it does not rest on and falls on, as BO2's
        // physics drops it (crates stood on Nuketown's posts in mid-air).
        let mut tipped = false;
        if f < 1.0 && !t.startsolid {
            let mut away = [0.0f32; 2];
            let mut held = 0;
            // Its four bottom corners, a little in from the edges.
            let (cx, cy) = (hi[0].min(-lo[0]) - 2.0, hi[1].min(-lo[1]) - 2.0);
            for (dx, dy) in [(-cx, -cy), (cx, -cy), (-cx, cy), (cx, cy)] {
                let from = [at[0] + dx, at[1] + dy, at[2] + lo[2] + 1.0];
                let under = frame(world).trace_static_world(
                    from,
                    [from[0], from[1], from[2] - 12.0],
                    [0.0; 3],
                    [0.0; 3],
                    crate::bullet_collision::MASK_SHOT,
                );
                if under.fraction < 1.0 {
                    held += 1;
                } else {
                    away[0] += dx;
                    away[1] += dy;
                }
            }
            let len = (away[0] * away[0] + away[1] * away[1]).sqrt();
            if held < 3 && len > 0.0 {
                let side = [
                    at[0] + away[0] / len * 12.0,
                    at[1] + away[1] / len * 12.0,
                    at[2] + 1.0,
                ];
                let s = frame(world).trace_static_world(
                    [at[0], at[1], at[2] + 1.0],
                    side,
                    lo,
                    hi,
                    crate::bullet_collision::MASK_SHOT,
                );
                if !s.startsolid && s.fraction > 0.0 {
                    at = std::array::from_fn(|i| {
                        let start = if i == 2 { at[2] + 1.0 } else { at[i] };
                        start + (side[i] - start) * s.fraction.clamp(0.0, 1.0)
                    });
                    tipped = true;
                    v = [0.0, 0.0, v[2].min(0.0)];
                }
            }
        }
        if let Some(e) = world.resource_mut::<Zm>().ents.get_mut(&n) {
            e.origin = at;
            if f < 1.0 && !tipped {
                e.angles = [0.0, e.angles[1], 0.0];
            }
        }
        if (f < 1.0 && !tipped) || at[2] < -20000.0 {
            streaks(world).falling.remove(&n);
            // PLACEHOLDER: BO2's sleep time is inside its physics solver,
            // not found; 0.3 s still covers several of the script's samples.
            streaks(world).settling.insert(n, 300);
        } else {
            streaks(world).falling.insert(n, v);
        }
    }
    notify_all(world, events);
}

/// The hint of the nearest usable model within his reach (a care package's
/// "Hold [Use] to capture"), as BO2 shows a trigger's.
pub(super) fn usable_hint(world: &World, at: [f32; 3]) -> Option<String> {
    let st = world.get_resource::<Streaks>()?;
    let zm = world.resource::<Zm>();
    st.usable
        .iter()
        .filter_map(|n| {
            let e = zm.ents.get(n)?;
            let d = ((e.origin[0] - at[0]).powi(2) + (e.origin[1] - at[1]).powi(2) + (e.origin[2] - at[2]).powi(2))
                .sqrt();
            (d <= 128.0).then(|| Some((d, e.hint.clone()?))).flatten()
        })
        .min_by(|a, b| a.0.total_cmp(&b.0))
        .map(|(_, h)| h)
}

/// Usable models: a living player pressing use within reach (BO2's
/// player_useRadius, 128) fires the nearest one's `trigger`.
fn use_models(world: &mut World) {
    let usable: Vec<u32> = streaks(world).usable.iter().copied().collect();
    if usable.is_empty() {
        return;
    }
    let players: Vec<(u32, gsc_t6::ObjRef)> = world
        .resource::<Zm>()
        .players
        .iter()
        .filter(|(_, p)| p.sessionstate == "playing")
        .map(|(c, p)| (*c, p.obj))
        .collect();
    let mut events = Vec::new();
    for (c, pobj) in players {
        let held = crate::script_player::buttons(&mut frame(world), ClientId(c))
            & (playerstate_t6::game::buttons::USE | playerstate_t6::game::buttons::USE_RELOAD)
            != 0;
        let pressed = held && !world.resource::<Zm>().use_held.contains(&c);
        if held && std::env::var_os("BO2MP_KSLOG").is_some() {
            diag::info!(
                Sim,
                "bo2mp kslog use {c}: held, pressed {pressed}, {} usable",
                usable.len()
            );
        }
        if !pressed {
            continue;
        }
        let Some(me) = frame(world).player(ClientId(c)).map(|ps| ps.origin) else {
            continue;
        };
        let best = usable
            .iter()
            .filter_map(|n| {
                let e = world.resource::<Zm>().ents.get(n)?;
                let d = ((e.origin[0] - me[0]).powi(2)
                    + (e.origin[1] - me[1]).powi(2)
                    + (e.origin[2] - me[2]).powi(2))
                .sqrt();
                (d <= 128.0).then_some((d, *n))
            })
            .min_by(|a, b| a.0.total_cmp(&b.0));
        if let Some((_, n)) = best {
            events.push((n, "trigger", vec![Value::Object(pobj)]));
        }
    }
    notify_all(world, events);
}

/// A player's radar as his HUD hears it.
struct RadarView {
    client: u32,
    team: String,
    /// Free-for-all: his own spy plane mode and satellite (`hasspyplane`,
    /// `hassatellite`), which `_spyplane` keeps on him instead of his team.
    own: Option<(i32, i32)>,
    /// Hardwired (`specialty_immunecounteruav`): an enemy Counter-UAV does
    /// not jam him (BO2's `_scrambler.csc` skips `setenemyglobalscrambler`).
    hardwired: bool,
}

/// Each player's HUD hears his radar (`bo2mp_radar`: spy plane mode,
/// satellite, jammed, free-for-all), as BO2's minimap reads it. Team games
/// keep it by team (`setteamspyplane`, `setteamsatellite`); a free-for-all
/// keeps it on each player, and every counter-UAV but his own jams him.
fn publish_radar(world: &mut World) {
    if world.resource::<Zm>().now_ms % 500 != 0 {
        return;
    }
    // An entity's client flags go with it (its number comes back as
    // something else).
    let live: BTreeSet<u32> = {
        let zm = world.resource::<Zm>();
        zm.ents.keys().chain(zm.players.keys()).copied().collect()
    };
    streaks(world).client_flags.retain(|n, _| live.contains(n));
    let (views, cuav) = super::with_vm(world, |vm, world| {
        let (team, owner) = (vm.intern("team"), vm.intern("owner"));
        let (teambased, spy, sat) = (vm.intern("teambased"), vm.intern("hasspyplane"), vm.intern("hassatellite"));
        let ffa = !gsc_t6::truthy(&vm.raw_field(vm.level, teambased));
        let zm = world.resource::<Zm>();
        let views: Vec<RadarView> = zm
            .players
            .iter()
            .map(|(c, p)| RadarView {
                client: *c,
                team: vm.to_text(&vm.raw_field(p.obj, team)),
                own: ffa.then(|| {
                    let n = |f| vm.raw_field(p.obj, f).as_int().unwrap_or(0);
                    (n(spy), n(sat))
                }),
                hardwired: p.perks.contains("specialty_immunecounteruav"),
            })
            .collect();
        // Counter-UAVs jamming: their team and owner. A plane carries the
        // jam flag from its arrival until it turns to leave (cleared before
        // its 10 s exit).
        let flags = &world.resource::<Streaks>().client_flags;
        let cuav: Vec<(String, Option<u32>)> = zm
            .ents
            .iter()
            .filter(|(n, _)| flags.get(n).is_some_and(|f| f & (1 << FLAG_COUNTERUAV) != 0))
            .filter_map(|(_, e)| e.obj)
            .map(|o| (vm.to_text(&vm.raw_field(o, team)), entnum(vm, &vm.raw_field(o, owner))))
            .collect();
        (views, cuav)
    })
    .unwrap_or_default();
    for v in views {
        let (uav, sat) = v.own.unwrap_or_else(|| {
            let st = streaks(world);
            (
                st.spyplane.get(&v.team).copied().unwrap_or(0),
                st.satellite.get(&v.team).copied().unwrap_or(0),
            )
        });
        let pulse = streaks(world).radar_pulse.get(&v.client).copied().unwrap_or(0);
        let emp = streaks(world).emp_jammed.contains(&v.client);
        let jammed = i32::from(
            emp || !v.hardwired && if v.own.is_some() {
                cuav.iter().any(|(_, o)| *o != Some(v.client))
            } else {
                cuav.iter().any(|(t, _)| *t != v.team)
            },
        );
        let ffa = i32::from(v.own.is_some());
        super::set_client_dvar(world, v.client, "bo2mp_radar", &format!("{uav} {sat} {jammed} {ffa} {pulse}"));
    }
}

/// BO2MP_KSLOG=1 (test aid): every second, each living player's weapon, its
/// state and what the scripts asked him to switch to.
fn hand_log(world: &mut World) {
    if std::env::var_os("BO2MP_KSLOG").is_none() {
        return;
    }
    let now = world.resource::<Zm>().now_ms;
    if now - streaks(world).log_at < 1000 {
        return;
    }
    streaks(world).log_at = now;
    let players: Vec<u32> = world.resource::<Zm>().players.keys().copied().collect();
    for c in players {
        let Some(ps) = frame(world).player(ClientId(c)).copied() else {
            continue;
        };
        if ps.health <= 0 {
            continue;
        }
        let name = super::weapon_text(world, ps.weapon);
        let switch_to = frame(world)
            .client_meta(ClientId(c))
            .map_or(0, |m| m.controls.switch_to);
        let to = super::weapon_text(world, switch_to);
        let facts = frame(world).combat_facts_for(ps.weapon).map(|f| {
            (
                f.weap_type,
                f.weap_class,
                f.fire_type,
                f.fire_time_ms,
                f.fire_delay_ms,
                f.clip_size,
            )
        });
        let held = crate::script_player::buttons(&mut frame(world), ClientId(c));
        diag::info!(
            Sim,
            "bo2mp kslog {c}: holds {name} state {} time {} delay {} switch_to {to} slots {:?} {:?} facts(type class fire time delay clip) {facts:?} buttons {held:#x}",
            ps.weapon_state_t6,
            ps.weapon_time,
            ps.weapon_delay,
            ps.action_slot_type,
            ps.action_slot_param
        );
    }
}

fn send_earthquake(world: &mut World, audience: crate::EventAudience, quake: crate::EntityEventPayload) {
    let t = super::tick(world);
    frame(world).push_entity_event(t, audience, entity_t6::EntityEventKind::EARTHQUAKE, quake);
}

/// bo2mp: an earthquake the engine's own code starts, as a client script's
/// `earthquake(scale, seconds, origin, radius)` (Express's train cars shake
/// the view near them while they move: mp_express.csc's
/// `train_move_think`, 0.2 for 0.25 s within 512, every 0.05 s). Everyone
/// within the radius feels it. A scale, length or radius not above 0 is
/// refused, as BO2's.
#[allow(dead_code)] // Express's train (on the maps branch until the sync) calls it
pub(super) fn shake(world: &mut World, scale: f32, seconds: f32, origin: [f32; 3], radius: f32) -> Result<(), String> {
    let quake = earthquake(scale, seconds, origin, radius)?;
    send_earthquake(world, crate::EventAudience::All, quake);
    Ok(())
}

/// bo2mp: `shake` that only client `client` feels (a client script run on
/// one player's machine: the AGR driver's rumble).
pub(super) fn shake_to(
    world: &mut World,
    client: u32,
    scale: f32,
    seconds: f32,
    origin: [f32; 3],
    radius: f32,
) -> Result<(), String> {
    let quake = earthquake(scale, seconds, origin, radius)?;
    send_earthquake(world, crate::EventAudience::Client(crate::world::ClientId(client)), quake);
    Ok(())
}

/// bo2mp: Black Ops II's earthquake event (its 105) at `origin`: the
/// length in milliseconds (rounded to the nearest) in event_parm, the scale
/// and the radius in origin2 (BO2 keeps them in the event's own fields).
/// BO2 refuses a scale, length or radius not above 0.
fn earthquake(
    scale: f32,
    seconds: f32,
    origin: [f32; 3],
    radius: f32,
) -> Result<crate::EntityEventPayload, String> {
    let duration_ms = (seconds * 1000.0).round_ties_even() as i32;
    if scale <= 0.0 {
        return Err("Scale must be greater than 0".into());
    }
    if duration_ms < 1 {
        return Err("duration must be greater than 0".into());
    }
    if radius <= 0.0 {
        return Err("Radius must be greater than 0".into());
    }
    Ok(crate::EntityEventPayload {
        number: i32::from(trace_t6::ENTITYNUM_WORLD),
        event_parm: duration_ms,
        origin,
        origin2: [scale, radius, 0.0],
        ..Default::default()
    })
}

#[cfg(test)]
mod earthquake_tests {
    use super::earthquake;

    #[test]
    fn an_earthquake_carries_its_length_scale_and_radius() {
        // The Hellstorm's launch: earthquake(0.2, 0.2, origin, 200).
        let p = earthquake(0.2, 0.2, [1.0, 2.0, 3.0], 200.0).unwrap();
        assert_eq!(p.event_parm, 200);
        assert_eq!(p.origin, [1.0, 2.0, 3.0]);
        assert_eq!(p.origin2, [0.2, 200.0, 0.0]);
        assert_eq!(earthquake(0.5, 0.15, [0.0; 3], 250.0).unwrap().event_parm, 150);
    }

    #[test]
    fn bo2_refuses_a_quake_with_nothing_to_it() {
        assert!(earthquake(0.0, 1.0, [0.0; 3], 100.0).is_err());
        assert!(earthquake(0.5, 0.0004, [0.0; 3], 100.0).is_err());
        assert!(earthquake(0.5, 1.0, [0.0; 3], 0.0).is_err());
    }
}

#[cfg(test)]
mod aim_tests {
    use super::aim_angles;

    #[test]
    fn straight_down_looks_down_and_straight_up_looks_up() {
        assert_eq!(aim_angles([0.0, 0.0, -5.0]), [90.0, 0.0, 0.0]);
        assert_eq!(aim_angles([0.0, 0.0, 5.0]), [270.0, 0.0, 0.0]);
        // Anything else is the shared math's.
        let a = aim_angles([1.0, 1.0, -1.0]);
        assert!(a[0] > 0.0 && a[0] < 90.0 && (a[1] - 45.0).abs() < 1e-3, "{a:?}");
    }
}
