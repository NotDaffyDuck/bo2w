//! Helicopter and drone engine loops. BO2 plays them from a client script
//! (`clientscripts/mp/_helicopter_sounds.csc`: `start_helicopter_sounds`,
//! picked by the vehicle's `vehicletype`), which this rebuild does not run.
//! The same loops, with that script's own numbers and update rules, run
//! here and go out with the other entity loops (`presence::publish_loops`),
//! each with the volume and pitch the script would set (`setloopstate`).
//!
//! Every number below is from the disassembly of `_helicopter_sounds.csc`
//! (`init_heli_sound_values` rows, `heli_idle_run_transition`,
//! `drone_up_down_transition`, `drone_rotate_angle`) and `_audio.csc`
//! (`scale_speed`). Not carried over: the script's per-part tags (`snd_rotor`,
//! `snd_tail_rotor`, `tag_body`: every loop sits on the vehicle's origin),
//! its fade times (`playloopsound` 0.5 / 2 s in, `stoploopsound` 4 s out)
//! and `setloopstate`'s rates (their meaning is the exe's; the target level
//! is applied at the script's own update steps), and the rotor-wash dust
//! (`terrain_trace`, a separate surface-sound system).

use std::collections::BTreeMap;

use bevy_ecs::prelude::{Resource, World};
use gsc_t6::ObjRef;

use super::Zm;

/// `self.mph_to_inches_per_sec`.
const MPH: f32 = 17.6;
/// `self.idle_run_trans_speed` (mph).
const IDLE_RUN: f32 = 5.0;

/// How one loop's level follows the vehicle.
#[derive(Clone, Copy)]
enum Drive {
    /// `heli_sound_play`: the alias's own level, never changed.
    Fixed,
    /// `heli_idle_run_transition` with its `helisoundvalues` row:
    /// speed-volume max, volume min, volume max, speed-pitch max, pitch min,
    /// pitch max (mph).
    Run([f32; 6]),
    /// `drone_up_down_transition`'s three loops.
    Up,
    Down,
    Vertical,
    /// `drone_rotate_angle`.
    Turn,
}

struct Part {
    loop_name: &'static str,
    drive: Drive,
}

struct Type {
    parts: &'static [Part],
    /// `heli_idle_run_transition`'s wait (0.5 s unless given; the drone
    /// passes 0.1 s and `updown`).
    wait_ms: i64,
    updown: bool,
}

const fn p(name: &'static str, drive: Drive) -> Part {
    Part { loop_name: name, drive }
}

/// cobra / hind rows (`init_heli_sound_values`, identical for both).
const COBRA_TURBINE: Drive = Drive::Run([65.0, 0.6, 0.8, 65.0, 1.0, 1.1]);
const COBRA_TOP: Drive = Drive::Run([45.0, 0.7, 1.0, 45.0, 0.95, 1.1]);
const COBRA_TAIL: Drive = Drive::Run([45.0, 0.5, 1.0, 45.0, 0.95, 1.1]);

/// `init_heli_sounds_player_controlled` + `play_player_controlled_sounds`.
const COBRA: Type = Type {
    parts: &[
        p("veh_cobra_rotor_lfe", Drive::Fixed),
        p("veh_cobra_turbine", COBRA_TURBINE),
        p("veh_cobra_rotor", COBRA_TOP),
        p("veh_cobra_tail", COBRA_TAIL),
    ],
    wait_ms: 500,
    updown: false,
};

/// `init_heli_sounds_ai_attack` + `play_attack_ai_sounds`.
const HIND: Type = Type {
    parts: &[
        p("veh_hind_rotor_lfe", Drive::Fixed),
        p("veh_hind_turbine", COBRA_TURBINE),
        p("veh_hind_rotor", COBRA_TOP),
        p("veh_hind_tail", COBRA_TAIL),
    ],
    wait_ms: 500,
    updown: false,
};

/// `init_heli_sounds_supply` + `play_supply_sounds` (no tail loop).
const SUPPLY: Type = Type {
    parts: &[
        p("veh_supply_rotor_lfe", Drive::Fixed),
        p("veh_supply_turbine", Drive::Run([65.0, 0.7, 1.0, 65.0, 1.0, 1.1])),
        p("veh_supply_rotor", Drive::Run([35.0, 0.95, 1.0, 100.0, 1.0, 1.1])),
    ],
    wait_ms: 500,
    updown: false,
};

