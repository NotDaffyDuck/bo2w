//! Riding and driving scorestreak vehicles (`vehicle usevehicle(player,
//! seat)`): the RC-XD and the Dragonfire are remote controlled, the VTOL
//! Warship is ridden as its gunner. `player unlink()` ends it (BO2's
//! scripts wait for his `unlink`).
//!
//! BO2's scripts run the streak itself (its timer, the RC-XD blowing up when
//! he fires, the HUD's client fields). The engine half kept here: his body
//! stays where he stood while his move keys, view and buttons drive the
//! vehicle; his own gun stays quiet; his screen rides it (the client's
//! vehicle view, `bo2mp_vehicle_view`), and BO2's HUD hears which vehicle
//! he is in (`bo2mp_vehicle`).

use std::sync::OnceLock;

use bevy_ecs::prelude::World;
use gsc_t6::{Array, Native, Value, Vm};

use super::super::{T6VehicleDrive, Zm, arg, entnum, frame};
use super::craft::Aim;
use super::{notify_all, streaks};
use crate::world::ClientId;
use movement_t6::gjk::MoverBrush;
use movement_t6::heli::{HeliCollision, HeliControls, HeliDef, HeliState, heli_move};
use playerstate_t6::game::buttons;

/// The usercmd buttons a command holds ("" holds none).
fn command_bits(command: &str) -> u32 {
    match command {
        "+gostand" => buttons::JUMP,
        "+stance" => buttons::CROUCH | buttons::PRONE,
        "+frag" => buttons::FRAG,
        "+smoke" => buttons::SMOKE,
        "+attack" => buttons::ATTACK,
        "+speed_throw" => buttons::ADS,
        "+melee" => buttons::MELEE_CHARGE,
        "+breath_sprint" => buttons::BREATH,
        _ => 0,
    }
}

/// How a vehicle is driven.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Drive {
    /// On its wheels (RC-XD): forward and back, turning to his view.
    Ground,
    /// A small drone (Dragonfire): flies where he looks, up on jump, down on
    /// crouch.
    Air,
    /// He is its gunner (VTOL Warship): it flies its own path, he aims.
    Gunner,
}

#[derive(Clone, Debug)]
pub(crate) struct Ride {
    pub vehicle: u32,
    /// Its vehicle type (the first word of his HUD's `bo2mp_vehicle`).
    pub kind: String,
    /// The seat he took (`usevehicle`): 0 drives.
    pub seat: i32,
    pub drive: Drive,
    /// Its own driving and camera values (his zones' vehicle def).
    pub info: T6VehicleDrive,
    /// Where his view sits on it in first person (its model's
    /// `tag_player`, model space).
    pub eye: [f32; 3],
    /// Its speed: forward for a ground vehicle, a velocity for a drone.
    pub vel: [f32; 3],
    /// Vertical speed (a ground vehicle falling or jumping).
    pub fall: f32,
    /// A ground vehicle's wheels: its front wheels' angle now (radians),
    /// the length between its front and back wheels, its boost left
    /// (seconds) and whether it is on, its hand brake last tick.
    pub steer: f32,
    pub axle: f32,
    pub boost_left: f32,
    pub boosting: bool,
    pub hand_brake: bool,
    /// Its back wheels (model space) and its client script's driving
    /// effects (`_rcbomb.csc`): when they next play (every 0.1 s), when dirt
    /// last hit his screen and the wait before the next, its boost sound
    /// played (once a car), his screen's splash count, a dice.
    pub back_wheels: [[f32; 3]; 2],
    pub fx_next: i64,
    pub screen_last: i64,
    pub screen_delay: i64,
    pub boost_heard: bool,
    pub screen_seq: u32,
    pub dice: u32,
    /// When each gun may fire again (server ms): turret, gunner.
    pub next_fire: [i64; 2],
    /// His client was told which model his view rides (once it has one).
    pub view_sent: bool,
    /// His keys drive it (BO2's `takeplayercontrol` takes them off it: the
    /// Dragonfire shot down falls on its own).
    pub controlled: bool,
    /// A drone's flight (BO2's helicopter move): its turn speed, lean and
    /// the height it holds, and the brush it slides through the map.
    pub heli: HeliState,
    pub brush: MoverBrush,
}

/// The brush a vehicle's flight slides (BO2's `useCollmap`): its model's
/// first collision piece, else its model's box, else a small box.
fn mover_brush(world: &mut World, model: &str) -> MoverBrush {
    let cap = frame(world).model_capability(model).flatten();
    let corners = |(lo, hi): ([f32; 3], [f32; 3])| -> Vec<[f32; 3]> {
        (0..8)
            .map(|c| std::array::from_fn(|k| if c >> k & 1 == 0 { lo[k] } else { hi[k] }))
            .collect()
    };
    let small = MoverBrush::new(&corners(([-12.0, -12.0, -8.0], [12.0, 12.0, 8.0])))
        .expect("a box's eight corners are a brush");
    cap.as_ref()
        .and_then(|c| c.collmap.as_ref())
        .and_then(|m| MoverBrush::new(&m.brush_verts))
        .or_else(|| cap.as_ref().and_then(|c| c.bounds).and_then(|b| MoverBrush::new(&corners(b))))
        .unwrap_or(small)
}

/// The other scorestreak vehicles a vehicle's move meets (BO2's collision
/// query adds each by its model's collision piece; a hidden one has no
/// contents, a model with none is not in it).
fn other_vehicles(world: &mut World, own: u32) -> Vec<crate::step::OtherVehicle> {
    let crafts: Vec<(u32, bool)> = streaks(world)
        .craft
        .iter()
        .filter(|(n, _)| **n != own)
        .map(|(n, c)| (*n, c.remote_control))
        .collect();
    crafts
        .into_iter()
        .filter_map(|(n, remote)| {
            let e = world.resource::<Zm>().ents.get(&n).filter(|e| !e.hidden)?;
            let (origin, angles, model) = (e.origin, e.angles, e.model.clone());
            let cap = frame(world).model_capability(&model).flatten()?;
            let verts = cap.collmap.as_ref()?.brush_verts.clone();
            crate::step::OtherVehicle::new(u16::try_from(n).ok()?, origin, angles, verts, remote)
        })
        .collect()
}

/// How a drone flies, from its def (BO2's helicopter values).
fn heli_def(info: &T6VehicleDrive) -> HeliDef {
    let h = &info.heli;
    HeliDef {
        max_speed: info.max_speed,
        accel: info.accel,
        max_speed_vertical: info.max_speed_vertical,
        accel_vertical: info.accel_vertical,
        rot_rate: h.rot_rate,
        rot_accel: h.rot_accel,
        max_body_pitch: h.max_body_pitch,
        max_body_roll: h.max_body_roll,
        tilt_from_accel: h.tilt_from_accel,
        tilt_from_decel: h.tilt_from_decel,
        tilt_from_vel: h.tilt_from_vel,
        tilt_speed: h.tilt_speed,
        decel: h.decel,
        tilt_from_axes: h.tilt_from_axes,
        tilt_from_fwd_yaw: h.tilt_from_fwd_yaw,
        vel_at_max_tilt: h.vel_at_max_tilt,
        tilt_momentum: h.tilt_momentum,
        maintain_height: h.maintain_height != 0,
        height_limit: h.height_limit,
        height_min: h.height_min,
        height_max: h.height_max,
        ceiling_min: h.ceiling_min,
    }
}

/// `a - b` in degrees, -180..180.
fn angle_delta(a: f32, b: f32) -> f32 {
    (a - b + 180.0).rem_euclid(360.0) - 180.0
}

/// A velocity that ran into a surface, with the part going into it taken
/// off (it slides along it); `None` when it was leaving it anyway.
fn slide_off(vel: [f32; 3], normal: [f32; 3]) -> Option<[f32; 3]> {
    let into: f32 = (0..3).map(|i| vel[i] * normal[i]).sum();
    (into < 0.0).then(|| std::array::from_fn(|i| vel[i] - normal[i] * into))
}

/// Angles turned on by a spin (degrees a second) over `dt`, pitch and roll
/// held to their most when it has them (`setmaxpitchroll`).
fn spun(ang: [f32; 3], spin: [f32; 3], most: Option<[f32; 2]>, dt: f32) -> [f32; 3] {
    let mut a: [f32; 3] = std::array::from_fn(|i| angle_delta(ang[i] + spin[i] * dt, 0.0));
    if let Some([pitch, roll]) = most {
        a[0] = a[0].clamp(-pitch, pitch);
        a[2] = a[2].clamp(-roll, roll);
    }
    a
}

