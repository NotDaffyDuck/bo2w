//! Script vehicles on paths, as BO2's server moves them: `attachpath(node)`
//! puts a vehicle on its chain of `info_vehicle_node`s (each node's `target`
//! names the next), `startpath()` sets it going. Each think (50 ms) it
//! steers at a point `lookahead` seconds of its speed down the path and
//! moves its speed that way (VP_UpdatePathPos), its speed, look-ahead and
//! radius blended between the nodes; then its yaw, pitch and roll ease
//! there and it leans by its acceleration (VEH_UpdatePathFollow);
//! `reached_node` at each node, `reached_end_node` at the last. A moving
//! vehicle runs over whoever it touches (VEH_TouchEntities): Express's
//! bullet train. The Warthog flies its strafing runs this way; Nuketown's
//! perk machines ride `perk_arrival_vehicle` down from the sky.
//!
//! bo2zm (Zombies): `setspeed(mph, speed_up_rate, decel)` and `setspeedimmediate`
//! take a path vehicle's speed (eased, miles per hour per second) from the
//! nodes until `resumespeed`; `setvehmaxspeed` caps it. Tranzit's bus is
//! driven this way (its stop schedule waits on `reached_node` and polls
//! `getspeed`). A vehicle of a planted type (the bus) stands on its wheels
//! on the road (BO2's VEH_GroundPlant), not at its nodes' height (28 units
//! up). `spawnvehicle`, `getvehiclearray`, `localtoworldcoords` and
//! `worldtolocalcoords` are Zombies' too; the scorestreak crafts rebind the
//! speed and spawn natives in MP (killstreaks/craft.rs). Flying vehicles
//! (`setvehgoalpos`) are `flight.rs`.
//!
//! Not ported: spline paths (a PT_SPLINE start runs as the old path), path
//! offsets, switch nodes, pushing entities, touching anything but players,
//! the riot shield's 1 damage, a driver as the attacker (and his team
//! check), stance-sized player boxes, touches of vehicles off a path.

use super::pairs::KeepByValue;
use std::collections::BTreeMap;

use bevy_ecs::prelude::World;
use gsc_t6::{ObjRef, ScriptValue, Vm};

use super::killstreaks::Streaks;
use super::{T6VehicleDrive, Zm, arg, entnum, num, vec3, with_vm};

const MPH: f32 = 17.6;
/// A server think, seconds.
const DT: f32 = 0.05;
/// Node flags (`spawnflags`, then what SP_info_vehicle_node adds).
const NODE_OFF_PATH: u32 = 2;
const NODE_ROTATE: u32 = 0x1_0000;
const NODE_SPEED: u32 = 0x2_0000;
const NODE_LOOKAHEAD: u32 = 0x4_0000;
const NODE_RADIUS: u32 = 0x8_0000;
const NODE_TENSION: u32 = 0x20_0000;
/// `trackTiltRate`, `trackAngleRate`: how fast a body's lean follows.
const TRACK_TILT_RATE: f32 = 3.0;
const TRACK_ANGLE_RATE: f32 = 1.0;

/// The vehicle types that stand on their wheels (BO2 plants its ground
/// vehicles; Tranzit's bus is the one driven on a path here).
const PLANTED: [&str; 1] = ["zombie_bus"];

/// Its wheel tags, in BO2's order: front left, front right, back left, back
/// right.
const WHEEL_TAGS: [&str; 4] = [
    "tag_wheel_front_left",
    "tag_wheel_front_right",
    "tag_wheel_back_left",
    "tag_wheel_back_right",
];

/// What a wheel stands on: solid, glass and vehicle clip.
const WHEEL_MASK: u32 = 0x211;

/// How fast a wheel off the ground falls faster (units a second, each
/// second: BO2's 40 each 20th of a second).
const WHEEL_GRAVITY: f32 = 800.0;

/// A vehicle node (`vehicle_node_t`).
#[derive(Clone, Debug, Default)]
pub(crate) struct Node {
    pub ent: u32,
    ident: String, next_name: String,
    origin: [f32; 3],
    rot: [f32; 3],
    /// Toward the next node (unit) and how far it is.
    dir: [f32; 3],
    length: f32,
    flags: u32,
    /// Units a second; seconds; units; -1 where none.
    speed: f32,
    look_ahead: f32,
    radius: f32,
    tension: f32,
    next: i32,
    prev: i32,
}

/// Where a vehicle is on its path (`vehicle_pathpos_t`).
#[derive(Clone, Debug, Default)]
pub(crate) struct PathPos {
    node: i32,
    prev: i32,
    end: bool,
    end_of_path: bool,
    frac: f32,
    speed: f32,
    look_ahead: f32,
    width: f32,
    manual_time: f32,
    wlaf: f32,
    distance: f32,
    origin: [f32; 3],
    rot: [f32; 3],
    look_pos: [f32; 3],
}

/// Its body (`vehicle_physic_t`).
#[derive(Clone, Debug, Default)]
pub(crate) struct Phys {
    origin: [f32; 3],
    prev_origin: [f32; 3],
    rot: [f32; 3],
    prev_angles: [f32; 3],
    motion_vec: [f32; 3],
    rot_vel: [f32; 3],
    body_tilt: [f32; 2],
}

#[derive(Clone, Debug, Default)]
pub(crate) struct Vehicle {
    /// Thinks since `attachpath`, for the test log.
    log_ticks: u32,
    pp: PathPos,
    phys: Phys,
    /// Its vehicle def (none: no lean, no crush).
    def: T6VehicleDrive,
    /// On its path (`startpath`) until `setvehgoalpos` takes it off.
    on_path: bool,
    /// Units a second.
    speed: f32,
    /// `reached_end_node` has been told (once per `attachpath`).
    end_told: bool,
}

/// A vehicle's speed state (units a second), as the scripts drive it.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct VSpeed {
    pub now: f32,
    /// The speed the script asked for (`setspeed`); `None` follows the nodes.
    pub target: Option<f32>,
    pub speed_up_rate: f32,
    pub decel: f32,
    /// `setvehmaxspeed`.
    pub max: Option<f32>,
}

/// A vehicle kept on the ground by its wheels.
#[derive(Clone, Debug, Default)]
pub(crate) struct Plant {
    /// Each wheel's place on the vehicle (forward, left, up), from its tags.
    pub wheels: Option<[[f32; 3]; 4]>,
    /// Times its wheel tags were looked for and not found.
    pub misses: u32,
    /// Each wheel's height and how fast it falls.
    pub z: [f32; 4],
    pub fall: [f32; 4],
    /// The heights are set (from where it stood when it set off).
    pub planted: bool
}

#[derive(Clone, Debug, Default)]
pub(crate) struct Vehicles {
    /// The map's vehicle nodes (G_SetupSplinePaths), on the first
    /// `attachpath`.
    pub nodes: Option<Vec<Node>>,
    pub list: BTreeMap<u32, Vehicle>,
    /// What the scripts asked of each vehicle's speed (bo2zm).
    pub speed: BTreeMap<u32, VSpeed>,
    /// Helicopters flying to goals or falling (`flight.rs`).
    pub flights: BTreeMap<u32, super::flight::Flight>,
    /// Vehicles kept on the ground by their wheels, by entity number.
    pub plants: BTreeMap<u32, Plant>,
}

fn sub(lhs: [f32; 3], rhs: [f32; 3]) -> [f32; 3] {
    std::array::from_fn(|i| lhs[i] - rhs[i])
}