/// `init_heli_sounds_gunner` + `play_gunner_sounds`.
const HUEY: Type = Type {
    parts: &[
        p("veh_huey_rotor_lfe", Drive::Fixed),
        p("veh_huey_radio", Drive::Fixed),
        p("veh_huey_turbine", Drive::Run([65.0, 0.7, 0.8, 65.0, 1.0, 1.1])),
        p("veh_huey_rotor", Drive::Run([45.0, 0.8, 1.0, 45.0, 0.95, 1.1])),
        p("veh_huey_tail", Drive::Run([45.0, 0.6, 1.0, 45.0, 0.95, 1.0])),
        p("veh_huey_door_wind", Drive::Run([45.0, 0.6, 1.0, 45.0, 0.95, 1.0])),
    ],
    wait_ms: 500,
    updown: false,
};

/// `init_heli_sounds_player_drone` + `play_player_drone_sounds`.
const QRDRONE: Type = Type {
    parts: &[
        p("veh_qrdrone_turbine_idle", Drive::Run([30.0, 0.8, 0.0, 16.0, 0.9, 1.1])),
        p("veh_qrdrone_turbine_moving", Drive::Run([30.0, 0.0, 0.9, 20.0, 0.9, 1.1])),
        p("veh_qrdrone_move_down", Drive::Down),
        p("veh_qrdrone_move_up", Drive::Up),
        p("veh_qrdrone_vertical", Drive::Vertical),
        p("veh_qrdrone_idle_rotate", Drive::Turn),
    ],
    wait_ms: 100,
    updown: true,
};

/// `init_heli_sounds_heli_guard` + `play_heli_guard_sounds`.
const HELI_GUARD: Type = Type {
    parts: &[
        p("veh_overwatch_lfe", Drive::Fixed),
        p("veh_overwatch_turbine", Drive::Run([10.0, 0.9, 1.0, 30.0, 0.9, 1.05])),
        p("veh_overwatch_rotor", Drive::Run([10.0, 0.9, 1.0, 30.0, 0.9, 1.1])),
    ],
    wait_ms: 500,
    updown: false,
};

/// `start_helicopter_sounds`' switch on `vehicletype`.
fn type_of(vehicletype: &str) -> Option<&'static Type> {
    Some(match vehicletype.to_ascii_lowercase().as_str() {
        "heli_ai_mp" | "zombie_cobra" => &HIND,
        "heli_guard_mp" => &HELI_GUARD,
        "heli_gunner_mp" => &HUEY,
        "heli_player_controlled_firstperson_mp"
        | "heli_player_controlled_mp"
        | "heli_player_gunner_mp" => &COBRA,
        "heli_supplydrop_mp" => &SUPPLY,
        "qrdrone_mp" => &QRDRONE,
        _ => return None,
    })
}

/// `_audio.csc` `scale_speed`.
fn scale_speed(x1: f32, x2: f32, y1: f32, y2: f32, z: f32) -> f32 {
    let z = z.max(x1).min(x2);
    (z - x1) / (x2 - x1) * (y2 - y1) + y1
}

struct Engine {
    obj: Option<ObjRef>,
    vehicletype: String,
    ty: &'static Type,
    loops_started: bool,
    /// Last tick's origin and time (its velocity: `getvelocity`).
    last: [f32; 3],
    last_ms: i64,
    speed_mph: f32,
    /// The run loops' next step (`wait_time`).
    run_at: i64,
    /// The drone's 0.1 s windows: when the next ends, and where it began.
    window_at: i64,
    window_z: f32,
    window_yaw: f32,
    /// `self.qrdrone_z_difference`.
    z_diff: f32,
    /// Each part's (volume, pitch).
    level: Vec<(f32, f32)>,
}

#[derive(Resource, Default)]
pub(crate) struct EngineSounds {
    by_ent: BTreeMap<u32, Engine>,
    cars: BTreeMap<u32, Car>,
    /// The Escort Drone's brass under its gun (`terrain_trace_brass`).
    brass: BTreeMap<u32, Brass>,
}

