//! bo2mp: the Swarm's client script (clientscripts/mp/_missile_swarm.csc):
//! while world field `missile_swarm` is up, each player's sky fills with
//! fake hunter-killer drones that fly across it far above him (up to 12 at
//! once, one or two at a time), each with its exhaust trail, and the swarm's
//! drone hum plays; when it goes down the hum fades and no more come (the
//! ones in the air fly on); an EMP (2) takes every one out at once.
//!
//! Black Ops II spawns them on each player's own client. Ours are entities
//! every client sees, made for the players who play here (bots have no
//! sky); their trails show only to the player each flies for. Not ported:
//! its checks for the killcam and a demo (neither shows the swarm).

use std::collections::BTreeMap;

use bevy_ecs::prelude::{Resource, World};
use gsc_t6::ObjKind;
use gsc_t6::math::{add, scale, sub, vector_to_angles};

use super::{LEVEL, Step, Zm, again, ms, rand_float, rand_int, rotate_to};
use crate::EventAudience;
use crate::world::ClientId;

const MODEL: &str = "veh_t6_drone_hunterkiller_viewmodel";
const LAUNCH: &str = "o_drone_hunter_launch";
const TRAIL: &str = "weapon/harpy_swarm/fx_hrpy_swrm_exhaust_trail_close";
const HUM: &str = "veh_harpy_drone_swarm_close__lp";
/// `missile_swarm_max`: drones in one player's sky at once.
const MAX: i32 = 12;
/// How far each side of the player a drone starts and ends.
const DIST: f32 = 10000.0;
/// Its value when an EMP takes the swarm out.
const EMP: i32 = 2;
/// `swarm_think` waits half a second while his sky is full.
const FULL_WAIT_MS: i64 = 500;
/// `swarm_sound`: the hum starts 2 s in; `deleteonmissileswarmstop` takes
/// its entity a second after it stops.
const HUM_WAIT_MS: i64 = 2000;
const HUM_GONE_MS: i64 = 1000;
/// `rotateto( ..., 0.05 )` before it flies.
const TURN_SECS: f32 = 0.05;
const TICK_MS: i64 = 50;

/// The swarm as the players' client scripts keep it.
#[derive(Resource, Default)]
pub(super) struct Swarm {
    /// The field as the client last saw it (its callback runs on a change).
    value: i32,
    /// `swarm_think` runs (between a start and `missile_swarm_stop`).
    on: bool,
    /// `missile_emp_death` is due: every drone goes at once.
    emp: bool,
    /// The step is queued.
    ticking: bool,
    /// Each player's `missile_swarm_count` and when he spawns again.
    skies: BTreeMap<u32, Sky>,
    drones: Vec<Drone>,
    /// The hum's entity and when it starts (`None` once it plays).
    hum: Option<(u32, Option<i64>)>,
    /// Hum entities to take away, and when.
    fading: Vec<(u32, i64)>,
}

#[derive(Clone, Copy, Default)]
struct Sky {
    count: i32,
    next: i64,
}

#[derive(Clone, Copy)]
struct Drone {
    n: u32,
    owner: u32,
    end: [f32; 3],
    leg: Leg,
}

#[derive(Clone, Copy)]
enum Leg {
    /// `projectile_move_think`'s first wait: its trail lights then.
    Wait(i64),
    /// Turned to fly; moves at the first.
    Turn(i64),
    /// Flying; gone at the first.
    Fly(i64),
}

/// `swarm_start`: the world field's callback.
pub(super) fn world_field(world: &mut World, name: &str, value: i32) {
    if name != "missile_swarm" {
        return;
    }
    let now = world.resource::<Zm>().now_ms;
    let mut s = world.get_resource_or_insert_with(Swarm::default);
    let old = std::mem::replace(&mut s.value, value);
    if value == old {
        return;
    }
    if value != 0 && value != EMP {
        s.on = true;
        s.skies.clear();
    } else {
        if value == EMP {
            s.emp = true;
        }
        s.on = false;
    }
    let queue = !std::mem::replace(&mut s.ticking, true);
    if queue {
        again(world, LEVEL, now, Step::Swarm);
    }
    diag::info!(Sim, "bo2mp swarm: missile_swarm {old} -> {value}");
}

/// Each snapshot while it has anything to do (it queues itself again).
pub(super) fn tick(world: &mut World, now: i64) {
    let Some(mut s) = world.get_resource_mut::<Swarm>().map(|s| std::mem::take(s.into_inner())) else {
        return;
    };
    if s.emp {
        s.emp = false;
        for d in std::mem::take(&mut s.drones) {
            gone(world, d.n);
        }
    }
    hum(world, &mut s, now);
    if s.on {
        spawn(world, &mut s, now);
    }
    fly(world, &mut s, now);
    s.ticking = s.on || !s.drones.is_empty() || s.hum.is_some() || !s.fading.is_empty();
    let more = s.ticking;
    world.insert_resource(s);
    if more {
        again(world, LEVEL, now + TICK_MS, Step::Swarm);
    }
}

