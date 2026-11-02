//! bo2mp: a dead player's body going limp, as BO2's own ragdoll does it.
//! Once the server says the body is a ragdoll (its corpse's trajectory
//! turns to one: the script's `startragdoll` and `launchragdoll`), the body
//! leaves its death animation where it is and falls under its own weight,
//! at the speed the animation had it moving.
//!
//! The body is the game's ragdoll definition 0 (`ragdoll.cfg`, read by
//! `sim::t6_ragdoll`). Each of its bodies is a stick between two joints of
//! the skeleton (an upper arm from shoulder to elbow); a body that others
//! hang from at points along it (the torso, which the neck, shoulders and
//! hips hang from) is held rigid. Joints keep to the file's limits: a hinge
//! (elbow, knee) bends one way only, within its range; a swivel (neck,
//! shoulder, hip) swings within its box. Twist along a limb is not
//! simulated: each limb turns with the body it hangs from. Bodies in the
//! file's self pairs stay apart; every joint stays out of the world's
//! brushes and meshes (a box about it a little smaller than its body),
//! sliding on what it rests on with its body's friction.
//!
//! Position-based steps of 1/90 s, gravity 800. What slows it is BO2's
//! own (rbint.c, ragdoll.c): each rigid body's drag (linear on the body's
//! centre, angular on how its joints move about that centre), a spin no
//! faster than 16 radians a second for any one body, and each joint's
//! friction from its `ragdoll_limit` lines (Ragdoll_UpdateFriction: the
//! file's friction x the joint damp scale, rising from 1x to 2x over its
//! first 3 s), which takes a fixed bit of spin out of a limb turning against
//! its parent each step. It sleeps (stops and keeps its last pose) as
//! Ragdoll_CheckIdle has it: once over 75% of its bodies are at rest
//! (rbint::update_stability: no joint of the body faster than 11.9 units a
//! second for 0.5 s while it touches the world, or 1 s), for over 0.25 s;
//! at once when it falls faster than 2500 units a second; or after 9 s.

use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};

use bevy::prelude::*;
use sim::ragdoll_defs::{RagdollDef, RagdollJointKind};
use diag::{info, warn};

const FALL_ACCEL: f32 = 800.0;
const STEP: f32 = 1.0 / 90.0;
const MAX_STEPS_PER_FRAME: u32 = 6;
const ITERATIONS: usize = 8;
/// What a body or a loose prop lands on: solid, glass and item clip. A Black
/// Ops II map's ground can be clip-only (Nuketown's has no solid bit), so the
/// solid bit alone lets a body fall through it.
pub(crate) const MASK_PHYS_WORLD: u32 = 0xc11;
/// `phys_dragLinear` and `phys_dragAngular` (per second; rbint.c takes
/// `1 - min(dt x drag, 0.9)` off a body's speed and spin each step), BO2's
/// defaults as its server registers them), x1 as phys.c gives a ragdoll body.
const DRAG_LINEAR: f32 = 0.03;
const DRAG_ANGULAR: f32 = 0.5;
/// rbint.c's `m_max_avel`, set for every rigid body (radians a second).
const MAX_SPIN: f32 = 16.0;
/// `phys_ragdoll_joint_damp_scale`, BO2's default.
const JOINT_DAMP_SCALE: f32 = 0.04;
/// `ragdoll_jointlerp_time`: a joint's friction rises from 1x to 2x over this.
const JOINT_FRICTION_RAMP_SECONDS: f32 = 3.0;
/// `ragdoll_bone`'s mass is in thousandths (Ragdoll_Bone_f).
const MASS_OF_HEFT: f32 = 0.001;
/// Above BO2's hardest MP throw (a barrel's `launchragdoll`, at most 143
/// units -> 8700 u/s); BO2 itself has none. Only a solver blow-up meets it.
const MAX_SPEED: f32 = 10000.0;
const START_SPEED_MAX: f32 = 600.0;
/// rbint::update_stability: a body is at rest once its largest speed
/// squared stays at most 141.61 (11.9 u/s) for 0.5 s while it touches the
/// world (a ragdoll body needs 1 contact, phys.c), or for 1 s untouched.
const REST_SPEED_SQ: f32 = 141.61;
const REST_TOUCHING_SECONDS: f32 = 0.5;
const REST_SECONDS: f32 = 1.0;
/// Ragdoll_CheckIdle: over this share of the bodies at rest, over this long.
const IDLE_SHARE: f32 = 0.75;
const IDLE_SECONDS: f32 = 0.25;
/// Ragdoll_CheckIdle: falling faster than this on average, it stops at
/// once (a body falling out of the world).
const IDLE_FALL_SPEED: f32 = -2500.0;
/// `ragdoll_max_life`, 9000 ms (Ragdoll_InitDvars).
const MAX_LIFE_SECONDS: f32 = 9.0;
/// A joint's box against the world, against its body's radius.
const BOX_OF_RADIUS: f32 = 0.85;
/// How far past the pose it starts in a joint's limit lets go (degrees).
const START_MARGIN_DEG: f32 = 3.0;
/// How far a hinge may lean off its plane from the pose it starts in.
const HINGE_LEAN_DEG: f32 = 5.0;
/// How hard a joint limit pulls back per pass (0..1).
const LIMIT_STIFFNESS: f32 = 0.5;
/// A stick swung further than this from where its parent carries it keeps
/// its own last turn (degrees); over the `STICK_BLEND_DEG` below that it
/// eases from one to the other.
const STICK_KEEP_DEG: f32 = 150.0;
const STICK_BLEND_DEG: f32 = 30.0;
/// `launchragdoll`'s vector as a speed, BO2's own: Ragdoll_Launch takes
/// ragdoll_bullet_force 0.5 x the vector over the bones as one kick each
/// (Ragdoll_LaunchUpdate's Phys_ObjAddForce, dv = F/m), and ragdoll.cfg's
/// human body weighs 8.2 x 0.001 (Ragdoll_Bone_f), so its centre leaves at
/// 0.5 / 0.0082 = 61 u/s per unit (zm_ragdoll's `LAUNCH_SPEED`). A plain
/// gunshot death has none: only `startragdoll`, from its death animation.
const LAUNCH_SCALE: f32 = 0.5 / 0.0082;

/// The Gravity Gun (the menu's guest weapons, `guest_weapons`): a held
/// body's root follows the beam's point this fast per unit away, at most
/// `BEAM_SPEED_MAX`; the limbs hang from it.
const BEAM_GAIN: f32 = 12.0;
const BEAM_SPEED_MAX: f32 = 1500.0;
/// A body flying faster than this into another's root (within
/// `KNOCK_REACH`) gives it `KNOCK_SHARE` of its speed and keeps the rest.
const KNOCK_SPEED: f32 = 250.0;
const KNOCK_REACH: f32 = 30.0;
const KNOCK_SHARE: f32 = 0.55;

/// A limp body as the Gravity Gun sees it: its root and speed (map units).
#[derive(Clone, Copy, Debug)]
pub(crate) struct BodySeen {
    pub at: Vec3,
    pub speed: Vec3,
}

#[derive(Default)]
struct Beam {
    /// Where each held body's root is pulled to.
    hold: HashMap<u32, Vec3>,
    /// A throw waiting for its body: the speed every joint takes.
    throw: HashMap<u32, Vec3>,
    seen: HashMap<u32, BodySeen>,
    /// bo2zm: script models gone limp (`zm_ragdoll`), seen the same way.
    limp: HashMap<u32, BodySeen>,
}

static BEAM: LazyLock<Mutex<Beam>> = LazyLock::new(Default::default);

/// BO2MP_RAGDOLLLOG=1: where each body goes, how far it turns and when
/// it settles (a line every tenth of a second, then one when it lies still).
static SPIN_LOG: LazyLock<bool> =
    LazyLock::new(|| std::env::var("BO2MP_RAGDOLLLOG").as_deref() == Ok("1"));
static SPIN_IDS: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(1);

/// One body's log: its first rigid body's turn added up step by step.
#[derive(Clone, Debug)]
struct SpinLog {
    id: u32,
    body: usize,
    last: Quat,
    turned: f32,
    from: Vec3,
    next: f32,
    /// The biggest one-step move of a joint against the root since the last line (a pop, not the body's travel).
    jump: f32,
}

fn beam() -> std::sync::MutexGuard<'static, Beam> {
    BEAM.lock().unwrap_or_else(|e| e.into_inner())
}

/// The Gravity Gun holds body `id` (its corpse's entity) at map point `at`,
/// or lets go (`None`).
pub(crate) fn beam_hold(id: u32, at: Option<Vec3>) {
    match at {
        Some(at) => beam().hold.insert(id, at),
        None => beam().hold.remove(&id),
    };
}