/// A car's engine (BO2's own, not a script's: `CG_UpdateVehicleEngineSounds`
/// and `CG_Vehicle_UpdateEngine`): its revs follow its throttle through its
/// gears, and its onload / offload loops fade and pitch with the revs and
/// with how the throttle moves. Numbers: the vehicle def's `engine` in his
/// zones (rc_car_medium_mp in common_mp.ff) and the exe's `EngineInertia`
/// 1.0; every step is BO2's 1/60 s frame.
struct CarEngineDef {
    idle_rpms: f32,
    max_rpms: f32,
    max_torque: f32,
    braking_coeff: f32,
    load_fade: [f32; 4],
    load_scale: f32,
    load_smoothing: f32,
    tilt_multiplier: f32,
    /// One gear: min rpm, max rpm, ratio (`loopLastGear` 0).
    gear: [f32; 3],
    /// (alias, its rev band: fade in from, full at, full to, gone at, the
    /// revs its pitch is 1 at).
    onload: &'static [(&'static str, [f32; 5])],
    offload: &'static [(&'static str, [f32; 5])],
}

const RC_CAR: CarEngineDef = CarEngineDef {
    idle_rpms: 750.0,
    max_rpms: 8500.0,
    max_torque: 1000.0,
    braking_coeff: 10.0,
    load_fade: [0.2, 0.6, 0.35, 0.53],
    load_scale: 4.0,
    load_smoothing: 0.035,
    tilt_multiplier: 1.0,
    gear: [950.0, 2500.0, 1.75],
    onload: &[
        ("mpl_veh_rc_idle_onload", [0.0, 0.01, 0.1, 0.29, 0.22]),
        ("mpl_veh_rc_eng_onload", [0.15, 0.21, 1.5, 2.0, 0.23]),
    ],
    offload: &[
        ("mpl_veh_rc_idle_offload", [0.0, 0.01, 0.1, 0.29, 0.23]),
        ("mpl_veh_rc_eng_offload", [0.15, 0.521, 1.5, 2.0, 0.23]),
    ],
};

const ENGINE_INERTIA: f32 = 1.0;
const FRAME: f32 = 0.016_666_668;

#[derive(Clone, Copy, PartialEq)]
/// BO2's engine states (its SHIFT_UP is only reached by a car with a gear
/// to shift into or one that loops its last: the RC-XD has neither).
enum Gear {
    Idle,
    InGear,
}

struct Car {
    obj: Option<ObjRef>,
    def: &'static CarEngineDef,
    /// What the car's driving sends each tick: its throttle (-1..1), its
    /// forward speed over its top speed, whether its wheels touch.
    throttle_in: f32,
    pace: f32,
    wheels_down: bool,
    /// The engine: its throttle last frame, revs, state, load.
    throttle: f32,
    rpms: f32,
    state: Gear,
    load: f32,
    /// Frames still to run (BO2 steps once a frame).
    owed: f32,
    last_ms: i64,
    /// Each loop's (volume, pitch): onload then offload.
    level: Vec<(f32, f32)>,
}

/// A car's driving this tick (the RC-XD's ride): its engine loops start
/// with its first drive.
pub(crate) fn car_drive(
    world: &mut World, n: u32,
    kind: &str,
    throttle: f32,
    speed_frac: f32,
    touching_ground: bool,
) {
    if !kind.to_ascii_lowercase().starts_with("rc_car") {
        return;
    }
    let (obj, now) = {
        let zm = world.resource::<Zm>();
        (zm.ents.get(&n).and_then(|e| e.obj), zm.clock_millis)
    };
    let mut sounds = world.get_resource_or_insert_with(EngineSounds::default);
    let car = sounds.cars.entry(n).or_insert_with(|| Car {
        obj,
        def: &RC_CAR,
        throttle_in: 0.0,
        pace: 0.0,
        wheels_down: true,
        throttle: 0.0,
        rpms: RC_CAR.idle_rpms,
        state: Gear::Idle,
        load: 0.0,
        owed: 0.0,
        last_ms: now,
        level: vec![(0.0, 1.0); RC_CAR.onload.len() + RC_CAR.offload.len()],
    });
    car.throttle_in = throttle.clamp(-1.0, 1.0);
    car.pace = speed_frac.clamp(0.0, 1.0);
    car.wheels_down = touching_ground;
}