/// What a back wheel stands on (BO2's `getwheelsurface`: "none" for a
/// surface with no type), or nothing in reach. The wheels meet what its
/// body meets (VEH_GroundTrace traces by the vehicle's clipmask).
fn wheel_surface(world: &mut World, at: [f32; 3]) -> Option<&'static str> {
    let t = frame(world).trace_static_world(
        [at[0], at[1], at[2] + 4.0],
        [at[0], at[1], at[2] - 16.0],
        [0.0; 3],
        [0.0; 3],
        BODY_MASK,
    );
    if t.fraction >= 1.0 {
        return None;
    }
    let surf = ((t.sflags >> 20) & 0x3f) as usize;
    Some(match surf {
        0 => "none",
        n => weapon_t6::surface::T6_SURFACE_TYPE_NAMES.get(n).copied().unwrap_or("none"),
    })
}

/// `_rcbomb.csc`'s surface lists.
fn dirt_surface(s: &str) -> bool {
    matches!(s, "dirt" | "foliage" | "grass" | "gravel" | "mud" | "sand" | "water")
}
fn snow_surface(s: &str) -> bool {
    matches!(s, "ice" | "snow")
}
fn concrete_surface(s: &str) -> bool {
    matches!(
        s,
        "asphalt" | "brick" | "ceramic" | "concrete" | "glass" | "metal" | "none" | "paintedmetal" | "plaster" | "rock"
    )
}

/// The RC-XD's driving effects, every 0.1 s (`_rcbomb.csc`
/// `play_driving_fx`, `play_boost_fx`): dust off its back wheels, the
/// driver's set in first person and everyone else's in third; dirt or dust
/// thrown on his screen; its boost sound once it passes 400.
#[allow(clippy::too_many_arguments)]
fn rcbomb_driving_fx(
    world: &mut World,
    driver: u32,
    ride: &mut Ride,
    origin: [f32; 3],
    yaw: f32,
    gas: f32,
    hand_brake: bool,
    pitch: f32,
    now: i64,
) {
    use crate::EventAudience;
    let info = ride.info;
    let speed = ride.vel[0];
    if speed > 400.0 && !ride.boost_heard {
        ride.boost_heard = true;
        super::super::natives_fx::sound(world, EventAudience::All, "mpl_veh_rc_boost", origin);
    }
    let top = if speed < 0.0 { info.max_speed * info.reverse_scale } else { info.max_speed };
    let fraction = if top > 0.0 { speed.abs() / top } else { 0.0 };
    // Its back wheels touching the ground, and on what.
    let surfaces: Vec<&'static str> = ride
        .back_wheels
        .map(|w| on_vehicle(origin, yaw, w))
        .into_iter()
        .filter_map(|w| wheel_surface(world, w))
        .collect();
    if surfaces.is_empty() {
        return;
    }
    let any = |f: fn(&str) -> bool| surfaces.iter().any(|s| f(s));
    // The snow set is not loaded in multiplayer: nothing on snow.
    let tread = if any(snow_surface) {
        None
    } else if any(dirt_surface) {
        Some("dust")
    } else if any(concrete_surface) {
        Some("concrete")
    } else {
        None
    };
    let base = |t: &str, suffix: &str| format!("vehicle/treadfx/fx_treadfx_rcbomb_{t}{suffix}");
    if let Some(t) = tread {
        // BO2's `ispeelingout` (full throttle under `m_peel_out_max_speed`)
        // and `iswheelsliding` (hand brake on over 100, or sliding sideways
        // faster than `skidSpeedMin`).
        let peeling = gas > 0.9 && speed < info.peel_out_max_speed;
        let sliding = (hand_brake && speed.abs() > 100.0) || ride.vel[1].abs() > info.skid_speed_min;
        for first in [true, false] {
            let fx = if peeling {
                base(t, "_peel")
            } else if sliding {
                base(t, "_drift")
            } else if speed < -5.0 {
                base(t, "_reverse")
            } else if fraction >= 0.25 && (!first || !(fraction < 0.4 && gas < 0.25)) {
                base(t, if first { "_first_person" } else { "" })
            } else if speed > 5.0 {
                base(t, "_slow")
            } else {
                String::new()
            };
            let to = if first {
                EventAudience::Client(ClientId(driver))
            } else {
                EventAudience::AllExcept(ClientId(driver))
            };
            if !fx.is_empty() {
                super::super::natives_fx::bolt_effect_to(world, to.clone(), ride.vehicle, &fx, "tag_origin", false);
            }
            if fraction >= 0.75 && speed > 0.0 {
                super::super::natives_fx::bolt_effect_to(world, to, ride.vehicle, &base(t, "_trail"), "tag_origin", false);
            }
        }
    }
    // Dirt on his screen: driving forward at a quarter of its speed, not
    // looking down past 40 (fewer from 20 down), every 250-500 ms.
    if tread.is_none() || speed <= 0.0 || fraction < 0.25 || pitch >= 40.0 {
        return;
    }
    let extra = if pitch > 20.0 { (1000.0 * (pitch - 20.0) / 20.0) as i64 } else { 0 };
    if ride.screen_last + ride.screen_delay + extra >= now {
        return;
    }
    let mut roll = || {
        ride.dice ^= ride.dice << 13;
        ride.dice ^= ride.dice >> 17;
        ride.dice ^= ride.dice << 5;
        ride.dice
    };
    let pick = (roll() % 4) as usize;
    let delay = 250 + i64::from(roll() % 250);
    let (menu, items) = if any(dirt_surface) {
        ("fullscreen_dirt", ["dirt", "dirt_right_splash", "dirt_left_splash", "blurred_dirt_random"])
    } else {
        ("fullscreen_dust", ["dust", "dust_right_splash", "dust_left_splash", "blurred_dust_random"])
    };
    ride.screen_seq += 1;
    ride.screen_last = now;
    ride.screen_delay = delay;
    let value = format!("{} {menu} {}", ride.screen_seq, items[pick]);
    super::super::set_client_dvar(world, driver, "bo2mp_rc_screen", &value);
}

/// A ground vehicle's box (half width, height).
const GROUND_HALF: f32 = 10.0;
const GROUND_HEIGHT: f32 = 14.0;
/// How far its wheels reach down for the ground under them.
const GROUND_STEP: f32 = 18.0;
/// How high a kerb it rolls up onto (anything taller stops it).
const CLIMB: f32 = 12.0;
/// What stops it in the world: BO2's vehicle clipmask (veh.c 0x800211),
/// vehicle clip but no player clip, so it fits the holes cut for it.
const BODY_MASK: u32 = 0x0080_0211;
/// What it bumps into among things (players, props): what stops a shot.
const THING_MASK: u32 = crate::bullet_collision::MASK_SHOT;
/// Its wheels' radius past their axles (the RC-XD's ~4.7).
const WHEEL_REACH: f32 = 5.0;

/// Where along a car (model x) its three body boxes sit: tail, middle,
/// nose, from its wheels.
fn car_parts(ride: &Ride) -> [f32; 3] {
    let back_x = (ride.back_wheels[0][0] + ride.back_wheels[1][0]) * 0.5;
    let tail = back_x - WHEEL_REACH;
    let nose = back_x + ride.axle + WHEEL_REACH;
    let mid = (tail + nose) * 0.5;
    [
        (tail + GROUND_HALF).min(mid),
        mid,
        (nose - GROUND_HALF).max(mid),
    ]
}

/// How far (0..1) a car's body moves from one place and heading to
/// another before the world stops it. A box already in a wall (it backed
/// up against one) does not hold the rest of it there.
fn body_sweep(
    world: &mut World,
    parts: &[f32; 3],
    from: [f32; 3],
    from_yaw: f32,
    to: [f32; 3],
    to_yaw: f32,
) -> f32 {
    let mins = [-GROUND_HALF, -GROUND_HALF, 2.0];
    let maxs = [GROUND_HALF, GROUND_HALF, GROUND_HEIGHT];
    let mut f = 1.0f32;
    let mut stuck = 0;
    for &x in parts {
        let a = on_vehicle(from, from_yaw, [x, 0.0, 0.0]);
        let b = on_vehicle(to, to_yaw, [x, 0.0, 0.0]);
        let t = frame(world).trace_static_world(a, b, mins, maxs, BODY_MASK);
        if t.startsolid {
            stuck += 1;
            continue;
        }
        f = f.min(t.fraction.clamp(0.0, 1.0));
    }
    if stuck == parts.len() { 0.0 } else { f }
}

