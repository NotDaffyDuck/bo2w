//! Flying script vehicles: a helicopter given `setvehgoalpos(pos, stop)`
//! flies straight at it at its own speeds (its vehicle data's top speed and
//! acceleration, or `setspeed`'s), easing in to a standstill when told to
//! stop there. As BO2 does: `near_goal` every tick while within the
//! `setneargoalnotifydist` distance plus one tick of travel; `goal` on
//! arrival (not stopping: within one tick of travel; stopping: at a
//! standstill, or inside the `sethoverparams` radius).
//! `pathvariableoffset(range, secs)` wanders the aim point: every `secs` a
//! new random offset inside +-range, eased (cosine) from the last one.
//! With no goal (`cancelaimove`, `clearvehgoalpos`) it keeps its velocity
//! under `setphysacceleration` and stops where it lands: Buried's ghost
//! lantern falls this way when a grenade hits it, spinning
//! (`setangularvelocity`) until it lands. `self.velocity` follows it.

use std::collections::BTreeMap;

use bevy_ecs::prelude::World;
use gsc_t6::{ScriptValue, Vm};

use super::{Zm, entnum, frame, num, vec3, with_vm};

/// A vehicle type's flight speeds, units a second (and a second squared).
#[derive(Clone, Copy, Debug)]
pub(crate) struct Speeds {
    pub max: f32,
    pub speed_up_rate: f32,
    pub max_up: f32,
    pub accel_up: f32,
}

/// The game's vehicle data (`t6list` on the map zones; loaded values,
/// already units a second).
fn speeds(vehicletype: &str) -> Speeds {
    let (max, speed_up_rate, max_up, accel_up) = match vehicletype {
        // Buried's ghost lantern (5 mph).
        "heli_quadrotor2_zm" => (88.0, 88.0, 792.0, 440.0),
        // Origins' Maxis drone (60 mph).
        "heli_quadrotor_zm" | "heli_quadrotor_upgraded_zm" => (1056.0, 352.0, 792.0, 440.0),
        "biplane_zm" => (2640.0, 528.0, 1144.0, 1056.0),
        "tank_mark_iv_zm" => (264.0, 35.2, 1144.0, 1056.0),
        // defaultvehicle_mp
        _ => (176.0, 220.88, 1144.0, 1056.0),
    };
    Speeds {
        max,
        speed_up_rate,
        max_up,
        accel_up,
    }
}

/// `pathvariableoffset`: the aim point's wander.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Wobble {
    range: [f32; 3],
    secs: f32,
    t: f32,
    from: [f32; 3],
    to: [f32; 3],
}

impl Wobble {
    fn now(&self) -> [f32; 3] {
        let k = if self.secs > 0.0 {
            (1.0 - (std::f32::consts::PI * (self.t / self.secs).min(1.0)).cos()) * 0.5
        } else {
            1.0
        };
        std::array::from_fn(|i| self.from[i] + (self.to[i] - self.from[i]) * k)
    }

    /// From where it is now towards a new random offset.
    fn retarget(&mut self, vm: &mut Vm<World>) {
        self.from = self.now();
        self.t = 0.0;
        self.to = std::array::from_fn(|i| (vm.rand_unit() * 2.0 - 1.0) * self.range[i]);
    }
}

#[derive(Clone, Debug)]
pub(crate) struct Flight {
    speeds: Speeds,
    goal: Option<[f32; 3]>,
    stop_at_goal: bool,
    /// `goal` was sent for this goal.
    arrived: bool,
    motion_vec: [f32; 3],
    /// `setphysacceleration`.
    speed_up_rate: [f32; 3],
    /// Degrees a second (pitch, yaw, roll).
    spin: [f32; 3],
    near_dist: f32,
    hover_radius: f32,
    wobble: Option<Wobble>,
    landed: bool,
    /// `near_goal` was logged for this goal (the test log).
    near_logged: bool,
}

/// BO2MP_T6_FLIGHTLOG=1 (or the lantern test): a line at each new goal,
/// the first `near_goal` and `goal` of each, and each landing.
fn log_on() -> bool {
    static ON: std::sync::LazyLock<bool> = std::sync::LazyLock::new(|| {
        ["BO2MP_T6_FLIGHTLOG", "BO2MP_T6_LANTERN"]
            .iter()
            .any(|k| std::env::var_os(k).is_some())
    });
    *ON
}

