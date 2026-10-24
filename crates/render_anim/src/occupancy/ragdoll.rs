//! bo2zm (Die Rise chat): a limp body, Black Ops II's ragdoll. The server
//! marks a body limp (`startragdoll`) and may throw it (`launchragdoll`, the
//! Trample Steam's kills); each client then drops it from the pose it had,
//! under gravity. Sixteen points stand for the body (pelvis, chest, neck,
//! head and the arm and leg joints), held at their lengths, each swept
//! against the map's clip every 17 ms step. The body is stiff at first and
//! goes limp over 3 s; every bone of the model follows the part it hangs
//! from. A body lies still (its pose kept) once most of it has stopped for a
//! quarter second, after 9 s, or when it falls fast; at most 16 move at once.

use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};

use asset_world::ClipCollision;
use bevy::math::{Mat3, Mat4, Quat, Vec3};
use bevy::prelude::Resource;
use xmodel_runtime::{DObj, DObjRagdoll};

const GRAVITY_Z: f32 = -800.0;
const MSEC_STEP: i32 = 17;
/// Steps one frame may run; a longer frame skips the rest.
const MAX_STEPS: i32 = 4;
const CONTENTS_SOLID: u32 = 1;
const ITERATIONS: usize = 8;
const DRAG: f32 = 0.03;
const BOUNCE: f32 = 0.1;
const FRICTION: f32 = 0.3;
const CONTACT_SLOP: f32 = 0.125;
/// How hard a limb holds the angle it had when the body went limp, at
/// first; it lets go over `STIFF_MS`.
const HOLD: f32 = 0.35;
const STIFF_MS: f32 = 3000.0;
/// The death animation's own motion carries on: the speed each point had
/// (from the pose a moment later), at most 600 and rising at most 50.
pub(crate) const AHEAD_S: f32 = 0.05;
const ANIM_SPEED_MAX: f32 = 600.0;
const ANIM_RISE_MAX: f32 = 50.0;
/// Speed per unit of the server's throw, BO2's own: Ragdoll_Launch splits
/// ragdoll_bullet_force 0.5 x the vector over the bones as one kick each
/// (Phys_ObjAddForce, the solver's dv = F/m), and ragdoll.cfg's human body
/// weighs 8.2 x 0.001 (Ragdoll_Bone_f), so the body leaves at 0.5 / 0.0082
/// = 61 u/s per unit (the Thunder Gun's 100-200 -> 6100-12200 u/s). No cap;
/// the air (phys_dragLinear 0.03) barely slows it.
const LAUNCH_SPEED: f32 = 0.5 / 0.0082;
/// A random share of the throw on each point: the body tumbles.
const LAUNCH_TUMBLE: f32 = 0.2;
const MAX_LAUNCHES: u16 = 2;
const STILL_SPEED: f32 = 6.0;
const STILL_MS: i32 = 250;
const STILL_SHARE: f32 = 0.75;
const MAX_LIFE_MS: i32 = 9000;
const FALL_STILL_VZ: f32 = -2500.0;
const MAX_MOVING: usize = 16;
/// A body nobody drew for this long is gone (a corpse lasts 15 s).
const KEEP_UNSEEN_MS: i32 = 20_000;

/// bo2mc 10-14 (the Gravity Gun, `minecraft_world::gravgun_play`): a held
/// body's pelvis follows the beam's point this fast per unit away (a
/// second's pull), at most `BEAM_SPEED_MAX`; the limbs hang from it.
const BEAM_GAIN: f32 = 12.0;
const BEAM_SPEED_MAX: f32 = 1500.0;
/// His "the ragdolls interact with each other ... double ragdoll": a body
/// flying faster than this into another's pelvis (within `KNOCK_REACH`)
/// gives it `KNOCK_SHARE` of its speed and keeps the rest.
const KNOCK_SPEED: f32 = 250.0;
const KNOCK_REACH: f32 = 30.0;
const KNOCK_SHARE: f32 = 0.55;

const PELVIS: usize = 0;
const CHEST: usize = 1;
const NECK: usize = 2;
const HEAD: usize = 3;
const SHOULDER_L: usize = 4;
const ELBOW_L: usize = 5;
const WRIST_L: usize = 6;
const SHOULDER_R: usize = 7;
const ELBOW_R: usize = 8;
const WRIST_R: usize = 9;
const HIP_L: usize = 10;
const KNEE_L: usize = 11;
const ANKLE_L: usize = 12;
const HIP_R: usize = 13;
const KNEE_R: usize = 14;
const ANKLE_R: usize = 15;
const POINTS: usize = 16;