fn dot(u: [f32; 3], w: [f32; 3]) -> f32 {
    u[0] * w[0] + u[1] * w[1] + u[2] * w[2]
}

fn len(a: [f32; 3]) -> f32 {
    dot(a, a).sqrt()
}

fn along(o: [f32; 3], dd: [f32; 3], st: f32) -> [f32; 3] {
    [o[0] + dd[0] * st, o[1] + dd[1] * st, o[2] + dd[2] * st]
}

fn norm180(a: f32) -> f32 {
    let t = a * 0.002_777_777_8;
    (t - (t + 0.5).floor()) * 360.0
}

fn clamp1(v: f32) -> f32 {
    v.clamp(-1.0, 1.0)
}

/// `v / d`, 0 where BO2 divides by a zero `maxBodyPitch` (the train).
fn ratio(v: f32, d: f32) -> f32 {
    if d == 0.0 { 0.0 } else { v / d }
}

fn diff_track(tt: f32, ch: f32, rate: f32, step_secs: f32) -> f32 {
    let d = (tt - ch) * rate * step_secs;
    let gap = (tt - ch).abs();
    if gap > 0.001 && d.abs() <= gap { ch + d } else { tt }
}

fn diff_track_angle(mut tt: f32, ch: f32, rate: f32, step_secs: f32) -> f32 {
    while tt - ch > 180.0 {
        tt -= 360.0;
    }
    while tt - ch < -180.0 {
        tt += 360.0;
    }
    let d = (tt - ch) * rate * step_secs;
    let gap = (tt - ch).abs();
    if gap > 0.001 && d.abs() <= gap {
        tt = ch + d;
    }
    norm180(tt)
}

fn vectoangles(v: [f32; 3]) -> [f32; 3] {
    if v[1] == 0.0 && v[0] == 0.0 {
        return [if -v[2] < 0.0 { 270.0 } else { 90.0 }, 0.0, 0.0];
    }
    let mut yaw = v[1].atan2(v[0]).to_degrees();
    if yaw < 0.0 {
        yaw += 360.0;
    }
    let mut pitch = -v[2].atan2(v[0].hypot(v[1])).to_degrees();
    if pitch < 0.0 {
        pitch += 360.0;
    }
    [pitch, yaw, 0.0_f32]
}

fn vectosignedangles(v: [f32; 3]) -> [f32; 3] {
    if v[1] == 0.0 && v[0] == 0.0 {
        return [if 0.0 <= -v[2] { 90.0 } else { -90.0 }, 0.0, 0.0];
    }
    let yaw = v[1].atan2(v[0]).to_degrees();
    [-v[2].atan2(v[0].hypot(v[1])).to_degrees(), yaw, 0.0]
}

/// Forward, right, up.
fn angle_vectors(lhs: [f32; 3]) -> [[f32; 3]; 3] {
    let (s_p, codepoint) = lhs[0].to_radians().sin_cos();
    let (s_y, c_y) = lhs[1].to_radians().sin_cos();
    let (s_r, c_r) = lhs[2].to_radians().sin_cos();
    [
        [codepoint * c_y, codepoint * s_y, -s_p],
        [-s_r * s_p * c_y + c_r * s_y, -s_r * s_p * s_y - c_r * c_y, -s_r * codepoint],
        [c_r * s_p * c_y + s_r * s_y, c_r * s_p * s_y - s_r * c_y, c_r * codepoint],
    ]
}

/// SP_info_vehicle_node for each node entity, then G_SetupSplinePaths:
/// links, directions and lengths, and each node's speed, look-ahead,
/// radius and tension (its own, else blended between the nearest nodes
/// that have one).
fn build_nodes(vm: &mut Vm<World>, world: &World) -> Vec<Node> {
    let keys = [
        "targetname",
        "target",
        "spawnflags",
        "speed",
        "lookahead",
        "radius",
        "tension",
    ]
    .map(|k| vm.intern(k));
    let vm = &*vm;
    let num = |o: ObjRef, k| -> Option<f32> {
        let v = vm.raw_field(o, k);
        v.as_float().or_else(|| vm.to_text(&v).trim().parse().ok())
    };
    let mut nodes: Vec<Node> = world
        .resource::<Zm>()
        .ents
        .iter()
        .where_value(|en| en.class_tag.starts_with("info_vehicle_node"))
        .filter_map(|(nn, en)| {
            let o = en.obj?;
            let mut node = Node {
                ent: *nn,
                ident: vm.to_text(&vm.raw_field(o, keys[0])),
                next_name: vm.to_text(&vm.raw_field(o, keys[1])),
                origin: en.origin,
                rot: en.rot,
                flags: {
                    let v = vm.raw_field(o, keys[2]);
                    v.as_int()
                        .map(|f| f as i64)
                        .or_else(|| vm.to_text(&v).trim().parse().ok())
                        .unwrap_or(0) as u32
                },
                speed: -1.0,
                look_ahead: -1.0,
                radius: -1.0,
                tension: -1.0,
                next: -1,
                prev: -1,
                ..Default::default()
            };
            if en.class_tag == "info_vehicle_node_rotate" {
                node.flags |= NODE_ROTATE;
            }
            if let Some(v) = num(o, keys[3]).filter(|v| *v >= 0.0) {
                node.flags |= NODE_SPEED;
                node.speed = v * MPH;
            }
            if let Some(v) = num(o, keys[4]).filter(|v| *v >= 0.0) {
                node.flags |= NODE_LOOKAHEAD;
                node.look_ahead = v;
            }
            if let Some(v) = num(o, keys[5]).filter(|v| *v >= 0.0) {
                node.flags |= NODE_RADIUS;
                node.radius = v;
            }
            if let Some(v) = num(o, keys[6]).filter(|v| *v >= 0.0) {
                node.flags |= NODE_TENSION;
                node.tension = v;
            }
            Some(node)
        })
        .collect();
    let count = nodes.len();
    for i in 0..count {
        if nodes[i].flags & NODE_OFF_PATH != 0 {
            continue;
        }
        let target = &nodes[i].next_name;
        let mut next = if target.is_empty() {
            -1
        } else {
            nodes
                .iter()
                .position(|nn| nn.ident == *target)
                .map_or(-1, |j| j as i32)
        };
        let name = &nodes[i].ident;
        let prev = if name.is_empty() {
            -1
        } else {
            (0..count)
                .find(|&j| j != i && nodes[j].next_name == *name)
                .map_or(-1, |j| j as i32)
        };
        if next == i as i32 {
            next = -1;
        }
        nodes[i].next = next;
        nodes[i].prev = prev;
    }
    for i in 0..count {
        let nn = &nodes[i];
        let (speed, look_ahead, radius) = (
            if nn.flags & NODE_SPEED != 0 { nn.speed } else { -1.0 },
            if nn.flags & NODE_LOOKAHEAD != 0 { nn.look_ahead } else { -1.0 },
            if nn.flags & NODE_RADIUS != 0 { nn.radius } else { -1.0 },
        );
        let dir = if nn.next >= 0 {
            sub(nodes[nn.next as usize].origin, nn.origin)
        } else if nn.prev >= 0 {
            sub(nn.origin, nodes[nn.prev as usize].origin)
        } else {
            [1.0, 0.0, 0.0]
        };
        let length = len(dir);
        let d = if length == 0.0 { 1.0 } else { length };
        let dir = [dir[0] / d, dir[1] / d, dir[2] / d];
        let nn = &mut nodes[i];
        nn.speed = speed;
        nn.look_ahead = look_ahead;
        nn.radius = radius;
        nn.dir = dir;
        nn.length = length;
        if nn.flags & NODE_ROTATE == 0 {
            nn.rot = vectoangles(dir);
        }
    }
    for i in 0..count {
        let speed = calc(&nodes, i, NODE_SPEED, |nn| nn.speed);
        nodes[i].speed = speed;
        let look_ahead = calc(&nodes, i, NODE_LOOKAHEAD, |nn| nn.look_ahead);
        nodes[i].look_ahead = look_ahead;
        let radius = calc(&nodes, i, NODE_RADIUS, |nn| nn.radius);
        nodes[i].radius = radius;
        let tension = calc(&nodes, i, NODE_TENSION, |nn| nn.tension);
        nodes[i].tension = tension;
        let nn = &mut nodes[i];
        nn.rot = nn.rot.map(norm180);
        if nn.speed <= 0.0 || nn.look_ahead <= 0.0 {
            nn.next = -1;
        }
        if nn.next < 0 {
            if nn.speed <= 0.0 {
                nn.speed = 1.0;
            }
            if nn.look_ahead <= 0.0 {
                nn.look_ahead = 1.0;
            }
        }
    }
    nodes
}

