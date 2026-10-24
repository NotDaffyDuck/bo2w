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
//! Not ported: spline paths (a PT_SPLINE start runs as the old path), path
//! offsets, switch nodes, manual speeds (`setspeed` on a path), ground
//! planting, pushing entities, touching anything but players, the riot
//! shield's 1 damage, a driver as the attacker (and his team check),
//! stance-sized player boxes, touches of vehicles off a path.

use std::collections::BTreeMap;

use bevy_ecs::prelude::World;
use gsc_t6::{ObjRef, Value, Vm};

use super::killstreaks::Streaks;
use super::{T6VehicleDrive, Zm, arg, entnum, with_vm};

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

/// A vehicle node (`vehicle_node_t`).
#[derive(Clone, Debug, Default)]
pub(crate) struct Node {
    pub ent: u32,
    name: String,
    target: String,
    origin: [f32; 3],
    angles: [f32; 3],
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
    angles: [f32; 3],
    look_pos: [f32; 3],
}

/// Its body (`vehicle_physic_t`).
#[derive(Clone, Debug, Default)]
pub(crate) struct Phys {
    origin: [f32; 3],
    prev_origin: [f32; 3],
    angles: [f32; 3],
    prev_angles: [f32; 3],
    vel: [f32; 3],
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
}

#[derive(Clone, Debug, Default)]
pub(crate) struct Vehicles {
    /// The map's vehicle nodes (G_SetupSplinePaths), on the first
    /// `attachpath`.
    pub nodes: Option<Vec<Node>>,
    pub list: BTreeMap<u32, Vehicle>,
}

fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn len(a: [f32; 3]) -> f32 {
    dot(a, a).sqrt()
}