/// How far (0..1) a car's body moves before a player (not its driver) or
/// a thing in the world (a care package, a turret, another car) stops it.
#[allow(clippy::too_many_arguments)]
fn body_blockers(
    world: &mut World,
    driver: ClientId,
    car: Option<crate::ScriptModelId>,
    parts: &[f32; 3],
    from: [f32; 3],
    from_yaw: f32,
    to: [f32; 3],
    to_yaw: f32,
) -> f32 {
    use crate::bullet_collision::{BulletTraceQuery, ColliderId, TraceOutcome};
    let mut f = 1.0f32;
    // Players: their boxes against each of its boxes along its move.
    let poses = frame(world).alive_collision_poses();
    for p in poses.iter().filter(|p| p.client != driver) {
        let lo = [p.origin[0] + p.mins[0] - GROUND_HALF, p.origin[1] + p.mins[1] - GROUND_HALF];
        let hi = [p.origin[0] + p.maxs[0] + GROUND_HALF, p.origin[1] + p.maxs[1] + GROUND_HALF];
        for &x in parts {
            let a = on_vehicle(from, from_yaw, [x, 0.0, 0.0]);
            let b = on_vehicle(to, to_yaw, [x, 0.0, 0.0]);
            if a[2] + GROUND_HEIGHT < p.origin[2] + p.mins[2] || a[2] + 2.0 > p.origin[2] + p.maxs[2] {
                continue;
            }
            let d = [b[0] - a[0], b[1] - a[1]];
            let inside = (0..2).all(|i| a[i] > lo[i] && a[i] < hi[i]);
            if inside {
                // Already against him: it may back away, not push on.
                let to_him = [p.origin[0] - a[0], p.origin[1] - a[1]];
                if d[0] * to_him[0] + d[1] * to_him[1] > 0.0 {
                    f = 0.0;
                }
                continue;
            }
            let (mut enter, mut leave) = (0.0f32, 1.0f32);
            for i in 0..2 {
                if d[i].abs() < 1e-6 {
                    if a[i] <= lo[i] || a[i] >= hi[i] {
                        enter = 2.0;
                    }
                    continue;
                }
                let (t0, t1) = ((lo[i] - a[i]) / d[i], (hi[i] - a[i]) / d[i]);
                enter = enter.max(t0.min(t1));
                leave = leave.min(t0.max(t1));
            }
            if enter <= leave && enter <= 1.0 {
                f = f.min(enter.max(0.0));
            }
        }
    }
    // Things: rays ahead of its leading end (nose going forward, tail
    // backing up), low and high, at its middle and both sides.
    let heading = on_vehicle([0.0; 3], from_yaw, [1.0, 0.0, 0.0]);
    let mv = [to[0] - from[0], to[1] - from[1], to[2] - from[2]];
    let len = mv[0].hypot(mv[1]);
    if len < 0.01 {
        return f;
    }
    let dir = [mv[0] / len, mv[1] / len];
    let lead = if heading[0] * mv[0] + heading[1] * mv[1] >= 0.0 {
        parts[2]
    } else {
        parts[0]
    };
    for side in [-(GROUND_HALF - 1.0), 0.0, GROUND_HALF - 1.0] {
        for h in [6.0, GROUND_HEIGHT - 2.0] {
            let a = on_vehicle(from, from_yaw, [lead, side, h]);
            let b = on_vehicle(to, to_yaw, [lead, side, h]);
            let end = [b[0] + dir[0] * GROUND_HALF, b[1] + dir[1] * GROUND_HALF, b[2]];
            let reach = len + GROUND_HALF;
            let hit = frame(world).current_sensor_trace(BulletTraceQuery {
                start: a,
                end,
                mask: THING_MASK,
                ignore: Some(driver),
                ignore_hit: None,
                ignore_model: car,
            });
            let at = match hit {
                TraceOutcome::Hit { fraction, collider, .. }
                    if !matches!(collider, ColliderId::World { .. }) =>
                {
                    fraction
                }
                TraceOutcome::StartSolid {
                    collider: Some(collider),
                    ..
                } if !matches!(collider, ColliderId::World { .. }) => 0.0,
                _ => continue,
            };
            f = f.min(((at * reach - GROUND_HALF) / len).clamp(0.0, 1.0));
        }
    }
    f
}

/// The lean the ground under a car's wheels gives it (pitch, roll in
/// degrees: nose down and right side down are positive): the ground under
/// each wheel, its front pair against its back pair, its left against its
/// right. The ground is what its body stands on (the vehicle clipmask).
fn ground_tilt(world: &mut World, ride: &Ride, at: [f32; 3], yaw: f32) -> [f32; 2] {
    let (sy, cy) = yaw.to_radians().sin_cos();
    let back_x = (ride.back_wheels[0][0] + ride.back_wheels[1][0]) * 0.5;
    let track = (ride.back_wheels[0][1] - ride.back_wheels[1][1]).abs().max(4.0);
    let front_x = back_x + ride.axle;
    let half = track * 0.5;
    // Front left, front right, back left, back right (model space).
    let wheels = [[front_x, half], [front_x, -half], [back_x, half], [back_x, -half]];
    let h = wheels.map(|[x, y]| {
        let wx = at[0] + x * cy - y * sy;
        let wy = at[1] + x * sy + y * cy;
        let t = frame(world).trace_static_world(
            [wx, wy, at[2] + GROUND_STEP],
            [wx, wy, at[2] - GROUND_STEP * 2.0],
            [0.0; 3],
            [0.0; 3],
            BODY_MASK,
        );
        if t.startsolid {
            at[2] + GROUND_STEP
        } else {
            at[2] + GROUND_STEP - GROUND_STEP * 3.0 * t.fraction.clamp(0.0, 1.0)
        }
    });
    let front = (h[0] + h[1]) * 0.5;
    let back = (h[2] + h[3]) * 0.5;
    let left = (h[0] + h[2]) * 0.5;
    let right = (h[1] + h[3]) * 0.5;
    let pitch = (back - front).atan2(ride.axle.max(1.0)).to_degrees();
    let roll = (left - right).atan2(track).to_degrees();
    [pitch.clamp(-45.0, 45.0), roll.clamp(-45.0, 45.0)]
}

/// A vehicle on wheels or treads (the RC-XD, the AGR).
pub(super) fn is_ground(kind: &str) -> bool {
    kind.starts_with("rc_car") || kind.contains("tank")
}

fn drive_of(kind: &str) -> Drive {
    if is_ground(kind) {
        Drive::Ground
    } else if kind.starts_with("qrdrone") {
        Drive::Air
    } else {
        Drive::Gunner
    }
}

/// A tag's place in a model at rest (model space), from his zones.
pub(super) fn tag_in_model(world: &mut World, model: &str, tag: &str) -> Option<[f32; 3]> {
    let cap = frame(world).model_capability(model)??;
    let bone = cap
        .pose
        .bone_names
        .iter()
        .position(|n| n.eq_ignore_ascii_case(tag))?;
    cap.pose.base_mat.get(bone).map(|(_, t)| t.to_array())
}

/// A driven vehicle's speed in the world (units a second).
pub(super) fn velocity_of(world: &World, ent: u32) -> Option<[f32; 3]> {
    let ride = world
        .get_resource::<super::Streaks>()?
        .rides
        .values()
        .find(|r| r.vehicle == ent)?;
    if !matches!(ride.drive, Drive::Ground) {
        return Some(ride.vel);
    }
    // On the ground: along it, across it, and its turn.
    let yaw = world.resource::<Zm>().ents.get(&ent)?.angles[1];
    let [f, s, _] = ride.vel;
    let (sy, cy) = yaw.to_radians().sin_cos();
    Some([cy * f + sy * s, sy * f - cy * s, 0.0])
}

/// Where the rider in `seat` of vehicle `n` sees from now (its
/// `tag_player`, turned with it as his aim is), and its vehicle def's type;
/// none when nobody sits there.
pub(super) fn seat_view(world: &World, n: u32, seat: i32) -> Option<([f32; 3], i32)> {
    let ride = world
        .get_resource::<super::Streaks>()?
        .rides
        .values()
        .find(|r| r.vehicle == n && r.seat == seat)?;
    let e = world.resource::<Zm>().ents.get(&n)?;
    Some((on_vehicle(e.origin, e.angles[1], ride.eye), ride.info.vehicle_type))
}

/// A model-space point on a vehicle turned by `yaw` and placed at `origin`.
fn on_vehicle(origin: [f32; 3], yaw: f32, local: [f32; 3]) -> [f32; 3] {
    let (s, c) = yaw.to_radians().sin_cos();
    [
        origin[0] + c * local[0] - s * local[1],
        origin[1] + s * local[0] + c * local[1],
        origin[2] + local[2],
    ]
}

/// He rides a missile (the Hellstorm) that has not let him go.
fn in_missile(world: &mut World, id: ClientId) -> bool {
    frame(world)
        .client_meta(id)
        .and_then(|m| m.remote_missile)
        .is_some_and(|l| l.unlink_at_ms.is_none())
}

/// Ends his ride: his own controls back, his screen back, `unlink`.
pub(super) fn end_ride(world: &mut World, client: u32) {
    if end_ride_quiet(world, client).is_some() {
        notify_all(world, vec![(client, "unlink", Vec::new())]);
    }
}