/// The Gravity Gun throws body `id` at `speed` (units a second).
pub(crate) fn beam_throw(id: u32, speed: Vec3) {
    let mut b = beam();
    b.hold.remove(&id);
    b.throw.insert(id, speed);
}

/// Every limp body drawn lately, by its id.
pub(crate) fn bodies_seen() -> HashMap<u32, BodySeen> {
    let b = beam();
    let mut all = b.limp.clone();
    all.extend(b.seen.iter().map(|(id, s)| (*id, *s)));
    all
}

/// bo2zm: a limp script model (`zm_ragdoll`) was drawn, or is gone.
pub(crate) fn note_limp(id: u32, seen: BodySeen) {
    beam().limp.insert(id, seen);
}

pub(crate) fn drop_limp(id: u32) {
    beam().limp.remove(&id);
}

/// What the beam does to body `id` this frame: where it holds it, and a
/// throw waiting for it.
pub(crate) fn beam_on(id: u32) -> (Option<Vec3>, Option<Vec3>) {
    let mut b = beam();
    (b.hold.get(&id).copied(), b.throw.remove(&id))
}

/// The limp bodies drawn this frame, for the Gravity Gun; the rest are
/// gone from its sight.
pub(crate) fn publish_seen(seen: HashMap<u32, BodySeen>) {
    beam().seen = seen;
}

/// His "double ragdoll": a body flying fast into another knocks it on.
pub(crate) fn knock(bodies: &mut [(u32, &mut Ragdoll)]) {
    for i in 0..bodies.len() {
        let (id, at, speed, held) = {
            let (id, r) = &bodies[i];
            (*id, r.x[r.rig.bodies[0].p0], r.v[r.rig.bodies[0].p0], r.held.is_some())
        };
        if speed.length() < KNOCK_SPEED || held {
            continue;
        }
        let mut struck = 0;
        for (j, (other, b)) in bodies.iter_mut().enumerate() {
            let core = b.rig.bodies[0].p0;
            if j == i || b.held.is_some() || b.x[core].distance(at) > KNOCK_REACH {
                continue;
            }
            // Only one going slower than the thrower takes the knock.
            if b.v[core].length() >= speed.length() * KNOCK_SHARE {
                continue;
            }
            b.v.iter_mut().for_each(|v| *v += speed * KNOCK_SHARE);
            b.wake();
            struck += 1;
            info!(World, "gravity gun: body #{id} knocks #{other} on at {:.0} u/s", speed.length());
        }
        if struck > 0 {
            bodies[i].1.v.iter_mut().for_each(|v| *v *= 1.0 - KNOCK_SHARE);
        }
    }
}

#[derive(Clone, Debug)]
struct Body {
    p0: usize,
    p1: usize,
    rad: f32,
    owner: Option<usize>,
    /// The skeleton bone at its start joint.
    bone_ix: usize,
    /// Its direction (start joint to end joint) and length when it began.
    dir_start: Vec3,
    len: f32,
    /// Its joints, with those of the bodies hanging from points along it;
    /// more than two make it rigid, kept to where they began (`rest`).
    cluster: Vec<usize>,
    rest: Vec<Vec3>,
    rest_center: Vec3,
    /// Its mass (`ragdoll_bone`'s, in BO2's units).
    mass: f32,
}

impl Body {
    fn rigid(&self) -> bool {
        self.cluster.len() > 2
    }
}

#[derive(Clone, Debug)]
struct Limit {
    body: usize,
    /// The skeleton parent of the body's start joint, and the body that
    /// carries that bone.
    parent_bone: usize,
    parent_body: usize,
    /// The joint's turn against its parent in the bind pose.
    rel_bind: Quat,
    /// The body's direction in its own joint's frame.
    e: Vec3,
    /// Per axis (y, z): the sign the file's angles take on this skeleton,
    /// and the (least, most) angle in radians, widened to the start pose.
    y: Option<(f32, f32, f32)>,
    z: Option<(f32, f32, f32)>,
    /// Its friction (`damp_k` at 1x): the most spin against its parent it
    /// takes out a second, as an angular impulse.
    damp: f32,
}

#[derive(Clone, Debug)]
struct Rig {
    bodies: Vec<Body>,
    limits: Vec<Limit>,
    /// Bodies kept apart: (a, b, the least distance between their sticks).
    pairs: Vec<(usize, usize, f32)>,
    inv_mass: Vec<f32>,
    /// Each joint's box against the world (half its size).
    half: Vec<f32>,
    friction: Vec<f32>,
    x_start: Vec<Vec3>,
    /// Every bone where it began (world), and its turn.
    bone_start: Vec<Mat4>,
    rot_start: Vec<Quat>,
    /// The body each bone moves with.
    owner: Vec<usize>,
}

#[derive(Clone, Copy, Debug)]
struct Contact {
    nrm: Vec3,
    point: Vec3,
    /// Its speed into the surface when it met it (negative going in).
    speed_in: f32,
}

/// One dead body going limp.
#[derive(Clone, Debug)]
pub struct Ragdoll {
    rig: Rig,
    x: Vec<Vec3>,
    /// The joints one step back, to draw between steps.
    x_prev: Vec<Vec3>,
    v: Vec<Vec3>,
    /// Each body's turn from where it began: a rigid body's last fit, a
    /// stick's as the last step left it.
    q: Vec<Quat>,
    carry: f32,
    age: f32,
    /// How long each joint has stayed under the rest speed.
    still: Vec<f32>,
    /// How long most of it has lain at rest (Ragdoll_CheckIdle).
    calm: f32,
    asleep: bool,
    /// The ground when there is no world to land on: just under its
    /// lowest joint at the start.
    floor: f32,
    slept_pose: Option<(Mat4, Vec<Mat4>)>,
    note: String,
    spin: Option<SpinLog>,
    /// The Gravity Gun's pull point while it holds the body.
    held: Option<Vec3>,
}

/// The definition's joints in order (each body's start, then its end).
fn particle_names(def: &RagdollDef) -> Vec<&str> {
    let mut names: Vec<&str> = Vec::new();
    for b in &def.bodies {
        for n in [b.joint.as_str(), b.child.as_str()] {
            if !names.contains(&n) {
                names.push(n);
            }
        }
    }
    names
}

fn particle_bones(def: &RagdollDef, dobj: &xmodel_runtime::PosedSkeleton) -> Option<Vec<usize>> {
    particle_names(def).iter().map(|n| dobj.find(n)).collect()
}

/// Where the ragdoll's joints are in the world in this pose: what a body
/// still in its death animation keeps from frame to frame, so its ragdoll
/// starts at the speed it was moving.
pub fn joint_points(
    def: &RagdollDef,
    dobj: &xmodel_runtime::PosedSkeleton,
    world_mats: &[Mat4],
    world_mats_from_local: Mat4,
) -> Option<Vec<Vec3>> {
    particle_bones(def, dobj)?
        .iter()
        .map(|&b| world_mats.get(b).map(|m| world_mats_from_local.transform_point3(m.w_axis.truncate())))
        .collect::<_>()
}