/// VP_CalcNodeSpeed and its kin: the node's own value, else blended by
/// distance between the nearest earlier and later nodes with one (either
/// alone where only one has one, 0 where none does).
fn calc(nodes: &[Node], i: usize, flag: u32, get: fn(&Node) -> f32) -> f32 {
    if nodes[i].flags & flag != 0 {
        return get(&nodes[i]);
    }
    let count = nodes.len();
    let (mut prev_v, mut back) = (-1.0, 0.0);
    let (mut p, mut steps) = (nodes[i].prev, 0);
    while p >= 0 && steps < count {
        let q = &nodes[p as usize];
        back += q.length;
        steps += 1;
        if get(q) >= 0.0 {
            prev_v = get(q);
            break;
        }
        p = q.prev;
        if p == i as i32 {
            break;
        }
    }
    let (mut next_v, mut fwd) = (-1.0, 0.0);
    let (mut x, mut steps) = (i, 0);
    while steps < count {
        let q = &nodes[x];
        steps += 1;
        if get(q) >= 0.0 {
            next_v = get(q);
            break;
        }
        if q.next < 0 || q.next == i as i32 {
            break;
        }
        fwd += q.length;
        x = q.next as usize;
    }
    if prev_v < 0.0 {
        return if next_v < 0.0 { 0.0 } else { next_v };
    }
    if next_v < 0.0 {
        return prev_v;
    }
    let total = fwd + back;
    if total <= 0.0 {
        return 0.0;
    }
    back / total * (next_v - prev_v) + prev_v
}

/// G_VehSetUpPathPos: at node `n`.
fn set_up_path_pos(nodes: &[Node], n: usize) -> PathPos {
    let node = &nodes[n];
    PathPos {
        node: n as i32,
        prev: n as i32,
        speed: node.speed,
        look_ahead: node.look_ahead,
        width: node.radius,
        origin: node.origin,
        look_pos: node.origin,
        rot: node.rot,
        ..Default::default()
    }
}

/// VP_GetLookAheadXYZ: the point its look-ahead reaches down the path.
fn look_ahead_xyz(nodes: &[Node], pp: &PathPos) -> [f32; 3] {
    let mut p = pp.node as usize;
    let mut dist = nodes[p].length * pp.frac + pp.look_ahead * pp.speed + pp.wlaf * pp.width;
    for _ in 0..nodes.len() {
        let n = &nodes[p];
        if n.next < 0 || n.length == 0.0 {
            dist = 0.0;
            break;
        }
        if dist < n.length {
            break;
        }
        dist -= n.length;
        p = n.next as usize;
    }
    along(nodes[p].origin, nodes[p].dir, dist)
}

/// From node `n`, the way to the point its own look-ahead reaches (and how
/// far, 1 at least).
fn node_look(nodes: &[Node], n: usize) -> ([f32; 3], f32) {
    let mut d = nodes[n].look_ahead * nodes[n].speed;
    let (mut p, mut k) = (n, 0);
    loop {
        k += 1;
        let q = &nodes[p];
        if q.next < 0 || q.length == 0.0 {
            d = 1.0;
            break;
        }
        if d < q.length {
            break;
        }
        d -= q.length;
        p = q.next as usize;
        if k >= nodes.len() {
            break;
        }
    }
    let a = sub(along(nodes[p].origin, nodes[p].dir, d), nodes[n].origin);
    let l = len(a);
    (a, if l <= 0.0 { 1.0 } else { l })
}

/// VP_UpdatePathPos: which node it is past and how far along, from where
/// it now is; its speed, look-ahead and width blended there.
fn update_path_pos(nodes: &[Node], pp: &mut PathPos) {
    let count = nodes.len();
    let mut frac = pp.frac;
    let mut cur = pp.node as usize;
    let mut prev = pp.prev;
    let (mut examined, mut steps);
    steps = 0;
    if count == 0 {
        return;
    }
    loop {
        let n = cur;
        steps += 1;
        examined = n;
        let next = nodes[n].next;
        if next < 0 || nodes[n].length == 0.0 {
            frac = 0.0;
            break;
        }
        let facing_x = next as usize;
        let (a, la) = node_look(nodes, n);
        let (b, lb) = node_look(nodes, facing_x);
        let from_cur = sub(pp.origin, nodes[n].origin);
        let to_next = sub(nodes[facing_x].origin, pp.origin);
        let f16 = dot(from_cur, a) / la;
        let f22 = dot(to_next, b) / lb;
        let past = if f22 < 0.0 {
            true
        } else if f16 >= 0.0 {
            let sum = f22 + f16;
            frac = if sum > 0.0 { f16 / sum } else { 0.0 };
            false
        } else if dot(nodes[n].dir, from_cur) >= 0.0 {
            let g = 1.0 - len(to_next) / nodes[n].length;
            frac = g.min(1.0);
            if g <= 0.0 {
                frac = 0.0;
                false
            } else {
                frac > 0.9
            }
        } else {
            frac = 0.0;
            false
        };
        if !past {
            break;
        }
        cur = facing_x;
        pp.distance += nodes[n].length;
        prev = n as i32;
        if steps >= count {
            break;
        }
    }
    pp.end_of_path = nodes[examined].next < 0;
    pp.prev = prev;
    pp.node = cur as i32;
    pp.frac = frac;
    let c = &nodes[cur];
    if c.next < 0 {
        pp.speed = c.speed;
        pp.look_ahead = c.look_ahead;
        pp.width = c.radius;
    } else {
        let x = &nodes[c.next as usize];
        pp.speed = c.speed + (x.speed - c.speed) * frac;
        pp.look_ahead = c.look_ahead + (x.look_ahead - c.look_ahead) * frac;
        pp.width = c.radius + (x.radius - c.radius) * frac;
    }
}