impl Car {
    /// `CG_Vehicle_UpdateEngine`, one frame.
    fn update_engine(&mut self, throttle: f32) {
        let d = self.def;
        let [_, max_rpm, ratio] = d.gear;
        let gear = self.state;
        match gear {
            Gear::Idle => {
                if throttle <= 0.1 {
                    let b = self.rpms / d.max_rpms * d.braking_coeff;
                    self.rpms +=
                        ((b + 1.0) * throttle - b) * d.max_torque / ENGINE_INERTIA * 2.0 * FRAME;
                } else {
                    self.state = Gear::InGear;
                }
                if self.rpms < d.idle_rpms {
                    self.rpms = d.idle_rpms;
                }
                return;
            }
            Gear::InGear => {
                if throttle == 0.0 {
                    self.state = Gear::Idle;
                    return;
                }
                let b = self.rpms / d.max_rpms * d.braking_coeff;
                self.rpms += ((b + 1.0) * throttle - b) * d.max_torque / ENGINE_INERTIA * ratio * FRAME;
                // One gear and no looping it: on the ground its revs stop
                // at the gear's top.
                if self.rpms > max_rpm && self.wheels_down {
                    self.rpms = max_rpm;
                }
            }
        }
        self.rpms = self.rpms.clamp(d.idle_rpms, d.max_rpms);
    }

    /// `CG_UpdateVehicleEngineSounds` for a nitrous car, one frame.
    fn frame(&mut self) {
        let d = self.def;
        let before = self.throttle.abs();
        let t = self.throttle_in.abs();
        self.throttle = if !self.wheels_down {
            t * 1.25
        } else if t > 0.0 {
            (t + self.pace) * 0.5
        } else {
            0.0
        };
        self.update_engine(self.throttle);
        let rpm = self.rpms.clamp(0.0, d.max_rpms) / d.max_rpms * (d.max_rpms - d.idle_rpms)
            + d.idle_rpms;
        self.load = (self.throttle - before) * d.load_smoothing + (1.0 - d.load_smoothing) * self.load;
        let load = (d.load_scale * self.load + 0.5).clamp(0.0, 1.0);
        let [f0, f1, f2, f3] = d.load_fade;
        let on = (load.clamp(f0, f1) - f0) / (f1 - f0);
        let off = 1.0 - (load.clamp(f2, f3) - f2) / (f3 - f2);
        let frac = rpm / d.max_rpms;
        let band = |p: &[f32; 5]| {
            let x = if frac >= p[0] && frac <= p[1] {
                ((frac - p[0]) / (p[1] - p[0])).clamp(0.0, 1.0)
            } else if frac > p[1] && frac < p[2] {
                1.0
            } else if frac >= p[2] && frac <= p[3] {
                1.0 - ((frac - p[2]) / (p[3] - p[2])).clamp(0.0, 1.0)
            } else {
                return 0.0;
            };
            (x * std::f32::consts::FRAC_PI_2).sin()
        };
        let pitch = |p: &[f32; 5]| {
            if d.tilt_multiplier > 0.0 {
                d.tilt_multiplier * frac / p[4]
            } else {
                1.0
            }
        };
        let mut level = self.level.iter_mut();
        for (_, p) in d.onload {
            if let Some(l) = level.next() {
                *l = (band(p) * on, pitch(p));
            }
        }
        for (_, p) in d.offload {
            if let Some(l) = level.next() {
                *l = (band(p) * off, pitch(p));
            }
        }
    }
}