impl Ragdoll {
    /// Starts from the body as drawn this frame (its bones in model space,
    /// placed by `world_from_local`), moving as it moved since `last` (its
    /// joints' world points `dt` seconds ago), plus `launch` (the script's
    /// `launchragdoll` vector). None when the skeleton lacks a joint the
    /// definition names.
    pub fn start(
        def: &RagdollDef,
        dobj: &xmodel_runtime::PosedSkeleton,
        world_in: &[Mat4],
        world_from_own_client: Mat4,
        last: Option<(&[Vec3], f32)>,
        launch: Vec3,
        solids: Option<&asset_world::CollisionClipData>,
    ) -> Option<Self> {
        let names = particle_names(def);
        let pbones = particle_bones(def, dobj)?;
        let count = dobj.bones.len().min(world_in.len());
        if def.bodies.is_empty() || pbones.iter().any(|&b| b >= count) {
            return None;
        }
        let n = names.len();
        let bone_start: Vec<Mat4> = world_in[..count].iter().map(|m| world_from_own_client * *m).collect();
        let rot_start: Vec<Quat> = bone_start.iter().map(|m| m.to_scale_rotation_translation().1).collect();
        let x: Vec<Vec3> = pbones.iter().map(|&b| bone_start[b].w_axis.truncate()).collect();
        let index = |name: &str| names.iter().position(|n| *n == name);

        let mut bodies: Vec<Body> = Vec::with_capacity(def.bodies.len());
        for b in &def.bodies {
            let (p0, p1) = (index(&b.joint)?, index(&b.child)?);
            let d = x[p1] - x[p0];
            bodies.push(Body {
                p0,
                p1,
                rad: b.reach.max(0.5),
                owner: b.above,
                bone_ix: pbones[p0],
                dir_start: d.try_normalize().unwrap_or(Vec3::Z),
                len: d.length(),
                cluster: vec![p0, p1],
                rest: Vec::new(),
                rest_center: Vec3::ZERO,
                mass: b.heft.max(0.05) * MASS_OF_HEFT,
            });
        }
        for i in 0..bodies.len() {
            let hung: Vec<usize> = bodies.iter().filter(|c| c.owner == Some(i)).map(|c| c.p0).collect();
            for p in hung {
                if !bodies[i].cluster.contains(&p) {
                    bodies[i].cluster.push(p);
                }
            }
        }

        // A rigid body's mass is spread over its joints; a stick's is split
        // between its ends.
        let mut mass = vec![0.0f32; n];
        for (b, src) in bodies.iter().zip(&def.bodies) {
            let m = src.heft.max(0.0);
            if b.rigid() {
                for &p in &b.cluster {
                    mass[p] += m / b.cluster.len() as f32;
                }
            } else {
                mass[b.p0] += m * 0.5;
                mass[b.p1] += m * 0.5;
            }
        }
        let inv_mass: Vec<f32> = mass.iter().map(|m| 1.0 / m.max(0.05)).collect();
        for b in &mut bodies {
            if b.rigid() {
                b.rest = b.cluster.iter().map(|&p| x[p]).collect();
                b.rest_center = weighted_center(&b.cluster, &b.rest, &inv_mass);
            }
        }

        let mut radius = vec![0.0f32; n];
        let mut friction = vec![0.0f32; n];
        for (b, src) in bodies.iter().zip(&def.bodies) {
            for p in [b.p0, b.p1] {
                radius[p] = radius[p].max(b.rad);
                friction[p] = friction[p].max(src.grip.max(0.0));
            }
        }
        let mut half: Vec<f32> = radius.iter().map(|r| (r * BOX_OF_RADIUS).max(1.0)).collect();
        // A joint the start pose has in a wall or the floor gets a smaller
        // box, so it does not fall through what it lies in.
        if let Some(solids) = solids {
            for (p, h) in half.iter_mut().enumerate() {
                let at = x[p].to_array();
                while *h > 0.25 && solids.sweep_box(at, at, [-*h; 3], [*h; 3], MASK_PHYS_WORLD).began_inside {
                    *h *= 0.5;
                }
            }
        }

        // Each bone moves with the nearest body that starts at it or above it.
        let mut owner = vec![0usize; count];
        for (i, own) in owner.iter_mut().enumerate() {
            let mut at = Some(i);
            for _ in 0..count {
                let Some(k) = at else { break };
                if let Some(b) = bodies.iter().position(|b| b.bone_ix == k) {
                    *own = b;
                    break;
                }
                at = dobj.bones[k].up.filter(|&p| p < count);
            }
        }

        let rest_turn = |i: usize| dobj.bones[i].rest_world_matrix.to_scale_rotation_translation().1;
        let mut limits = Vec::new();
        let mut notes = Vec::new();
        for j in &def.joints {
            let Some(b) = bodies.get(j.body) else { continue };
            let Some(parent_bone) = dobj.bones[b.bone_ix].up.filter(|&p| p < count) else {
                continue;
            };
            let rel_bind = rest_turn(parent_bone).inverse() * rest_turn(b.bone_ix);
            let e = (rot_start[b.bone_ix].inverse() * b.dir_start).normalize();
            let frame = rot_start[parent_bone] * rel_bind;
            let s0 = swing(e, (frame.inverse() * b.dir_start).normalize());
            let hinge = j.kind == RagdollJointKind::Hinge;
            let z = j.limits[2].map(|(lo, hi)| fit_axis(lo, hi, s0.z));
            let y = if hinge {
                let lean = HINGE_LEAN_DEG.to_radians();
                Some((1.0, s0.y - lean, s0.y + lean))
            } else {
                j.limits[1].map(|(lo, hi)| fit_axis(lo, hi, s0.y))
            };
            let sign = |a: Option<(f32, f32, f32)>| match a {
                Some((s, _, _)) if s < 0.0 => "-",
                Some(_) => "+",
                None => "free",
            };
            // A hinge takes its first limit line's friction, a swivel the
            // larger of its first two (Ragdoll_UpdateFriction).
            let fr = |k: usize| j.frictions.get(k).copied().unwrap_or(0.0).max(0.0);
            let damp = JOINT_DAMP_SCALE * if hinge { fr(0) } else { fr(0).max(fr(1)) };
            notes.push(format!(
                "{} {} y{} z{} (starts y {:.0} z {:.0}) friction {:.2}",
                def.bodies[j.body].joint,
                if hinge { "hinge" } else { "swivel" },
                sign(y),
                sign(z),
                s0.y.to_degrees(),
                s0.z.to_degrees(),
                damp
            ));
            limits.push(Limit {
                body: j.body,
                parent_bone,
                parent_body: owner[parent_bone],
                rel_bind,
                e,
                y,
                z,
                damp,
            });
        }

        let mut pair_list = Vec::new();
        for &(a, b) in &def.self_pairs {
            let (Some(ba), Some(bb)) = (bodies.get(a), bodies.get(b)) else { continue };
            if [ba.p0, ba.p1].iter().any(|p| *p == bb.p0 || *p == bb.p1) {
                continue;
            }
            let (s, t) = closest_on_sticks(x[ba.p0], x[ba.p1], x[bb.p0], x[bb.p1]);
            let gap = (x[ba.p0].lerp(x[ba.p1], s) - x[bb.p0].lerp(x[bb.p1], t)).length();
            pair_list.push((a, b, (ba.rad + bb.rad).min(gap)));
        }

        let mut v = vec![Vec3::ZERO; n];
        if let Some((prev, dt)) = last
            && prev.len() == n
            && (0.004..=0.1).contains(&dt)
        {
            for (vi, (now, then)) in v.iter_mut().zip(x.iter().zip(prev)) {
                *vi = ((*now - *then) / dt).clamp_length_max(START_SPEED_MAX);
            }
        }
        for vi in &mut v {
            *vi += launch * LAUNCH_SCALE;
        }
        let floor = x.iter().map(|p| p.z).fold(f32::MAX, f32::min) - 1.0;
        let cue = format!(
            "bo2mp ragdoll: {} bodies, {} joints, {} pairs; limits on this skeleton: {}",
            bodies.len(),
            limits.len(),
            pair_list.len(),
            notes.join("; ")
        );
        let rigs = bodies.len();
        let spin = SPIN_LOG.then(|| {
            let id = SPIN_IDS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let from = x[bodies[0].p0];
            info!(
                World,
                "ragdoll log: #{id} starts at ({:.0},{:.0},{:.0}) launch {:.0} u/s, root speed {:.0} u/s",
                from.x,
                from.y,
                from.z,
                (launch * LAUNCH_SCALE).length(),
                v[bodies[0].p0].length()
            );
            let body = bodies.iter().position(|b| b.rigid()).unwrap_or(0);
            SpinLog { id, body, last: Quat::IDENTITY, turned: 0.0, from, next: 0.0, jump: 0.0 }
        });
        Some(Self {
            rig: Rig {
                bodies,
                limits,
                pairs: pair_list,
                inv_mass,
                half,
                friction,
                x_start: x.clone(),
                bone_start,
                rot_start,
                owner,
            },
            x_prev: x.clone(),
            x,
            v,
            q: vec![Quat::IDENTITY; rigs],
            carry: 0.0,
            age: 0.0,
            still: vec![0.0; rigs],
            calm: 0.0,
            asleep: false,
            floor,
            slept_pose: None,
            note: cue,
            spin,
            held: None,
        })
    }

    /// What its joints' limits came out as on this skeleton (for the log).
    pub fn note(&self) -> &str {
        &self.note
    }

    pub fn asleep(&self) -> bool {
        self.asleep
    }