/// Ends his ride from inside a script call (`player unlink()`, a new
/// `usevehicle`), where the scripts' machine is already running: BO2's
/// `VEH_UnlinkPlayer` tells him `unlink` (the vehicle as its argument) and
/// the vehicle `unlink` (him), which the RC-XD's `exitcarwaiter` waits on
/// to put the laptop away.
pub(super) fn end_ride_in(vm: &mut Vm<World>, world: &mut World, client: u32) {
    let Some(vehicle) = end_ride_quiet(world, client) else {
        return;
    };
    let (player, car) = {
        let zm = world.resource::<Zm>();
        (
            zm.players.get(&client).map(|p| p.obj),
            zm.ents.get(&vehicle).and_then(|e| e.obj),
        )
    };
    let as_value = |o: Option<gsc_t6::ObjRef>| o.map(Value::Object).unwrap_or(Value::Undefined);
    if let Some(p) = player
        && vm.alive(p)
    {
        vm.notify_str(world, p, "unlink", &[as_value(car)]);
    }
    if let Some(c) = car
        && vm.alive(c)
    {
        vm.notify_str(world, c, "unlink", &[as_value(player)]);
    }
}

/// Ends his ride without telling the scripts; the vehicle he rode.
fn end_ride_quiet(world: &mut World, client: u32) -> Option<u32> {
    let ride = streaks(world).rides.remove(&client)?;
    if let Some(c) = streaks(world).craft.get_mut(&ride.vehicle) {
        c.rider = None;
    }
    {
        let mut f = frame(world);
        if f.client_meta(ClientId(client)).is_some() {
            let m = f.client_meta_mut(ClientId(client));
            m.controls.weapons_disabled = false;
            m.controls.jump_disabled = false;
            m.view_pitch_clamp = None;
        }
    }
    super::super::set_client_dvar(world, client, "bo2mp_vehicle_view", "");
    super::clear_vehicle_hud(world, client, &ride.kind);
    Some(ride.vehicle)
}

pub(super) fn bind(vm: &mut Vm<World>) {
    macro_rules! m {
        ($name:literal, $body:expr) => {
            vm.bind($name, true, $body);
        };
    }
    // `vehicle usevehicle(player, seat)`: he takes it.
    m!("usevehicle", |vm, world, s, a| {
        let (Some(v), Ok(id)) = (entnum(vm, s), super::super::client(vm, world, arg(a, 0))) else {
            return Ok(Value::Undefined);
        };
        let seat = arg(a, 1).as_int().unwrap_or(0);
        let kind = match s {
            Value::Object(o) => {
                let f = vm.intern("vehicletype");
                vm.to_text(&vm.raw_field(*o, f))
            }
            _ => String::new(),
        };
        end_ride_in(vm, world, id.0);
        let drive = drive_of(&kind);
        let info = streaks(world)
            .vehicle_drive
            .get(&kind.to_ascii_lowercase())
            .copied()
            .unwrap_or_default();
        let model = world
            .resource::<Zm>()
            .ents
            .get(&v)
            .map(|e| e.model.clone())
            .unwrap_or_default();
        let eye = tag_in_model(world, &model, "tag_player").unwrap_or([0.0; 3]);
        // Front to back wheels (BO2's `m_steer_front_back_length`).
        let axle = match (
            tag_in_model(world, &model, "tag_wheel_front_left"),
            tag_in_model(world, &model, "tag_wheel_back_left"),
        ) {
            (Some(f), Some(b)) if (f[0] - b[0]).abs() > 1.0 => (f[0] - b[0]).abs(),
            _ => 16.0,
        };
        let back_wheels = ["tag_wheel_back_left", "tag_wheel_back_right"]
            .map(|t| tag_in_model(world, &model, t).unwrap_or([-axle * 0.5, 0.0, 6.0]));
        let brush = mover_brush(world, &model);
        streaks(world).rides.insert(
            id.0,
            Ride {
                vehicle: v,
                kind: kind.clone(),
                seat,
                drive,
                info,
                eye,
                vel: [0.0; 3],
                fall: 0.0,
                steer: 0.0,
                axle,
                boost_left: info.boost_duration,
                boosting: false,
                hand_brake: false,
                back_wheels,
                fx_next: 0,
                screen_last: 0,
                screen_delay: 0,
                boost_heard: false,
                screen_seq: 0,
                dice: v.wrapping_mul(2_654_435_761) | 1,
                next_fire: [0; 2],
                view_sent: false,
                controlled: true,
                heli: HeliState::default(),
                brush,
            },
        );
        // His view keeps to its limits: the gun's in first person, the
        // camera's in third.
        let pitch = if info.third_person_driver == 0 {
            info.turret_pitch
        } else {
            info.camera_pitch
        };
        {
            let mut f = frame(world);
            if f.client_meta(id).is_some() {
                f.client_meta_mut(id).view_pitch_clamp = (pitch != [0.0; 2]).then_some(pitch);
            }
        }
        if let Some(c) = streaks(world).craft.get_mut(&v) {
            c.rider = Some(id.0);
            // Driven, not flown to goals.
            if drive != Drive::Gunner {
                c.goal = None;
            }
        }
        // His view starts the vehicle's way.
        let yaw = world
            .resource::<Zm>()
            .ents
            .get(&v)
            .map_or(0.0, |e| e.angles[1]);
        if drive != Drive::Gunner {
            super::super::set_player_view(world, id, [0.0, yaw, 0.0]);
        }
        // "<type> <seat> <up> <down> <switch seat> <attack> <attack 2>": the
        // last five are the commands his keys give the vehicle's buttons
        // ("-" none), the HUD's key prompts (`[{+vehiclemoveup}]`).
        let buttons = info.buttons.map(|c| if c.is_empty() { "-" } else { c }).join(" ");
        super::super::set_client_dvar(world, id.0, "bo2mp_vehicle", &format!("{kind} {seat} {buttons}"));
        // The RC-XD's lights (its client script's `spawned`).
        if kind.to_ascii_lowercase().starts_with("rc_car") {
            super::super::clientfields::rcbomb_spawned(vm, world, s);
        }
        // The Dragonfire's (its `spawned`: `qrdronefx( 0 )`).
        if kind.to_ascii_lowercase().starts_with("qrdrone") {
            super::super::clientfields::qrdrone_spawned(vm, world, s);
        }
        diag::info!(
            Sim,
            "bo2mp vehicle: player {} takes {kind} (ent {v}, seat {seat}, {drive:?}, {model} \
             tag_player {eye:?}, axle {axle}, {info:?})",
            id.0
        );
        Ok(Value::Undefined)
    });
    // He lets go (`player unlink()`), or an entity comes off its parent.
    m!("unlink", |vm, world, s, _| {
        let Some(n) = entnum(vm, s) else {
            return Ok(Value::Undefined);
        };
        if streaks(world).rides.contains_key(&n) {
            end_ride_in(vm, world, n);
        }
        super::view_link::end(world, n);
        let mut zm = world.resource_mut::<Zm>();
        if let Some(p) = zm.players.get_mut(&n) {
            p.linked = None;
        } else if let Some(e) = zm.ents.get_mut(&n) {
            e.link = None;
        }
        Ok(Value::Undefined)
    });
    // He steers something from where he stands: drives a vehicle, or rides
    // the Hellstorm's missile (bots and objectives leave him be).
    m!("isremotecontrolling", |vm, world, s, _| {
        let n = entnum(vm, s).unwrap_or(u32::MAX);
        let driving = streaks(world)
            .rides
            .get(&n)
            .is_some_and(|r| r.drive != Drive::Gunner);
        Ok(Value::bool(driving || in_missile(world, ClientId(n))))
    });
    m!("isinvehicle", |vm, world, s, _| {
        let n = entnum(vm, s).unwrap_or(u32::MAX);
        Ok(Value::bool(streaks(world).rides.contains_key(&n)))
    });
    // Who sits in a seat (`vehicle getseatoccupant(seat)`: the RC-XD blows
    // up on a death trigger only while driven), and his seat
    // (`vehicle getoccupantseat(player)`).
    m!("getseatoccupant", |vm, world, s, a| {
        let v = entnum(vm, s).unwrap_or(u32::MAX);
        let seat = arg(a, 0).as_int().unwrap_or(0);
        let rider = streaks(world)
            .rides
            .iter()
            .find(|(_, r)| r.vehicle == v && r.seat == seat)
            .map(|(c, _)| *c);
        Ok(rider.map_or(Value::Undefined, |c| super::craft::world_obj(world, c)))
    });
    m!("getoccupantseat", |vm, world, s, a| {
        let v = entnum(vm, s).unwrap_or(u32::MAX);
        let who = entnum(vm, arg(a, 0)).unwrap_or(u32::MAX);
        let seat = streaks(world)
            .rides
            .get(&who)
            .filter(|r| r.vehicle == v)
            .map(|r| r.seat);
        Ok(seat.map_or(Value::Undefined, Value::Int))
    });
    m!("getvehicleoccupied", |vm, world, s, _| {
        let n = entnum(vm, s).unwrap_or(u32::MAX);
        let v = streaks(world).rides.get(&n).map(|r| r.vehicle);
        Ok(v.map_or(Value::Undefined, |v| super::craft::world_obj(world, v)))
    });
    // `vehicle launchvehicle(velocity, point, ...)`: a push (the RC-XD's
    // jump).
    m!("launchvehicle", |vm, world, s, a| {
        let v = entnum(vm, s).unwrap_or(u32::MAX);
        let push = arg(a, 0).as_vec3().unwrap_or([0.0; 3]);
        let mut st = streaks(world);
        if let Some(r) = st.rides.values_mut().find(|r| r.vehicle == v) {
            // BO2's push is a small number the physics scales; a hop.
            r.fall = (push[2] * 30.0).clamp(0.0, 400.0);
        }
        Ok(Value::Undefined)
    });
    // Who rides it, seat by seat (`vehicle getvehoccupants()`).
    m!("getvehoccupants", |vm, world, s, _| {
        let v = entnum(vm, s).unwrap_or(u32::MAX);
        let mut riders: Vec<(i32, u32)> = streaks(world)
            .rides
            .iter()
            .filter(|(_, r)| r.vehicle == v)
            .map(|(c, r)| (r.seat, *c))
            .collect();
        riders.sort_unstable();
        let mut out = Array::new();
        for (_, c) in riders {
            out.push(super::craft::world_obj(world, c));
        }
        Ok(Value::array(out))
    });
    // Its model (`vehicle vehgetmodel()`); BO2 errors on anything else.
    m!("vehgetmodel", |vm, world, s, _| {
        let n = entnum(vm, s).unwrap_or(u32::MAX);
        let model = super::is_vehicle(world, n)
            .then(|| world.resource::<Zm>().ents.get(&n).map(|e| e.model.clone()))
            .flatten()
            .filter(|m| !m.is_empty());
        match model {
            Some(m) => Ok(vm.string(&m)),
            None => Err("Incorrect usage of GetModelName()".to_owned()),
        }
    });
    // bo2mp: how many vehicles a map may have (`getmaxvehicles()`:
    // `_vehicles.gsc` counts its map's own against it). Ours: any entity
    // may be one, so all of them (1024).
    vm.bind("getmaxvehicles", false, |_, _, _, _| Ok(Value::Int(1024)));
    // A seat's weapon (`vehicle seatgetweapon(seat)`): the driver's is the
    // vehicle's own (its turret's, or `setvehweapon`), a gunner's its def's.
    m!("seatgetweapon", |vm, world, s, a| {
        let v = entnum(vm, s).unwrap_or(u32::MAX);
        let seat = arg(a, 0).as_int().unwrap_or(0);
        let w = streaks(world).craft.get(&v).and_then(|c| match seat {
            0 => Some(c.weapon.clone()),
            n => usize::try_from(n - 1).ok().and_then(|g| c.gunners.get(g).cloned()),
        });
        Ok(match w.filter(|w| !w.is_empty()) {
            Some(w) => vm.string(&w),
            None => Value::Undefined,
        })
    });
    // `vehicle takeplayercontrol()`: its driver's keys stop driving it
    // (the Dragonfire shot down: its scripts crash it).
    m!("takeplayercontrol", |vm, world, s, _| {
        let v = entnum(vm, s).unwrap_or(u32::MAX);
        for r in streaks(world).rides.values_mut() {
            if r.vehicle == v && r.seat == 0 {
                r.controlled = false;
            }
        }
        Ok(Value::Undefined)
    });
    // The water's top over or under a point (`getwaterheight(point)`: the
    // Dragonfire dies in water): BO2 looks from 200 above it to 200 below
    // through water only, else there is none.
    vm.bind("getwaterheight", false, |_, world, _, a| {
        use movement_t6::water::{CONTENTS_WATER, NO_WATER, WATER_SEARCH};
        let p = super::super::vec3(a, 0)?;
        let start = [p[0], p[1], p[2] + WATER_SEARCH];
        let end = [p[0], p[1], p[2] - WATER_SEARCH];
        let t = frame(world).trace_static_world(start, end, [0.0; 3], [0.0; 3], CONTENTS_WATER);
        Ok(Value::Float(if t.fraction < 1.0 {
            start[2] + (end[2] - start[2]) * t.fraction
        } else {
            NO_WATER
        }))
    });
    // A driven vehicle's velocity is its ride's (`getvelocity`: the
    // Dragonfire's crash pushes on from it); anything else as before.
    let _ = PREV_VELOCITY.set(vm.bound("getvelocity", true));
    m!("getvelocity", |vm, world, s, a| {
        let v = entnum(vm, s).unwrap_or(u32::MAX);
        let yaw = world.resource::<Zm>().ents.get(&v).map_or(0.0, |e| e.angles[1]);
        let ridden = streaks(world)
            .rides
            .values()
            .find(|r| r.vehicle == v)
            .map(|r| match r.drive {
                // Forward and side speed along its heading, and its fall.
                Drive::Ground => {
                    let (sy, cy) = yaw.to_radians().sin_cos();
                    [cy * r.vel[0] + sy * r.vel[1], sy * r.vel[0] - cy * r.vel[1], r.fall]
                }
                Drive::Air => r.vel,
                Drive::Gunner => [0.0; 3],
            });
        match (ridden, PREV_VELOCITY.get().copied().flatten()) {
            (Some(vel), _) => Ok(Value::Vec3(vel)),
            (None, Some(f)) => f(vm, world, s, a),
            (None, None) => Ok(Value::Vec3([0.0; 3])),
        }
    });
}