/// A vehicle was spawned (`spawnhelicopter`, `spawnvehicle`): its engine
/// loops start if BO2's client script has some for its type.
pub(crate) fn spawned(world: &mut World, n: u32, vehicletype: &str) {
    if vehicletype.eq_ignore_ascii_case("heli_guard_mp") {
        brass_spawned(world, n);
    }
    let Some(ty) = type_of(vehicletype) else {
        return;
    };
    let (obj, origin, angles, now) = {
        let zm = world.resource::<Zm>();
        let Some(e) = zm.ents.get(&n) else {
            return;
        };
        (e.obj, e.origin, e.rot, zm.clock_millis)
    };
    // Up/down loops start silent (`setloopstate(alias, 0, 0)`); the rest at
    // their alias's level until the first step sets them.
    let level = ty
        .parts
        .iter()
        .map(|p| match p.drive {
            Drive::Up | Drive::Down | Drive::Vertical | Drive::Turn => (0.0, 1.0),
            _ => (1.0, 1.0),
        })
        .collect();
    world
        .get_resource_or_insert_with(EngineSounds::default)
        .by_ent
        .insert(
            n,
            Engine {
                obj,
                vehicletype: vehicletype.to_ascii_lowercase(),
                ty,
                loops_started: false,
                last: origin,
                last_ms: now,
                speed_mph: 0.0,
                run_at: now,
                window_at: now + 100,
                window_z: origin[2],
                window_yaw: angles[1].abs(),
                z_diff: 0.0,
                level,
            },
        );
}

/// `_helicopter_sounds.csc`'s `terrain_trace_brass` for the Escort Drone:
/// about once a second (1 to 1.2 s) it traces straight down from where it
/// is, once it has moved 12 units since the last trace, and while its gun
/// fires (`vehicle_is_firing`) the spent brass loops on the surface there:
/// `prj_brass_loop_water` on water, `prj_brass_loop_dirt` on the rest of
/// its table, nothing new on a surface outside it. Like the script, the
/// first trace only notes the surface, and the loop starts, moves or stops
/// only on a trace that counts.
struct Brass {
    obj: Option<ObjRef>,
    /// `self.isfiring`.
    firing: bool,
    next_ms: i64,
    /// Where it last traced from, and the surface it found.
    last: Option<[f32; 3]>,
    surf: Option<&'static str>,
    /// The loop that plays and where it lies.
    playing: Option<(&'static str, [f32; 3])>,
    seed: u32,
}

/// `init_terrain_sounds`: the loop for brass on a surface.
fn brass_alias(surface: &str) -> Option<&'static str> {
    Some(match surface {
        "water" => "prj_brass_loop_water",
        "default" | "metal" | "concrete" | "wood" | "dirt" | "gravel" | "grass" | "mud" | "snow"
        | "asphalt" | "brick" | "glass" | "plaster" | "sand" | "rock" | "foliage" => {
            "prj_brass_loop_dirt"
        }
        _ => return None,
    })
}

impl Brass {
    /// `wait( 1 + randomfloatrange( 0, 0.2 ) )`, in milliseconds.
    fn wait_ms(&mut self) -> i64 {
        self.seed = self.seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        1000 + i64::from((self.seed >> 8) % 201)
    }

    /// One trace that counts: the surface under it and where.
    fn traced(&mut self, surf: &'static str, at: [f32; 3]) {
        let prev = self.surf.replace(surf);
        if prev.and_then(brass_alias).is_none() {
            return;
        }
        let Some(new) = brass_alias(surf) else {
            return;
        };
        self.playing = self.firing.then_some((new, at));
    }
}

fn brass_spawned(world: &mut World, n: u32) {
    let zm = world.resource::<Zm>();
    let Some(obj) = zm.ents.get(&n).map(|e| e.obj) else {
        return;
    };
    let now = zm.clock_millis;
    let mut b = Brass {
        obj,
        firing: false,
        next_ms: now,
        last: None,
        surf: None,
        playing: None,
        seed: n.wrapping_mul(0x9e37_79b9) | 1,
    };
    b.next_ms = now + b.wait_ms();
    world.get_resource_or_insert_with(EngineSounds::default).brass.insert(n, b);
}

/// `vehicle_is_firing`: the Escort Drone's gun fires (or stops).
pub(crate) fn set_firing(world: &mut World, n: u32, on: bool) {
    if let Some(mut e) = world.get_resource_mut::<EngineSounds>()
        && let Some(b) = e.brass.get_mut(&n)
    {
        b.firing = on;
    }
}