/// Where a flying vehicle is headed and how fast it goes (the test log).
pub(crate) fn describe(world: &World, v: u32) -> Option<String> {
    let f = world.resource::<Zm>().vehicles.flights.get(&v)?;
    let goal = f.goal.map_or_else(
        || "none".to_owned(),
        |g| format!("({:.0} {:.0} {:.0})", g[0], g[1], g[2]),
    );
    Some(format!(
        "speed {:.0} goal {goal} landed {} spin {:.0}",
        len(f.motion_vec),
        f.landed,
        f.spin[1]
    ))
}

/// The vehicle's flight state, made on first use.
fn flight<'w>(vm: &mut Vm<World>, world: &'w mut World, s: &ScriptValue) -> Option<&'w mut Flight> {
    let v = entnum(vm, s)?;
    let obj = world.resource::<Zm>().ents.get(&v)?.obj?;
    if !world.resource::<Zm>().vehicles.flights.contains_key(&v) {
        let key = vm.intern("vehicletype");
        let ty = vm.to_text(&vm.raw_field(obj, key));
        let f = Flight {
            speeds: speeds(&ty),
            goal: None,
            stop_at_goal: false,
            arrived: false,
            motion_vec: [0.0; 3],
            speed_up_rate: [0.0; 3],
            spin: [0.0; 3],
            near_dist: 0.0,
            hover_radius: 0.0,
            wobble: None,
            landed: false,
            near_logged: false,
        };
        // `self.velocity` reads before its first tick (the lantern's crash).
        let vk = vm.intern("velocity");
        vm.set_raw_field(obj, vk, ScriptValue::Vec3([0.0; 3]));
        world.resource_mut::<Zm>().vehicles.flights.insert(v, f);
    }
    world
        .resource_mut::<Zm>()
        .into_inner()
        .vehicles
        .flights
        .get_mut(&v)
}