    /// Throws it by `launch` (a `launchragdoll` after it went limp). Its
    /// life starts over, as BO2's explosion kick sets a body running with
    /// its state clock at 0 (stateMsec), so a body long at rest moves again.
    pub fn fling(&mut self, launch: Vec3) {
        for v in &mut self.v {
            *v += launch * LAUNCH_SCALE;
        }
        self.asleep = false;
        self.slept_pose = None;
        self.calm = 0.0;
        self.still.iter_mut().for_each(|s| *s = 0.0);
        self.age = 0.0;
        if *SPIN_LOG {
            let core = self.rig.bodies[0].p0;
            let from = self.x[core];
            let id = self.spin.as_ref().map_or_else(|| SPIN_IDS.fetch_add(1, std::sync::atomic::Ordering::Relaxed), |l| l.id);
            info!(
                World,
                "ragdoll log: #{id} thrown again at ({:.0},{:.0},{:.0}) launch {:.0} u/s, root speed {:.0} u/s",
                from.x,
                from.y,
                from.z,
                (launch * LAUNCH_SCALE).length(),
                self.v[core].length()
            );
            let body = self.rig.bodies.iter().position(|b| b.rigid()).unwrap_or(0);
            self.spin = Some(SpinLog { id, body, last: self.q[body], turned: 0.0, from, next: 0.0, jump: 0.0 });
        }
    }

    /// It moves again, its life counted from now.
    fn wake(&mut self) {
        self.asleep = false;
        self.slept_pose = None;
        self.calm = 0.0;
        self.still.iter_mut().for_each(|s| *s = 0.0);
        self.age = self.age.min(0.5);
    }

    /// The Gravity Gun holds it at `held` (map units) or lets go, and its
    /// throw: every joint at `thrown` (units a second).
    pub fn beam(&mut self, held: Option<Vec3>, thrown: Option<Vec3>) {
        if held.is_some() || thrown.is_some() {
            self.wake();
        }
        self.held = held;
        if let Some(speed) = thrown {
            let n = self.v.len().max(1) as f32;
            for (k, v) in self.v.iter_mut().enumerate() {
                // A little tumble: the far joints a little faster.
                *v = speed * (0.9 + 0.2 * k as f32 / n);
            }
        }
    }

    /// Where it is for the Gravity Gun.
    pub(crate) fn seen(&self) -> BodySeen {
        let core = self.rig.bodies[0].p0;
        BodySeen { at: self.x[core], speed: self.v[core] }
    }

    /// Moves it on by `dt` seconds.
    pub fn step(&mut self, dt: f32, solids: Option<&asset_world::CollisionClipData>) {
        if self.asleep {
            return;
        }
        self.carry += dt.clamp(0.0, 0.1);
        let mut steps_in = 0;
        while self.carry >= STEP && steps_in < MAX_STEPS_PER_FRAME {
            self.x_prev.clone_from(&self.x);
            self.substep(STEP, solids);
            if let Some(log) = &mut self.spin {
                let core = self.rig.bodies[0].p0;
                let root = self.x[core] - self.x_prev[core];
                let jump = self.x.iter().zip(&self.x_prev).map(|(a, b)| (*a - *b - root).length()).fold(0.0, f32::max);
                log.jump = log.jump.max(jump);
            }
            self.carry -= STEP;
            steps_in += 1;
            if self.asleep {
                self.carry = 0.0;
                break;
            }
        }
        self.carry = self.carry.min(STEP);
        self.log_spin();
    }

    fn log_spin(&mut self) {
        let Some(log) = &mut self.spin else {
            return;
        };
        let now = self.q.get(log.body).copied().unwrap_or(Quat::IDENTITY);
        log.turned += log.last.angle_between(now).to_degrees();
        log.last = now;
        let core = self.rig.bodies[0].p0;
        let at = self.x[core];
        if self.asleep {
            info!(
                World,
                "ragdoll log: #{} settles t={:.2} turned {:.0} deg, root moved {:.0} u, at ({:.0},{:.0},{:.0})",
                log.id,
                self.age,
                log.turned,
                (at - log.from).length(),
                at.x,
                at.y,
                at.z
            );
            self.spin = None;
        } else if self.age >= log.next {
            log.next = self.age + 0.1;
            info!(
                World,
                "ragdoll log: #{} t={:.2} root ({:.0},{:.0},{:.0}) speed {:.0} u/s turned {:.0} deg jump {:.1} u",
                log.id,
                self.age,
                at.x,
                at.y,
                at.z,
                self.v[core].length(),
                log.turned,
                log.jump
            );
            log.jump = 0.0;
        }
    }

    fn substep(&mut self, h: f32, solids: Option<&asset_world::CollisionClipData>) {
        let rig = &self.rig;
        let n = self.x.len();
        let old = self.x.clone();
        let mut p = Vec::with_capacity(n);
        let mut contacts: Vec<Option<Contact>> = vec![None; n];
        if let Some(at) = self.held {
            // The beam carries the root; the rest hangs from it.
            let core = rig.bodies[0].p0;
            let pull = ((at - self.x[core]) * BEAM_GAIN).clamp_length_max(BEAM_SPEED_MAX);
            for (k, v) in self.v.iter_mut().enumerate() {
                *v = v.lerp(pull, if k == core { 1.0 } else { 0.35 });
            }
            self.v[core].z += FALL_ACCEL * h;
            self.age = self.age.min(0.5);
        }
        for i in 0..n {
            let mut v = self.v[i];
            v.z -= FALL_ACCEL * h;
            v = v.clamp_length_max(MAX_SPEED);
            self.v[i] = v;
            let (at, hit) = sweep(solids, self.floor, rig.half[i], old[i], old[i] + v * h);
            p.push(at);
            contacts[i] = hit.map(|normal| Contact { nrm: normal, point: at, speed_in: v.dot(normal) });
        }
        let swept = p.clone();
        for _ in 0..ITERATIONS {
            solve_sticks(rig, &mut p);
            solve_rigid(rig, &mut p, &mut self.q);
            solve_limits(rig, &mut p, &self.q);
            solve_pairs(rig, &mut p);
            for (pi, c) in p.iter_mut().zip(&contacts) {
                if let Some(c) = c {
                    let d = (*pi - c.point).dot(c.nrm);
                    if d < 0.0 {
                        *pi -= c.nrm * d;
                    }
                }
            }
        }
        // What the passes moved, through the world: every move, however
        // small (a joint nudged a fraction of a unit under a floor's face
        // is under it, and the next step falls through).
        for i in 0..n {
            if (p[i] - swept[i]).length_squared() > 1e-6 {
                let (at, hit) = sweep(solids, self.floor, rig.half[i], old[i], p[i]);
                if let Some(normal) = hit {
                    p[i] = at;
                    if contacts[i].is_none() {
                        contacts[i] = Some(Contact { nrm: normal, point: at, speed_in: self.v[i].dot(normal) });
                    }
                }
            }
        }
        if p.iter().any(|pi| !pi.is_finite()) {
            // Never a body flung to nowhere: it stops where it was.
            warn!(World, "bo2mp ragdoll: a step went wrong; the body stops where it was");
            self.v.iter_mut().for_each(|v| *v = Vec3::ZERO);
            self.asleep = true;
            return;
        }
        // Speeds from the moves; what touches the world stops going into
        // it and drags along it (by how hard it pressed in).
        for i in 0..n {
            let mut v = (p[i] - old[i]) / h;
            if let Some(c) = contacts[i] {
                let normal_speed = v.dot(c.nrm);
                if normal_speed < 0.0 {
                    v -= c.nrm * normal_speed;
                }
                let vt = v - c.nrm * v.dot(c.nrm);
                let speed = vt.length();
                if speed > 1e-4 {
                    let cut = rig.friction[i] * (-c.speed_in).max(0.0);
                    v -= vt * (cut.min(speed) / speed);
                }
            }
            self.v[i] = v;
        }
        let friction_scale = 1.0 + (self.age / JOINT_FRICTION_RAMP_SECONDS).min(1.0);
        bo2_slowing(rig, &p, &mut self.v, h, friction_scale);
        // Each stick's turn as this step leaves it, for the next.
        let turns = body_turns(rig, &p, &self.q);
        for (i, _) in rig.bodies.iter().enumerate().filter(|(_, b)| !b.rigid()) {
            self.q[i] = turns[i];
        }
        self.x = p;
        self.age += h;
        let (mut resting, mut fall) = (0usize, 0.0f32);
        for (b, s) in rig.bodies.iter().zip(self.still.iter_mut()) {
            let fastest = b.cluster.iter().map(|&i| self.v[i].length_squared()).fold(0.0f32, f32::max);
            let touching = b.cluster.iter().any(|&i| contacts[i].is_some());
            *s = if fastest <= REST_SPEED_SQ { *s + h } else { 0.0 };
            resting += usize::from(*s >= REST_SECONDS || (touching && *s >= REST_TOUCHING_SECONDS));
            fall += body_motion(rig, b, &self.x, &self.v).1.z;
        }
        let bodies = rig.bodies.len().max(1) as f32;
        let falling_out = fall / bodies < IDLE_FALL_SPEED;
        self.calm = if resting as f32 > IDLE_SHARE * bodies { self.calm + h } else { 0.0 };
        if self.held.is_none()
            && ((self.calm > IDLE_SECONDS && self.age > 0.5) || falling_out || self.age > MAX_LIFE_SECONDS)
        {
            self.asleep = true;
            self.v.iter_mut().for_each(|v| *v = Vec3::ZERO);
            self.x_prev.clone_from(&self.x);
        }
    }