/// VP_GetAngles: between rotate nodes it turns as they say.
fn path_angles(nodes: &[Node], pp: &mut PathPos) {
    let cur = pp.node as usize;
    let next = if nodes[cur].flags & NODE_OFF_PATH != 0 {
        cur as i32 + 1
    } else {
        nodes[cur].next
    };
    let cur_rot = nodes[cur].flags & NODE_ROTATE != 0;
    if next < 0 || next as usize >= nodes.len() {
        if cur_rot {
            pp.rot = nodes[cur].rot;
        }
        return;
    }
    let facing_x = &nodes[next as usize];
    let next_rot = facing_x.flags & NODE_ROTATE != 0;
    let (from, to) = match (cur_rot, next_rot) {
        (true, true) => (nodes[cur].rot, facing_x.rot),
        (false, true) => (pp.rot, facing_x.rot),
        (true, false) => (nodes[cur].rot, pp.rot),
        (false, false) => return,
    };
    for i in 0..3 {
        pp.rot[i] = norm180(norm180(to[i] - from[i]) * pp.frac + from[i]);
    }
}

/// G_VehUpdatePathPos: one think's move toward its look-ahead point.
fn step_path_pos(nodes: &[Node], pp: &mut PathPos) {
    if pp.end {
        return;
    }
    pp.look_pos = look_ahead_xyz(nodes, pp);
    let d = sub(pp.look_pos, pp.origin);
    let l = len(d);
    if l <= 0.0 {
        pp.end = true;
        return;
    }
    let dir = [d[0] / l, d[1] / l, d[2] / l];
    pp.rot = vectoangles(dir).map(norm180);
    pp.origin = along(pp.origin, dir, pp.speed * DT);
    update_path_pos(nodes, pp);
    path_angles(nodes, pp);
}

/// VEH_UpdatePathPosition: this think's place and angles to make for, and
/// the nodes it reached.
fn path_position(nodes: &[Node], veh: &mut Vehicle, sp: &VSpeed) -> ([f32; 3], [f32; 3], Vec<usize>) {
    // The nodes' speed, or what `setspeed` eased it to (bo2zm).
    let node_speed = veh.pp.speed;
    veh.speed = match sp.target {
        Some(t) if veh.speed < t => (veh.speed + sp.speed_up_rate * DT).min(t),
        Some(t) => (veh.speed - sp.decel * DT).max(t),
        None => node_speed,
    };
    if let Some(m) = sp.max {
        veh.speed = veh.speed.min(m);
    }
    let mut ratio = if node_speed > 0.0 {
        veh.speed / node_speed
    } else {
        0.0
    } + veh.pp.manual_time;
    let mut local = veh.pp.clone();
    let start = local.node;
    let mut reached = Vec::new();
    while ratio >= 1.0 {
        step_path_pos(nodes, &mut local);
        ratio -= 1.0;
        if local.node != start {
            reached.push(local.node as usize);
        }
    }
    veh.pp = local.clone();
    if ratio > 0.0 {
        step_path_pos(nodes, &mut local);
    }
    if veh.pp.end {
        veh.speed = 0.0;
    }
    let (a, b) = (veh.pp.origin, local.origin);
    let origin = std::array::from_fn(|i| (b[i] - a[i]) * ratio + a[i]);
    let (a, b) = (veh.pp.rot, local.rot);
    let angles = std::array::from_fn(|i| a[i] + norm180(b[i] - a[i]) * ratio);
    veh.pp.manual_time = ratio;
    (origin, angles, reached)
}

/// VEH_UpdatePathFollow: there, its yaw, pitch and roll eased toward the
/// path's, or leaning by its acceleration.
fn path_follow(ph: &mut Phys, def: &T6VehicleDrive, tgt: [f32; 3], wanted: [f32; 3], autotilt: bool) {
    ph.prev_angles = ph.rot;
    ph.prev_origin = ph.origin;
    ph.origin = tgt;
    let old_vel = ph.motion_vec;
    ph.motion_vec = std::array::from_fn(|i| (ph.origin[i] - ph.prev_origin[i]) * 20.0);
    ph.rot[1] = diff_track_angle(wanted[1], ph.prev_angles[1], 4.0, DT);
    let leans = matches!(def.vehicle_type, 3 | 4 | 6);
    if def.vehicle_type == 3 {
        ph.rot[0] = diff_track_angle(wanted[0], ph.prev_angles[0], 6.0, DT);
    }
    let max_acc = def.speed_up_rate * 0.056_818_184;
    if !autotilt || !leans || max_acc == 0.0 {
        ph.rot[0] = diff_track_angle(wanted[0], ph.prev_angles[0], 6.0, DT);
        ph.rot[2] = diff_track_angle(wanted[2], ph.prev_angles[2], 6.0, DT);
        if leans {
            ph.body_tilt[0] = clamp1(ratio(ph.rot[0], def.heli.max_body_pitch));
            ph.body_tilt[1] = clamp1(ratio(ph.rot[2], def.heli.max_body_roll));
        }
    } else {
        tilt_for_accel(ph, def, old_vel, max_acc, if def.vehicle_type == 3 { 0.75 } else { 1.0 });
    }
    ph.rot_vel = std::array::from_fn(|i| norm180(ph.rot[i] - ph.prev_angles[i]) * 20.0);
}

/// VEH_UpdateTiltForAcceleration: it pitches and rolls with how hard it
/// speeds up and turns.
fn tilt_for_accel(ph: &mut Phys, def: &T6VehicleDrive, old_vel: [f32; 3], max_acc: f32, factor: f32) {
    let t = def.vehicle_type;
    let axis = angle_vectors([if t == 3 { ph.rot[0] } else { 0.0 }, ph.rot[1], 0.0]);
    let mut a: [f32; 3] = std::array::from_fn(|i| ph.motion_vec[i] - old_vel[i] * factor);
    let l = len(a);
    if max_acc < l {
        a = a.map(|v| v * (max_acc / l));
    }
    let local = [dot(a, axis[0]), dot(a, axis[1]), dot(a, axis[2])];
    let pitch = match t {
        6 => local[0] / max_acc * def.heli.tilt_from_accel[0],
        3 => ratio(vectosignedangles(ph.motion_vec)[0], def.heli.max_body_pitch),
        _ => (-(local[2] / max_acc) * def.heli.tilt_from_accel[0]).min(0.0),
    };
    let mut pitch = clamp1(pitch);
    let roll = clamp1(local[1] / max_acc * def.heli.tilt_from_accel[1]);
    if t == 6 {
        pitch = diff_track(pitch, ph.body_tilt[0], TRACK_TILT_RATE, DT);
    }
    ph.body_tilt[0] = pitch;
    ph.body_tilt[1] = diff_track(roll, ph.body_tilt[1], TRACK_TILT_RATE, DT);
    if t == 6 || t == 4 {
        ph.rot[0] = diff_track_angle(
            def.heli.max_body_pitch * ph.body_tilt[0],
            ph.prev_angles[0],
            TRACK_ANGLE_RATE,
            DT,
        );
    }
    let roll_to = def.heli.max_body_roll * ph.body_tilt[1];
    if t == 6 {
        ph.rot[2] = diff_track_angle(roll_to, ph.prev_angles[2], TRACK_ANGLE_RATE, DT);
        return;
    }
    let want = (norm180(roll_to - ph.rot[2]) * 20.0).clamp(-def.heli.rot_rate, def.heli.rot_rate);
    let acc = ((want - ph.rot_vel[2]) * 20.0).clamp(-def.heli.rot_accel, def.heli.rot_accel);
    ph.rot_vel[2] += acc * DT;
    ph.rot[2] = norm180(ph.rot[2] + ph.rot_vel[2] * DT);
}

