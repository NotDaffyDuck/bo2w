//! The match's players as the start-up scripts see them: one script object per
//! connected client, a handful of fields the engine answers for, and the
//! entry points the step loop uses to kill and retire players.

use std::sync::Arc;

use bevy_ecs::prelude::World;

use super::runtime::{Runtime, run_now};
use crate::step_view::StepWorldView;
use crate::gsc_host::lang::{Fault, Location, Value};
use crate::{LoadoutClassSpec, LoadoutClass, SeatNo, PlayerSessionStage, StepNo, client_script};

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
    let before_nul = name.split(|&byte| byte == 0).next().unwrap_or(name);
    String::from_utf8_lossy(before_nul).into_owned()
}

/// A player object's field, when the engine owns it. Anything else lives on the
/// script heap.
pub(crate) fn load_field(world: &mut World, object: u64, name: &str) -> Option<Value> {
    let client = world.resource::<Runtime>().receiver_for(object)?;
    if name == "sessionstate" {
        return world
            .resource::<Runtime>()
            .players
            .get(&client)
            .map(|slot| Value::String(slot.sessionstate.clone()));
    }
    let id = SeatNo(client);
    let frame = StepWorldView::from_world(world);
    let ps = frame.avatar(id);
    let meta = frame.client_meta(id);
    Some(match name {
        "origin" => Value::Vector(ps.map_or([0.0; 3], |ps| ps.origin)),
        "angles" => Value::Vector(ps.map_or([0.0; 3], |ps| ps.look_angles)),
        "health" => Value::Int(ps.map_or(0, |ps| ps.health)),
        "maxhealth" => Value::Int(
            meta.map(|m| m.hit_point_ceiling)
                .filter(|&n| n > 0)
                .unwrap_or(ps.map_or(0, |ps| ps.hit_point_ceiling)),
        ),
        "name" => Value::String(client_name(&meta?.name).into()),
        "score" => Value::Int(meta?.score),
        "kills" => Value::Int(meta?.frags),
        "deaths" => Value::Int(meta?.died),
        "sessionteam" => Value::string(team_name(meta?.seat_side)),
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
    let Some(client) = world.resource::<Runtime>().receiver_for(object) else {
        return Ok(false);
    };
    let id = SeatNo(client);
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
            let mut frame = StepWorldView::from_world(world);
            if name == "origin" {
                frame.place_at(id, *v);
            } else {
                frame.write_aim(id, *v);
            }
        }
        "health" => {
            let n = number()?;
            if let Some(ps) = StepWorldView::from_world(world).occupant_mut(id) {
                ps.health = n;
            }
        }
        "maxhealth" => {
            let n = number()?.max(1);
            let mut frame = StepWorldView::from_world(world);
            frame.client_meta_mut(id).hit_point_ceiling = n;
            if let Some(ps) = frame.occupant_mut(id) {
                ps.hit_point_ceiling = n;
                ps.health = ps.health.min(n);
            }
        }
        "score" | "kills" | "deaths" => {
            let n = number()?;
            let mut frame = StepWorldView::from_world(world);
            if frame.client_meta(id).is_some() {
                let meta = frame.client_meta_mut(id);
                match name {
                    "score" => meta.score = n,
                    "kills" => meta.frags = n,
                    _ => meta.died = n,
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
            let mut frame = StepWorldView::from_world(world);
            if frame.client_meta(id).is_some() {
                let meta = frame.client_meta_mut(id);
                meta.seat_side = team;
                if matches!(
                    meta.lifecycle,
                    PlayerSessionStage::Spectating | PlayerSessionStage::PickingLoadout
                ) {
                    meta.lifecycle = client_script::spectator_lifecycle(team);
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
    if !world.resource::<crate::frame_step::FrameTicket>().reason.moves_server_world_forward() {
        return;
    }
    {
        let runtime = world.resource::<Runtime>();
        if runtime.program.is_none() || runtime.fault.is_some() || !runtime.started {
            return;
        }
    }
    for id in StepWorldView::from_world(world).ordered_player_ids() {
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
    if !world.resource::<crate::frame_step::FrameTicket>().reason.moves_server_world_forward() {
        return;
    }
    let queued = std::mem::take(&mut world.resource_mut::<Runtime>().disconnects);
    for client in queued {
        StepWorldView::from_world(world).drop_player_record(SeatNo(client));
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

pub(crate) fn choose_class(world: &mut World, who: u32, class: &LoadoutClassSpec) {
    world
        .resource_mut::<Runtime>()
        .personal_classes
        .insert((who, class.no.0), class.clone());
}

pub(crate) fn personal_class(world: &World, who: u32, class: LoadoutClass) -> Option<LoadoutClassSpec> {
    world
        .resource::<Runtime>()
        .personal_classes
        .get(&(who, class.0))
        .cloned()
}

/// A hit on a player. The match's start-up scripts have nothing to say about
/// it, so only the BO2 side (if a BO2 map's scripts run) takes it.
pub(crate) fn player_damage(world: &mut World, _tick: StepNo, hit: &client_script::Hit) {
    let _ = crate::zombie_rt::player_hit(world, hit);
}

fn alive(frame: &StepWorldView, seat: SeatNo) -> bool {
    frame
        .client_meta(seat)
        .is_some_and(|meta| meta.lifecycle == PlayerSessionStage::Alive)
}

/// Kill a living client by their own hand.
pub(crate) fn force_death(world: &mut World, tick: StepNo, client: u32) {
    let id = SeatNo(client);
    let mut frame = StepWorldView::from_world(world);
    if alive(&frame, id) {
        client_script::kill(&mut frame, tick, id, Some(id), None);
    }
}

/// BO2's grenade-suicide event (MP): a grenade held past its fuse kills its
/// holder outright, with no blast and no damage callback. Thrown back, it
/// counts as the thrower's grenade kill.
pub(crate) fn grenade_suicide(
    world: &mut World,
    tick: StepNo,
    client: u32,
    thrower: Option<u32>,
    gun: u32,
) {
    let id = SeatNo(client);
    let mut frame = StepWorldView::from_world(world);
    let alive = alive(&frame, id);
    diag::info!(Sim, "grenade suicide: client {client} alive {alive} thrower {thrower:?} weapon {gun}");
    if alive {
        let by = thrower.map_or(id, SeatNo);
        client_script::kill(&mut frame, tick, id, Some(by), None);
    }
}
