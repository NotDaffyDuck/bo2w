//! The match's players as the start-up scripts see them: one script object per
//! connected client, a handful of fields the engine answers for, and the
//! entry points the step loop uses to kill and retire players.

use std::sync::Arc;

use bevy_ecs::prelude::World;

use super::runtime::{Runtime, run_now};
use crate::frame::FrameWorld;
use crate::script::lang::{Fault, Location, Value};
use crate::{ClassDef, ClassId, ClientId, ClientLifecycle, ScriptSeat, Tick, script_player};

const CONNECT: &str = "maps/mp/gametypes/_callbacksetup::codecallback_playerconnect";

/// A connected client's script object and the session state it declared.
#[derive(Clone, Debug)]
pub(crate) struct PlayerSlot {
    pub(crate) object: u64,
    pub(crate) sessionstate: Arc<str>,
}

impl PlayerSlot {
    fn new(object: u64) -> Self {
        Self { object, sessionstate: "spectator".into() }
    }
}

fn team_name(team: i32) -> &'static str {
    match team {
        entity_t6::team::AXIS => "axis",
        entity_t6::team::ALLIES => "allies",
        entity_t6::team::SPECTATOR => "spectator",
        _ => "none",
    }
}

/// A client name is a NUL-terminated byte string.
fn client_name(name: &[u8]) -> String {
    let end = name.iter().position(|&b| b == 0).unwrap_or(name.len());
    String::from_utf8_lossy(&name[..end]).into_owned()
}

/// A player object's field, when the engine owns it. Anything else lives on the
/// script heap.
pub(crate) fn load_field(world: &mut World, object: u64, name: &str) -> Option<Value> {
    let client = world.resource::<Runtime>().client_of(object)?;
    if name == "sessionstate" {
        return world
            .resource::<Runtime>()
            .players
            .get(&client)
            .map(|slot| Value::String(slot.sessionstate.clone()));
    }
    let id = ClientId(client);
    let frame = FrameWorld::from_world(world);
    let ps = frame.player(id);
    let meta = frame.client_meta(id);
    Some(match name {
        "origin" => Value::Vector(ps.map_or([0.0; 3], |ps| ps.origin)),
        "angles" => Value::Vector(ps.map_or([0.0; 3], |ps| ps.viewangles)),
        "health" => Value::Int(ps.map_or(0, |ps| ps.health)),
        "maxhealth" => Value::Int(
            meta.map(|m| m.max_health)
                .filter(|&n| n > 0)
                .unwrap_or(ps.map_or(0, |ps| ps.max_health)),
        ),
        "name" => Value::String(client_name(&meta?.name).into()),
        "score" => Value::Int(meta?.score),
        "kills" => Value::Int(meta?.kills),
        "deaths" => Value::Int(meta?.deaths),
        "sessionteam" => Value::string(team_name(meta?.client_state_team)),
        _ => return None,
    })
}

/// Write a player object's engine-owned field. `Ok(false)` means the field is
/// not the engine's and belongs on the heap.
pub(crate) fn store_field(
    world: &mut World,
    object: u64,
    name: &str,
    value: &Value,
) -> Result<bool, String> {
    let Some(client) = world.resource::<Runtime>().client_of(object) else {
        return Ok(false);
    };
    let id = ClientId(client);
    let number = || match value {
        Value::Int(n) => Ok(*n),
        Value::Float(x) => Ok(*x as i32),
        other => Err(format!("player field {name} takes a number, not {other:?}")),
    };
    match name {
        "sessionstate" => {
            let Value::String(state) = value else {
                return Err("sessionstate takes a string".into());
            };
            if !matches!(&**state, "playing" | "dead" | "spectator" | "intermission") {
                return Err(format!("invalid sessionstate '{state}'"));
            }
            if let Some(slot) = world.resource_mut::<Runtime>().players.get_mut(&client) {
                slot.sessionstate = state.clone();
            }
        }
        "origin" | "angles" => {
            let Value::Vector(v) = value else {
                return Err(format!("player field {name} takes a vector"));
            };
            let mut frame = FrameWorld::from_world(world);
            if name == "origin" {
                frame.set_origin(id, *v);
            } else {
                frame.set_viewangles(id, *v);
            }
        }
        "health" => {
            let n = number()?;
            if let Some(ps) = FrameWorld::from_world(world).player_mut(id) {
                ps.health = n;
            }
        }
        "maxhealth" => {
            let n = number()?.max(1);
            let mut frame = FrameWorld::from_world(world);
            frame.client_meta_mut(id).max_health = n;
            if let Some(ps) = frame.player_mut(id) {
                ps.max_health = n;
                ps.health = ps.health.min(n);
            }
        }
        "score" | "kills" | "deaths" => {
            let n = number()?;
            let mut frame = FrameWorld::from_world(world);
            if frame.client_meta(id).is_some() {
                let meta = frame.client_meta_mut(id);
                match name {
                    "score" => meta.score = n,
                    "kills" => meta.kills = n,
                    _ => meta.deaths = n,
                }
            }
        }
        "sessionteam" => {
            let Value::String(team) = value else {
                return Err("sessionteam takes a string".into());
            };
            let team = match &**team {
                "axis" => entity_t6::team::AXIS,
                "allies" => entity_t6::team::ALLIES,
                "spectator" => entity_t6::team::SPECTATOR,
                "none" => entity_t6::team::FREE,
                other => return Err(format!("invalid sessionteam '{other}'")),
            };
            let mut frame = FrameWorld::from_world(world);
            if frame.client_meta(id).is_some() {
                let meta = frame.client_meta_mut(id);
                meta.client_state_team = team;
                if matches!(
                    meta.lifecycle,
                    ClientLifecycle::Spectating | ClientLifecycle::ChoosingClass
                ) {
                    meta.lifecycle = script_player::spectator_lifecycle(team);
                }
            }
        }
        "name" => return Err("player field name is read-only".into()),
        _ => return Ok(false),
    }
    Ok(true)
}