/// One engine loop this tick: whose it is, its alias, where it plays, how
/// loud and its pitch.
pub(super) struct EngineLoop {
    pub(super) ent: u32,
    pub(super) alias: &'static str,
    pub(super) at: [f32; 3],
    pub(super) volume: f32,
    pub(super) pitch: f32,
}

/// This tick's brass loops (a drone's end with it).
fn brass_rows(game: &mut World, engines: &mut EngineSounds, dest: &mut Vec<EngineLoop>) {
    let now = game.resource::<Zm>().clock_millis;
    let ns: Vec<u32> = engines.brass.keys().copied().collect();
    for nn in ns {
        let living = game.resource::<Zm>().ents.get(&nn).map(|en| (en.obj, en.origin));
        let Some(rhs) = engines.brass.get_mut(&nn) else { continue };
        let Some(spot) = living.filter(|l| l.0 == rhs.obj).map(|l| l.1) else {
            engines.brass.remove(&nn);
            continue;
        };
        if now >= rhs.next_ms {
            rhs.next_ms = now + rhs.wait_ms();
            let moved = rhs.last.is_none_or(|l| {
                let d = [spot[0] - l[0], spot[1] - l[1], spot[2] - l[2]];
                d[0] * d[0] + d[1] * d[1] + d[2] * d[2] >= 144.0
            });
            if moved {
                rhs.last = Some(spot);
                let end = [spot[0], spot[1], spot[2] - 100_000.0];
                let t = super::frame(game).trace_static_world(
                    spot,
                    end,
                    [0.0; 3],
                    [0.0; 3],
                    crate::shot_trace::MASK_SHOT,
                );
                let f = t.fraction.clamp(0.0, 1.0);
                let at = std::array::from_fn(|i| spot[i] + (end[i] - spot[i]) * f);
                let surf = if f < 1.0 {
                    weapon_t6::surface::T6_SURFACE_TYPE_NAMES
                        .get(((t.sflags >> 20) & 0x3f) as usize)
                        .copied()
                        .unwrap_or("default")
                } else {
                    "none"
                };
                rhs.traced(surf, at);
            }
        }
        if let Some((alias, at)) = rhs.playing {
            dest.push(EngineLoop { ent: nn, alias, at, volume: 1.0, pitch: 1.0 });
        }
    }
}

fn sndlog() -> bool {
    std::env::var_os("BO2MP_SNDLOG").is_some()
}

/// This tick's engine loops. A vehicle's loops end with it
/// (`entityshutdown`).
pub(super) fn rows(game: &mut World) -> Vec<EngineLoop> {
    let Some(mut engines) = game.remove_resource::<EngineSounds>() else {
        return Vec::new();
    };
    let mut out = Vec::new();
    {
        let zm = game.resource::<Zm>();
        let now_ms = zm.clock_millis;
        engines.by_ent.retain(|n, eng| {
            let alive = zm.ents.get(n).filter(|e| e.obj == eng.obj);
            let Some(e) = alive else {
                if sndlog() && eng.loops_started {
                    for part in eng.ty.parts {
                        diag::info!(
                            Sim,
                            "bo2mp sndlog engine stop {} on ent {n} ({})",
                            part.loop_name,
                            eng.vehicletype
                        );
                    }
                }
                return false;
            };
            step(eng, e.origin, e.rot, now_ms);
            if !eng.loops_started {
                eng.loops_started = true;
                if sndlog() {
                    for part in eng.ty.parts {
                        diag::info!(
                            Sim,
                            "bo2mp sndlog engine start {} on ent {n} ({})",
                            part.loop_name,
                            eng.vehicletype
                        );
                    }
                }
            }
            for (part, (vol, pitch)) in eng.ty.parts.iter().zip(&eng.level) {
                out.push(EngineLoop { ent: *n, alias: part.loop_name, at: e.origin, volume: *vol, pitch: *pitch });
            }
            true
        });
    }
    {
        let zm = game.resource::<Zm>();
        let now_ms = zm.clock_millis;
        engines.cars.retain(|n, car| {
            let Some(e) = zm.ents.get(n).filter(|e| e.obj == car.obj) else {
                return false;
            };
            car.owed += (now_ms - car.last_ms).max(0) as f32 / 1000.0 / FRAME;
            car.last_ms = now_ms;
            // BO2 runs its engine once a frame: as many frames as went by
            // (at most a second's).
            let frames = (car.owed.floor() as i32).min(60);
            car.owed -= car.owed.floor();
            for _ in 0..frames {
                car.frame();
            }
            // Its loops sit 5 units over its origin.
            let at = [e.origin[0], e.origin[1], e.origin[2] + 5.0];
            let names = car.def.onload.iter().chain(car.def.offload);
            for ((alias, _), (vol, pitch)) in names.zip(&car.level) {
                out.push(EngineLoop { ent: *n, alias: *alias, at, volume: *vol, pitch: *pitch });
            }
            if sndlog() && frames > 0 && now_ms % 1000 < 50 {
                diag::info!(
                    Sim,
                    "bo2mp sndlog car {n}: throttle {:.2} rpm {:.0} levels {:?}",
                    car.throttle,
                    car.rpms,
                    car.level
                );
            }
            true
        });
    }
    brass_rows(game, &mut engines, &mut out);
    game.insert_resource(engines);
    out
}