fn along(o: [f32; 3], d: [f32; 3], s: f32) -> [f32; 3] {
    [o[0] + d[0] * s, o[1] + d[1] * s, o[2] + d[2] * s]
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

fn diff_track(t: f32, c: f32, rate: f32, dt: f32) -> f32 {
    let d = (t - c) * rate * dt;
    let gap = (t - c).abs();
    if gap > 0.001 && d.abs() <= gap { c + d } else { t }
}

fn diff_track_angle(mut t: f32, c: f32, rate: f32, dt: f32) -> f32 {
    while t - c > 180.0 {
        t -= 360.0;
    }
    while t - c < -180.0 {
        t += 360.0;
    }
    let d = (t - c) * rate * dt;
    let gap = (t - c).abs();
    if gap > 0.001 && d.abs() <= gap {
        t = c + d;
    }
    norm180(t)
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
    [pitch, yaw, 0.0]
}

fn vectosignedangles(v: [f32; 3]) -> [f32; 3] {
    if v[1] == 0.0 && v[0] == 0.0 {
        return [if 0.0 <= -v[2] { 90.0 } else { -90.0 }, 0.0, 0.0];
    }
    let yaw = v[1].atan2(v[0]).to_degrees();
    [-v[2].atan2(v[0].hypot(v[1])).to_degrees(), yaw, 0.0]
}

/// Forward, right, up.
fn angle_vectors(a: [f32; 3]) -> [[f32; 3]; 3] {
    let (sp, cp) = a[0].to_radians().sin_cos();
    let (sy, cy) = a[1].to_radians().sin_cos();
    let (sr, cr) = a[2].to_radians().sin_cos();
    [
        [cp * cy, cp * sy, -sp],
        [-sr * sp * cy + cr * sy, -sr * sp * sy - cr * cy, -sr * cp],
        [cr * sp * cy + sr * sy, cr * sp * sy - sr * cy, cr * cp],
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
        .filter(|(_, e)| e.classname.starts_with("info_vehicle_node"))
        .filter_map(|(n, e)| {
            let o = e.obj?;
            let mut node = Node {
                ent: *n,
                name: vm.to_text(&vm.raw_field(o, keys[0])),
                target: vm.to_text(&vm.raw_field(o, keys[1])),
                origin: e.origin,
                angles: e.angles,
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
            if e.classname == "info_vehicle_node_rotate" {
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
        let target = &nodes[i].target;
        let mut next = if target.is_empty() {
            -1
        } else {
            nodes
                .iter()
                .position(|n| n.name == *target)
                .map_or(-1, |j| j as i32)
        };
        let name = &nodes[i].name;
        let prev = if name.is_empty() {
            -1
        } else {
            (0..count)
                .find(|&j| j != i && nodes[j].target == *name)
                .map_or(-1, |j| j as i32)
        };
        if next == i as i32 {
            next = -1;
        }
        nodes[i].next = next;
        nodes[i].prev = prev;
    }
    for i in 0..count {
        let n = &nodes[i];
        let (speed, look_ahead, radius) = (
            if n.flags & NODE_SPEED != 0 { n.speed } else { -1.0 },
            if n.flags & NODE_LOOKAHEAD != 0 { n.look_ahead } else { -1.0 },
            if n.flags & NODE_RADIUS != 0 { n.radius } else { -1.0 },
        );
        let dir = if n.next >= 0 {
            sub(nodes[n.next as usize].origin, n.origin)
        } else if n.prev >= 0 {
            sub(n.origin, nodes[n.prev as usize].origin)
        } else {
            [1.0, 0.0, 0.0]
        };
        let length = len(dir);
        let d = if length == 0.0 { 1.0 } else { length };
        let dir = [dir[0] / d, dir[1] / d, dir[2] / d];
        let n = &mut nodes[i];
        n.speed = speed;
        n.look_ahead = look_ahead;
        n.radius = radius;
        n.dir = dir;
        n.length = length;
        if n.flags & NODE_ROTATE == 0 {
            n.angles = vectoangles(dir);
        }
    }
    for i in 0..count {
        let speed = calc(&nodes, i, NODE_SPEED, |n| n.speed);
        nodes[i].speed = speed;
        let look_ahead = calc(&nodes, i, NODE_LOOKAHEAD, |n| n.look_ahead);
        nodes[i].look_ahead = look_ahead;
        let radius = calc(&nodes, i, NODE_RADIUS, |n| n.radius);
        nodes[i].radius = radius;
        let tension = calc(&nodes, i, NODE_TENSION, |n| n.tension);
        nodes[i].tension = tension;
        let n = &mut nodes[i];
        n.angles = n.angles.map(norm180);
        if n.speed <= 0.0 || n.look_ahead <= 0.0 {
            n.next = -1;
        }
        if n.next < 0 {
            if n.speed <= 0.0 {
                n.speed = 1.0;
            }
            if n.look_ahead <= 0.0 {
                n.look_ahead = 1.0;
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
    let mut p = nodes[i].prev;
    let mut steps = 0;
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
    let mut x = i;
    let mut steps = 0;
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
        angles: node.angles,
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
    let mut p = n;
    let mut k = 0;
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
    let mut examined;
    let mut steps = 0;
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
        let nx = next as usize;
        let (a, la) = node_look(nodes, n);
        let (b, lb) = node_look(nodes, nx);
        let from_cur = sub(pp.origin, nodes[n].origin);
        let to_next = sub(nodes[nx].origin, pp.origin);
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
        cur = nx;
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
            pp.angles = nodes[cur].angles;
        }
        return;
    }
    let nx = &nodes[next as usize];
    let next_rot = nx.flags & NODE_ROTATE != 0;
    let (from, to) = match (cur_rot, next_rot) {
        (true, true) => (nodes[cur].angles, nx.angles),
        (false, true) => (pp.angles, nx.angles),
        (true, false) => (nodes[cur].angles, pp.angles),
        (false, false) => return,
    };
    for i in 0..3 {
        pp.angles[i] = norm180(norm180(to[i] - from[i]) * pp.frac + from[i]);
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
    pp.angles = vectoangles(dir).map(norm180);
    pp.origin = along(pp.origin, dir, pp.speed * DT);
    update_path_pos(nodes, pp);
    path_angles(nodes, pp);
}

/// VEH_UpdatePathPosition: this think's place and angles to make for, and
/// the nodes it reached.
fn path_position(nodes: &[Node], veh: &mut Vehicle) -> ([f32; 3], [f32; 3], Vec<usize>) {
    veh.speed = veh.pp.speed;
    let mut ratio = if veh.pp.speed > 0.0 {
        veh.speed / veh.pp.speed
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
    let (a, b) = (veh.pp.angles, local.angles);
    let angles = std::array::from_fn(|i| a[i] + norm180(b[i] - a[i]) * ratio);
    veh.pp.manual_time = ratio;
    (origin, angles, reached)
}

/// VEH_UpdatePathFollow: there, its yaw, pitch and roll eased toward the
/// path's, or leaning by its acceleration.
fn path_follow(ph: &mut Phys, def: &T6VehicleDrive, target: [f32; 3], want: [f32; 3], autotilt: bool) {
    ph.prev_angles = ph.angles;
    ph.prev_origin = ph.origin;
    ph.origin = target;
    let old_vel = ph.vel;
    ph.vel = std::array::from_fn(|i| (ph.origin[i] - ph.prev_origin[i]) * 20.0);
    ph.angles[1] = diff_track_angle(want[1], ph.prev_angles[1], 4.0, DT);
    let leans = matches!(def.vehicle_type, 3 | 4 | 6);
    if def.vehicle_type == 3 {
        ph.angles[0] = diff_track_angle(want[0], ph.prev_angles[0], 6.0, DT);
    }
    let max_acc = def.accel * 0.056_818_184;
    if !autotilt || !leans || max_acc == 0.0 {
        ph.angles[0] = diff_track_angle(want[0], ph.prev_angles[0], 6.0, DT);
        ph.angles[2] = diff_track_angle(want[2], ph.prev_angles[2], 6.0, DT);
        if leans {
            ph.body_tilt[0] = clamp1(ratio(ph.angles[0], def.heli.max_body_pitch));
            ph.body_tilt[1] = clamp1(ratio(ph.angles[2], def.heli.max_body_roll));
        }
    } else {
        tilt_for_accel(ph, def, old_vel, max_acc, if def.vehicle_type == 3 { 0.75 } else { 1.0 });
    }
    ph.rot_vel = std::array::from_fn(|i| norm180(ph.angles[i] - ph.prev_angles[i]) * 20.0);
}

/// VEH_UpdateTiltForAcceleration: it pitches and rolls with how hard it
/// speeds up and turns.
fn tilt_for_accel(ph: &mut Phys, def: &T6VehicleDrive, old_vel: [f32; 3], max_acc: f32, scale: f32) {
    let t = def.vehicle_type;
    let axis = angle_vectors([if t == 3 { ph.angles[0] } else { 0.0 }, ph.angles[1], 0.0]);
    let mut a: [f32; 3] = std::array::from_fn(|i| ph.vel[i] - old_vel[i] * scale);
    let l = len(a);
    if max_acc < l {
        a = a.map(|v| v * (max_acc / l));
    }
    let local = [dot(a, axis[0]), dot(a, axis[1]), dot(a, axis[2])];
    let pitch = match t {
        6 => local[0] / max_acc * def.heli.tilt_from_accel[0],
        3 => ratio(vectosignedangles(ph.vel)[0], def.heli.max_body_pitch),
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
        ph.angles[0] = diff_track_angle(
            def.heli.max_body_pitch * ph.body_tilt[0],
            ph.prev_angles[0],
            TRACK_ANGLE_RATE,
            DT,
        );
    }
    let roll_to = def.heli.max_body_roll * ph.body_tilt[1];
    if t == 6 {
        ph.angles[2] = diff_track_angle(roll_to, ph.prev_angles[2], TRACK_ANGLE_RATE, DT);
        return;
    }
    let want = (norm180(roll_to - ph.angles[2]) * 20.0).clamp(-def.heli.rot_rate, def.heli.rot_rate);
    let acc = ((want - ph.rot_vel[2]) * 20.0).clamp(-def.heli.rot_accel, def.heli.rot_accel);
    ph.rot_vel[2] += acc * DT;
    ph.angles[2] = norm180(ph.angles[2] + ph.rot_vel[2] * DT);
}

/// VEH_TouchEntities for a vehicle on its path: each player its body
/// touches (a box 0.15 s of its speed ahead), and what it hits him for.
fn touches(world: &mut World, v: u32, veh: &Vehicle) -> Vec<(u32, i32, [f32; 3])> {
    let Some((origin, angles, model)) = world
        .resource::<Zm>()
        .ents
        .get(&v)
        .map(|e| (e.origin, e.angles, e.model.clone()))
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
    let cm = f.clip_cmodels();
    let Some(&handle) = cm.model_cmodels.get(&model) else {
        return Vec::new();
    };
    let Some(map) = f.clip_bsp().t6 else {
        return Vec::new();
    };
    let Some(radius) = map.cmodels.get(handle as usize).map(|m| m.radius) else {
        return Vec::new();
    };
    let vel = veh.phys.vel;
    let lead = vel.map(|x| x * 0.15);
    let moved = sub(origin, veh.phys.prev_origin);
    let reach = radius + 1.0;
    let mut area_min: [f32; 3] = std::array::from_fn(|i| origin[i] - reach);
    let mut area_max: [f32; 3] = std::array::from_fn(|i| origin[i] + reach);
    for i in 0..3 {
        if moved[i] > 0.0 {
            area_max[i] += moved[i];
        } else {
            area_min[i] += moved[i];
        }
        area_min[i] += lead[i];
        area_max[i] += lead[i];
    }
    let mins = crate::bullet_collision::PLAYER_MINS.map(|x| x - 1.0);
    let maxs = crate::bullet_collision::PLAYER_MAXS.map(|x| x + 1.0);
    let speed = len(vel);
    let mut out = Vec::new();
    for c in players {
        let Some(ps) = f.player(crate::world::ClientId(c)) else {
            continue;
        };
        let at = sub(ps.origin, lead);
        if (0..3).any(|i| at[i] + maxs[i] < area_min[i] || at[i] + mins[i] > area_max[i]) {
            continue;
        }
        let tr = crate::world::t6_transformed_trace(
            map,
            at,
            at,
            mins,
            maxs,
            crate::world::T6Model::Brush(handle),
            u32::MAX,
            origin,
            angles,
        );
        if !tr.startsolid && !tr.allsolid {
            continue;
        }
        let mut dmg = 0;
        let mut dir = [0.0; 3];
        if veh.def.collision_damage > 0.0 && speed >= 0.001 {
            dir = vel.map(|x| x / speed);
            if ps.pm_flags & playerstate_t6::game::pm_flags::PRONE != 0 {
                dmg = 999_999;
            } else {
                let off = [ps.origin[0] - origin[0], ps.origin[1] - origin[1]];
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
    let list = match world.get_resource_mut::<crate::step::PlayerModelTouches>() {
        Some(mut t) => std::mem::take(&mut t.0),
        None => return,
    };
    if list.is_empty() {
        return;
    }
    let pairs: Vec<(ObjRef, ObjRef)> = list
        .into_iter()
        .filter_map(|(c, owner)| {
            let id = owner.script_model()?;
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
            vm.notify_str(world, p, "touch", &[Value::Object(e)]);
            vm.notify_str(world, e, "touch", &[Value::Object(p)]);
        }
    });
}

/// `BO2MP_T6_VEHLOG=1` (a test aid): where path vehicles are each second.
fn veh_log() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("BO2MP_T6_VEHLOG").is_some())
}

/// Each tick: vehicles along their paths (VEH_UpdatePath, VEH_CheckPathEnd,
/// VEH_TouchEntities).
pub(crate) fn advance(world: &mut World, _dt: f32) {
    model_touches(world);
    let ids: Vec<u32> = world.resource::<Zm>().vehicles.list.keys().copied().collect();
    for v in ids {
        let Some(mut veh) = world.resource::<Zm>().vehicles.list.get(&v).cloned() else {
            continue;
        };
        let Some((obj, origin, angles)) =
            world.resource::<Zm>().ents.get(&v).map(|e| (e.obj, e.origin, e.angles))
        else {
            world.resource_mut::<Zm>().vehicles.list.remove(&v);
            continue;
        };
        // `setvehgoalpos` took it off its path: it flies on as a craft.
        let goal = world
            .get_resource::<Streaks>()
            .and_then(|s| s.craft.get(&v).map(|c| c.goal.is_some()))
            .unwrap_or(false);
        if veh.on_path && goal {
            veh.on_path = false;
        }
        let mut reached = Vec::new();
        let mut end = None;
        if veh.on_path {
            let zm = world.resource::<Zm>();
            let nodes = zm.vehicles.nodes.as_deref().unwrap_or(&[]);
            if veh.pp.node >= 0 && (veh.pp.node as usize) < nodes.len() {
                let (target, want, at) = path_position(nodes, &mut veh);
                let cur = veh.pp.node as usize;
                let autotilt = nodes[cur].flags & NODE_ROTATE == 0;
                let def = veh.def;
                path_follow(&mut veh.phys, &def, target, want, autotilt);
                reached = at.into_iter().map(|i| nodes[i].ent).collect();
                if veh_log() {
                    veh.log_ticks += 1;
                    if veh.log_ticks % 20 == 1 || !reached.is_empty() {
                        diag::info!(
                            Sim,
                            "bo2mp t6 vehicle: ent{v} node {} frac {:.2} speed {:.0} at {:?} angles {:?} reached {:?}",
                            nodes[cur].name,
                            veh.pp.frac,
                            veh.speed,
                            veh.phys.origin,
                            veh.phys.angles,
                            reached
                        );
                    }
                }
                if veh.pp.end_of_path {
                    end = Some(nodes[cur].ent);
                }
            }
            let mut zm = world.resource_mut::<Zm>();
            if let Some(e) = zm.ents.get_mut(&v) {
                e.origin = veh.phys.origin;
                e.angles = veh.phys.angles;
            }
        } else {
            veh.phys.prev_origin = veh.phys.origin;
            veh.phys.prev_angles = veh.phys.angles;
            veh.phys.vel = std::array::from_fn(|i| (origin[i] - veh.phys.origin[i]) * 20.0);
            veh.phys.origin = origin;
            veh.phys.angles = angles;
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
                if let Some(no) = node_obj(world, n) {
                    vm.notify_str(world, o, "reached_node", &[Value::Object(no)]);
                }
            }
            if let Some(n) = end {
                vm.notify_str(world, o, "reached_end_node", &[]);
                if let Some(no) = node_obj(world, n) {
                    vm.notify_str(world, o, "reached_node", &[Value::Object(no)]);
                }
            }
            for (c, dmg, dir) in hits {
                let Some(p) = world.resource::<Zm>().players.get(&c).map(|p| p.obj) else {
                    continue;
                };
                vm.notify_str(world, p, "touch", &[Value::Object(o)]);
                vm.notify_str(world, o, "touch", &[Value::Object(p)]);
                if dmg > 0 {
                    let point = super::frame(world)
                        .player(crate::world::ClientId(c))
                        .map_or([0.0; 3], |ps| ps.origin);
                    super::natives_ai::player_damage(
                        vm,
                        world,
                        c,
                        Value::Object(o),
                        Value::Object(o),
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

pub(super) fn bind(vm: &mut Vm<World>) {
    vm.bind("attachpath", true, |vm, world, s, a| {
        let (Some(v), Some(node)) = (entnum(vm, s), entnum(vm, arg(a, 0))) else {
            return Ok(Value::Undefined);
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
            return Ok(Value::Undefined);
        };
        let pp = set_up_path_pos(nodes, i);
        if veh_log() {
            diag::info!(
                Sim,
                "bo2mp t6 vehicle: ent{v} attachpath {} kind {kind:?} max_speed {} accel {} at {:?}",
                nodes[i].name,
                def.max_speed,
                def.accel,
                pp.origin
            );
        }
        let mut veh = zm.vehicles.list.get(&v).cloned().unwrap_or_default();
        veh.def = def;
        veh.phys.origin = pp.origin;
        veh.phys.prev_origin = pp.origin;
        veh.phys.angles = pp.angles;
        veh.phys.prev_angles = pp.angles;
        let mut zm = world.resource_mut::<Zm>();
        if let Some(e) = zm.ents.get_mut(&v) {
            e.origin = pp.origin;
            e.angles = pp.angles;
        }
        veh.pp = pp;
        zm.vehicles.list.insert(v, veh);
        Ok(Value::Undefined)
    });
    vm.bind("startpath", true, |vm, world, s, _| {
        let Some(v) = entnum(vm, s) else {
            return Ok(Value::Undefined);
        };
        let mut zm = world.resource_mut::<Zm>();
        let Some(veh) = zm.vehicles.list.get_mut(&v) else {
            return Ok(Value::Undefined);
        };
        veh.on_path = true;
        drop(zm);
        if let Some(mut st) = world.get_resource_mut::<Streaks>()
            && let Some(c) = st.craft.get_mut(&v)
        {
            c.goal = None;
        }
        Ok(Value::Undefined)
    });
    for name in [
        "setspeed",
        "setspeedimmediate",
        "resumespeed",
        "setvehgoalpos",
        "vehicle_detachfrompath",
        // Its cheap physics (`setcheapflag`): nothing a server port needs.
        "setcheapflag",
    ] {
        vm.bind(name, true, |_, _, _, _| Ok(Value::Undefined));
    }
}