    /// The body as it lies now: every bone of the skeleton in model space
    /// (written into `world`, which holds the same skeleton) and where that
    /// model sits (at its root joint, unturned).
    pub fn pose(&mut self, world: &mut [Mat4]) -> Mat4 {
        if let Some((placed, bones)) = &self.slept_pose {
            let count = world.len().min(bones.len());
            world[..count].copy_from_slice(&bones[..count]);
            return *placed;
        }
        let rig = &self.rig;
        let alpha = if self.asleep { 1.0 } else { (self.carry / STEP).clamp(0.0, 1.0) };
        let p: Vec<Vec3> = self.x_prev.iter().zip(&self.x).map(|(a, b)| a.lerp(*b, alpha)).collect();
        let mut q = self.q.clone();
        for (i, b) in rig.bodies.iter().enumerate().filter(|(_, b)| b.rigid()) {
            q[i] = fit_turn(rig, b, &p, q[i]);
        }
        let turns = body_turns(rig, &p, &q);
        let root = p[rig.bodies[0].p0];
        let count = world.len().min(rig.bone_start.len());
        for (i, bone) in world.iter_mut().enumerate().take(count) {
            let own = rig.owner[i];
            let b = &rig.bodies[own];
            *bone = Mat4::from_translation(p[b.p0] - root)
                * Mat4::from_quat(turns[own])
                * Mat4::from_translation(-rig.x_start[b.p0])
                * rig.bone_start[i];
        }
        let placed = Mat4::from_translation(root);
        if self.asleep {
            self.slept_pose = Some((placed, world[..count].to_vec()));
        }
        placed
    }
}

/// A body's centre, its centre's speed and its spin (radians a second,
/// about the centre) from its joints' speeds; a stick's turn along itself
/// is not simulated, so it has none.
fn body_motion(rig: &Rig, b: &Body, p: &[Vec3], v: &[Vec3]) -> (Vec3, Vec3, Vec3) {
    let at: Vec<Vec3> = b.cluster.iter().map(|&i| p[i]).collect();
    let speeds: Vec<Vec3> = b.cluster.iter().map(|&i| v[i]).collect();
    let c = weighted_center(&b.cluster, &at, &rig.inv_mass);
    let vc = weighted_center(&b.cluster, &speeds, &rig.inv_mass);
    let (mut l, mut inertia) = (Vec3::ZERO, 0.0f32);
    for &i in &b.cluster {
        let (r, m) = (p[i] - c, 1.0 / rig.inv_mass[i]);
        l += r.cross(v[i] - vc) * m;
        inertia += r.length_squared() * m;
    }
    let w = if inertia > 1e-6 { l / inertia } else { Vec3::ZERO };
    (c, vc, w)
}

/// Turns body `b`'s joint speeds by `dw` (radians a second) about `about`.
fn turn_body(b: &Body, p: &[Vec3], v: &mut [Vec3], about: Vec3, dw: Vec3) {
    for &i in &b.cluster {
        v[i] += dw.cross(p[i] - about);
    }
}

/// A body's inertia about its centre: a stick's `m (3r^2 + L^2) / 12`, a
/// rigid body's from how far its joints lie from its centre.
fn body_inertia(rig: &Rig, b: &Body, p: &[Vec3]) -> f32 {
    if b.rigid() {
        let at: Vec<Vec3> = b.cluster.iter().map(|&i| p[i]).collect();
        let c = weighted_center(&b.cluster, &at, &rig.inv_mass);
        let r2 = at.iter().map(|q| (*q - c).length_squared()).sum::<f32>() / at.len() as f32;
        b.mass * r2.max(1.0)
    } else {
        b.mass * (3.0 * b.rad * b.rad + b.len * b.len) / 12.0
    }
}

/// BO2's slowing of a ragdoll each step (rbint.c, rigid_body_constraint_ragdoll):
/// each body's spin held to `MAX_SPIN`, then its drag, `DRAG_LINEAR` on its
/// speed and `DRAG_ANGULAR` on its spin; then each joint's friction takes
/// up to `damp x scale x h` of angular impulse out of the limb's spin
/// against its parent, each body turning about its own centre by its
/// inertia (an impulse that turns, so the body's speed stays). Every turn
/// is about a body's own weighted centre, so none of it pushes the body
/// along.
fn bo2_slowing(rig: &Rig, p: &[Vec3], v: &mut [Vec3], h: f32, scale: f32) {
    let keep_lin = 1.0 - (h * DRAG_LINEAR).min(0.9);
    let keep_ang = 1.0 - (h * DRAG_ANGULAR).min(0.9);
    // Bodies share their joints, so holding one body's spin moves its
    // neighbours': the limbs first, then the rigid bodies they hang from,
    // twice over.
    let order: Vec<&Body> = rig.bodies.iter().filter(|b| !b.rigid()).chain(rig.bodies.iter().filter(|b| b.rigid())).collect();
    for _ in 0..2 {
        for b in &order {
            let (c, _, w) = body_motion(rig, b, p, v);
            let spin = w.length();
            if spin > MAX_SPIN {
                turn_body(b, p, v, c, w * (MAX_SPIN / spin - 1.0));
            }
        }
    }
    // The linear drag on every joint slows each body's speed and spin
    // alike; each spin then takes the rest of its angular drag.
    v.iter_mut().for_each(|vi| *vi *= keep_lin);
    for b in &order {
        let (c, _, w) = body_motion(rig, b, p, v);
        turn_body(b, p, v, c, w * (keep_ang / keep_lin - 1.0));
    }
    for l in &rig.limits {
        if l.damp <= 0.0 || l.parent_body == l.body {
            continue;
        }
        let (child, parent) = (&rig.bodies[l.body], &rig.bodies[l.parent_body]);
        let (cc, _, wc) = body_motion(rig, child, p, v);
        let (cp, _, wp) = body_motion(rig, parent, p, v);
        let rel = wc - wp;
        let speed = rel.length();
        if speed < 1e-4 {
            continue;
        }
        let ic = body_inertia(rig, child, p).max(1e-6);
        let ip = body_inertia(rig, parent, p).max(1e-6);
        let impulse = (l.damp * scale * h).min(speed / (1.0 / ic + 1.0 / ip));
        let dir = rel / speed;
        turn_body(child, p, v, cc, -dir * (impulse / ic));
        turn_body(parent, p, v, cp, dir * (impulse / ip));
    }
}

fn weighted_center(cluster: &[usize], points: &[Vec3], inv_mass: &[f32]) -> Vec3 {
    let (sum, total) = cluster.iter().zip(points).fold((Vec3::ZERO, 0.0f32), |(s, t), (&i, p)| {
        let m = 1.0 / inv_mass[i];
        (s + *p * m, t + m)
    });
    if total > 0.0 { sum / total } else { Vec3::ZERO }
}

/// The ragdoll's ground: the world's brushes and meshes it lands on, or with no
/// world loaded a flat floor. Where a joint's box moving `from` to `to`
/// stops, and the face it stopped on.
fn sweep(
    solids: Option<&asset_world::CollisionClipData>,
    floor: f32,
    half: f32,
    from: Vec3,
    to: Vec3,
) -> (Vec3, Option<Vec3>) {
    match solids {
        Some(solids) => {
            if from.distance_squared(to) < 1e-8 {
                return (to, None);
            }
            let hit_in = solids.sweep_box(from.to_array(), to.to_array(), [-half; 3], [half; 3], MASK_PHYS_WORLD);
            // A box that starts in something moves free until it is out,
            // but never down: down is deeper into a floor.
            if hit_in.began_inside {
                return match to.z < from.z {
                    true => (Vec3::new(to.x, to.y, from.z), Some(Vec3::Z)),
                    false => (to, None),
                };
            }
            if hit_in.fraction >= 1.0 {
                return (to, None);
            }
            let normal = Vec3::from_array(hit_in.normal).try_normalize().unwrap_or(Vec3::Z);
            (Vec3::from_array(hit_in.finish_point), Some(normal))
        }
        None if to.z < floor => (Vec3::new(to.x, to.y, floor), Some(Vec3::Z)),
        None => (to, None),
    }
}