/// VEH_TouchEntities for a vehicle on its path: each player its body
/// touches (a box 0.15 s of its speed ahead), and what it hits him for.
fn touches(world: &mut World, v: u32, veh: &Vehicle) -> Vec<(u32, i32, [f32; 3])> {
    let Some((spot, rot, mdl)) = world
        .resource::<Zm>()
        .ents
        .get(&v)
        .map(|e| (e.origin, e.rot, e.mesh.clone()))
    else {
        return Vec::new();
    };
    let rides: Vec<u32> = world
        .get_resource::<Streaks>()
        .map(|s| s.rides.keys().copied().collect())
        .unwrap_or_default();
    let players: Vec<u32> = world
        .resource::<Zm>()
        .players
        .iter()
        .filter(|(c, p)| p.sessionstate == "playing" && p.linked.is_none() && !rides.contains(c))
        .map(|(c, _)| *c)
        .collect();
    if players.is_empty() {
        return Vec::new();
    }
    let f = super::frame(world);
    let cm = f.collision_submodel_set();
    let Some(&handle) = cm.model_cmodels.get(&mdl) else {
        return Vec::new();
    };
    let Some(map) = f.collision_tree().t6 else {
        return Vec::new();
    };
    let Some(radius) = map.cmodels.get(handle as usize).map(|m| m.radius) else {
        return Vec::new();
    };
    let motion_vec = veh.phys.motion_vec;
    let lead = motion_vec.map(|x| x * 0.15);
    let moved = sub(spot, veh.phys.prev_origin);
    let reach = radius + 1.0;
    let mut area_min: [f32; 3] = std::array::from_fn(|i| spot[i] - reach);
    let mut area_max: [f32; 3] = std::array::from_fn(|i| spot[i] + reach);
    for i in 0..3 {
        if moved[i] > 0.0 {
            area_max[i] += moved[i];
        } else {
            area_min[i] += moved[i];
        }
        area_min[i] += lead[i];
        area_max[i] += lead[i];
    }
    let mins = crate::shot_trace::BODY_BOX_LOWER.map(|x| x - 1.0);
    let maxs = crate::shot_trace::BODY_BOX_UPPER.map(|x| x + 1.0);
    let speed = len(motion_vec);
    let mut out = Vec::new();
    for c in players {
        let Some(ps) = f.avatar(crate::sim_parts::ClientId(c)) else {
            continue;
        };
        let at = sub(ps.origin, lead);
        if (0..3).any(|i| at[i] + maxs[i] < area_min[i] || at[i] + mins[i] > area_max[i]) {
            continue;
        }
        let tr = crate::sim_parts::t6_transformed_trace(
            map,
            at,
            at,
            mins,
            maxs,
            crate::sim_parts::T6Model::Brush(handle),
            u32::MAX,
            spot,
            rot,
        );
        if !tr.starts_inside && !tr.fully_embedded {
            continue;
        }
        let mut dmg = 0;
        let mut dir = [0.0; 3];
        if veh.def.collision_damage > 0.0 && speed >= 0.001 {
            dir = motion_vec.map(|x| x / speed);
            if ps.movement_state_bits & playerstate_t6::game::movement_state_bits::LYING_FLAT != 0 {
                dmg = 999_999;
            } else {
                let off = [ps.origin[0] - spot[0], ps.origin[1] - spot[1]];
                let l = off[0].hypot(off[1]);
                let off = if l > 0.0 { [off[0] / l, off[1] / l] } else { [0.0; 2] };
                let d = dir[0] * off[0] + dir[1] * off[1];
                if d >= 0.8 && veh.speed.abs() >= 88.0 {
                    let frac = ratio(veh.speed.abs() - 88.0, veh.def.collision_speed).min(1.0);
                    dmg = ((d - 0.8) * 5.000_000_5 * frac * veh.def.collision_damage) as i32;
                }
            }
        }
        out.push((c, dmg.max(0), dir));
    }
    out
}

/// What each player's move touched this tick (the script models his
/// Pmove ran into): "touch" on him and on it (G_TouchEnts). Express's
/// carriages kill whoever they touch this way.
fn model_touches(world: &mut World) {
    let list = match world.get_resource_mut::<crate::frame_step::PlayerModelTouches>() {
        Some(mut t) => std::mem::take(&mut t.0),
        None => return,
    };
    if list.is_empty() {
        return;
    }
    let pairs: Vec<(ObjRef, ObjRef)> = list
        .into_iter()
        .filter_map(|(c, owner)| {
            let id = owner.scripted_model()?;
            let zm = world.resource::<Zm>();
            let n = zm
                .presences
                .by_ent
                .iter()
                .find(|(_, s)| s.id == id)
                .map(|(n, _)| *n)
                .or_else(|| super::brushes::ent_of(world, id))?;
            Some((zm.players.get(&c)?.obj, zm.ents.get(&n)?.obj?))
        })
        .collect();
    with_vm(world, |vm, world| {
        for (p, e) in pairs {
            vm.notify_str(world, p, "touch", &[ScriptValue::Object(e)]);
            vm.notify_str(world, e, "touch", &[ScriptValue::Object(p)]);
        }
    });
}

/// `BO2MP_T6_VEHLOG=1` (a test aid): where path vehicles are each second.
fn veh_log() -> bool {
    static VEHLOG_ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *VEHLOG_ON.get_or_init(|| std::env::var_os("BO2MP_T6_VEHLOG").is_some())
}