/// `swarm_sound` and `deleteonmissileswarmstop`.
fn hum(world: &mut World, s: &mut Swarm, now: i64) {
    match s.hum {
        None if s.on => {
            // Black Ops II spawns it at the field's `self.origin`; the
            // world has none, so it is the map's origin.
            if let Some(n) = new_ent(world, [0.0; 3], "") {
                s.hum = Some((n, Some(now + HUM_WAIT_MS)));
            }
        }
        Some((n, Some(at))) if s.on && now >= at => {
            if let Some(e) = world.resource_mut::<Zm>().ents.get_mut(&n) {
                e.loop_sound = Some(HUM.to_owned());
            }
            s.hum = Some((n, None));
        }
        Some((n, _)) if !s.on => {
            if let Some(e) = world.resource_mut::<Zm>().ents.get_mut(&n) {
                e.loop_sound = None;
            }
            s.fading.push((n, now + HUM_GONE_MS));
            s.hum = None;
        }
        _ => {}
    }
    let (done, keep): (Vec<_>, Vec<_>) = std::mem::take(&mut s.fading).into_iter().partition(|f| now >= f.1);
    s.fading = keep;
    for (n, _) in done {
        gone(world, n);
    }
}

/// `swarm_think`, for each player who plays here.
fn spawn(world: &mut World, s: &mut Swarm, now: i64) {
    let players: Vec<u32> = world
        .resource::<Zm>()
        .players
        .keys()
        .copied()
        .filter(|&c| !super::super::is_bot(world, c))
        .collect();
    s.skies.retain(|c, _| players.contains(c));
    for c in players {
        let sky = *s.skies.entry(c).or_default();
        if now < sky.next {
            continue;
        }
        let mut count = sky.count.max(0);
        let next = if count > MAX {
            now + FULL_WAIT_MS
        } else {
            let k = 1 + rand_int(world, 2) as i32;
            count += k;
            for _ in 0..k {
                if let Some(d) = drone(world, c, now) {
                    s.drones.push(d);
                }
            }
            now + ms(count as f32 / MAX as f32)
        };
        s.skies.insert(c, Sky { count, next });
    }
}

/// `projectile_spawn`: one drone across player `c`'s sky.
fn drone(world: &mut World, c: u32, now: i64) -> Option<Drone> {
    let at = super::super::frame(world).player(ClientId(c)).map(|ps| ps.origin)?;
    let up = 1000.0 + rand_int(world, 300) as f32;
    let yaw = rand_int(world, 360) as f32;
    let (fwd, _, _) = gsc_t6::math::angle_vectors([0.0, yaw, 0.0]);
    let base = add(at, [0.0, 0.0, up]);
    let start = add(base, scale(fwd, -DIST));
    let end = add(base, scale(fwd, DIST));
    let n = new_ent(world, start, MODEL)?;
    super::super::mp_fxanims::play(world, n, LAUNCH, false);
    let wait = rand_float(world, 0.5, 1.0);
    Some(Drone { n, owner: c, end, leg: Leg::Wait(now + ms(wait)) })
}

/// `projectile_move_think`: trail, turn, fly, gone.
fn fly(world: &mut World, s: &mut Swarm, now: i64) {
    let mut keep = Vec::with_capacity(s.drones.len());
    for mut d in std::mem::take(&mut s.drones) {
        if !world.resource::<Zm>().ents.contains_key(&d.n) {
            continue;
        }
        match d.leg {
            Leg::Wait(at) if now >= at => {
                super::super::natives_fx::bolt_effect_to(
                    world,
                    EventAudience::Client(ClientId(d.owner)),
                    d.n,
                    TRAIL,
                    "tag_origin",
                    false,
                );
                let from = world.resource::<Zm>().ents.get(&d.n).map_or(d.end, |e| e.origin);
                rotate_to(world, d.n, vector_to_angles(sub(d.end, from)), now, TURN_SECS, 0.0, 0.0);
                d.leg = Leg::Turn(now + ms(TURN_SECS));
            }
            Leg::Turn(at) if now >= at => {
                let secs = rand_float(world, 12.0, 18.0);
                let mut zm = world.resource_mut::<Zm>();
                if let Some(from) = zm.ents.get(&d.n).map(|e| e.origin) {
                    zm.movers.move_to(d.n, from, d.end, now, secs, 0.0, 0.0);
                }
                d.leg = Leg::Fly(now + ms(secs));
            }
            Leg::Fly(at) if now >= at => {
                if let Some(sky) = s.skies.get_mut(&d.owner) {
                    sky.count -= 1;
                }
                gone(world, d.n);
                continue;
            }
            _ => {}
        }
        keep.push(d);
    }
    s.drones = keep;
}

/// A client script's `spawn( localclientnum, origin, "script_model" )`.
fn new_ent(world: &mut World, origin: [f32; 3], model: &str) -> Option<u32> {
    let n = world.resource_mut::<Zm>().alloc_entnum();
    let obj = super::super::with_vm(world, |vm, _| vm.alloc_object(ObjKind::Entity(n)))?;
    let ent = super::super::Ent {
        obj: Some(obj),
        classname: "script_model".to_owned(),
        origin,
        model: model.to_owned(),
        ..Default::default()
    };
    world.resource_mut::<Zm>().ents.insert(n, ent);
    Some(n)
}

/// Its `delete()`.
fn gone(world: &mut World, n: u32) {
    super::super::mp_fxanims::forget(world, n);
    let obj = {
        let mut zm = world.resource_mut::<Zm>();
        zm.movers.list.remove(&n);
        zm.ents.remove(&n).and_then(|e| e.obj)
    };
    if let Some(o) = obj {
        super::super::with_vm(world, |vm, w| vm.free_object(w, o));
    }
}