/// Give every client that has none a script object and run the connect
/// callback for it.
pub(crate) fn sync_players(world: &mut World) {
    if !world.resource::<crate::step::StepRequest>().reason.advances_authority_world() {
        return;
    }
    {
        let runtime = world.resource::<Runtime>();
        if runtime.program.is_none() || runtime.fault.is_some() || !runtime.started {
            return;
        }
    }
    for id in FrameWorld::from_world(world).client_ids_sorted() {
        let mut runtime = world.resource_mut::<Runtime>();
        if runtime.players.contains_key(&id.0) {
            continue;
        }
        let object = match runtime.heap.new_object() {
            Ok(object) => object,
            Err(message) => {
                runtime.fault = Some(Fault::at(&Location::engine("<engine>", "connect"), message));
                return;
            }
        };
        runtime.players.insert(id.0, PlayerSlot::new(object));
        runtime.entity_count += 1;
        if run_now(world, CONNECT, Value::Object(object), Vec::new()).is_err() {
            return;
        }
    }
}

/// Retire the clients whose disconnect the carrier queued.
pub(crate) fn apply_disconnects(world: &mut World) {
    if !world.resource::<crate::step::StepRequest>().reason.advances_authority_world() {
        return;
    }
    let queued = std::mem::take(&mut world.resource_mut::<Runtime>().disconnects);
    for client in queued {
        FrameWorld::from_world(world).retire_client(ClientId(client));
    }
}

/// A client left: free its script object and forget its classes.
pub(crate) fn disconnect_player(world: &mut World, client: u32) {
    let mut runtime = world.resource_mut::<Runtime>();
    runtime.personal_classes.retain(|(owner, _), _| *owner != client);
    if let Some(slot) = runtime.players.remove(&client) {
        runtime.heap.drop_object(slot.object);
        runtime.entity_count = runtime.entity_count.saturating_sub(1);
    }
}

pub(crate) fn choose_class(world: &mut World, client: u32, class: &ClassDef) {
    world
        .resource_mut::<Runtime>()
        .personal_classes
        .insert((client, class.id.0), class.clone());
}

pub(crate) fn personal_class(world: &World, client: u32, class: ClassId) -> Option<ClassDef> {
    world
        .resource::<Runtime>()
        .personal_classes
        .get(&(client, class.0))
        .cloned()
}

/// A hit on a player. The match's start-up scripts have nothing to say about
/// it, so only the BO2 side (if a BO2 map's scripts run) takes it.
pub(crate) fn player_damage(world: &mut World, _tick: Tick, hit: &script_player::Hit) {
    let _ = crate::t6::player_hit(world, hit);
}

fn alive(frame: &FrameWorld, id: ClientId) -> bool {
    frame
        .client_meta(id)
        .is_some_and(|meta| meta.lifecycle == ClientLifecycle::Alive)
}

/// Kill a living client by their own hand.
pub(crate) fn force_death(world: &mut World, tick: Tick, client: u32) {
    let id = ClientId(client);
    let mut frame = FrameWorld::from_world(world);
    if alive(&frame, id) {
        script_player::kill(&mut frame, tick, id, Some(id), None);
    }
}

/// BO2's grenade-suicide event (MP): a grenade held past its fuse kills its
/// holder outright, with no blast and no damage callback. Thrown back, it
/// counts as the thrower's grenade kill.
pub(crate) fn grenade_suicide(
    world: &mut World,
    tick: Tick,
    client: u32,
    thrower: Option<u32>,
    weapon: u32,
) {
    let id = ClientId(client);
    let mut frame = FrameWorld::from_world(world);
    let alive = alive(&frame, id);
    diag::info!(Sim, "grenade suicide: client {client} alive {alive} thrower {thrower:?} weapon {weapon}");
    if alive {
        let by = thrower.map_or(id, ClientId);
        script_player::kill(&mut frame, tick, id, Some(by), None);
    }
}

/// Answers the stubs never ask for: the step loop still reports these.
pub(crate) fn is_t5(_: &World) -> bool {
    false
}

pub(crate) fn give_killstreak(_: &mut World, _client: u32, _name: &str) {}

pub(crate) fn answer_menu(_: &mut World, _client: u32, _menu: &str, _response: &str) {}

pub(crate) fn answer_join(_: &mut World, _client: u32) {}

pub(crate) fn note_team_answer(_: &mut World, _client: u32, _menu: &str) {}

pub(crate) fn choose_default_class(_: &mut World, _client: u32, _index: u8) {}

pub(crate) fn flashbang(
    _: &mut World,
    _target: ClientId,
    _origin: [f32; 3],
    _amount_distance: f32,
    _amount_angle: f32,
    _attacker: ClientId,
) {
}

/// Killcam seats belong to the BO2 side; the start-up scripts never run one.
pub(crate) fn script_seats(_: &World) -> Vec<(ClientId, ScriptSeat)> {
    Vec::new()
}

pub(crate) fn store_seat_archive(_: &mut World, _client: ClientId, _ms: i32) {}