fn solve_sticks(rig: &Rig, p: &mut [Vec3]) {
    for b in rig.bodies.iter().filter(|b| !b.rigid()) {
        let (wa, wb) = (rig.inv_mass[b.p0], rig.inv_mass[b.p1]);
        let d = p[b.p1] - p[b.p0];
        let l = d.length();
        if l < 1e-4 || wa + wb <= 0.0 {
            continue;
        }
        let k = (l - b.len) / (l * (wa + wb));
        p[b.p0] += d * (k * wa);
        p[b.p1] -= d * (k * wb);
    }
}

fn solve_rigid(rig: &Rig, p: &mut [Vec3], q: &mut [Quat]) {
    for (i, b) in rig.bodies.iter().enumerate().filter(|(_, b)| b.rigid()) {
        let turn = fit_turn(rig, b, p, q[i]);
        q[i] = turn;
        let center = weighted_center(&b.cluster, &b.cluster.iter().map(|&k| p[k]).collect::<Vec<_>>(), &rig.inv_mass);
        for (k, &pi) in b.cluster.iter().enumerate() {
            p[pi] = center + turn * (b.rest[k] - b.rest_center);
        }
    }
}

/// The turn that best carries a rigid body's start shape onto its joints
/// now (from `warm`, the last one).
fn fit_turn(rig: &Rig, b: &Body, p: &[Vec3], warm: Quat) -> Quat {
    let now: Vec<Vec3> = b.cluster.iter().map(|&k| p[k]).collect();
    let center = weighted_center(&b.cluster, &now, &rig.inv_mass);
    let mut a = Mat3::ZERO;
    for (k, &pi) in b.cluster.iter().enumerate() {
        let x = (p[pi] - center) / rig.inv_mass[pi];
        let r = b.rest[k] - b.rest_center;
        a = a + Mat3::from_cols(x * r.x, x * r.y, x * r.z);
    }
    extract_rotation(a, warm)
}

/// The turn closest to `a` (Müller et al. 2016), from `q`.
fn extract_rotation(a: Mat3, mut q: Quat) -> Quat {
    for _ in 0..16 {
        let r = Mat3::from_quat(q);
        let num = r.x_axis.cross(a.x_axis) + r.y_axis.cross(a.y_axis) + r.z_axis.cross(a.z_axis);
        let den = (r.x_axis.dot(a.x_axis) + r.y_axis.dot(a.y_axis) + r.z_axis.dot(a.z_axis)).abs() + 1e-9;
        let w = num / den;
        let angle = w.length();
        if angle < 1e-7 {
            break;
        }
        q = (Quat::from_axis_angle(w / angle, angle) * q).normalize();
    }
    q
}

/// Each body's turn from where it began: a rigid body's from its fit, a
/// stick's the least turn from where its parent carried it to where it
/// points now. Swung near right round from that (an arm thrown over the
/// head), the least turn has no steady twist and flipped half round in one
/// step, its hinge with it (a hand thrown 19 u); there a stick keeps its
/// last turn (`q`, a step back) and swings from that, as BO2's bodies,
/// each its own rigid body, keep theirs.
fn body_turns(rig: &Rig, p: &[Vec3], q: &[Quat]) -> Vec<Quat> {
    let mut turns: Vec<Quat> = Vec::with_capacity(rig.bodies.len());
    for (i, b) in rig.bodies.iter().enumerate() {
        let parent = b.owner.and_then(|k| turns.get(k).copied()).unwrap_or(Quat::IDENTITY);
        let turn = if b.rigid() {
            q[i]
        } else {
            let carried = (parent * b.dir_start).normalize();
            match (p[b.p1] - p[b.p0]).try_normalize() {
                Some(now) => {
                    let follow = Quat::from_rotation_arc(carried, now) * parent;
                    let share = ((STICK_KEEP_DEG - carried.angle_between(now).to_degrees()) / STICK_BLEND_DEG)
                        .clamp(0.0, 1.0);
                    if share >= 1.0 {
                        follow
                    } else {
                        let kept = Quat::from_rotation_arc((q[i] * b.dir_start).normalize(), now) * q[i];
                        kept.slerp(follow, share).normalize()
                    }
                }
                None => parent,
            }
        };
        turns.push(turn);
    }
    turns
}

fn solve_limits(rig: &Rig, p: &mut [Vec3], q: &[Quat]) {
    let turns = body_turns(rig, p, q);
    for l in &rig.limits {
        let b = &rig.bodies[l.body];
        let frame = turns[l.parent_body] * rig.rot_start[l.parent_bone] * l.rel_bind;
        let d = p[b.p1] - p[b.p0];
        let len = d.length();
        if len < 1e-3 {
            continue;
        }
        let s = swing(l.e, (frame.inverse() * (d / len)).normalize());
        let mut t = s;
        if let Some((sign, lo, hi)) = l.y {
            t.y = sign * (sign * s.y).clamp(lo, hi);
        }
        if let Some((sign, lo, hi)) = l.z {
            t.z = sign * (sign * s.z).clamp(lo, hi);
        }
        if (t - s).length_squared() < 1e-8 {
            continue;
        }
        let t = t - l.e * t.dot(l.e);
        let target = p[b.p0] + frame * (Quat::from_scaled_axis(t) * l.e) * len;
        // Both ends share the turn by mass: moving only the child pushed the
        // body along by nothing (an arm bent into the floor slid it off at
        // 1000+ u/s).
        let (w0, w1) = (rig.inv_mass[b.p0], rig.inv_mass[b.p1]);
        if w0 + w1 <= 0.0 {
            continue;
        }
        let fix = (target - p[b.p1]) * (LIMIT_STIFFNESS / (w0 + w1));
        p[b.p1] += fix * w1;
        p[b.p0] -= fix * w0;
    }
}

fn solve_pairs(rig: &Rig, p: &mut [Vec3]) {
    for &(a, b, least) in &rig.pairs {
        let (ba, bb) = (&rig.bodies[a], &rig.bodies[b]);
        let (s, t) = closest_on_sticks(p[ba.p0], p[ba.p1], p[bb.p0], p[bb.p1]);
        let d = p[ba.p0].lerp(p[ba.p1], s) - p[bb.p0].lerp(p[bb.p1], t);
        let dist = d.length();
        if dist >= least || dist < 1e-4 {
            continue;
        }
        let n = d / dist;
        let w = |i: usize, f: f32| f * f * rig.inv_mass[i];
        let sum = w(ba.p0, 1.0 - s) + w(ba.p1, s) + w(bb.p0, 1.0 - t) + w(bb.p1, t);
        if sum <= 0.0 {
            continue;
        }
        let lambda = (least - dist) / sum;
        p[ba.p0] += n * (lambda * (1.0 - s) * rig.inv_mass[ba.p0]);
        p[ba.p1] += n * (lambda * s * rig.inv_mass[ba.p1]);
        p[bb.p0] -= n * (lambda * (1.0 - t) * rig.inv_mass[bb.p0]);
        p[bb.p1] -= n * (lambda * t * rig.inv_mass[bb.p1]);
    }
}

/// The nearest points of two sticks, as how far along each (0..1).
fn closest_on_sticks(p1: Vec3, q1: Vec3, p2: Vec3, q2: Vec3) -> (f32, f32) {
    const EPS: f32 = 1e-6;
    let (d1, d2, r) = (q1 - p1, q2 - p2, p1 - p2);
    let (a, e, f) = (d1.dot(d1), d2.dot(d2), d2.dot(r));
    if a <= EPS && e <= EPS {
        return (0.0, 0.0);
    }
    if a <= EPS {
        return (0.0, (f / e).clamp(0.0, 1.0));
    }
    let c = d1.dot(r);
    if e <= EPS {
        return ((-c / a).clamp(0.0, 1.0), 0.0);
    }
    let b = d1.dot(d2);
    let denom = a * e - b * b;
    let mut s = if denom > EPS { ((b * f - c * e) / denom).clamp(0.0, 1.0) } else { 0.0 };
    let mut t = (b * s + f) / e;
    if t < 0.0 {
        t = 0.0;
        s = (-c / a).clamp(0.0, 1.0);
    } else if t > 1.0 {
        t = 1.0;
        s = ((b - c) / a).clamp(0.0, 1.0);
    }
    (s, t)
}

/// The least turn (axis times angle, radians) that takes unit `from` to
/// unit `to`.
fn swing(from: Vec3, to: Vec3) -> Vec3 {
    let axis = from.cross(to);
    let sin = axis.length();
    let cos = from.dot(to);
    if sin < 1e-6 {
        if cos >= 0.0 {
            return Vec3::ZERO;
        }
        let other = if from.x.abs() < 0.9 { Vec3::X } else { Vec3::Y };
        return from.cross(other).normalize() * std::f32::consts::PI;
    }
    axis * (sin.atan2(cos) / sin)
}