fn len(v: [f32; 3]) -> f32 {
    (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt()
}

/// Each tick: flying vehicles towards their goals, falling ones down.
pub(crate) fn advance(game: &mut World, dt: f32) {
    if game.resource::<Zm>().vehicles.flights.is_empty() {
        return;
    }
    with_vm(game, |vm, game| {
        let members: Vec<u32> = game
            .resource::<Zm>()
            .vehicles
            .flights
            .keys()
            .copied()
            .collect();
        let vel_key = vm.intern("velocity");
        // (vehicle, notify)
        let mut notes: Vec<(gsc_t6::ObjRef, &str)> = Vec::new();
        for v in members {
            let zm = game.resource::<Zm>();
            let Some((mut pos, mut ang, Some(obj))) =
                zm.ents.get(&v).map(|e| (e.origin, e.rot, e.obj))
            else {
                game.resource_mut::<Zm>().vehicles.flights.remove(&v);
                continue;
            };
            let Some(mut f) = zm.vehicles.flights.get(&v).cloned() else {
                continue;
            };
            // `setspeed` (miles per hour, eased) overrides the top speed.
            let (max, speed_up_rate) = zm
                .vehicles
                .speed
                .get(&v)
                .and_then(|s| s.target.map(|t| (t, s.speed_up_rate)))
                .unwrap_or((f.speeds.max, f.speeds.speed_up_rate));
            let off = match f.wobble.as_mut() {
                Some(w) => {
                    w.t += dt;
                    if w.t >= w.secs {
                        w.retarget(vm);
                    }
                    w.now()
                }
                None => [0.0; 3],
            };
            if let Some(g) = f.goal {
                let aim: [f32; 3] = std::array::from_fn(|i| g[i] + off[i]);
                let d: [f32; 3] = std::array::from_fn(|i| aim[i] - pos[i]);
                let dist = len(d);
                // The velocity it wants: at the goal at full speed, or
                // slowing to a stop there.
                let mut want = [0.0; 3];
                if dist > 1e-3 {
                    let flat = d[0].hypot(d[1]);
                    let mut h = max;
                    let mut up = f.speeds.max_up;
                    if f.stop_at_goal {
                        h = h.min((2.0 * speed_up_rate * flat).sqrt());
                        up = up.min((2.0 * f.speeds.accel_up * d[2].abs()).sqrt());
                    }
                    let s = if flat > 1e-3 { h / dist } else { 0.0 };
                    want = [d[0] * s, d[1] * s, (d[2] * h / dist).clamp(-up, up)];
                    if flat <= 1e-3 {
                        want[2] = d[2].signum() * up;
                    }
                }
                let dh = [want[0] - f.motion_vec[0], want[1] - f.motion_vec[1]];
                let lh = dh[0].hypot(dh[1]);
                let step = speed_up_rate * dt;
                let k = if lh > step { step / lh } else { 1.0 };
                f.motion_vec[0] += dh[0] * k;
                f.motion_vec[1] += dh[1] * k;
                f.motion_vec[2] += (want[2] - f.motion_vec[2]).clamp(-f.speeds.accel_up * dt, f.speeds.accel_up * dt);
                let travel = len(f.motion_vec) * dt;
                if f.stop_at_goal && travel >= dist {
                    pos = aim;
                    f.motion_vec = [0.0; 3];
                } else {
                    pos = std::array::from_fn(|i| pos[i] + f.motion_vec[i] * dt);
                }
                if f.motion_vec[0].hypot(f.motion_vec[1]) > 1.0 {
                    ang[1] = f.motion_vec[1].atan2(f.motion_vec[0]).to_degrees();
                }
                let dist = len(std::array::from_fn(|i| aim[i] - pos[i]));
                let speed = len(f.motion_vec);
                if f.near_dist != 0.0 && dist < speed * 0.05 + f.near_dist {
                    notes.push((obj, "near_goal"));
                    if log_on() && !f.near_logged {
                        f.near_logged = true;
                        diag::info!(
                            Sim,
                            "bo2zm t6 flight {v} near_goal at ({:.0} {:.0} {:.0}) {dist:.0} from it, speed {speed:.0}",
                            pos[0],
                            pos[1],
                            pos[2]
                        );
                    }
                }
                if !f.arrived {
                    let there = if !f.stop_at_goal {
                        dist <= speed * 0.05
                    } else if f.hover_radius > 0.0 {
                        dist <= f.hover_radius
                    } else {
                        dist <= 2.0 && speed <= 2.0
                    };
                    if there {
                        f.arrived = true;
                        if log_on() {
                            diag::info!(
                                Sim,
                                "bo2zm t6 flight {v} goal at ({:.0} {:.0} {:.0})",
                                pos[0],
                                pos[1],
                                pos[2]
                            );
                        }
                        if f.stop_at_goal {
                            notes.push((obj, "near_goal"));
                        }
                        notes.push((obj, "goal"));
                    }
                }
            } else if !f.landed {
                if f.speed_up_rate == [0.0; 3] {
                    // No goal and no pull: it slows to a hover.
                    let lh = f.motion_vec[0].hypot(f.motion_vec[1]);
                    let k = if lh > 0.0 {
                        (lh - f.speeds.speed_up_rate * dt).max(0.0) / lh
                    } else {
                        0.0
                    };
                    let up = f.speeds.accel_up * dt;
                    f.motion_vec = [f.motion_vec[0] * k, f.motion_vec[1] * k, f.motion_vec[2] - f.motion_vec[2].clamp(-up, up)];
                }
                f.motion_vec = std::array::from_fn(|i| f.motion_vec[i] + f.speed_up_rate[i] * dt);
                let next: [f32; 3] = std::array::from_fn(|i| pos[i] + f.motion_vec[i] * dt);
                if len(f.motion_vec) > 1e-3 {
                    let tr = frame(game).trace_static_world(
                        pos,
                        next,
                        [0.0; 3],
                        [0.0; 3],
                        crate::shot_trace::MASK_SHOT,
                    );
                    if tr.fraction < 1.0 {
                        let t = (tr.fraction - 0.01).max(0.0);
                        pos = std::array::from_fn(|i| pos[i] + (next[i] - pos[i]) * t);
                        f.motion_vec = [0.0; 3];
                        f.spin = [0.0; 3];
                        f.landed = true;
                        if log_on() {
                            diag::info!(
                                Sim,
                                "bo2zm t6 flight {v} landed at ({:.0} {:.0} {:.0})",
                                pos[0],
                                pos[1],
                                pos[2]
                            );
                        }
                    } else {
                        pos = next;
                    }
                }
            }
            for i in 0..3 {
                ang[i] = (ang[i] + f.spin[i] * dt) % 360.0;
            }
            vm.set_raw_field(obj, vel_key, ScriptValue::Vec3(f.motion_vec));
            let mut zm = game.resource_mut::<Zm>();
            if let Some(e) = zm.ents.get_mut(&v) {
                e.origin = pos;
                e.rot = ang;
            }
            zm.vehicles.flights.insert(v, f);
        }
        for (obj, name) in notes {
            vm.notify_str(game, obj, name, &[]);
        }
    });
}

pub(super) fn bind(vm: &mut Vm<World>) {
    // setvehgoalpos(pos, stop_at_goal, path_mode, ...): straight at it
    // (the drone's path search always finds its way here).
    vm.bind("setvehgoalpos", true, |vm, world, s, a| {
        let goal = vec3(a, 0)?;
        let stop = super::arg(a, 1).as_int().unwrap_or(0) != 0;
        if let Some(f) = flight(vm, world, s) {
            f.goal = Some(goal);
            f.stop_at_goal = stop;
            f.arrived = false;
            f.landed = false;
            f.near_logged = false;
        }
        if log_on() {
            diag::info!(
                Sim,
                "bo2zm t6 flight {} new goal ({:.0} {:.0} {:.0}) stop {stop}",
                entnum(vm, s).unwrap_or(0),
                goal[0],
                goal[1],
                goal[2]
            );
        }
        Ok(ScriptValue::Int(1))
    });
    for name in ["clearvehgoalpos", "cancelaimove"] {
        vm.bind(name, true, |vm, world, s, _| {
            if let Some(f) = flight(vm, world, s) {
                f.goal = None;
            }
            Ok(ScriptValue::Undefined)
        });
    }
    vm.bind("setneargoalnotifydist", true, |vm, world, s, a| {
        let d = num(a, 0)?;
        if let Some(f) = flight(vm, world, s) {
            f.near_dist = d;
        }
        Ok(ScriptValue::Undefined)
    });
    // sethoverparams(radius, speed, speed_up_rate): the radius counts as there.
    vm.bind("sethoverparams", true, |vm, world, s, a| {
        let r = num(a, 0)?;
        if let Some(f) = flight(vm, world, s) {
            f.hover_radius = r;
        }
        Ok(ScriptValue::Undefined)
    });
    vm.bind("pathvariableoffset", true, |vm, world, s, a| {
        let range = vec3(a, 0)?;
        let secs = num(a, 1)?;
        let Some(f) = flight(vm, world, s) else {
            return Ok(ScriptValue::Undefined);
        };
        let mut w = f.wobble.unwrap_or_default();
        w.range = range;
        w.secs = secs;
        // The random pick needs the VM, so it is taken after the borrow.
        let mut w2 = w;
        w2.retarget(vm);
        if let Some(f) = flight(vm, world, s) {
            f.wobble = Some(w2);
        }
        Ok(ScriptValue::Undefined)
    });
    vm.bind("pathvariableoffsetclear", true, |vm, world, s, _| {
        if let Some(f) = flight(vm, world, s) {
            f.wobble = None;
        }
        Ok(ScriptValue::Undefined)
    });
    vm.bind("setvehvelocity", true, |vm, world, s, a| {
        let v = vec3(a, 0)?;
        if let Some(f) = flight(vm, world, s) {
            f.motion_vec = v;
            f.landed = false;
        }
        Ok(ScriptValue::Undefined)
    });
    vm.bind("setphysacceleration", true, |vm, world, s, a| {
        let v = vec3(a, 0)?;
        if let Some(f) = flight(vm, world, s) {
            f.speed_up_rate = v;
            f.landed = false;
        }
        Ok(ScriptValue::Undefined)
    });
    vm.bind("getangularvelocity", true, |vm, world, s, _| {
        Ok(ScriptValue::Vec3(flight(vm, world, s).map_or([0.0; 3], |f| f.spin)))
    });
    vm.bind("setangularvelocity", true, |vm, world, s, a| {
        let v = vec3(a, 0)?;
        if let Some(f) = flight(vm, world, s) {
            f.spin = v;
        }
        Ok(ScriptValue::Undefined)
    });
    // Nobody rides these; the height lock keeps the drone's cruise height,
    // which its goals already carry.
    for name in ["makevehicleunusable", "setheliheightlock"] {
        vm.bind(name, true, |_, _, _, _| Ok(ScriptValue::Undefined));
    }
}