/// Each point's bone, how far it keeps off a wall and its weight.
const POINT_BONES: [(&str, f32, f32); POINTS] = [
    ("j_mainroot", 4.5, 1.2),
    ("j_spine4", 4.5, 1.2),
    ("j_neck", 3.0, 0.5),
    ("j_head", 3.5, 0.4),
    ("j_shoulder_le", 2.5, 0.4),
    ("j_elbow_le", 2.5, 0.35),
    ("j_wrist_le", 2.0, 0.2),
    ("j_shoulder_ri", 2.5, 0.4),
    ("j_elbow_ri", 2.5, 0.35),
    ("j_wrist_ri", 2.0, 0.2),
    ("j_hip_le", 3.0, 0.6),
    ("j_knee_le", 3.0, 0.5),
    ("j_ankle_le", 2.5, 0.3),
    ("j_hip_ri", 3.0, 0.6),
    ("j_knee_ri", 3.0, 0.5),
    ("j_ankle_ri", 2.5, 0.3),
];

/// The torso: one stiff piece (part 0, turning about the pelvis).
const TORSO: [usize; 7] = [PELVIS, CHEST, NECK, SHOULDER_L, SHOULDER_R, HIP_L, HIP_R];

/// The parts that swing, parts 1 to 9: (from point, to point, parent part),
/// parents first.
const LIMBS: [(usize, usize, usize); 9] = [
    (NECK, HEAD, 0),
    (SHOULDER_L, ELBOW_L, 0),
    (ELBOW_L, WRIST_L, 2),
    (SHOULDER_R, ELBOW_R, 0),
    (ELBOW_R, WRIST_R, 4),
    (HIP_L, KNEE_L, 0),
    (KNEE_L, ANKLE_L, 6),
    (HIP_R, KNEE_R, 0),
    (KNEE_R, ANKLE_R, 8),
];
const PARTS: usize = 10;

/// The bone each swinging part starts at: every bone under it moves with
/// it (the hand with the forearm, the foot with the shin, the head model
/// with the neck); the rest move with the torso.
fn part_of_bone(name: &str) -> Option<u8> {
    Some(match name {
        "j_neck" => 1,
        "j_shoulder_le" => 2,
        "j_elbow_le" => 3,
        "j_shoulder_ri" => 4,
        "j_elbow_ri" => 5,
        "j_hip_le" => 6,
        "j_knee_le" => 7,
        "j_hip_ri" => 8,
        "j_knee_ri" => 9,
        _ => return None,
    })
}

fn pivot(part: usize) -> usize {
    match part {
        0 => PELVIS,
        limb => LIMBS[limb - 1].0,
    }
}

/// One limp body's pose this frame.
pub(crate) struct RagdollPose {
    /// Each bone's matrix in the model's own space (as `pose_dobj` gives it).
    pub bones: Vec<Mat4>,
    /// How far the pelvis has moved since the body went limp.
    pub shift: Vec3,
    /// Changes whenever the pose does.
    pub stamp: u32,
}

/// A limp body as the Gravity Gun sees it: its pelvis and speed (map
/// units), and whether it lies still.
#[derive(Clone, Copy, Debug)]
pub(crate) struct BodySeen {
    pub at: Vec3,
    pub speed: Vec3,
    pub lying: bool,
}

#[derive(Default)]
struct Beam {
    /// Where each held body's pelvis is pulled to.
    hold: HashMap<u32, Vec3>,
    /// A throw waiting for its body: the speed every point takes.
    throw: HashMap<u32, Vec3>,
    seen: HashMap<u32, BodySeen>,
}

static BEAM: LazyLock<Mutex<Beam>> = LazyLock::new(Default::default);

fn beam() -> std::sync::MutexGuard<'static, Beam> {
    BEAM.lock().unwrap_or_else(|e| e.into_inner())
}

/// The Gravity Gun holds body `id` at map point `at`, or lets go (`None`).
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
    beam().seen.clone()
}

#[derive(Resource, Default)]
pub(crate) struct Ragdolls {
    bodies: HashMap<u32, Slot>,
}

enum Slot {
    Body(Box<Body>),
    /// Not this skeleton (a dog), or its pelvis started in a wall: it keeps
    /// its animation.
    Refused { start_ms: i32, seen_ms: i32 },
}

struct Link {
    a: usize,
    b: usize,
    min: f32,
    max: f32,
}