/// A limit's sign on this skeleton and its range in radians: the file's
/// range (degrees) as it stands, or turned round when the start pose fits
/// it clearly better that way; then widened to take in the start pose.
fn fit_axis(lo: f32, hi: f32, start: f32) -> (f32, f32, f32) {
    let (lo, hi) = (lo.to_radians(), hi.to_radians());
    let miss = |v: f32| (lo - v).max(0.0) + (v - hi).max(0.0);
    let sign = if miss(-start) + 2f32.to_radians() < miss(start) { -1.0 } else { 1.0 };
    let v = sign * start;
    let margin = START_MARGIN_DEG.to_radians();
    (sign, lo.min(v - margin), hi.max(v + margin))
}

mod tests {
    #![cfg(test)]
    use super::*;
    use sim::ragdoll_defs::parse;

    /// A made-up body plan in the file's format (not the game's numbers).
    const DEF: &str = "\
        ragdoll_bone 0 j_mainroot j_neck 5 0.5 3 0.3 -1 0\n\
        ragdoll_bone 0 j_neck j_head 3 0.5 0.5 0.3 0 0\n\
        ragdoll_bone 0 j_shoulder_le j_elbow_le 2 0.5 0.5 0.3 0 0\n\
        ragdoll_bone 0 j_elbow_le j_wrist_le 2 0.5 0.5 0.3 2 0\n\
        ragdoll_bone 0 j_shoulder_ri j_elbow_ri 2 0.5 0.5 0.3 0 1\n\
        ragdoll_bone 0 j_elbow_ri j_wrist_ri 2 0.5 0.5 0.3 4 1\n\
        ragdoll_bone 0 j_hip_le j_knee_le 3 0.5 1 0.3 0 0\n\
        ragdoll_bone 0 j_knee_le j_ankle_le 3 0.5 1 0.3 6 0\n\
        ragdoll_bone 0 j_hip_ri j_knee_ri 3 0.5 1 0.3 0 1\n\
        ragdoll_bone 0 j_knee_ri j_ankle_ri 3 0.5 1 0.3 8 1\n\
        ragdoll_selfpair 0 3 5\n\
        ragdoll_selfpair 0 7 9\n\
        ragdoll_joint 0 1 swivel\n\
        ragdoll_joint 0 3 hinge\n\
        ragdoll_joint 0 7 hinge\n\
        ragdoll_limit 0 0 y 0 -40 40\n\
        ragdoll_limit 0 0 z 0 -40 40\n\
        ragdoll_limit 0 1 z 0 -150 0\n\
        ragdoll_limit 0 2 z 0 -150 0\n";

    /// A standing stick figure: every bone's x along it, pointing down.
    fn figure() -> (xmodel_runtime::PosedSkeleton, Vec<Mat4>) {
        let rows: [(&str, Option<usize>, [f32; 3]); 18] = [
            ("tag_origin", None, [0.0, 0.0, 0.0]),
            ("j_mainroot", Some(0), [0.0, 0.0, 40.0]),
            ("j_spine4", Some(1), [0.0, 0.0, 55.0]),
            ("j_neck", Some(2), [0.0, 0.0, 60.0]),
            ("j_head", Some(3), [0.0, 0.0, 66.0]),
            ("j_clavicle_le", Some(2), [0.0, 3.0, 58.0]),
            ("j_shoulder_le", Some(5), [0.0, 8.0, 58.0]),
            ("j_elbow_le", Some(6), [0.0, 8.0, 46.0]),
            ("j_wrist_le", Some(7), [0.0, 8.0, 34.0]),
            ("j_clavicle_ri", Some(2), [0.0, -3.0, 58.0]),
            ("j_shoulder_ri", Some(9), [0.0, -8.0, 58.0]),
            ("j_elbow_ri", Some(10), [0.0, -8.0, 46.0]),
            ("j_wrist_ri", Some(11), [0.0, -8.0, 34.0]),
            ("j_hip_le", Some(1), [0.0, 4.0, 38.0]),
            ("j_knee_le", Some(13), [0.0, 4.0, 20.0]),
            ("j_ankle_le", Some(14), [0.0, 4.0, 2.0]),
            ("j_hip_ri", Some(1), [0.0, -4.0, 38.0]),
            ("j_knee_ri", Some(16), [0.0, -4.0, 20.0]),
        ];
        let turn = Quat::from_mat3(&Mat3::from_cols(Vec3::NEG_Z, Vec3::NEG_X, Vec3::Y));
        let mut world: Vec<Mat4> = rows
            .iter()
            .map(|(_, _, at)| Mat4::from_rotation_translation(turn, Vec3::from_array(*at)))
            .collect::<_>();
        world.push(Mat4::from_rotation_translation(turn, (Vec3::NEG_Y * 4.0).with_z(2.0)));
        let mut bones: Vec<xmodel_runtime::SkelJoint> = rows
            .iter()
            .enumerate()
            .map(|(i, (name, parent, _))| bone(name, *parent, &world, i))
            .collect::<_>();
        bones.push(bone("j_ankle_ri", Some(17), &world, 18));
        let dobj = xmodel_runtime::PosedSkeleton {
            models: vec![xmodel_runtime::AttachedModelIndex {
                name: "figure".into(),
                first_joint: 0,
                joint_total: bones.len(),
                size_scale: 1.0,
            }],
            bones,
            mirrored_part_pairs: Vec::new(),
        };
        (dobj, world)
    }

    fn bone(caption: &str, owner: Option<usize>, world_v: &[Mat4], i: usize) -> xmodel_runtime::SkelJoint {
        let local = owner.map_or(world_v[i], |p| world_v[p].inverse() * world_v[i]);
        let (_, rotation, translation) = local.to_scale_rotation_translation();
        xmodel_runtime::SkelJoint {
            name: caption.into(),
            model: 0,
            up: owner,
            rest_orientation: rotation,
            rest_offset: translation,
            rest_world_matrix: world_v[i],
            ignores_size_scaling: false,
        }
    }

    #[test]
    fn swing_is_axis_times_angle() {
        let s = swing(Vec3::X, Vec3::Y);
        assert!((s - Vec3::Z * std::f32::consts::FRAC_PI_2).length() < 1e-5, "{s}");
        let s = swing(Vec3::NEG_X, Vec3::NEG_Y);
        assert!((s - Vec3::Z * std::f32::consts::FRAC_PI_2).length() < 1e-5, "{s}");
        assert_eq!(swing(Vec3::Z, Vec3::Z), Vec3::ZERO);
    }

    #[test]
    fn finds_a_turn_from_its_matrix() {
        let want = Quat::from_axis_angle(Vec3::new(1.0, 2.0, 3.0).normalize(), 1.2);
        let got = extract_rotation(Mat3::from_quat(want) * 7.0, Quat::IDENTITY);
        assert!(got.dot(want).abs() > 0.9999, "{got} vs {want}");
    }

    /// BO2's spin cap holds every body to 16 radians a second, and a joint's
    /// friction (its `ragdoll_limit` lines' x 0.04) slows a limb turning
    /// against its parent.
    #[test]
    fn spin_is_capped_and_joints_slow_their_limbs() {
        let def = DEF.replace("ragdoll_limit 0 1 z 0 -150 0", "ragdoll_limit 0 1 z 60 -150 0");
        let defs = parse(&def);
        let (dobj, world) = figure();
        let doll = Ragdoll::start(&defs[0], &dobj, &world, Mat4::IDENTITY, None, Vec3::ZERO, None)
            .expect("the figure has every joint");
        let rig = &doll.rig;
        let elbow = rig.limits.iter().find(|l| l.damp > 0.0).expect("the elbow's friction");
        assert!((elbow.damp - 2.4).abs() < 1e-4, "{}", elbow.damp);
        let torso = &rig.bodies[0];
        assert!(torso.rigid());
        let (c, _, _) = body_motion(rig, torso, &doll.x, &doll.v);
        let mut v: Vec<Vec3> = doll.x.iter().map(|p| (Vec3::X * 40.0).cross(*p - c)).collect();
        bo2_slowing(rig, &doll.x, &mut v, STEP, 1.0);
        let spin = body_motion(rig, torso, &doll.x, &v).2.length();
        assert!(spin <= MAX_SPIN + 0.5, "the torso spins at {spin}");
        // The forearm alone swinging about the elbow.
        let arm = &rig.bodies[elbow.body];
        let mut v = vec![Vec3::ZERO; doll.x.len()];
        v[arm.p1] = (Vec3::X * 10.0).cross(doll.x[arm.p1] - doll.x[arm.p0]);
        let before = body_motion(rig, arm, &doll.x, &v).2.length();
        bo2_slowing(rig, &doll.x, &mut v, STEP, 1.0);
        let after = body_motion(rig, arm, &doll.x, &v).2.length();
        assert!(after < before * 0.9, "the joint's friction left {after} of {before}");
    }