/// Each tick: vehicles along their paths (VEH_UpdatePath, VEH_CheckPathEnd,
/// VEH_TouchEntities).
pub(crate) fn advance(world: &mut World, dt: f32) {
    chase_cam(world);
    super::flight::advance(world, dt);
    model_touches(world);
    let ids: Vec<u32> = world.resource::<Zm>().vehicles.list.keys().copied().collect();
    for v in ids {
        let Some(mut veh) = world.resource::<Zm>().vehicles.list.get(&v).cloned() else {
            continue;
        };
        let Some((obj, origin, angles)) =
            world.resource::<Zm>().ents.get(&v).map(|e| (e.obj, e.origin, e.rot))
        else {
            world.resource_mut::<Zm>().vehicles.list.remove(&v);
            continue;
        };
        // `setvehgoalpos` took it off its path: it flies on as a craft.
        let goal = world
            .get_resource::<Streaks>()
            .and_then(|s| s.craft.get(&v).map(|c| c.goal.is_some()))
            .unwrap_or_default()
            || world.resource::<Zm>().vehicles.flights.contains_key(&v);
        if veh.on_path && goal {
            veh.on_path = false;
        }
        let mut reached = Vec::new();
        let mut end = None;
        let mut s_p = world.resource::<Zm>().vehicles.speed.get(&v).copied().unwrap_or_default();
        if veh.on_path {
            let zm = world.resource::<Zm>();
            let graph_nodes = zm.vehicles.nodes.as_deref().unwrap_or(&[]);
            if veh.pp.node >= 0 && (veh.pp.node as usize) < graph_nodes.len() {
                let (tgt, wanted, at) = path_position(graph_nodes, &mut veh, &s_p);
                let cur = veh.pp.node as usize;
                let autotilt = graph_nodes[cur].flags & NODE_ROTATE == 0;
                let def = veh.def;
                path_follow(&mut veh.phys, &def, tgt, wanted, autotilt);
                reached = at.into_iter().map(|i| graph_nodes[i].ent).collect();
                if veh_log() {
                    veh.log_ticks += 1;
                    if veh.log_ticks % 20 == 1 || !reached.is_empty() {
                        diag::info!(
                            Sim,
                            "bo2mp t6 vehicle: ent{v} node {} frac {:.2} speed {:.0} at {:?} angles {:?} reached {:?}",
                            graph_nodes[cur].ident,
                            veh.pp.frac,
                            veh.speed,
                            veh.phys.origin,
                            veh.phys.rot,
                            reached
                        );
                    }
                }
                if veh.pp.end_of_path && !veh.end_told {
                    veh.end_told = true;
                    end = Some(graph_nodes[cur].ent);
                }
            }
            // bo2zm: a planted vehicle (Tranzit's bus) stands on its wheels
            // on the road, tilted to it, at the height they find.
            if world.resource::<Zm>().vehicles.plants.contains_key(&v) {
                find_wheels(world, v);
                let lean = [angles[0], veh.phys.rot[1], angles[2]];
                if let Some((z, pitch, roll)) =
                    ground_plant(world, v, veh.phys.origin, origin, lean, dt)
                {
                    veh.phys.origin[2] = z;
                    veh.phys.rot[0] = pitch;
                    veh.phys.rot[2] = roll;
                }
            }
            let mut zm = world.resource_mut::<Zm>();
            if let Some(e) = zm.ents.get_mut(&v) {
                e.origin = veh.phys.origin;
                e.rot = veh.phys.rot;
            }
        } else {
            veh.phys.prev_origin = veh.phys.origin;
            veh.phys.prev_angles = veh.phys.rot;
            veh.phys.motion_vec = std::array::from_fn(|i| (origin[i] - veh.phys.origin[i]) * 20.0);
            veh.phys.origin = origin;
            veh.phys.rot = angles;
        }
        if let Some(mut s) = world.get_resource_mut::<Streaks>()
            && let Some(c) = s.craft.get_mut(&v)
        {
            if veh.on_path {
                c.cur = veh.speed;
            } else {
                veh.speed = c.cur;
            }
        }
        if veh.on_path {
            s_p.now = veh.speed;
            world.resource_mut::<Zm>().vehicles.speed.insert(v, s_p);
        }
        let mph = veh.speed / MPH;
        let hits = if veh.on_path { touches(world, v, &veh) } else { Vec::new() };
        world.resource_mut::<Zm>().vehicles.list.insert(v, veh);
        let Some(o) = obj else {
            continue;
        };
        if reached.is_empty() && end.is_none() && hits.is_empty() {
            continue;
        }
        with_vm(world, |vm, world| {
            let node_obj =
                |world: &World, n: u32| world.resource::<Zm>().ents.get(&n).and_then(|e| e.obj);
            for n in reached {
                if veh_log() {
                    let zm = world.resource::<Zm>();
                    let at = zm.ents.get(&n).map_or([0.0; 3], |e| e.origin);
                    diag::info!(
                        Sim,
                        "bo2zm t6 vehicle {v} reached node {n} at ({:.0} {:.0} {:.0}) {mph:.1} mph, {} ms{}",
                        at[0],
                        at[1],
                        at[2],
                        zm.clock_millis,
                        if end == Some(n) { ", end of path" } else { "" }
                    );
                }
                if let Some(no) = node_obj(world, n) {
                    vm.notify_str(world, o, "reached_node", &[ScriptValue::Object(no)]);
                }
            }
            if let Some(n) = end {
                vm.notify_str(world, o, "reached_end_node", &[]);
                if let Some(no) = node_obj(world, n) {
                    vm.notify_str(world, o, "reached_node", &[ScriptValue::Object(no)]);
                }
            }
            for (c, dmg, dir) in hits {
                let Some(p) = world.resource::<Zm>().players.get(&c).map(|p| p.obj) else {
                    continue;
                };
                vm.notify_str(world, p, "touch", &[ScriptValue::Object(o)]);
                vm.notify_str(world, o, "touch", &[ScriptValue::Object(p)]);
                if dmg > 0 {
                    let point = super::frame(world)
                        .avatar(crate::sim_parts::ClientId(c))
                        .map_or([0.0; 3], |ps| ps.origin);
                    super::natives_ai::player_damage(
                        vm,
                        world,
                        c,
                        ScriptValue::Object(o),
                        ScriptValue::Object(o),
                        dmg,
                        0,
                        "MOD_CRUSH",
                        "none",
                        point,
                        dir,
                        "none",
                    );
                }
            }
        });
    }
}

/// An entity's origin and angles.
fn pose(world: &World, n: u32) -> Option<([f32; 3], [f32; 3])> {
    world.resource::<Zm>().ents.get(&n).map(|e| (e.origin, e.rot))
}

/// A script vehicle's velocity (units a second): the way it moved this
/// think, zero while parked; `None` for an entity that is not one (the
/// cattle catcher's shove adds the bus's `getvelocity` to its own).
pub(crate) fn velocity(world: &World, n: u32) -> Option<[f32; 3]> {
    let zm = world.resource::<Zm>();
    zm.ents.get(&n).filter(|e| e.class_tag == "script_vehicle")?;
    Some(
        zm.vehicles
            .list
            .get(&n)
            .filter(|v| v.on_path)
            .map_or([0.0; 3], |v| v.phys.motion_vec),
    )
}

/// A vehicle's speed now (units a second); 0 off its path.
fn speed_now(vm: &Vm<World>, world: &World, s: &ScriptValue) -> f32 {
    let zm = world.resource::<Zm>();
    match entnum(vm, s).and_then(|v| zm.vehicles.list.get(&v)) {
        Some(v) if v.on_path => v.speed,
        _ => 0.0,
    }
}

fn set_speed(
    vm: &mut Vm<World>, game: &mut World,
    s: &ScriptValue,
    a: &[ScriptValue],
    immediate: bool,
) -> Result<ScriptValue, String> {
    let Some(v) = entnum(vm, s) else {
        return Ok(ScriptValue::Undefined);
    };
    let mph = num(a, 0).unwrap_or(0.0).max(0.0);
    let speed_up_rate = num(a, 1).unwrap_or(10.0).max(0.1);
    let decel = num(a, 2).unwrap_or(speed_up_rate).max(0.1);
    let mut zm = game.resource_mut::<Zm>();
    let sp = zm.vehicles.speed.entry(v).or_default();
    sp.target = Some(mph * MPH);
    sp.speed_up_rate = speed_up_rate * MPH;
    sp.decel = decel * MPH;
    if immediate {
        sp.now = mph * MPH;
        if let Some(veh) = zm.vehicles.list.get_mut(&v) {
            veh.speed = mph * MPH;
        }
    }
    Ok(ScriptValue::Undefined)
}

/// BO2MP_T6_CHASECAM="back up": from 12 s player 0 is held `back` units
/// behind and `up` above the first script-driven vehicle (Tranzit's bus),
/// looking at it: shots of it on the road.
fn chase_cam(world: &mut World) {
    static CAM: std::sync::OnceLock<Option<(f32, f32)>> = std::sync::OnceLock::new();
    let Some((back, up)) = *CAM.get_or_init(|| {
        let v = std::env::var("BO2MP_T6_CHASECAM").ok()?;
        let mut n = v.split_whitespace().filter_map(|x| x.parse().ok());
        Some((n.next()?, n.next()?))
    }) else {
        return;
    };
    let zm = world.resource::<Zm>();
    if zm.clock_millis < 12_000 {
        return;
    }
    let Some((o, ang)) = zm.vehicles.speed.keys().find_map(|v| pose(world, *v)) else {
        return;
    };
    let (f, _, _) = gsc_t6::math::facing_axes([0.0, ang[1], 0.0]);
    let eye = [o[0] - f[0] * back, o[1] - f[1] * back, o[2] + up];
    let d = sub([o[0], o[1], o[2] + 60.0], eye);
    let pitch = -(d[2].atan2((d[0] * d[0] + d[1] * d[1]).sqrt())).to_degrees();
    let yaw = d[1].atan2(d[0]).to_degrees();
    let client = crate::sim_parts::ClientId(0);
    super::teleport_player(world, client, [eye[0], eye[1], eye[2] - 60.0]);
    super::set_player_view(world, client, [pitch, yaw, 0.0]);
}