fn step(eng: &mut Engine, spot: [f32; 3], rot: [f32; 3], now_ms: i64) {
    let secs = (now_ms - eng.last_ms) as f32 / 1000.0;
    if secs > 0.0 {
        let moved: [f32; 3] = std::array::from_fn(|axis| spot[axis] - eng.last[axis]);
        let dist = (moved[0] * moved[0] + moved[1] * moved[1] + moved[2] * moved[2]).sqrt();
        eng.speed_mph = dist / secs / MPH;
        eng.last = spot;
        eng.last_ms = now_ms;
    }
    let ty = eng.ty;
    // The drone's 0.1 s windows (`drone_up_down_transition`,
    // `drone_rotate_angle`).
    if ty.updown && now_ms >= eng.window_at {
        eng.z_diff = eng.window_z - spot[2];
        let turning = (eng.window_yaw - rot[1].abs()).abs();
        eng.window_z = spot[2];
        eng.window_yaw = rot[1].abs();
        eng.window_at = now_ms + 100;
        let z = eng.z_diff;
        for (part, level) in ty.parts.iter().zip(eng.level.iter_mut()) {
            match part.drive {
                Drive::Up => {
                    *level = if z < 0.0 {
                        (scale_speed(5.0, 40.0, 0.0, 1.0, -z), scale_speed(5.0, 40.0, 0.9, 1.1, -z))
                    } else {
                        (0.0, 1.0)
                    };
                }
                Drive::Vertical => {
                    *level = if z < 0.0 {
                        (scale_speed(5.0, 50.0, 0.0, 1.0, -z), scale_speed(5.0, 50.0, 0.9, 1.1, -z))
                    } else {
                        (scale_speed(5.0, 50.0, 0.0, 1.0, z), scale_speed(5.0, 50.0, 0.95, 0.8, z))
                    };
                }
                // Rising leaves the down loop as it was (the script sets it
                // only while falling or level).
                Drive::Down if z >= 0.0 => {
                    *level = (scale_speed(5.0, 50.0, 0.0, 1.0, z), scale_speed(5.0, 50.0, 1.0, 0.8, z));
                }
                Drive::Turn => {
                    *level = (
                        scale_speed(0.0, 5.0, 0.0, 0.4, turning),
                        scale_speed(0.0, 4.0, 0.9, 1.05, turning),
                    );
                }
                _ => {}
            }
        }
    }
    if now_ms >= eng.run_at {
        eng.run_at = now_ms + ty.wait_ms;
        let s = eng.speed_mph;
        let vertical = if ty.updown {
            scale_speed(5.0, 50.0, 0.0, 1.0, eng.z_diff.abs())
        } else {
            0.0
        };
        for (part, level) in ty.parts.iter().zip(eng.level.iter_mut()) {
            if let Drive::Run([sv, v0, v1, sp, p0, p1]) = part.drive {
                *level = (
                    scale_speed(IDLE_RUN, sv, v0, v1, s) - vertical,
                    scale_speed(IDLE_RUN, sp, p0, p1, s),
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scale_speed_clamps_and_maps() {
        assert_eq!(scale_speed(5.0, 65.0, 0.6, 0.8, 0.0), 0.6);
        assert_eq!(scale_speed(5.0, 65.0, 0.6, 0.8, 100.0), 0.8);
        assert!((scale_speed(5.0, 65.0, 0.6, 0.8, 35.0) - 0.7).abs() < 1e-6);
    }

    #[test]
    fn rc_car_revs_and_fades_like_bo2() {
        let mut car = Car {
            obj: None,
            def: &RC_CAR,
            throttle_in: 0.0,
            pace: 0.0,
            wheels_down: true,
            throttle: 0.0,
            rpms: RC_CAR.idle_rpms,
            state: Gear::Idle,
            load: 0.0,
            owed: 0.0,
            last_ms: 0,
            level: vec![(0.0, 1.0); 4],
        };
        car.frame();
        // Idle: the idle loop up, the engine loop low, both under pitch 1.
        let idle = car.level.clone();
        assert!(idle[1].0 < idle[3].0 + 1.0);
        car.throttle_in = 1.0;
        car.pace = 1.0;
        for _ in 0..120 {
            car.frame();
        }
        // Flat out on the ground: the gear's top, the engine loop full and
        // pitched up, the idle loop gone.
        assert_eq!(car.rpms, 2500.0);
        assert!(car.level[2].1 > 1.4 && car.level[0].1 > 1.4);
        assert_eq!(band_zero(&car.level[0]), true);
        car.wheels_down = false;
        for _ in 0..120 {
            car.frame();
        }
        // In the air it revs past the gear's top.
        assert!(car.rpms > 2500.0);
    }

    fn band_zero(l: &(f32, f32)) -> bool {
        l.0 == 0.0
    }

    #[test]
    fn types_follow_the_client_script() {
        assert_eq!(type_of("heli_ai_mp").unwrap().parts[0].loop_name, "veh_hind_rotor_lfe");
        assert_eq!(type_of("heli_player_gunner_mp").unwrap().parts[1].loop_name, "veh_cobra_turbine");
        assert_eq!(type_of("QRDRONE_MP").unwrap().parts.len(), 6);
        assert!(type_of("plane").is_none());
    }
}

#[cfg(test)]
mod brass_tests {
    use super::*;

    fn drone(firing: bool) -> Brass {
        Brass { obj: None, firing, next_ms: 0, last: None, surf: None, playing: None, seed: 1 }
    }

    /// The first trace only notes the surface; the loop for the surface
    /// starts on the next while the gun fires, and stops on one when it
    /// does not; a surface off the table (on either trace) leaves it as it
    /// was.
    #[test]
    fn brass_loops_on_the_surface_under_the_firing_drone() {
        let mut b = drone(true);
        b.traced("concrete", [1.0, 0.0, 0.0]);
        assert_eq!(b.playing, None);
        b.traced("water", [2.0, 0.0, 0.0]);
        assert_eq!(b.playing, Some(("prj_brass_loop_water", [2.0, 0.0, 0.0])));
        b.traced("ice", [3.0, 0.0, 0.0]);
        assert_eq!(b.playing, Some(("prj_brass_loop_water", [2.0, 0.0, 0.0])));
        b.traced("dirt", [4.0, 0.0, 0.0]);
        assert_eq!(b.playing, Some(("prj_brass_loop_water", [2.0, 0.0, 0.0])), "ice before it");
        b.traced("grass", [5.0, 0.0, 0.0]);
        assert_eq!(b.playing, Some(("prj_brass_loop_dirt", [5.0, 0.0, 0.0])));
        b.firing = false;
        b.traced("grass", [6.0, 0.0, 0.0]);
        assert_eq!(b.playing, None);
    }

    #[test]
    fn brass_waits_one_to_one_point_two_seconds() {
        let mut b = drone(false);
        for _ in 0..200 {
            let w = b.wait_ms();
            assert!((1000..=1200).contains(&w), "{w}");
        }
    }
}