struct Body {
    /// The server's mark: a new one is a new fall.
    start_ms: i32,
    /// Each bone's part.
    parts_of_bones: Vec<u8>,
    /// Each bone's world matrix when the body went limp.
    start_world: Vec<Mat4>,
    /// The points then.
    rest: [Vec3; POINTS],
    torso0: Quat,
    /// The way the body faced, in the torso's own frame: knees bend this way.
    forward_local: Vec3,
    inv_mass: [f32; POINTS],
    links: Vec<Link>,
    x: [Vec3; POINTS],
    v: [Vec3; POINTS],
    still_ms: [i32; POINTS],
    born_ms: i32,
    clock_ms: i32,
    seen_ms: i32,
    /// The server's throw count last seen, and the throws taken (at most 2).
    launches: u16,
    thrown: u16,
    steps: u32,
    lying: Option<&'static str>,
    seed: u32,
    /// bo2mc: the Gravity Gun's pull point while it holds the body, and
    /// when it last held or threw it (its 9 s to lie still count from then).
    held: Option<Vec3>,
    woke_ms: i32,
}

impl Ragdolls {
    /// How far a limp body's pelvis has moved (where to cull and light it).
    pub(crate) fn shift(&self, id: u32) -> Option<Vec3> {
        match self.bodies.get(&id)? {
            Slot::Body(body) => Some(body.x[PELVIS] - body.rest[PELVIS]),
            Slot::Refused { .. } => None,
        }
    }

    /// The body's pose now, run on to `now_ms`. `start` gives the pose
    /// when it goes limp, in the model's space: now and `AHEAD_S` later.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn pose(
        &mut self,
        id: u32,
        ragdoll: &DObjRagdoll,
        dobj: &DObj,
        world_from_local: Mat4,
        now_ms: i32,
        clip: Option<&ClipCollision>,
        start: impl FnOnce() -> Option<(Vec<Mat4>, Vec<Mat4>)>,
    ) -> Option<RagdollPose> {
        let fresh = match self.bodies.get(&id) {
            Some(Slot::Body(body)) => {
                body.start_ms != ragdoll.start_ms || body.parts_of_bones.len() != dobj.bones.len()
            }
            Some(Slot::Refused { start_ms, .. }) => *start_ms != ragdoll.start_ms,
            None => true,
        };
        if fresh {
            let made = start().ok_or("no pose").and_then(|(now, ahead)| {
                Body::start(id, ragdoll, dobj, world_from_local, &now, &ahead, clip, now_ms)
            });
            match made {
                Ok(body) => {
                    self.make_room(id);
                    diag::info!(
                        World,
                        "ragdoll start #{id}: thrown {} x{} at {}",
                        show(Vec3::from_array(ragdoll.launch)),
                        ragdoll.launches,
                        show(body.x[PELVIS])
                    );
                    self.bodies.insert(id, Slot::Body(Box::new(body)));
                }
                Err(why) => {
                    diag::info!(World, "ragdoll refused #{id}: {why}");
                    self.bodies.insert(
                        id,
                        Slot::Refused {
                            start_ms: ragdoll.start_ms,
                            seen_ms: now_ms,
                        },
                    );
                }
            }
        }
        let (held, thrown) = {
            let mut b = beam();
            (b.hold.get(&id).copied(), b.throw.remove(&id))
        };
        let pose = match self.bodies.get_mut(&id)? {
            Slot::Body(body) => {
                body.seen_ms = now_ms;
                body.throw(ragdoll);
                body.beam(held, thrown, now_ms);
                body.run(id, now_ms, clip);
                Some(body.pose(world_from_local))
            }
            Slot::Refused { seen_ms, .. } => {
                *seen_ms = now_ms;
                return None;
            }
        };
        self.knock(id);
        if let Some(Slot::Body(body)) = self.bodies.get(&id) {
            let seen = BodySeen { at: body.x[PELVIS], speed: body.v[PELVIS], lying: body.lying.is_some() };
            beam().seen.insert(id, seen);
        }
        pose
    }

    /// bo2mc (his "double ragdoll"): body `id` flying fast into another
    /// knocks it on.
    fn knock(&mut self, id: u32) {
        let Some(Slot::Body(body)) = self.bodies.get(&id) else {
            return;
        };
        let (at, speed) = (body.x[PELVIS], body.v[PELVIS]);
        if speed.length() < KNOCK_SPEED || body.held.is_some() {
            return;
        }
        let mut struck = 0;
        for (other, slot) in self.bodies.iter_mut() {
            let Slot::Body(b) = slot else { continue };
            if *other == id || b.held.is_some() || b.x[PELVIS].distance(at) > KNOCK_REACH {
                continue;
            }
            // Only one going slower than the thrower takes the knock.
            if b.v[PELVIS].length() >= speed.length() * KNOCK_SHARE {
                continue;
            }
            for k in 0..POINTS {
                b.v[k] += speed * KNOCK_SHARE;
                b.still_ms[k] = 0;
            }
            b.lying = None;
            b.woke_ms = b.clock_ms;
            struck += 1;
            diag::info!(World, "gravity gun: body #{id} knocks #{other} on at {:.0} u/s", speed.length());
        }
        if struck > 0
            && let Some(Slot::Body(body)) = self.bodies.get_mut(&id)
        {
            for v in &mut body.v {
                *v *= 1.0 - KNOCK_SHARE;
            }
        }
    }

    /// At most `MAX_MOVING` bodies move: the oldest lies still for a new one.
    fn make_room(&mut self, id: u32) {
        loop {
            let moving = self.bodies.iter_mut().filter_map(|(other, slot)| match slot {
                Slot::Body(body) if *other != id && body.lying.is_none() => Some(body),
                _ => None,
            });
            let (count, oldest) = moving.fold((0usize, None::<&mut Box<Body>>), |(n, old), b| {
                let older = match &old {
                    Some(o) => b.born_ms < o.born_ms,
                    None => true,
                };
                (n + 1, if older { Some(b) } else { old })
            });
            match oldest {
                Some(body) if count >= MAX_MOVING => body.lying = Some("crowd"),
                _ => return,
            }
        }
    }

    /// Bodies nobody drew for a while are gone.
    pub(crate) fn forget_unseen(&mut self, now_ms: i32) {
        self.bodies.retain(|_, slot| {
            let seen = match slot {
                Slot::Body(body) => body.seen_ms,
                Slot::Refused { seen_ms, .. } => *seen_ms,
            };
            seen <= now_ms && now_ms - seen <= KEEP_UNSEEN_MS
        });
    }
}