/// A planted vehicle's wheels' places on it, from its wheel tags where it
/// stands now (once found; a miss is logged by the tag lookup, and given up
/// after a few seconds of tries: it then drives at its nodes' height).
fn find_wheels(world: &mut World, v: u32) {
    let Some(plant) = world.resource::<Zm>().vehicles.plants.get(&v) else {
        return;
    };
    if plant.wheels.is_some() || plant.misses >= 100 {
        return;
    }
    let Some((o, ang)) = pose(world, v) else {
        return;
    };
    let (f, r, u) = gsc_t6::math::facing_axes(ang);
    let mut wheels = [[0.0f32; 3]; 4];
    let mut found = true;
    for (i, tag) in WHEEL_TAGS.iter().enumerate() {
        let Some((p, _)) = super::tags::tag_world(world, v, tag) else {
            found = false;
            break;
        };
        let d = sub(p, o);
        wheels[i] = [dot(d, f), -dot(d, r), dot(d, u)];
    }
    let mut zm = world.resource_mut::<Zm>();
    let Some(plant) = zm.vehicles.plants.get_mut(&v) else {
        return;
    };
    if found {
        plant.wheels = Some(wheels);
        diag::info!(
            Sim,
            "bo2zm t6 vehicle {v} stands on its wheels at ({:.0} {:.0} {:.0}) ({:.0} {:.0} {:.0}) ({:.0} {:.0} {:.0}) ({:.0} {:.0} {:.0})",
            wheels[0][0], wheels[0][1], wheels[0][2],
            wheels[1][0], wheels[1][1], wheels[1][2],
            wheels[2][0], wheels[2][1], wheels[2][2],
            wheels[3][0], wheels[3][1], wheels[3][2]
        );
    } else {
        plant.misses += 1;
        if plant.misses == 100 {
            diag::info!(Sim, "bo2zm t6 vehicle {v}: no wheel tags, it drives at its nodes' height");
        }
    }
}

/// BO2's VEH_GroundPlant: each wheel, where the vehicle is now (at the height
/// it had), traced from 64 units above to 256 below; a wheel off the ground
/// falls to it, one under it is lifted onto it (one over a gap holds). The
/// vehicle stands on the plane through its wheels (resting on the highest),
/// tilted to it (eased, 6 times the gap a second, at most 60 degrees) at its
/// height there. Its height, pitch and roll; `None` until its wheels are
/// known.
fn ground_plant(world: &mut World,
    v: u32,
    pos: [f32; 3],
    was_at: [f32; 3],
    ang: [f32; 3],
    dt: f32,
) -> Option<(f32, f32, f32)> {
    use gsc_t6::math::{cross, normalize};
    let mut plant = world.resource::<Zm>().vehicles.plants.get(&v).cloned()?;
    let wheels = plant.wheels?;
    let (f, r, u) = gsc_t6::math::facing_axes(ang);
    let base = [pos[0], pos[1], was_at[2]];
    let mut w = [[0.0f32; 3]; 4];
    for (i, o) in wheels.iter().enumerate() {
        let p: [f32; 3] =
            std::array::from_fn(|k| base[k] + f[k] * o[0] - r[k] * o[1] + u[k] * o[2]);
        if !plant.planted {
            plant.z[i] = p[2];
        }
        let top = [p[0], p[1], p[2] + 64.0];
        let bottom = [p[0], p[1], p[2] - 256.0];
        let t = super::frame(world).trace_static_world(top, bottom, [0.0; 3], [0.0; 3], WHEEL_MASK);
        let ground = (!t.starts_inside && t.fraction < 1.0)
            .then(|| top[2] + (bottom[2] - top[2]) * t.fraction);
        match ground {
            Some(g) => {
                plant.fall[i] -= WHEEL_GRAVITY * dt;
                plant.z[i] += plant.fall[i] * dt;
                if plant.z[i] < g {
                    plant.z[i] = g;
                    plant.fall[i] = 0.0;
                }
            }
            None => plant.fall[i] = 0.0,
        }
        w[i] = [p[0], p[1], plant.z[i]];
    }
    plant.planted = true;
    let mid = |a: [f32; 3], b: [f32; 3]| -> [f32; 3] { std::array::from_fn(|k| (a[k] + b[k]) * 0.5) };
    let right = normalize(sub(mid(w[1], w[3]), mid(w[0], w[2])));
    let fwd = normalize(sub(mid(w[0], w[1]), mid(w[2], w[3])));
    let n = normalize(cross(right, fwd));
    world.resource_mut::<Zm>().vehicles.plants.insert(v, plant);
    if n[2] < 0.1 {
        return None;
    }
    let d = w.iter().map(|wi| dot(n, *wi)).fold(f32::MIN, f32::max);
    let z = (d - n[0] * pos[0] - n[1] * pos[1]) / n[2];
    let side = normalize(cross(n, f));
    let on = normalize(cross(side, n));
    let [pitch, _, roll] = super::tags::axis_to_angles([on, side, n]);
    let pitch = diff_track_angle(pitch, ang[0], 6.0, dt).clamp(-60.0, 60.0);
    let roll = diff_track_angle(roll, ang[2], 6.0, dt).clamp(-60.0, 60.0);
    Some((z, pitch, roll))
}