/// The `getvelocity` bound before this file's (a player's).
static PREV_VELOCITY: OnceLock<Option<Native<World>>> = OnceLock::new();

/// Each tick (his command is in): drive each ridden vehicle and keep his
/// body where it stood.
pub(super) fn advance(world: &mut World) {
    let rides: Vec<(u32, Ride)> = streaks(world)
        .rides
        .iter()
        .map(|(c, r)| (*c, r.clone()))
        .collect();
    if rides.is_empty() {
        return;
    }
    let dt = crate::MATCH_TICK_MS as f32 / 1000.0;
    let now = world.resource::<Zm>().now_ms;
    for (c, mut ride) in rides {
        let id = ClientId(c);
        // Gone (blown up, deleted): the ride ends. His death does not end
        // it: BO2's `_rcbomb.gsc` never threads `cardeathwaiter`, and his
        // `killstreak_waitamount` holds his killcam and respawn until the
        // car is done, so he drives on dead.
        let Some((pos, ang)) = world
            .resource::<Zm>()
            .ents
            .get(&ride.vehicle)
            .map(|e| (e.origin, e.angles))
        else {
            end_ride(world, c);
            continue;
        };
        // His screen rides the vehicle's model once it has one (it is
        // made the tick after the vehicle), with its def's own values:
        // "<model> 0 <range> <height> 0 <up> <down> <fov>" behind it (third
        // person), "<model> 1 <x> <y> <z> <up> <down> <fov>" from its
        // `tag_player` (first person).
        if !ride.view_sent {
            let model = world
                .resource::<Zm>()
                .presences
                .by_ent
                .get(&ride.vehicle)
                .map(|p| (p.id.to_wire(), p.number));
            if let Some((m, number)) = model {
                let i = ride.info;
                let (mode, at, [up, down]) = if i.third_person_driver == 0 {
                    (1, ride.eye, i.turret_pitch)
                } else {
                    (0, [i.camera_range, i.camera_height, 0.0], i.camera_pitch)
                };
                let [x, y, z] = at;
                // Then its camera's spring and its top speed.
                let view = format!(
                    "{m} {mode} {x} {y} {z} {up} {down} {} {number} {} {} {}",
                    i.camera_fov, i.camera_spring[0], i.camera_spring[1], i.max_speed
                );
                super::super::set_client_dvar(world, c, "bo2mp_vehicle_view", &view);
                ride.view_sent = true;
            }
        }
        // His command: read it, then keep his body still.
        let (fwd, right, held, cmd_angles) = {
            let mut req = world.resource_mut::<crate::step::StepRequest>();
            let mut out = (0i8, 0i8, 0u32, None);
            for (cid, cmd) in &mut req.input.cmds {
                if *cid != id {
                    continue;
                }
                out = (cmd.forwardmove, cmd.rightmove, out.2 | cmd.buttons, Some(cmd.angles));
                cmd.forwardmove = 0;
                cmd.rightmove = 0;
            }
            out
        };
        // While he rides a missile his keys steer it, not this; nor once
        // its scripts took it off him.
        let (fwd, right, held) = if in_missile(world, id) || !ride.controlled {
            (0, 0, 0)
        } else {
            (fwd, right, held)
        };
        {
            let mut f = frame(world);
            if f.client_meta(id).is_some() {
                let ctl = &mut f.client_meta_mut(id).controls;
                ctl.weapons_disabled = true;
                ctl.jump_disabled = true;
            }
        }
        // Blown up (BO2's `blowup` hides it, then waits 1 s before he
        // leaves it): it stops where it went off and takes no more of his
        // keys, so his view stays on the blast.
        if matches!(ride.drive, Drive::Ground)
            && world.resource::<Zm>().ents.get(&ride.vehicle).is_some_and(|e| e.hidden)
        {
            ride.vel = [0.0; 3];
            let kind = ride.kind.clone();
            super::super::engine_sounds::car_drive(world, ride.vehicle, &kind, 0.0, 0.0, true);
            if let Some(r) = streaks(world).rides.get_mut(&c)
                && r.vehicle == ride.vehicle
            {
                *r = ride;
            }
            continue;
        }
        // Dead, his own view stays where he fell, but his mouse still
        // steers the car: its look comes straight from his command.
        let view = frame(world).player(id).map_or([0.0; 3], |ps| {
            match cmd_angles {
                Some(a)
                    if matches!(
                        ps.pm_type,
                        playerstate_t6::game::PM_TYPE_DEAD | playerstate_t6::game::PM_TYPE_DEAD_LINKED
                    ) =>
                {
                    std::array::from_fn(|i| {
                        let d = a[i] as f32 * weapon_t6::recoil::SHORT2ANGLE + ps.delta_angles[i];
                        (d + 180.0).rem_euclid(360.0) - 180.0
                    })
                }
                _ => ps.viewangles,
            }
        });
        let (f3, _, _) = gsc_t6::math::angle_vectors(view);
        let (raw_fwd, raw_right) = (fwd, right);
        let fwd = f32::from(fwd) / 127.0;
        let mut new_pos = pos;
        let mut new_ang = ang;
        let info = ride.info;
        // Its speed heads for what his keys ask at its own acceleration.
        let toward = |v: f32, want: f32, accel: f32| {
            let d = want - v;
            v + d.clamp(-accel * dt, accel * dt)
        };
        // What it hit, told its scripts once it is kept (they push it off).
        let mut hits = Vec::new();
        match ride.drive {
            Drive::Air if !ride.controlled => {
                // Off his keys (the Dragonfire shot down): it flies on its
                // own physics, falling at its scripts' acceleration
                // (`setphysacceleration`), pushed by them
                // (`setvehvelocity`) and spinning as they set it
                // (`setangularvelocity`), its tilt held to its most
                // (`setmaxpitchroll`). What it hits stops it going in, and
                // its scripts hear it (`veh_collision`, its velocity and the
                // surface's normal: a wall nudges it off, the ground ends
                // it).
                let (accel, spin, most) = streaks(world)
                    .craft
                    .get(&ride.vehicle)
                    .map_or(([0.0; 3], [0.0; 3], None), |c| (c.phys_accel, c.rot_vel, c.max_pitch_roll));
                for i in 0..3 {
                    ride.vel[i] += accel[i] * dt;
                }
                let want: [f32; 3] = std::array::from_fn(|i| pos[i] + ride.vel[i] * dt);
                let t = frame(world).trace_static_world(
                    pos,
                    want,
                    [-12.0, -12.0, -8.0],
                    [12.0, 12.0, 8.0],
                    crate::bullet_collision::MASK_SHOT,
                );
                let f = if t.startsolid {
                    0.0
                } else {
                    t.fraction.clamp(0.0, 1.0)
                };
                new_pos = std::array::from_fn(|i| pos[i] + (want[i] - pos[i]) * f);
                if f < 1.0
                    && let Some(off) = slide_off(ride.vel, t.normal)
                {
                    hits.push((
                        ride.vehicle,
                        "veh_collision",
                        vec![Value::Vec3(ride.vel), Value::Vec3(t.normal)],
                    ));
                    ride.vel = off;
                }
                new_ang = spun(ang, spin, most, dt);
            }
            Drive::Ground => {
                // BO2's NitrousVehicle in its "strafe" camera mode (the
                // RC-XD's cameraMode 2): forward drives it toward where he
                // looks, back reverses it; it turns on its front wheels.
                let speed = ride.vel[0];
                let car_yaw = ang[1];
                // His stick past its dead zone (40 of 127), scaled over the
                // rest (88): a key is full.
                let stick = ((fwd.abs() * 127.0 - 40.0) / 88.0).clamp(0.0, 1.0);
                // Back aims its tail where he looks (BO2 heads for his
                // look turned round, then takes the half turn off), so it
                // backs up straight instead of swinging round.
                let reversing = fwd < -40.0 / 127.0;
                let mut delta = if reversing {
                    let d = angle_delta(view[1] + 180.0, car_yaw);
                    d - 180.0f32.copysign(d)
                } else {
                    angle_delta(view[1], car_yaw)
                };
                let throttle = if fwd > 40.0 / 127.0 {
                    stick
                } else if reversing {
                    -stick
                } else {
                    0.0
                };
                // Rolling the other way than he asks: the wheels steer the
                // other way.
                if throttle != 0.0 && ((speed > 25.0 && reversing) || (speed < -25.0 && !reversing)) {
                    delta = -delta;
                }
                let mut sf = if delta > 0.0 {
                    1.0
                } else if delta < 0.0 {
                    -1.0
                } else {
                    0.0
                } * if reversing { -1.0 } else { 1.0 };
                let max_deg = info.steer_angle_max.to_degrees() * if reversing { 2.0 } else { 1.0 };
                if delta.abs() < max_deg {
                    sf *= delta.abs() / max_deg;
                }
                let want = sf.clamp(-1.0, 1.0) * info.steer_angle_max;
                let turn = info.steer_speed * dt;
                ride.steer += (want - ride.steer).clamp(-turn, turn);
                // Its front wheels show this steer (BO2 networks its steer
                // yaw, full lock 45 degrees), not the turn its grip allows.
                let shown_id = world.resource::<Zm>().presences.by_ent.get(&ride.vehicle).map(|s| s.id);
                if let Some(id) = shown_id {
                    let mut f = frame(world);
                    if let Some(dobj) = f.collision_owner_mut(id).and_then(|r| r.dobj.as_mut()) {
                        let deg = ride.steer / info.steer_angle_max.max(0.01) * 45.0;
                        dobj.semantic_state.vehicle_steer = Some(deg);
                    }
                }
                // Its hand brake: his key, or by itself in a hard turn at
                // speed (`m_auto_hand_brake_min_speed`) when it is not
                // already swinging that way.
                let spin = ride.vel[2];
                let auto = delta.abs() > 30.0
                    && speed.hypot(ride.vel[1]) > info.auto_hand_brake_min_speed
                    && !(delta < 0.0 && spin > 0.0)
                    && !(delta > 0.0 && spin < 0.0);
                let hand_bits = command_bits(info.hand_brake_button) | buttons::FRAG;
                let hand_brake = held & hand_bits != 0 || auto;
                // Its brake: the hand brake on while it rolls against him.
                let brake = ride.hand_brake
                    && throttle != 0.0
                    && ((speed > 25.0 && reversing) || (speed < -25.0 && !reversing));
                ride.hand_brake = hand_brake;
                // Its boost: one tank (`boostDuration`), started by his key
                // for its boost button and burnt to the end.
                if !ride.boosting && ride.boost_left > 0.0 && held & command_bits(info.boost_button) != 0 {
                    ride.boosting = true;
                }
                let (mut top, mut accel) = (info.max_speed, info.accel);
                if ride.boosting {
                    top *= info.boost_speed_increase.max(1.0);
                    accel *= info.boost_accel_mult.max(1.0);
                    ride.boost_left -= dt;
                    if ride.boost_left <= 0.0 {
                        ride.boosting = false;
                    }
                }
                if throttle < 0.0 {
                    top *= info.reverse_scale;
                    accel *= info.reverse_scale;
                }
                // More pull from a standstill (BO2's low_end_speed 300,
                // percent_to_give 1.5).
                if speed.abs() < 300.0 {
                    accel += (1.0 - speed.abs() / 300.0) * accel * 1.5;
                }
                // Its tyres hold it up to their grip (gravity 800).
                let [fric_fwd, fric_side, fric_brake, fric_hand] = info.tire_fric;
                let (grip_fwd, grip_side) = if brake {
                    (fric_brake, fric_brake)
                } else if hand_brake {
                    (if throttle == 0.0 { fric_hand } else { fric_fwd }, fric_hand)
                } else {
                    (fric_fwd, fric_side)
                };
                let grip_fwd = grip_fwd.max(0.05) * 800.0;
                let grip_side = grip_side.max(0.05) * 800.0;
                ride.vel[0] = if brake || (hand_brake && throttle == 0.0) {
                    toward(speed, 0.0, grip_fwd)
                } else if throttle == 0.0 {
                    toward(speed, 0.0, info.accel)
                } else {
                    toward(speed, top * throttle, accel.min(grip_fwd * 2.5))
                };
                ride.vel[1] = toward(ride.vel[1], 0.0, grip_side);
                // It turns on its wheels as far as its front tyres grip
                // (side grip over its speed); locked back wheels let it
                // swing round its front ones.
                let mut yaw_rate = ride.vel[0] * ride.steer.tan() / ride.axle.max(1.0);
                let front = fric_side.max(0.05) * 800.0 / ride.vel[0].abs().max(1.0);
                yaw_rate = yaw_rate.clamp(-front, front);
                if hand_brake {
                    yaw_rate *= 2.0;
                }
                let yaw_rate = yaw_rate.to_degrees();
                ride.vel[2] = yaw_rate;
                let new_yaw = car_yaw + yaw_rate * dt;
                // Sliding: its speed keeps its old heading, the part off
                // its new heading is side slip its tyres wear down.
                let d = (new_yaw - car_yaw).to_radians();
                let (sd, cd) = d.sin_cos();
                let (f0, s0) = (ride.vel[0], ride.vel[1]);
                ride.vel[0] = f0 * cd - s0 * sd;
                ride.vel[1] = f0 * sd + s0 * cd;
                let (sy, cy) = new_yaw.to_radians().sin_cos();
                let heading = [cy, sy];
                let side = [sy, -cy];
                let mv = [
                    (heading[0] * ride.vel[0] + side[0] * ride.vel[1]) * dt,
                    (heading[1] * ride.vel[0] + side[1] * ride.vel[1]) * dt,
                ];
                // Its body is solid the length of the car: three boxes from
                // tail to nose stop it at walls and props, and its nose or
                // tail stops at players and things in the way. Turning is
                // held back with it, so it never swings into a wall.
                let parts = car_parts(&ride);
                let car = world
                    .resource::<Zm>()
                    .presences
                    .by_ent
                    .get(&ride.vehicle)
                    .map(|p| p.id);
                // Up a step, along, then down onto the ground under it
                // (kerbs, slopes and stairs), falling when nothing is there.
                let up = [pos[0], pos[1], pos[2] + CLIMB];
                let f = body_sweep(world, &parts, pos, car_yaw, up, car_yaw);
                let raised = [pos[0], pos[1], pos[2] + CLIMB * f];
                let lift = raised[2] - pos[2];
                let want = [raised[0] + mv[0], raised[1] + mv[1], raised[2]];
                let mut f = body_sweep(world, &parts, raised, car_yaw, want, new_yaw);
                if mv[0].hypot(mv[1]) > 0.01 {
                    f = f.min(body_blockers(world, id, car, &parts, raised, car_yaw, want, new_yaw));
                }
                let new_yaw = car_yaw + (new_yaw - car_yaw) * f;
                new_pos = std::array::from_fn(|i| raised[i] + (want[i] - raised[i]) * f);
                if f < 1.0 {
                    // It hit: it stops dead.
                    ride.vel[0] = 0.0;
                    ride.vel[1] = 0.0;
                }
                ride.fall -= 800.0 * dt;
                let drop = [new_pos[0], new_pos[1], new_pos[2] - lift + ride.fall * dt];
                let f = body_sweep(world, &parts, new_pos, new_yaw, drop, new_yaw);
                if f < 1.0 {
                    ride.fall = 0.0;
                }
                new_pos = std::array::from_fn(|i| new_pos[i] + (drop[i] - new_pos[i]) * f);
                // Its body sits on its four wheels: BO2's nitrous car rides
                // its own physics body on its suspension, so on a kerb, a
                // ramp or rubble it leans with the ground under its wheels
                // (nose down a slope, one side up a kerb); in the air it
                // keeps the lean it left with.
                let on_ground = ride.fall == 0.0;
                let [pitch, roll] = if on_ground {
                    let target = ground_tilt(world, &ride, new_pos, new_yaw);
                    let k = (dt * 12.0).min(1.0);
                    [ang[0] + (target[0] - ang[0]) * k, ang[2] + (target[1] - ang[2]) * k]
                } else {
                    [ang[0], ang[2]]
                };
                new_ang = [pitch, new_yaw.rem_euclid(360.0), roll];
                let kind = ride.kind.clone();
                super::super::engine_sounds::car_drive(
                    world,
                    ride.vehicle,
                    &kind,
                    throttle,
                    ride.vel[0] / info.max_speed.max(1.0),
                    on_ground,
                );
                if ride.kind.to_ascii_lowercase().starts_with("rc_car") && now >= ride.fx_next {
                    ride.fx_next = now + 100;
                    let gas = throttle.max(0.0);
                    rcbomb_driving_fx(world, c, &mut ride, new_pos, new_ang[1], gas, hand_brake, view[0], now);
                }
            }
            Drive::Air => {
                // BO2's helicopter flight (movement_t6::heli): his move keys,
                // and up and down on the vehicle's own buttons (the
                // Dragonfire's moveUpButtonName / moveDownButtonName, on the
                // keys he has for what BO2's controller binds put on them);
                // it turns to his view, leans with what it does, holds its
                // height over the ground, and its brush slides through the
                // map, the players' bodies, the movers and the other
                // scorestreak vehicles.
                // bo2mp: and on the keyboard Space (jump) up and Ctrl
                // (crouch or prone) down, as he plays BO2 on the PC.
                let controls = HeliControls {
                    forward: raw_fwd,
                    right: raw_right,
                    up: held & (command_bits(info.buttons[0]) | buttons::JUMP) != 0,
                    down: held & (command_bits(info.buttons[1]) | buttons::CROUCH | buttons::PRONE) != 0,
                    view_yaw: view[1],
                };
                ride.heli.origin = pos;
                ride.heli.angles = ang;
                ride.heli.velocity = ride.vel;
                let content = frame(world).content();
                let gjk = content.clip_brushes().iter().any(|b| !b.verts.is_empty());
                let others = other_vehicles(world, ride.vehicle);
                let clip = crate::step::VehicleClip::gather(&mut frame(world), ride.vehicle as u16).with_vehicles(others);
                let collision = clip.collision(&content, gjk);
                let flight = HeliCollision { collision: &collision, brush: ride.brush, gjk };
                heli_move(&heli_def(&info), &mut ride.heli, &controls, dt, &flight);
                new_pos = ride.heli.origin;
                new_ang = ride.heli.angles;
                ride.vel = ride.heli.velocity;
            }
            Drive::Gunner => {}
        }
        if let Some(e) = world.resource_mut::<Zm>().ents.get_mut(&ride.vehicle) {
            e.origin = new_pos;
            e.angles = new_ang;
        }
        // His guns: fire shoots the turret's, aim-down-sights the gunner's
        // (the VTOL's rockets), where he looks. The RC-XD's fire is its
        // script's (it blows up).
        // A driven ground vehicle with a gun (the AGR): its fire is its
        // scripts' (`turret_fire` on it: they fire it, pace it, hold it
        // while stunned) at where he looks; aim-down-sight fires its rocket
        // gunner and tells him (`missile_fire`) unless its scripts hold it
        // (reloading). The RC-XD has no gun: its fire is its blowing up.
        let armed_ground = ride.drive == Drive::Ground
            && streaks(world)
                .craft
                .get(&ride.vehicle)
                .is_some_and(|c| !c.weapon.is_empty());
        if armed_ground {
            let eye = on_vehicle(new_pos, new_ang[1], ride.eye);
            let far: [f32; 3] = std::array::from_fn(|i| eye[i] + f3[i] * 8192.0);
            let t = frame(world).trace_static_world(
                eye,
                far,
                [0.0; 3],
                [0.0; 3],
                crate::bullet_collision::MASK_SHOT,
            );
            let fr = t.fraction.clamp(0.0, 1.0);
            let aim: [f32; 3] = std::array::from_fn(|i| eye[i] + (far[i] - eye[i]) * fr);
            let rockets_off = {
                let mut st = streaks(world);
                let Some(craft) = st.craft.get_mut(&ride.vehicle) else {
                    continue;
                };
                craft.aim = Some(Aim::Point(aim));
                craft.gunner_off[0]
            };
            let mut events = Vec::new();
            if held & buttons::ATTACK != 0 && now >= ride.next_fire[0] {
                events.push((ride.vehicle, "turret_fire", Vec::new()));
                ride.next_fire[0] = now + 100;
            }
            if held & buttons::ADS != 0 && now >= ride.next_fire[1] && !rockets_off {
                let interval = super::craft::fire_from(world, ride.vehicle, Some(0), Aim::Point(aim));
                if let Some(ms) = interval {
                    ride.next_fire[1] = now + i64::from(ms.max(50));
                    events.push((c, "missile_fire", Vec::new()));
                }
            }
            notify_all(world, events);
        }
        if ride.drive != Drive::Ground {
            let eye = on_vehicle(new_pos, new_ang[1], ride.eye);
            let far: [f32; 3] = std::array::from_fn(|i| eye[i] + f3[i] * 8192.0);
            let t = frame(world).trace_static_world(
                eye,
                far,
                [0.0; 3],
                [0.0; 3],
                crate::bullet_collision::MASK_SHOT,
            );
            let fr = t.fraction.clamp(0.0, 1.0);
            let aim: [f32; 3] = std::array::from_fn(|i| eye[i] + (far[i] - eye[i]) * fr);
            for (slot, button, gunner) in [
                (0usize, buttons::ATTACK, None),
                (1, buttons::ADS, Some(0usize)),
            ] {
                // A gun that overheats heats while held and locks when full.
                let (may_fire, note) =
                    super::craft::heat_step(world, ride.vehicle, gunner, held & button != 0);
                if let Some(note) = note {
                    notify_all(world, vec![(ride.vehicle, note, Vec::new())]);
                }
                if held & button == 0 || !may_fire || now < ride.next_fire[slot] {
                    continue;
                }
                let interval =
                    super::craft::fire_from(world, ride.vehicle, gunner, Aim::Point(aim));
                if let Some(ms) = interval {
                    ride.next_fire[slot] = now + i64::from(ms.max(50));
                }
            }
        }
        // Kept for next tick (unless its scripts put him in another).
        if let Some(r) = streaks(world).rides.get_mut(&c)
            && r.vehicle == ride.vehicle
        {
            *r = ride;
        }
        notify_all(world, hits);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_fall_onto_the_ground_stops_going_down_and_keeps_sliding() {
        let off = slide_off([30.0, 0.0, -200.0], [0.0, 0.0, 1.0]).expect("it hit");
        assert_eq!(off, [30.0, 0.0, 0.0]);
        // Leaving the surface is no hit.
        assert_eq!(slide_off([0.0, 0.0, 50.0], [0.0, 0.0, 1.0]), None);
    }

    #[test]
    fn a_crash_spin_turns_it_and_holds_its_roll() {
        // The Dragonfire's crash: 180 of pitch, 90 of roll at most.
        let a = spun([0.0, 170.0, 80.0], [100.0, 300.0, 400.0], Some([180.0, 90.0]), 0.05);
        assert!((a[0] - 5.0).abs() < 1e-3);
        // Yaw wraps round past 180.
        assert!((a[1] - -175.0).abs() < 1e-3);
        assert_eq!(a[2], 90.0);
    }

    /// The Dragonfire's def as his zones carry it (qrdrone_mp).
    fn dragonfire() -> T6VehicleDrive {
        T6VehicleDrive {
            max_speed: 352.0,
            accel: 440.0,
            max_speed_vertical: 193.6,
            accel_vertical: 369.6,
            heli: crate::t6::T6HeliDrive {
                rot_rate: 200.0,
                rot_accel: 1500.0,
                max_body_pitch: 20.0,
                max_body_roll: 25.0,
                tilt_from_accel: [0.01, 0.01],
                tilt_from_decel: [0.05, 0.2],
                tilt_from_vel: [0.0, 0.0],
                tilt_speed: [5.0, 5.0],
                use_collmap: 1,
                decel: [2.0, 2.0, 2.5],
                tilt_from_axes: 0.4,
                tilt_from_fwd_yaw: 0.5,
                vel_at_max_tilt: 0.75,
                tilt_momentum: 0.1,
                accel_toward_view: 0,
                maintain_height: 1,
                height_limit: 15.0,
                height_min: 20.0,
                height_max: 200.0,
                ceiling_min: 45.0,
            },
            ..T6VehicleDrive::default()
        }
    }

    /// A wide floor 8 thick, its top at 0, with its corners (for BO2's own
    /// collision).
    fn ground() -> crate::world::SimBrush {
        slab([-4096.0, -4096.0, -8.0], [4096.0, 4096.0, 0.0], crate::bullet_collision::CONTENTS_SOLID)
    }

    /// A box brush of one kind of contents.
    fn slab(lo: [f32; 3], hi: [f32; 3], contents: u32) -> crate::world::SimBrush {
        crate::world::SimBrush {
            planes: vec![
                [1.0, 0.0, 0.0, hi[0]],
                [-1.0, 0.0, 0.0, -lo[0]],
                [0.0, 1.0, 0.0, hi[1]],
                [0.0, -1.0, 0.0, -lo[1]],
                [0.0, 0.0, 1.0, hi[2]],
                [0.0, 0.0, -1.0, -lo[2]],
            ],
            contents,
            plane_surface_flags: vec![0; 6],
            glass_encoded: 0,
            verts: (0..8)
                .map(|c: u32| std::array::from_fn(|k| if c >> k & 1 == 0 { lo[k] } else { hi[k] }))
                .collect(),
            hit_surface_flags: 0,
        }
    }

    /// The RC-XD meets the world by BO2's vehicle clipmask (VEH_InitEntity's
    /// 0x800211): a wall of player clip (the holes cut for it in Nuketown's
    /// fences) lets its body through, vehicle clip and solid stop it, and its
    /// wheels stand on vehicle clip as its body does.
    #[test]
    fn the_rc_xd_meets_vehicle_clip_not_player_clip() {
        const PLAYER_CLIP: u32 = 0x0001_0000;
        const VEHICLE_CLIP: u32 = 0x0000_0200;
        let (bsp, mesh) = (crate::world::SimClipBsp::default(), crate::world::SimClipMesh::default());
        let trace = |brush: crate::world::SimBrush, from: [f32; 3], to: [f32; 3], body: bool| {
            let (mins, maxs) = if body {
                ([-GROUND_HALF, -GROUND_HALF, 2.0], [GROUND_HALF, GROUND_HALF, GROUND_HEIGHT])
            } else {
                ([0.0; 3], [0.0; 3])
            };
            crate::world::clip_trace(&[brush], &bsp, &mesh, from, to, mins, maxs, BODY_MASK, &|_| true).fraction
        };
        let wall = |contents| slab([40.0, -64.0, -8.0], [48.0, 64.0, 64.0], contents);
        let (from, to) = ([0.0; 3], [100.0, 0.0, 0.0]);
        assert_eq!(trace(wall(PLAYER_CLIP), from, to, true), 1.0);
        let solid = trace(wall(crate::bullet_collision::CONTENTS_SOLID), from, to, true);
        assert!(solid > 0.25 && solid < 0.4, "{solid}");
        assert_eq!(trace(wall(VEHICLE_CLIP), from, to, true), solid);
        let ramp = slab([-64.0, -64.0, -8.0], [64.0, 64.0, 6.0], VEHICLE_CLIP);
        let f = trace(ramp, [0.0, 0.0, 24.0], [0.0, 0.0, -12.0], false);
        assert!((f - 0.5).abs() < 0.01, "{f}");
    }

    /// Driven over a map (its box traces, and BO2's own collision): full
    /// forward it flies off at its height, and held down it comes to rest
    /// with its brush on the floor.
    #[test]
    fn the_dragonfire_flies_over_the_map_at_its_height() {
        let def = heli_def(&dragonfire());
        assert_eq!(def.max_body_pitch, 20.0);
        assert!(def.maintain_height);
        let brush = MoverBrush::new(
            &(0..8)
                .map(|c: u32| {
                    std::array::from_fn(|k| if c >> k & 1 == 0 { [-16.0, -16.0, -15.5][k] } else { [16.0, 16.0, 9.5][k] })
                })
                .collect::<Vec<_>>(),
        )
        .expect("a brush");
        let brushes = [ground()];
        let (bsp, mesh) = (crate::world::SimClipBsp::default(), crate::world::SimClipMesh::default());
        for gjk in [false, true] {
            let collision = crate::step::bo2_map_collision(&brushes, &bsp, &mesh, gjk);
            let flight = HeliCollision { collision: &collision, brush, gjk };
            let mut st = HeliState { origin: [0.0, 0.0, 60.0], ..HeliState::default() };
            let ahead = HeliControls { forward: 127, ..HeliControls::default() };
            for _ in 0..40 {
                heli_move(&def, &mut st, &ahead, 0.05, &flight);
            }
            assert!(st.origin[0] > 300.0 && st.velocity[0] > 250.0, "{gjk} {st:?}");
            assert!((st.origin[2] - 60.0).abs() < 0.5, "{gjk} {st:?}");
            assert!(st.angles[0] > 5.0, "{gjk} {st:?}");
            let down = HeliControls { down: true, ..HeliControls::default() };
            for _ in 0..60 {
                heli_move(&def, &mut st, &down, 0.05, &flight);
            }
            assert!((st.origin[2] - 15.5).abs() < 0.2, "{gjk} {st:?}");
        }
    }
}