impl Body {
    #[allow(clippy::too_many_arguments)]
    fn start(
        id: u32,
        ragdoll: &DObjRagdoll,
        dobj: &DObj,
        world_from_local: Mat4,
        now: &[Mat4],
        ahead: &[Mat4],
        clip: Option<&ClipCollision>,
        now_ms: i32,
    ) -> Result<Body, &'static str> {
        if now.len() != dobj.bones.len() || ahead.len() != now.len() {
            return Err("pose does not fit the model");
        }
        let start_world: Vec<Mat4> = now.iter().map(|m| world_from_local * *m).collect();
        let find = |name: &str| dobj.bones.iter().position(|b| b.name == name);
        let mut x = [Vec3::ZERO; POINTS];
        let mut v = [Vec3::ZERO; POINTS];
        for (k, (name, _, _)) in POINT_BONES.iter().enumerate() {
            let Some(bone) = find(name) else {
                if k == HEAD {
                    continue;
                }
                return Err("not a body's skeleton");
            };
            x[k] = start_world[bone].w_axis.truncate();
            let later = (world_from_local * ahead[bone]).w_axis.truncate();
            let mut speed = ((later - x[k]) / AHEAD_S).clamp_length_max(ANIM_SPEED_MAX);
            speed.z = speed.z.min(ANIM_RISE_MAX);
            v[k] = speed;
        }
        if find("j_head").is_none() {
            x[HEAD] = x[NECK] + (x[NECK] - x[CHEST]).normalize_or(Vec3::Z) * 6.0;
            v[HEAD] = v[NECK];
        }
        if !x.iter().chain(&v).all(|p| p.is_finite()) {
            return Err("pose not finite");
        }
        let rest = x;
        let torso0 = torso_frame(&rest).ok_or("torso folded flat")?;
        let mut links = Vec::new();
        for (i, &a) in TORSO.iter().enumerate() {
            for &b in &TORSO[i + 1..] {
                let d = rest[a].distance(rest[b]);
                links.push(Link { a, b, min: d, max: d });
            }
        }
        for &(a, b, _) in &LIMBS {
            let d = rest[a].distance(rest[b]);
            links.push(Link { a, b, min: d, max: d });
        }
        // Elbows and knees fold only so far; the head nods only so far.
        for (s, e, w, share) in [
            (SHOULDER_L, ELBOW_L, WRIST_L, 0.5),
            (SHOULDER_R, ELBOW_R, WRIST_R, 0.5),
            (HIP_L, KNEE_L, ANKLE_L, 0.4),
            (HIP_R, KNEE_R, ANKLE_R, 0.4),
        ] {
            let reach = rest[s].distance(rest[e]) + rest[e].distance(rest[w]);
            links.push(Link {
                a: s,
                b: w,
                min: reach * share,
                max: reach,
            });
        }
        let reach = rest[CHEST].distance(rest[NECK]) + rest[NECK].distance(rest[HEAD]);
        links.push(Link {
            a: CHEST,
            b: HEAD,
            min: rest[CHEST].distance(rest[HEAD]).min(reach) * 0.85,
            max: reach,
        });
        // A joint through a wall from the pelvis comes back inside it; a
        // pelvis in a wall keeps the animation.
        if let Some(clip) = clip {
            let pelvis = x[PELVIS];
            let up = (pelvis + Vec3::Z).to_array();
            if clip
                .sweep_box(pelvis.to_array(), up, [0.0; 3], [0.0; 3], CONTENTS_SOLID)
                .startsolid
            {
                return Err("pelvis in a wall");
            }
            for point in x.iter_mut().skip(1) {
                let hit = clip.sweep_box(
                    pelvis.to_array(),
                    point.to_array(),
                    [0.0; 3],
                    [0.0; 3],
                    CONTENTS_SOLID,
                );
                if hit.fraction < 1.0 {
                    *point = pelvis + (*point - pelvis) * hit.fraction * 0.8;
                }
            }
        }
        let parts_of_bones = dobj
            .bones
            .iter()
            .enumerate()
            .map(|(i, _)| {
                let mut at = Some(i);
                while let Some(bone) = at {
                    if let Some(part) = part_of_bone(&dobj.bones[bone].name) {
                        return part;
                    }
                    at = dobj.bones[bone].parent;
                }
                0
            })
            .collect();
        let facing = world_from_local
            .x_axis
            .truncate()
            .normalize_or(Vec3::X);
        Ok(Body {
            start_ms: ragdoll.start_ms,
            parts_of_bones,
            start_world,
            rest,
            torso0,
            forward_local: torso0.inverse() * facing,
            inv_mass: POINT_BONES.map(|(_, _, mass)| 1.0 / mass),
            links,
            x,
            v,
            still_ms: [0; POINTS],
            born_ms: now_ms,
            clock_ms: now_ms,
            seen_ms: now_ms,
            launches: 0,
            thrown: 0,
            steps: 0,
            lying: None,
            seed: id.wrapping_mul(0x9E37_79B9) ^ (ragdoll.start_ms as u32) | 1,
            held: None,
            woke_ms: now_ms,
        })
    }

    fn random(&mut self) -> f32 {
        self.seed ^= self.seed << 13;
        self.seed ^= self.seed >> 17;
        self.seed ^= self.seed << 5;
        self.seed as f32 / u32::MAX as f32 * 2.0 - 1.0
    }

    /// The server's new throws (two at most per body): the same push on
    /// every point, so the light hands and head fly off faster, and a
    /// random share each, so it tumbles. A body lying still moves again.
    fn throw(&mut self, ragdoll: &DObjRagdoll) {
        let new = ragdoll.launches.wrapping_sub(self.launches);
        self.launches = ragdoll.launches;
        let take = new.min(MAX_LAUNCHES - self.thrown);
        if take == 0 {
            return;
        }
        self.thrown += take;
        let speed = Vec3::from_array(ragdoll.launch) * LAUNCH_SPEED;
        let weight: f32 = self.inv_mass.iter().map(|inv| 1.0 / inv).sum();
        let mean = weight / POINTS as f32;
        for _ in 0..take {
            for k in 0..POINTS {
                let tumble = Vec3::new(self.random(), self.random(), self.random());
                self.v[k] += speed * (mean * self.inv_mass[k])
                    + tumble * (speed.length() * LAUNCH_TUMBLE);
                self.still_ms[k] = 0;
            }
        }
        self.lying = None;
    }

    /// bo2mc: the Gravity Gun holds the body at `held` (map units) or lets
    /// go, and its throw: every point at `thrown` (units a second), plus a
    /// tumble.
    fn beam(&mut self, held: Option<Vec3>, thrown: Option<Vec3>, now_ms: i32) {
        if held.is_some() || thrown.is_some() {
            self.lying = None;
            self.woke_ms = now_ms;
            self.still_ms = [0; POINTS];
        }
        self.held = held;
        if let Some(speed) = thrown {
            for k in 0..POINTS {
                let tumble = Vec3::new(self.random(), self.random(), self.random());
                self.v[k] = speed + tumble * (speed.length() * LAUNCH_TUMBLE);
            }
        }
    }

    fn run(&mut self, id: u32, now_ms: i32, clip: Option<&ClipCollision>) {
        if self.lying.is_some() {
            self.clock_ms = now_ms;
            return;
        }
        let mut steps = 0;
        while self.clock_ms + MSEC_STEP <= now_ms {
            if steps == MAX_STEPS {
                self.clock_ms = now_ms;
                break;
            }
            steps += 1;
            self.clock_ms += MSEC_STEP;
            self.step(clip);
            // IW4L_RAGLOG=1: the pelvis every 0.1 s (where a throw goes).
            static RAGLOG: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
            if *RAGLOG.get_or_init(|| std::env::var_os("IW4L_RAGLOG").is_some())
                && self.steps % 6 == 0
            {
                diag::info!(
                    World,
                    "ragdoll fly #{id} {} ms: pelvis {} speed {}",
                    self.clock_ms - self.born_ms,
                    show(self.x[PELVIS]),
                    show(self.v[PELVIS])
                );
            }
            if self.held.is_some() {
                self.woke_ms = self.clock_ms;
                continue;
            }
            if let Some(why) = self.settled() {
                self.lying = Some(why);
                diag::info!(
                    World,
                    "ragdoll still #{id} ({why}) after {} ms at {}, moved {:.0}",
                    self.clock_ms - self.born_ms,
                    show(self.x[PELVIS]),
                    self.x[PELVIS].distance(self.rest[PELVIS])
                );
                break;
            }
        }
    }

    fn step(&mut self, clip: Option<&ClipCollision>) {
        let dt = MSEC_STEP as f32 / 1000.0;
        let age = (self.clock_ms - self.born_ms) as f32;
        let loose = (1.0 - age / STIFF_MS).max(0.0);
        let hold = HOLD * loose * loose;
        let mut p = self.x;
        if let Some(at) = self.held {
            // The beam carries the pelvis; the rest hangs from it.
            let pull = ((at - self.x[PELVIS]) * BEAM_GAIN).clamp_length_max(BEAM_SPEED_MAX);
            for k in 0..POINTS {
                let w = if k == PELVIS { 1.0 } else { 0.35 };
                self.v[k] = self.v[k].lerp(pull, w);
            }
            self.v[PELVIS].z -= GRAVITY_Z * dt;
        }
        for k in 0..POINTS {
            self.v[k].z += GRAVITY_Z * dt;
            self.v[k] *= 1.0 - DRAG * dt;
            p[k] = self.x[k] + self.v[k] * dt;
        }
        let parts = self.parts(&p);
        if hold > 0.0 {
            for &(a, b, parent) in &LIMBS {
                let held = p[a] + parts[parent] * (self.rest[b] - self.rest[a]);
                p[b] += (held - p[b]) * hold;
            }
        }
        let forward = parts[0] * self.forward_local;
        for _ in 0..ITERATIONS {
            for link in &self.links {
                let delta = p[link.b] - p[link.a];
                let d = delta.length();
                if d < 1e-4 {
                    continue;
                }
                let want = d.clamp(link.min, link.max);
                if want == d {
                    continue;
                }
                let (wa, wb) = (self.inv_mass[link.a], self.inv_mass[link.b]);
                let fix = delta * ((d - want) / (d * (wa + wb)));
                p[link.a] += fix * wa;
                p[link.b] -= fix * wb;
            }
            // Knees bend forward only.
            for (hip, knee, ankle) in [(HIP_L, KNEE_L, ANKLE_L), (HIP_R, KNEE_R, ANKLE_R)] {
                let axis = p[ankle] - p[hip];
                let len2 = axis.length_squared();
                if len2 < 1e-3 {
                    continue;
                }
                let on = p[hip] + axis * ((p[knee] - p[hip]).dot(axis) / len2);
                let back = (p[knee] - on).dot(forward);
                if back < 0.0 {
                    p[knee] -= forward * back;
                }
            }
        }
        let mut touch = [None; POINTS];
        if let Some(clip) = clip {
            for k in 0..POINTS {
                let r = POINT_BONES[k].1;
                let (from, to) = (self.x[k].to_array(), p[k].to_array());
                let mut hit = clip.sweep_box(from, to, [-r; 3], [r; 3], CONTENTS_SOLID);
                if hit.startsolid {
                    hit = clip.sweep_box(from, to, [0.0; 3], [0.0; 3], CONTENTS_SOLID);
                    if hit.startsolid {
                        continue;
                    }
                }
                if hit.fraction < 1.0 {
                    let n = Vec3::from_array(hit.normal);
                    let end = Vec3::from_array(hit.endpos) + n * CONTACT_SLOP;
                    // bo2zm 10-13: the rest of the move slides along what
                    // it hit (a body thrown into a wall drops down it; it
                    // stuck there in mid-air when the move stopped dead).
                    let left = p[k] - end;
                    let slid = end + left - n * left.dot(n).min(0.0);
                    let again = clip.sweep_box(
                        end.to_array(),
                        slid.to_array(),
                        [-r; 3],
                        [r; 3],
                        CONTENTS_SOLID,
                    );
                    p[k] = if again.startsolid {
                        end
                    } else if again.fraction < 1.0 {
                        Vec3::from_array(again.endpos)
                            + Vec3::from_array(again.normal) * CONTACT_SLOP
                    } else {
                        slid
                    };
                    touch[k] = Some(n);
                }
            }
        }
        for k in 0..POINTS {
            let mut vel = (p[k] - self.x[k]) / dt;
            if let Some(n) = touch[k] {
                let vn = vel.dot(n);
                if vn < 0.0 {
                    vel -= n * ((1.0 + BOUNCE) * vn);
                }
                // A floor holds a body by its weight; a wall barely
                // (it slides down it).
                let along = vel - n * vel.dot(n);
                vel -= along * (FRICTION * n.z.clamp(0.1, 1.0));
            }
            if !vel.is_finite() || !p[k].is_finite() {
                vel = Vec3::ZERO;
                p[k] = self.x[k];
            }
            self.v[k] = vel;
            self.x[k] = p[k];
            self.still_ms[k] = if vel.length() < STILL_SPEED {
                self.still_ms[k] + MSEC_STEP
            } else {
                0
            };
        }
        self.steps = self.steps.wrapping_add(1);
    }

    fn settled(&self) -> Option<&'static str> {
        let still = self.still_ms.iter().filter(|ms| **ms >= STILL_MS).count();
        if still as f32 > POINTS as f32 * STILL_SHARE {
            return Some("still");
        }
        if self.clock_ms - self.born_ms.max(self.woke_ms) >= MAX_LIFE_MS {
            return Some("9 s");
        }
        let fall = self.v.iter().map(|v| v.z).sum::<f32>() / POINTS as f32;
        if fall < FALL_STILL_VZ {
            return Some("falling");
        }
        None
    }

    /// Each part's turn since the body went limp: the torso from its frame,
    /// each limb carried by its parent then swung to where it points now.
    fn parts(&self, p: &[Vec3; POINTS]) -> [Quat; PARTS] {
        let mut turn = [Quat::IDENTITY; PARTS];
        if let Some(now) = torso_frame(p) {
            turn[0] = (now * self.torso0.inverse()).normalize();
        }
        for (i, &(a, b, parent)) in LIMBS.iter().enumerate() {
            let carried = turn[parent] * (self.rest[b] - self.rest[a]);
            turn[i + 1] = match (carried.try_normalize(), (p[b] - p[a]).try_normalize()) {
                (Some(from), Some(to)) => (Quat::from_rotation_arc(from, to) * turn[parent]).normalize(),
                _ => turn[parent],
            };
        }
        turn
    }

    fn pose(&self, world_from_local: Mat4) -> RagdollPose {
        let turn = self.parts(&self.x);
        let moves: [Mat4; PARTS] = std::array::from_fn(|part| {
            let at = pivot(part);
            Mat4::from_translation(self.x[at])
                * Mat4::from_quat(turn[part])
                * Mat4::from_translation(-self.rest[at])
        });
        let local_from_world = world_from_local.inverse();
        let bones = self
            .start_world
            .iter()
            .zip(&self.parts_of_bones)
            .map(|(world, part)| local_from_world * moves[*part as usize] * *world)
            .collect();
        RagdollPose {
            bones,
            shift: self.x[PELVIS] - self.rest[PELVIS],
            stamp: self.steps,
        }
    }
}