pub(super) fn bind(vm: &mut Vm<World>) {
    vm.bind("attachpath", true, |vm, world, s, a| {
        let (Some(v), Some(node)) = (entnum(vm, s), entnum(vm, arg(a, 0))) else {
            return Ok(ScriptValue::Undefined);
        };
        if world.resource::<Zm>().vehicles.nodes.is_none() {
            let nodes = build_nodes(vm, world);
            world.resource_mut::<Zm>().vehicles.nodes = Some(nodes);
        }
        let k = vm.intern("vehicletype");
        let kind = world
            .resource::<Zm>()
            .ents
            .get(&v)
            .and_then(|e| e.obj)
            .map(|o| vm.to_text(&vm.raw_field(o, k)).to_ascii_lowercase())
            .unwrap_or_default();
        let def = world
            .get_resource::<Streaks>()
            .and_then(|s| s.vehicle_drive.get(&kind).copied())
            .unwrap_or_default();
        let zm = world.resource::<Zm>();
        let nodes = zm.vehicles.nodes.as_deref().unwrap_or(&[]);
        let Some(i) = nodes.iter().position(|n| n.ent == node) else {
            return Ok(ScriptValue::Undefined);
        };
        let pp = set_up_path_pos(nodes, i);
        if veh_log() {
            diag::info!(
                Sim,
                "bo2mp t6 vehicle: ent{v} attachpath {} kind {kind:?} max_speed {} accel {} at {:?}",
                nodes[i].ident,
                def.top_velocity,
                def.speed_up_rate,
                pp.origin
            );
        }
        let mut veh = zm.vehicles.list.get(&v).cloned().unwrap_or_default();
        veh.def = def;
        veh.end_told = false;
        // bo2zm: a planted one keeps its height; its wheels set it on the road.
        let planted = PLANTED.contains(&kind.as_str());
        let mut at = pp.origin;
        if planted && let Some(e) = zm.ents.get(&v) {
            at[2] = e.origin[2];
        }
        veh.phys.origin = at;
        veh.phys.prev_origin = at;
        veh.phys.rot = pp.rot;
        veh.phys.prev_angles = pp.rot;
        let mut zm = world.resource_mut::<Zm>();
        if planted { zm.vehicles.plants.entry(v).or_default(); }
        if let Some(e) = zm.ents.get_mut(&v) {
            e.origin = at;
            e.rot = pp.rot;
        }
        veh.pp = pp;
        zm.vehicles.list.insert(v, veh);
        Ok(ScriptValue::Undefined)
    });
    vm.bind("startpath", true, |vm, world, s, _| {
        let Some(v) = entnum(vm, s) else {
            return Ok(ScriptValue::Undefined);
        };
        let mut zm = world.resource_mut::<Zm>();
        let Some(veh) = zm.vehicles.list.get_mut(&v) else {
            return Ok(ScriptValue::Undefined);
        };
        veh.on_path = true;
        veh.end_told = false;
        drop(zm);
        if let Some(mut st) = world.get_resource_mut::<Streaks>()
            && let Some(c) = st.craft.get_mut(&v)
        {
            c.goal = None;
        }
        Ok(ScriptValue::Undefined)
    });
    // bo2zm: setspeed(mph, speed_up_rate, decel) / setspeedimmediate(mph): the script
    // drives the vehicle from here on.
    vm.bind("setspeed", true, |vm, world, s, a| {
        set_speed(vm, world, s, a, false)
    });
    vm.bind("setspeedimmediate", true, |vm, world, s, a| {
        set_speed(vm, world, s, a, true)
    });
    vm.bind("resumespeed", true, |vm, world, s, _| {
        if let Some(v) = entnum(vm, s) {
            let mut zm = world.resource_mut::<Zm>();
            zm.vehicles.speed.entry(v).or_default().target = None;
        }
        Ok(ScriptValue::Undefined)
    });
    vm.bind("setvehmaxspeed", true, |vm, world, s, a| {
        if let Some(v) = entnum(vm, s) {
            let mph = num(a, 0).unwrap_or(0.0);
            let mut zm = world.resource_mut::<Zm>();
            zm.vehicles.speed.entry(v).or_default().max = (mph > 0.0).then_some(mph * MPH);
        }
        Ok(ScriptValue::Undefined)
    });
    // getspeed: units a second; getspeedmph: miles per hour.
    vm.bind("getspeed", true, |vm, world, s, _| {
        Ok(ScriptValue::Float(speed_now(vm, world, s)))
    });
    vm.bind("getspeedmph", true, |vm, world, s, _| {
        Ok(ScriptValue::Float(speed_now(vm, world, s) / MPH))
    });
    // Turret aim (Origins' tank flamethrowers): the turret's pose isn't
    // drawn apart from the hull yet, so the aim is accepted and dropped.
    vm.bind("setturrettargetvec", true, |_, _, _, _| Ok(ScriptValue::Undefined));
    vm.bind("setturrettargetent", true, |_, _, _, _| Ok(ScriptValue::Undefined));
    // Entity space <-> world space (forward, left, up), as `linkto` offsets.
    vm.bind("localtoworldcoords", true, |vm, world, s, a| {
        let local = vec3(a, 0)?;
        let Some((o, ang)) = entnum(vm, s).and_then(|n| pose(world, n)) else {
            return Ok(ScriptValue::Vec3(local));
        };
        let (f, r, u) = gsc_t6::math::facing_axes(ang);
        Ok(ScriptValue::Vec3(std::array::from_fn(|i| {
            o[i] + f[i] * local[0] - r[i] * local[1] + u[i] * local[2]
        })))
    });
    vm.bind("worldtolocalcoords", true, |vm, world, s, a| {
        let p = vec3(a, 0)?;
        let Some((o, ang)) = entnum(vm, s).and_then(|n| pose(world, n)) else {
            return Ok(ScriptValue::Vec3(p));
        };
        let (f, r, u) = gsc_t6::math::facing_axes(ang);
        let d = sub(p, o);
        Ok(ScriptValue::Vec3([dot(d, f), -dot(d, r), dot(d, u)]))
    });
    // spawnvehicle(mdl, target_name, vehicletype, origin, angles): a
    // script vehicle (Origins' crystal biplane, Maxis drone); it drives a
    // path once given one (attachpath/startpath).
    // TODO(rw-zm): killstreaks/craft.rs's spawn_craft (MP's `spawnvehicle`)
    // also starts the engine sounds and client fields; this one does not.
    vm.bind("spawnvehicle", false, |vm, world, _, a| {
        let body_text = |vm: &Vm<World>, i: usize| match arg(a, i) {
            ScriptValue::Undefined => String::new(),
            v => vm.to_text(v),
        };
        let (mdl, target_name, vehicletype) = (body_text(vm, 0), body_text(vm, 1), body_text(vm, 2));
        let origin = vec3(a, 3).unwrap_or([0.0; 3]);
        let angles = vec3(a, 4).unwrap_or([0.0; 3]);
        let n = world.resource_mut::<Zm>().alloc_entnum();
        let obj = vm.alloc_object(gsc_t6::ObjKind::Entity(n));
        world.resource_mut::<Zm>().ents.insert(
            n,
            super::Ent {
                obj: Some(obj),
                class_tag: "script_vehicle".to_owned(),
                origin,
                rot: angles,
                mesh: mdl,
                solid: true,
                ..Default::default()
            },
        );
        for (k, v) in [
            ("targetname", target_name),
            ("vehicletype", vehicletype),
            ("classname", "script_vehicle".to_owned()),
        ] {
            let k = vm.intern(k);
            let v = vm.string(&v);
            vm.set_field(world, obj, k, v);
        }
        Ok(ScriptValue::Object(obj))
    });
    for builtin in [
        "vehicle_detachfrompath",
        // Its cheap physics (`setcheapflag`): nothing a server port needs.
        "setcheapflag",
    ] {
        vm.bind(builtin, true, |_, _, _, _| Ok(ScriptValue::Undefined));
    }
    // getvehiclearray(team?): the live script vehicles, picked by their team
    // field like getaiarray (Mob's Acid Gat adds the "axis" ones to the
    // zombies it may aim at).
    vm.bind("getvehiclearray", false, |vm, world, _, a| {
        let team = match arg(a, 0) {
            ScriptValue::Undefined => None,
            v => Some(vm.to_text(v)),
        };
        let team_s = vm.intern("team");
        let objs: Vec<ObjRef> = world
            .resource::<Zm>()
            .ents
            .values()
            .filter(|e| e.class_tag == "script_vehicle")
            .filter_map(|e| e.obj)
            .collect();
        let out = objs
            .into_iter()
            .filter(|o| vm.alive(*o))
            .filter(|o| match &team {
                None => true,
                Some(t) if t == "all" => true,
                Some(t) => vm.to_text(&vm.raw_field(*o, team_s)) == *t,
            })
            .map(ScriptValue::Object)
            .collect();
        Ok(super::list(out))
    });
    super::flight::bind(vm);
}