    #[test]
    fn a_pushed_body_falls_and_lies_still_on_the_floor() {
        let defs = parse(DEF);
        let (dobj, world) = figure();
        let ident = Mat4::IDENTITY;
        let mut doll = Ragdoll::start(&defs[0], &dobj, &world, ident, None, Vec3::X * (240.0 / LAUNCH_SCALE), None)
            .expect("the figure has every joint");
        assert_eq!(doll.rig.limits.len(), 3, "{}", doll.note());
        for _ in 0..(8 * 60) {
            doll.step(1.0 / 60.0, None);
        }
        assert!(doll.asleep(), "still moving: {:?}", doll.v);
        for (i, p) in doll.x.iter().enumerate() {
            assert!(p.is_finite() && p.z >= doll.floor - 0.5, "joint {i} at {p}");
        }
        for b in doll.rig.bodies.iter().filter(|b| !b.rigid()) {
            let len = (doll.x[b.p1] - doll.x[b.p0]).length();
            assert!((len - b.len).abs() < b.len * 0.05, "stick {} {} grew to {len}", b.p0, b.p1);
        }
        let mut bones = world.clone();
        let placed = doll.pose(&mut bones);
        let head = (placed * bones[dobj.find("j_head").unwrap()]).w_axis.truncate();
        assert!(head.z < 25.0, "the head stayed up at {head}");
        // Lying still keeps the same pose.
        let again = doll.pose(&mut bones);
        assert_eq!(placed, again);
    }

    /// A body long at rest, kicked again (BO2's Ragdoll_ExplosionEvent sets
    /// it running with its clock at 0): it moves off and lies still again,
    /// even past its life (MAX_LIFE_SECONDS) counted from the first fall.
    #[test]
    fn a_body_at_rest_kicked_again_moves_and_settles() {
        let defs = parse(DEF);
        let (dobj, world) = figure();
        let mut doll = Ragdoll::start(&defs[0], &dobj, &world, Mat4::IDENTITY, None, Vec3::ZERO, None)
            .expect("the figure has every joint");
        let steps = ((MAX_LIFE_SECONDS + 2.0) * 60.0) as usize;
        for _ in 0..steps {
            doll.step(1.0 / 60.0, None);
        }
        assert!(doll.asleep());
        let before = doll.x.clone();
        doll.fling(Vec3::new(40.0, 0.0, 15.0));
        assert!(!doll.asleep());
        for _ in 0..30 {
            doll.step(1.0 / 60.0, None);
        }
        assert!(!doll.asleep(), "a kicked body went back to sleep at once");
        let moved = doll.x.iter().zip(&before).fold(0.0f32, |m, (a, b)| m.max(a.distance(*b)));
        assert!(moved > 20.0, "moved only {moved}");
        // Slid along the floor or come to rest, its life (counted again from
        // the kick) ends it, as BO2's ragdoll_max_life does.
        for _ in 0..((MAX_LIFE_SECONDS + 1.0) * 60.0) as usize {
            doll.step(1.0 / 60.0, None);
        }
        assert!(doll.asleep(), "still moving: {:?}", doll.v);
    }

    /// BO2's barrel death (`start_explosive_ragdoll`: x, y 50-100 either
    /// way, z 10-20) throws the body's centre at 61 u/s per unit, and a
    /// gunshot death (no `launchragdoll`) adds nothing to its fall.
    #[test]
    fn a_barrel_death_throws_the_body_at_bo2s_speed() {
        let defs = parse(DEF);
        let (dobj, world) = figure();
        let centre = |doll: &Ragdoll| {
            let m: f32 = doll.rig.inv_mass.iter().map(|w| 1.0 / w).sum();
            doll.v.iter().zip(&doll.rig.inv_mass).map(|(v, w)| *v / *w).sum::<Vec3>() / m
        };
        let launch = Vec3::new(-75.0, 60.0, 15.0);
        let thrown = Ragdoll::start(&defs[0], &dobj, &world, Mat4::IDENTITY, None, launch, None).unwrap();
        let want = launch * 0.5 / 0.0082;
        assert!(centre(&thrown).distance(want) < 1.0, "{} vs {want}", centre(&thrown));
        assert!(want.length() > 5000.0);
        let shot = Ragdoll::start(&defs[0], &dobj, &world, Mat4::IDENTITY, None, Vec3::ZERO, None).unwrap();
        assert_eq!(centre(&shot), Vec3::ZERO);
        // Thrown later (`fling`), the same speed; the cap leaves it alone.
        let mut later = shot;
        later.fling(launch);
        later.step(1.0 / 90.0, None);
        assert!(centre(&later).distance(want).abs() < want.length() * 0.02, "{}", centre(&later));
    }
    /// Bodies dropped on Nuketown's own collision where two blew up in a
    /// hidden match (#3 spun up to 1400 u/s on the ground). Needs the zone:
    /// `BO2MP_RAGDOLL_ZONE=<path>/mp_nuketown_2020.ff cargo test -- --ignored`.
    #[test]
    #[ignore]
    fn bodies_on_nuketown_never_speed_up_on_their_own() {
        let Ok(path) = std::env::var("BO2MP_RAGDOLL_ZONE") else { return };
        let capture = asset_t6::capture_zone(std::path::Path::new(&path)).expect("zone");
        let solids = asset_world::build_t6_clip_collision(capture.clip.as_ref().expect("clipMap")).expect("collision");
        let defs = parse(DEF);
        let (dobj, world) = figure();
        let mut worst = (0.0f32, String::new());
        let mut worst_jump = (0.0f32, String::new());
        for at in [Vec3::new(1404.0, 223.0, -48.0), Vec3::new(-1002.0, 149.0, -50.0)] {
            for yaw in 0..8 {
                for (tilt, lift) in [(0.0f32, -14.0f32), (90.0, -2.0), (-90.0, -2.0)] {
                    let place = Mat4::from_translation(at + Vec3::Z * lift)
                        * Mat4::from_rotation_z(yaw as f32 * 45f32.to_radians())
                        * Mat4::from_rotation_x(tilt.to_radians());
                    let mut doll = Ragdoll::start(&defs[0], &dobj, &world, place, None, Vec3::X * 30.0, Some(&solids))
                        .expect("the figure has every joint");
                    // Once down (1.5 s), held over a quarter second: a wrist
                    // landing can pop for one step, a blown-up body keeps going.
                    let (mut top, mut last) = (0.0f32, [0.0f32; 15]);
                    // And the biggest one-step jump of any joint once down.
                    let mut jump = (0.0f32, 0usize);
                    for frame in 0..(10 * 60) {
                        doll.step(1.0 / 60.0, Some(&solids));
                        last[frame % 15] = doll.v.iter().fold(0.0f32, |t, v| t.max(v.length()));
                        if frame > 90 {
                            top = top.max(last.iter().sum::<f32>() / 15.0);
                            for (k, (a, b)) in doll.x_prev.iter().zip(&doll.x).enumerate() {
                                if a.distance(*b) > jump.0 {
                                    jump = (a.distance(*b), k);
                                }
                            }
                        }
                    }
                    if jump.0 > worst_jump.0 {
                        worst_jump = (jump.0, format!("joint {} at {at} yaw {} tilt {tilt}", jump.1, yaw * 45));
                    }
                    let case = format!(
                        "at {at} yaw {} tilt {tilt}: held {top:.0} u/s, jump {:.1} u (joint {}), root at {}",
                        yaw * 45,
                        jump.0,
                        jump.1,
                        doll.x[0]
                    );
                    println!("{case}");
                    assert!(doll.asleep, "a body never lay still: {case}");
                    if top > worst.0 {
                        worst = (top, case);
                    }
                }
            }
        }
        println!("biggest one-step jump {:.1} u: {}", worst_jump.0, worst_jump.1);
        assert!(worst.0 < 300.0, "a body sped up on its own: {}", worst.1);
        // An arm over the head flipped its elbow and threw the hand 19 u.
        assert!(worst_jump.0 < 4.0, "a joint jumped {:.1} u in one step: {}", worst_jump.0, worst_jump.1);
    }

}