/// The torso's frame: up the spine, across the shoulders and hips.
fn torso_frame(p: &[Vec3; POINTS]) -> Option<Quat> {
    let up = (p[NECK] - p[PELVIS]).try_normalize()?;
    let across = p[SHOULDER_L] - p[SHOULDER_R] + p[HIP_L] - p[HIP_R];
    let side = (across - up * across.dot(up)).try_normalize()?;
    let forward = side.cross(up);
    Some(Quat::from_mat3(&Mat3::from_cols(forward, side, up)).normalize())
}

fn show(p: Vec3) -> String {
    format!("({:.0} {:.0} {:.0})", p.x, p.y, p.z)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A standing body's points, facing +X, 1 unit = 1 inch.
    fn standing() -> [Vec3; POINTS] {
        let mut x = [Vec3::ZERO; POINTS];
        x[PELVIS] = Vec3::new(0.0, 0.0, 37.0);
        x[CHEST] = Vec3::new(0.0, 0.0, 52.0);
        x[NECK] = Vec3::new(0.0, 0.0, 60.0);
        x[HEAD] = Vec3::new(1.0, 0.0, 66.0);
        x[SHOULDER_L] = Vec3::new(0.0, 7.0, 57.0);
        x[ELBOW_L] = Vec3::new(0.0, 8.0, 46.0);
        x[WRIST_L] = Vec3::new(1.0, 8.0, 36.0);
        x[SHOULDER_R] = Vec3::new(0.0, -7.0, 57.0);
        x[ELBOW_R] = Vec3::new(0.0, -8.0, 46.0);
        x[WRIST_R] = Vec3::new(1.0, -8.0, 36.0);
        x[HIP_L] = Vec3::new(0.0, 5.0, 37.0);
        x[KNEE_L] = Vec3::new(0.5, 5.0, 20.0);
        x[ANKLE_L] = Vec3::new(0.0, 5.0, 5.0);
        x[HIP_R] = Vec3::new(0.0, -5.0, 37.0);
        x[KNEE_R] = Vec3::new(0.5, -5.0, 20.0);
        x[ANKLE_R] = Vec3::new(0.0, -5.0, 5.0);
        x
    }

    fn body(x: [Vec3; POINTS]) -> Body {
        let torso0 = torso_frame(&x).unwrap();
        let mut links = Vec::new();
        for (i, &a) in TORSO.iter().enumerate() {
            for &b in &TORSO[i + 1..] {
                let d = x[a].distance(x[b]);
                links.push(Link { a, b, min: d, max: d });
            }
        }
        for &(a, b, _) in &LIMBS {
            let d = x[a].distance(x[b]);
            links.push(Link { a, b, min: d, max: d });
        }
        Body {
            start_ms: 0,
            parts_of_bones: Vec::new(),
            start_world: Vec::new(),
            rest: x,
            torso0,
            forward_local: torso0.inverse() * Vec3::X,
            inv_mass: POINT_BONES.map(|(_, _, mass)| 1.0 / mass),
            links,
            x,
            v: [Vec3::ZERO; POINTS],
            still_ms: [0; POINTS],
            born_ms: 0,
            clock_ms: 0,
            seen_ms: 0,
            launches: 0,
            thrown: 0,
            steps: 0,
            lying: None,
            seed: 7,
            held: None,
            woke_ms: 0,
        }
    }

    #[test]
    fn a_throw_moves_the_body_at_four_times_the_vector() {
        let mut b = body(standing());
        b.throw(&DObjRagdoll {
            start_ms: 0,
            launch: [128.0, 0.0, 221.7],
            launches: 1,
        });
        let weight: f32 = b.inv_mass.iter().map(|inv| 1.0 / inv).sum();
        let centre = (0..POINTS)
            .map(|k| b.v[k] / b.inv_mass[k])
            .sum::<Vec3>()
            / weight;
        let want = Vec3::new(512.0, 0.0, 886.8);
        assert!(centre.distance(want) < want.length() * 0.15, "{centre}");
        // A third throw is not taken.
        b.throw(&DObjRagdoll {
            start_ms: 0,
            launch: [128.0, 0.0, 221.7],
            launches: 3,
        });
        assert_eq!(b.thrown, 2);
    }

    #[test]
    fn a_free_fall_keeps_its_lengths_and_falls_under_gravity() {
        let mut b = body(standing());
        for _ in 0..30 {
            b.clock_ms += MSEC_STEP;
            b.step(None);
        }
        let t = 30.0 * MSEC_STEP as f32 / 1000.0;
        let drop = b.rest[PELVIS].z - b.x[PELVIS].z;
        let want = 0.5 * 800.0 * t * t;
        assert!((drop - want).abs() < want * 0.1, "{drop} vs {want}");
        for link in &b.links {
            let d = b.x[link.a].distance(b.x[link.b]);
            assert!(d >= link.min - 0.2 && d <= link.max + 0.2);
        }
    }

    #[test]
    fn the_parts_follow_the_points() {
        let x = standing();
        let b = body(x);
        let turn = b.parts(&x);
        for q in turn {
            assert!(q.angle_between(Quat::IDENTITY) < 1e-3);
        }
        // The whole body turned a quarter about Z: every part turns with it.
        let spin = Quat::from_rotation_z(std::f32::consts::FRAC_PI_2);
        let moved = x.map(|p| spin * p);
        for q in b.parts(&moved) {
            assert!(q.angle_between(spin) < 1e-3);
        }
    }

    #[test]
    fn bones_map_to_their_parts() {
        assert_eq!(part_of_bone("j_knee_ri"), Some(9));
        assert_eq!(part_of_bone("j_spine4"), None);
    }
}
