//! bo2mp: Black Ops II multiplayer's spawn system, the engine half of
//! `_spawning.gsc` (BO2's SpawnSystem_* functions, the `spawnsystem_*` dvars
//! at their defaults). The scripts add influencers (each player's enemy and
//! friend spheres, a grenade's, a dead friend's, a chopper's or a guided
//! missile's cylinder, artillery's column) and give each team its spawn
//! points; `getsortedspawnpoints` weighs every point the team may use, sorts
//! them best first, then walks the best twenty and takes the first that
//! would telefrag nobody and that no enemy sees (from 65 units up, within
//! 2600 units). Defaults kept: no score culling, no spawning on another
//! team's points, enemy influencers stack, friend ones do not (the strongest
//! counts) and enemy-spawned ones do not (the most negative counts).
//!
//! Where ours differs, for what it cannot see: sight and the weapon
//! cylinder's push-through trace hit the map as a bullet sees it
//! (`MASK_SHOT`), not BO2's own sight mask; a spawn box telefrags players
//! only (BO2 also counts vehicles in it); `addboxinfluencer` stays a stub
//! (no multiplayer script adds one); sorts are stable where BO2's are not.

use bevy_ecs::prelude::World;
use gsc_t6::{ObjRef, Value, Vm};

use super::killstreaks::Streaks;
use super::objectives::team_number;
use super::{Zm, arg, entnum, flag, frame, int, num, text, vec3};
use crate::frame::FrameWorld;
use crate::world::ClientId;

/// Influencer types (`level.spawnsystem.einfluencer_type_*`): normal 0,
/// player 1, weapon 2, dog 3, vehicle 4, squad 5, game mode 6, enemy
/// spawned 7.
const TYPE_PLAYER: i32 = 1;
const TYPE_WEAPON: i32 = 2;
const TYPE_VEHICLE: i32 = 4;
const TYPE_GAME_MODE: i32 = 6;
const TYPE_ENEMY_SPAWNED: i32 = 7;
const TYPES: usize = 8;
/// Score curves (`einfluencer_curve_*`); 0 is constant.
const CURVE_LINEAR: i32 = 1;
const CURVE_STEEP: i32 = 2;
const CURVE_INVERSE_LINEAR: i32 = 3;
const CURVE_NEG_TO_POS: i32 = 4;
const MAX_INFLUENCERS: usize = 272;
const MAX_POINTS: usize = 199;
/// The teams a point is weighed for: free, allies, axis, team3..team8.
const TEAMS: usize = 9;
/// `spawnsystem_sight_check_max_distance`, `spawnsystem_sight_height_offset`.
const SIGHT_RANGE: f32 = 2600.0;
const SIGHT_HEIGHT: f32 = 65.0;
/// A hurt player's own good influencers rest this long.
const DAMAGE_REST_MS: i64 = 5000;
/// `spawnsystem_weapon_influencer_update_interval`, `_push_through`,
/// `_min_length`.
const WEAPON_UPDATE_MS: i64 = 151;
const WEAPON_PUSH_THROUGH: f32 = 50.0;
const WEAPON_MIN_LENGTH: f32 = 0.1;
/// The best points the pick walks.
const WALK: usize = 20;
/// A score below this makes a dangerous influencer: a point it covers is
/// never picked while a safe one is left, and its score counts a thousandth.
const DANGEROUS_SCORE: f32 = -10000.0;
/// The body a spawn fills (a standing player's box).
const SPAWN_MINS: [f32; 3] = [-15.0, -15.0, 0.0];
const SPAWN_MAXS: [f32; 3] = [15.0, 15.0, 70.0];
/// BO2's "player is alive" for a telefrag: below the dead movement types.
const PM_DEAD: i32 = playerstate_t6::game::PM_TYPE_DEAD;

/// What an influencer rides (BO2 allows these entity types only).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Rider {
    Player,
    Actor,
    Vehicle,
    Helicopter,
    ScriptMover,
    Missile,
}

#[derive(Clone, Copy, Debug)]
struct Ride {
    num: u32,
    obj: ObjRef,
    kind: Rider,
}

#[derive(Clone, Debug, Default)]
struct Influencer {
    used: bool,
    active: bool,
    kind: i32,
    cylinder: bool,
    radius: f32,
    /// The length it was made with (a vehicle's and a weapon's grow from it).
    preset_length: f32,
    score: f32,
    curve: i32,
    dangerous: bool,
    origin: [f32; 3],
    up: [f32; 3],
    /// Its own turn (from the forward and up it was made with); a rider's
    /// angles turn it further.
    quat: [f32; 4],
    length: f32,
    mid: [f32; 3],
    bounding_radius: f32,
    /// Its reach along x: the points between count it.
    bounds: [f32; 2],
    team_mask: i32,
    expire: i64,
    disable: i64,
    ride: Option<Ride>,
    weapon_update: i64,
}

#[derive(Clone, Debug)]
struct Point {
    num: u32,
    obj: ObjRef,
    origin: [f32; 3],
    yaw: f32,
    team_mask: i32,
    weight: [f32; TEAMS],
    base: [f32; TEAMS],
    dangerous: bool,
    /// The second spot an enemy must not see (its `target`), else zero.
    los: [f32; 3],
}

#[derive(Default)]
pub(crate) struct SpawnSystem {
    infl: Vec<Influencer>,
    /// Influencers in use, by their reach's low end.
    sorted: Vec<usize>,
    last_update: Option<i64>,
    points: Vec<Point>,
}

/// A rider as it is now.
struct Live {
    origin: [f32; 3],
    angles: [f32; 3],
    view_height: f32,
    /// A player flying free (noclip, ufo): his influencers stay put.
    frozen: bool,
    /// A craft's speed now (units a second).
    speed: f32,
}

/// What a new influencer is.
struct New {
    kind: i32,
    cylinder: bool,
    origin: [f32; 3],
    forward: [f32; 3],
    up: [f32; 3],
    radius: f32,
    length: f32,
    score: f32,
    mask: i32,
}

fn with<R>(world: &mut World, f: impl FnOnce(&mut SpawnSystem, &mut World) -> R) -> R {
    let mut s = std::mem::take(&mut world.resource_mut::<Zm>().spawns);
    let r = f(&mut s, world);
    world.resource_mut::<Zm>().spawns = s;
    r
}

// ---- natives -------------------------------------------------------------

/// `addsphereinfluencer(type, origin, radius, score, team mask, name,
/// [curve], [timeout seconds], [entity])`: its slot, or nothing when the
/// entity is not a kind an influencer rides.
pub(super) fn add_sphere(vm: &mut Vm<World>, world: &mut World, a: &[Value]) -> Result<Value, String> {
    let kind = kind(a, 7)?;
    let new = New {
        kind,
        cylinder: false,
        origin: vec3(a, 1)?,
        forward: [0.0; 3],
        up: [0.0; 3],
        radius: num(a, 2)?,
        length: 0.0,
        score: num(a, 3)?,
        mask: int(a, 4)?,
    };
    add(vm, world, a, 6, new)
}

/// `addcylinderinfluencer(type, origin, forward, up, radius, length, score,
/// team mask, name, [curve], [timeout seconds], [entity])`.
pub(super) fn add_cylinder(vm: &mut Vm<World>, world: &mut World, a: &[Value]) -> Result<Value, String> {
    let kind = kind(a, 6)?;
    let new = New {
        kind,
        cylinder: true,
        origin: vec3(a, 1)?,
        forward: vec3(a, 2)?,
        up: vec3(a, 3)?,
        radius: num(a, 4)?,
        length: num(a, 5)?,
        score: num(a, 6)?,
        mask: int(a, 7)?,
    };
    add(vm, world, a, 9, new)
}

fn kind(a: &[Value], max: i32) -> Result<i32, String> {
    let k = int(a, 0)?;
    if (0..=max).contains(&k) { Ok(k) } else { Err(format!("influencer type {k} is not 0 to {max}")) }
}

/// The optional curve, timeout and entity from argument `at` on.
fn add(vm: &mut Vm<World>, world: &mut World, a: &[Value], at: usize, new: New) -> Result<Value, String> {
    let curve = match arg(a, at) {
        Value::Undefined => 0,
        _ => int(a, at)?,
    };
    if !(0..=CURVE_NEG_TO_POS).contains(&curve) {
        return Err(format!("score curve {curve} is not 0 to {CURVE_NEG_TO_POS}"));
    }
    let timeout = match arg(a, at + 1) {
        Value::Undefined => 0,
        _ => {
            let t = num(a, at + 1)?;
            if t > 0.001 { (t * 1000.0) as i64 } else { 0 }
        }
    };
    let ride = match arg(a, at + 2) {
        Value::Undefined => None,
        v => {
            let n = entnum(vm, v).ok_or("the influencer's entity is not an entity")?;
            match classify(world, n) {
                Some((obj, kind)) => Some(Ride { num: n, obj, kind }),
                None => return Ok(Value::Undefined),
            }
        }
    };
    let now = world.resource::<Zm>().now_ms;
    with(world, |s, _| s.add(new, curve, timeout, ride, now)).map(|i| Value::Int(i as i32))
}

/// `removeinfluencer(id)`.
pub(super) fn remove(world: &mut World, a: &[Value]) -> Result<Value, String> {
    let id = int(a, 0)?;
    with(world, |s, _| {
        if let Some(i) = s.slot(id, "removeinfluencer") {
            s.free(i);
        }
    });
    Ok(Value::Undefined)
}

/// `enableinfluencer(id, on)`.
pub(super) fn enable(world: &mut World, a: &[Value]) -> Result<Value, String> {
    let (id, on) = (int(a, 0)?, flag(a, 1, false));
    with(world, |s, _| {
        if let Some(i) = s.slot(id, "enableinfluencer") {
            s.infl[i].active = on;
        }
    });
    Ok(Value::Undefined)
}

/// `setinfluencerteammask(id, mask)`.
pub(super) fn set_team_mask(world: &mut World, a: &[Value]) -> Result<Value, String> {
    let (id, mask) = (int(a, 0)?, int(a, 1)?);
    with(world, |s, _| {
        if let Some(i) = s.slot(id, "setinfluencerteammask") {
            s.infl[i].team_mask = mask;
        }
    });
    Ok(Value::Undefined)
}

/// `addspawnpoints(team, points)`: a point given twice gets the team added.
pub(super) fn add_points(vm: &mut Vm<World>, world: &mut World, a: &[Value]) -> Result<Value, String> {
    let bit = 1i32 << team_number(&text(vm, a, 0))?;
    let Value::Array(list) = arg(a, 1) else {
        return Err(format!("argument 2 is {}, not an array", arg(a, 1).type_name()));
    };
    let vals: Vec<Value> = list.read().values_in_order().cloned().collect();
    let (target_f, name_f) = (vm.intern("target"), vm.intern("targetname"));
    let field = |vm: &Vm<World>, o: ObjRef, f| match vm.raw_field(o, f) {
        Value::Undefined => String::new(),
        v => vm.to_text(&v),
    };
    let mut new = Vec::new();
    for v in &vals {
        let Some(n) = entnum(vm, v) else { continue };
        let zm = world.resource::<Zm>();
        let Some((obj, origin, yaw)) = zm.ents.get(&n).and_then(|e| Some((e.obj?, e.origin, e.angles[1]))) else {
            continue;
        };
        // Its `target`'s spot (the first entity named so), else zero.
        let target = field(vm, obj, target_f);
        let los = if target.is_empty() {
            [0.0; 3]
        } else {
            zm.ents
                .values()
                .find(|e| e.obj.is_some_and(|o| field(vm, o, name_f) == target))
                .map_or([0.0; 3], |e| e.origin)
        };
        new.push((n, obj, origin, yaw, los));
    }
    with(world, |s, _| {
        for (num, obj, origin, yaw, los) in new {
            if let Some(p) = s.points.iter_mut().find(|p| p.num == num) {
                p.team_mask |= bit;
                continue;
            }
            if s.points.len() >= MAX_POINTS {
                diag::warn!(Sim, "bo2mp spawn system: more than {MAX_POINTS} spawn points, the rest left out");
                break;
            }
            s.points.push(Point {
                num,
                obj,
                origin,
                yaw,
                team_mask: bit,
                weight: [0.0; TEAMS],
                base: [0.0; TEAMS],
                dangerous: false,
                los,
            });
        }
        s.points.sort_by(|x, y| x.origin[0].partial_cmp(&y.origin[0]).unwrap_or(std::cmp::Ordering::Equal));
    });
    Ok(Value::Undefined)
}

/// `clearspawnpoints()`.
pub(super) fn clear_points(world: &mut World) -> Result<Value, String> {
    world.resource_mut::<Zm>().spawns.points.clear();
    Ok(Value::Undefined)
}

/// `setspawnpointsbaseweight(team mask, origin, angle, weight)`: the points
/// facing `origin` within `angle` degrees (as each point faces now) get
/// `weight` for the teams in the mask, the rest nothing (a flag's base in
/// Capture the Flag).
pub(super) fn set_base_weight(world: &mut World, a: &[Value]) -> Result<Value, String> {
    let mut mask = int(a, 0)?;
    if mask == 0 {
        mask = -1;
    }
    let (origin, angle, weight) = (vec3(a, 1)?, num(a, 2)?, num(a, 3)?);
    let cos_a = (angle * 0.017_453_292).cos();
    with(world, |s, world| {
        let zm = world.resource::<Zm>();
        for p in &mut s.points {
            let angles = zm
                .ents
                .get(&p.num)
                .filter(|e| e.obj == Some(p.obj))
                .map_or([0.0, p.yaw, 0.0], |e| e.angles);
            let (fwd, _, _) = game_math::angle_vectors(angles);
            let d = sub(origin, p.origin);
            let mut len = game_math::vec3_length(d);
            if len <= 0.0 {
                len = 1.0;
            }
            let dir = [d[0] / len, d[1] / len, d[2] / len];
            let w = if dot(fwd, dir) < cos_a { 0.0 } else { weight };
            for t in 0..TEAMS {
                if mask & (1 << t) != 0 {
                    p.base[t] = w;
                }
            }
        }
    });
    Ok(Value::Undefined)
}

/// `getsortedspawnpoints(point team, influencer team, enemy mask, player,
/// predicted)`'s one point, or None when there is none to pick (no points
/// given, a team the points cannot weigh for): the caller's old pick then.
pub(super) fn sorted_point(vm: &mut Vm<World>, world: &mut World, a: &[Value]) -> Result<Option<ObjRef>, String> {
    if world.resource::<Zm>().spawns.points.is_empty() {
        return Ok(None);
    }
    let (Ok(pt), Ok(it)) = (team_number(&text(vm, a, 0)), team_number(&text(vm, a, 1))) else {
        return Ok(None);
    };
    let (pt, it) = (usize::from(pt), usize::from(it));
    if pt >= TEAMS || it >= TEAMS {
        return Ok(None);
    }
    let mask = int(a, 2)?;
    let player = entnum(vm, arg(a, 3));
    let viewers = viewers(vm, world, mask, player);
    let bodies = bodies(world);
    let clock = std::time::Instant::now();
    let pick = with(world, |s, world| s.pick(world, pt, it, player, &viewers, &bodies));
    let ms = clock.elapsed().as_secs_f64() * 1000.0;
    if ms > 10.0 {
        diag::warn!(Sim, "bo2mp spawn pick took {ms:.1} ms ({} enemies looking)", viewers.len());
    }
    Ok(pick.filter(|o| vm.alive(*o)))
}

/// Each frame: influencers whose entity went go (or stay where they were
/// while their time runs), timed-out ones go, ridden ones follow.
pub(crate) fn advance(world: &mut World) {
    if world.resource::<Zm>().spawns.infl.is_empty() {
        return;
    }
    with(world, |s, world| s.update(world));
}

/// A player hurt (BO2 calls it from G_DamageClient right after the player
/// damage callback, not for a player in last stand, before lagged damage):
/// his own good influencers (the friend sphere that draws his team to him)
/// rest five seconds.
pub(crate) fn player_damaged(world: &mut World, victim: u32) {
    let mut zm = world.resource_mut::<Zm>();
    let now = zm.now_ms;
    for inf in zm.spawns.infl.iter_mut() {
        if inf.used && inf.active && inf.score > 0.0 && inf.ride.is_some_and(|r| r.num == victim) {
            inf.disable = now + DAMAGE_REST_MS;
        }
    }
}

// ---- the system ----------------------------------------------------------

impl SpawnSystem {
    fn add(&mut self, new: New, curve: i32, timeout: i64, ride: Option<Ride>, now: i64) -> Result<usize, String> {
        let i = match self.infl.iter().position(|x| !x.used) {
            Some(i) => i,
            None if self.infl.len() < MAX_INFLUENCERS => {
                self.infl.push(Influencer::default());
                self.infl.len() - 1
            }
            None => return Err(format!("more than {MAX_INFLUENCERS} spawn influencers")),
        };
        let (mut score, mut dangerous) = (new.score, false);
        if score < DANGEROUS_SCORE {
            score *= 0.001;
            dangerous = true;
        }
        let side = cross(new.up, new.forward);
        let bounding_radius = if new.cylinder { new.length * 0.5 + new.radius } else { new.radius };
        let o = new.origin;
        self.infl[i] = Influencer {
            used: true,
            active: true,
            kind: new.kind,
            cylinder: new.cylinder,
            radius: new.radius,
            preset_length: new.length,
            score,
            curve,
            dangerous,
            origin: o,
            up: new.up,
            quat: axis_to_quat([new.forward, side, new.up]),
            length: new.length,
            mid: o,
            bounding_radius,
            bounds: [o[0] - bounding_radius, o[0] + bounding_radius],
            team_mask: if new.mask == 0 { -1 } else { new.mask },
            expire: if timeout != 0 { now + timeout } else { 0 },
            disable: 0,
            ride,
            weapon_update: 0,
        };
        let low = o[0] - bounding_radius;
        let at = self.sorted.partition_point(|&j| self.infl[j].bounds[0] <= low);
        self.sorted.insert(at, i);
        Ok(i)
    }

    fn slot(&self, id: i32, what: &str) -> Option<usize> {
        let i = usize::try_from(id).ok().filter(|&i| self.infl.get(i).is_some_and(|x| x.used));
        if i.is_none() {
            diag::warn!(Sim, "bo2mp spawn system: {what} on influencer {id}, which is not in use");
        }
        i
    }

    fn free(&mut self, i: usize) {
        self.infl[i].used = false;
        self.infl[i].ride = None;
        self.sorted.retain(|&j| j != i);
    }

    /// Once a frame (BO2's SpawnSystem_Update at the level's time).
    fn update(&mut self, world: &mut World) {
        let now = world.resource::<Zm>().now_ms;
        if self.last_update == Some(now) {
            return;
        }
        self.last_update = Some(now);
        // Their entity gone: still timed, they stay where they were;
        // else they go with it.
        for i in 0..self.infl.len() {
            let x = &self.infl[i];
            let Some(r) = x.ride.filter(|_| x.used) else { continue };
            if !exists(world, &r) {
                if now < x.expire {
                    self.infl[i].ride = None;
                } else {
                    self.free(i);
                }
            }
        }
        for i in 0..self.infl.len() {
            let x = &self.infl[i];
            if x.used && x.expire != 0 && x.expire < now {
                self.free(i);
            }
        }
        // Ridden ones follow their rider; their reach moves with them.
        for i in 0..self.infl.len() {
            let x = &self.infl[i];
            let Some(r) = x.ride.filter(|_| x.used && x.active) else { continue };
            let Some(live) = live(world, &r) else { continue };
            let x = &mut self.infl[i];
            if !live.frozen {
                x.origin = live.origin;
                if x.kind == TYPE_WEAPON {
                    x.origin[2] += live.view_height;
                }
            }
            x.bounds = [x.origin[0] - x.bounding_radius, x.origin[0] + x.bounding_radius];
            if x.cylinder {
                update_cylinder(x, &r, &live, now, world);
            }
        }
        let SpawnSystem { infl, sorted, .. } = self;
        sorted.sort_by(|&a, &b| {
            infl[a].bounds[0].partial_cmp(&infl[b].bounds[0]).unwrap_or(std::cmp::Ordering::Equal)
        });
    }

    /// The weight of point `pi` for `pt` with the influencers for `it`
    /// (BO2's ComputePointWeight); it marks the point dangerous when a
    /// dangerous influencer covers it.
    fn weigh(&mut self, pi: usize, pt: usize, it: usize, player: Option<u32>, now: i64) {
        let p = self.points[pi].origin;
        let mut sum = [0.0f32; TYPES];
        let (mut friend_max, mut spawned_min) = (0.0f32, 0.0f32);
        let mut dangerous = false;
        for &j in &self.sorted {
            let x = &self.infl[j];
            if !(x.used && x.active) || x.team_mask & (1 << it) == 0 || x.disable >= now {
                continue;
            }
            if p[0] < x.bounds[0] || p[0] > x.bounds[1] {
                continue;
            }
            // Never his own.
            if x.kind == TYPE_PLAYER && player.is_some() && x.ride.map(|r| r.num) == player {
                continue;
            }
            let w = x.weight_at(p);
            if w == 0.0 {
                continue;
            }
            if x.dangerous {
                dangerous = true;
            }
            match x.kind {
                // Enemies stack; of friends, the strongest counts.
                TYPE_PLAYER if w <= 0.0 => sum[TYPE_PLAYER as usize] += w,
                TYPE_PLAYER => friend_max = friend_max.max(w),
                TYPE_GAME_MODE => {
                    if w.abs() > sum[TYPE_GAME_MODE as usize].abs() {
                        sum[TYPE_GAME_MODE as usize] = w;
                    }
                }
                TYPE_ENEMY_SPAWNED => spawned_min = spawned_min.min(w),
                k => sum[k as usize] += w,
            }
        }
        let point = &mut self.points[pi];
        let mut w = point.base[pt];
        for v in sum {
            w += v;
        }
        w += friend_max;
        w += spawned_min;
        point.weight[pt] = w;
        point.dangerous = dangerous;
    }

    /// The point to spawn at (BO2's UpdateSpawnPointsForPlayer with no
    /// culling and no non-team spawns): weigh, best first, then the first
    /// safe one of the best twenty; none safe, the best that would not
    /// telefrag; all would, the best.
    fn pick(
        &mut self,
        world: &mut World,
        pt: usize,
        it: usize,
        player: Option<u32>,
        viewers: &[[f32; 3]],
        bodies: &[([f32; 3], [f32; 3])],
    ) -> Option<ObjRef> {
        self.update(world);
        let now = world.resource::<Zm>().now_ms;
        let bit = 1i32 << pt;
        let mut order: Vec<usize> = (0..self.points.len()).filter(|&i| self.points[i].team_mask & bit != 0).collect();
        if order.is_empty() {
            return None;
        }
        for &i in &order {
            self.weigh(i, pt, it, player, now);
        }
        order.sort_by(|&x, &y| {
            let (a, b) = (self.points[x].weight[pt], self.points[y].weight[pt]);
            b.partial_cmp(&a).unwrap_or(std::cmp::Ordering::Equal)
        });
        let n = order.len().min(WALK);
        let i = self.check_dangerous(world, &order, n, viewers, bodies);
        // (BO2 weighs again ignoring danger and takes the first: the same
        // order, so the same point.)
        Some(self.points[order[if i < n { i } else { 0 }]].obj)
    }

    /// BO2's CheckDangerousPoints on its last chance: the first of the best
    /// `n` that would telefrag nobody, is not dangerous, and that no enemy
    /// sees (nor its `target` spot); none, the first that would not
    /// telefrag (the count when all would).
    fn check_dangerous(
        &self,
        world: &mut World,
        order: &[usize],
        n: usize,
        viewers: &[[f32; 3]],
        bodies: &[([f32; 3], [f32; 3])],
    ) -> usize {
        let f = frame(world);
        let lim = order.len().min(n);
        let mut first_ok = order.len();
        let mut i = 0;
        while i < lim {
            let p = &self.points[order[i]];
            if !telefrag(p.origin, bodies) {
                first_ok = first_ok.min(i);
                if !p.dangerous
                    && !visible(&f, viewers, p.origin)
                    && (p.los == [0.0; 3] || !visible(&f, viewers, p.los))
                {
                    return i;
                }
            }
            i += 1;
        }
        first_ok
    }
}

impl Influencer {
    /// Its weight at `p` (BO2's ComputeSphereInfluence /
    /// ComputeCylinderInfluence): inside, its score shaped by its curve
    /// over the distance from its centre (a cylinder's, from its axis).
    fn weight_at(&self, p: [f32; 3]) -> f32 {
        let r_sq = self.radius * self.radius;
        if !self.cylinder {
            let d = sub(p, self.origin);
            let d_sq = dot(d, d);
            return if d_sq < r_sq { influence_weight(self.curve, r_sq, d_sq, self.score) } else { 0.0 };
        }
        let mut len = game_math::vec3_length(self.up);
        if len <= 0.0 {
            len = 1.0;
        }
        let u = [self.up[0] / len, self.up[1] / len, self.up[2] / len];
        let v = sub(p, self.mid);
        let axial = dot(u, v).abs();
        if axial >= self.length * 0.5 {
            return 0.0;
        }
        let radial_sq = dot(v, v) - axial * axial;
        if radial_sq < r_sq { influence_weight(self.curve, r_sq, radial_sq, self.score) } else { 0.0 }
    }
}

fn influence_weight(curve: i32, r_sq: f32, d_sq: f32, score: f32) -> f32 {
    let t = ((1.0 / r_sq) * d_sq).sqrt();
    match curve {
        CURVE_LINEAR => (1.0 - t) * score,
        CURVE_STEEP => score * (1.0 - t) * (1.0 - t),
        CURVE_INVERSE_LINEAR => t * score,
        CURVE_NEG_TO_POS => ((t + t) - 1.0) * score,
        _ => score,
    }
}

/// A ridden cylinder: turned by its rider (a chopper or a guided missile
/// lays it along its nose, tipped 15 degrees down), a vehicle's grown by
/// its speed, a weapon's cut where the map stops a shot; its middle half
/// its length along it (BO2's UpdateCylinderInfluencer).
fn update_cylinder(x: &mut Influencer, r: &Ride, live: &Live, now: i64, world: &mut World) {
    if !live.frozen {
        let flying = matches!(r.kind, Rider::Helicopter | Rider::Missile);
        let mut angles = live.angles;
        if flying {
            angles[0] = (angle_normalize_180(angles[0]) + 15.0).clamp(-90.0, 90.0);
        }
        let turn = quat_normalize(axis_to_quat(game_math::angles_to_axis(angles)));
        let axis = unit_quat_to_axis(quat_normalize(quat_mul(x.quat, turn)));
        x.up = if flying { axis[0] } else { axis[2] };
        match x.kind {
            TYPE_VEHICLE => {
                let mut len = x.radius * 0.5;
                if live.speed < -0.001 {
                    len = -len;
                }
                x.length = len + x.preset_length * live.speed;
            }
            TYPE_WEAPON if now > x.weapon_update + WEAPON_UPDATE_MS => {
                let o = x.origin;
                let end = [
                    o[0] + x.up[0] * x.preset_length,
                    o[1] + x.up[1] * x.preset_length,
                    o[2] + x.up[2] * x.preset_length,
                ];
                let frac = frame(world)
                    .trace_static_world(o, end, [0.0; 3], [0.0; 3], crate::bullet_collision::MASK_SHOT)
                    .fraction;
                let f = frac.max(WEAPON_MIN_LENGTH);
                x.length = if frac >= 0.001 {
                    (x.preset_length * f + WEAPON_PUSH_THROUGH).min(x.preset_length)
                } else {
                    1.0
                };
                x.weapon_update = now;
            }
            _ => {}
        }
    }
    let h = x.length * 0.5;
    x.mid = [x.origin[0] + x.up[0] * h, x.origin[1] + x.up[1] * h, x.origin[2] + x.up[2] * h];
}

// ---- riders, eyes, bodies --------------------------------------------------

/// What entity `n` is to an influencer (None: not a kind one rides).
fn classify(world: &World, n: u32) -> Option<(ObjRef, Rider)> {
    let zm = world.resource::<Zm>();
    if let Some(p) = zm.players.get(&n) {
        return Some((p.obj, Rider::Player));
    }
    let e = zm.ents.get(&n)?;
    let obj = e.obj?;
    if zm.actors.contains_key(&n) {
        return Some((obj, Rider::Actor));
    }
    if let Some(c) = world.get_resource::<Streaks>().and_then(|s| s.craft.get(&n)) {
        return Some((obj, if c.ground { Rider::Vehicle } else { Rider::Helicopter }));
    }
    let kind = match e.classname.to_ascii_lowercase().as_str() {
        "script_vehicle" => Rider::Vehicle,
        "grenade" | "rocket" => Rider::Missile,
        "script_model" | "script_brushmodel" | "script_origin" => Rider::ScriptMover,
        _ => return None,
    };
    Some((obj, kind))
}

/// The rider is still the entity it rode (its number not freed or reused).
fn exists(world: &World, r: &Ride) -> bool {
    let zm = world.resource::<Zm>();
    match r.kind {
        Rider::Player => zm.players.get(&r.num).is_some_and(|p| p.obj == r.obj),
        _ => zm.ents.get(&r.num).is_some_and(|e| e.obj == Some(r.obj)),
    }
}

fn live(world: &mut World, r: &Ride) -> Option<Live> {
    if r.kind == Rider::Player {
        let f = frame(world);
        let ps = f.player(ClientId(r.num))?;
        return Some(Live {
            origin: ps.origin,
            angles: ps.viewangles,
            view_height: ps.view_height_current,
            frozen: matches!(ps.pm_type, 2 | 3),
            speed: 0.0,
        });
    }
    let (origin, angles) = world.resource::<Zm>().ents.get(&r.num).map(|e| (e.origin, e.angles))?;
    let speed = world.get_resource::<Streaks>().and_then(|s| s.craft.get(&r.num)).map_or(0.0, |c| c.cur);
    Some(Live { origin, angles, view_height: 0.0, frozen: false, speed })
}

/// Where the players in play on the teams of `mask` stand (not `ignore`).
fn viewers(vm: &mut Vm<World>, world: &mut World, mask: i32, ignore: Option<u32>) -> Vec<[f32; 3]> {
    let team_f = vm.intern("sessionteam");
    let players: Vec<(u32, ObjRef)> = world
        .resource::<Zm>()
        .players
        .iter()
        .filter(|(n, p)| Some(**n) != ignore && p.sessionstate == "playing")
        .map(|(n, p)| (*n, p.obj))
        .collect();
    let mut out = Vec::new();
    for (n, o) in players {
        let team = team_number(&vm.to_text(&vm.raw_field(o, team_f))).unwrap_or(0);
        if mask & (1i32 << team) == 0 {
            continue;
        }
        if let Some(ps) = frame(world).player(ClientId(n)) {
            out.push(ps.origin);
        }
    }
    out
}

/// An enemy within sight range sees the spot (eyes and spot 65 units up).
/// BO2's sight trace: static models (fences, cars) block it too.
fn visible(f: &FrameWorld, viewers: &[[f32; 3]], pos: [f32; 3]) -> bool {
    let to = [pos[0], pos[1], pos[2] + SIGHT_HEIGHT];
    viewers.iter().any(|v| {
        let (dx, dy) = (v[0] - pos[0], v[1] - pos[1]);
        dx * dx + dy * dy <= SIGHT_RANGE * SIGHT_RANGE
            && f.trace_static_world_models(
                [v[0], v[1], v[2] + SIGHT_HEIGHT],
                to,
                crate::bullet_collision::MASK_SHOT,
            )
            .fraction
                >= 1.0
    })
}

/// The boxes of the living players in play.
fn bodies(world: &mut World) -> Vec<([f32; 3], [f32; 3])> {
    let clients: Vec<u32> = world
        .resource::<Zm>()
        .players
        .iter()
        .filter(|(_, p)| p.sessionstate == "playing")
        .map(|(n, _)| *n)
        .collect();
    let mut out = Vec::new();
    for n in clients {
        if frame(world).player(ClientId(n)).is_some_and(|ps| ps.pm_type < PM_DEAD)
            && let Some(b) = super::triggers::player_box(world, n)
        {
            out.push(b);
        }
    }
    out
}

fn telefrag(o: [f32; 3], bodies: &[([f32; 3], [f32; 3])]) -> bool {
    let lo: [f32; 3] = std::array::from_fn(|k| o[k] + SPAWN_MINS[k]);
    let hi: [f32; 3] = std::array::from_fn(|k| o[k] + SPAWN_MAXS[k]);
    bodies.iter().any(|(a, b)| (0..3).all(|k| lo[k] <= b[k] && hi[k] >= a[k]))
}

// ---- math ------------------------------------------------------------------

fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}

fn angle_normalize_180(a: f32) -> f32 {
    a - 360.0 * (a / 360.0 + 0.5).floor()
}

/// BO2's AxisToQuat (x, y, z, w): the first of its four forms long enough,
/// normalised; a zero axis gives no turn.
fn axis_to_quat(m: [[f32; 3]; 3]) -> [f32; 4] {
    let p = [m[0][0], m[0][1], m[0][2], m[1][0], m[1][1], m[1][2], m[2][0], m[2][1], m[2][2]];
    let forms = [
        [p[5] - p[7], p[6] - p[2], p[1] - p[3], p[0] + p[4] + p[8] + 1.0],
        [p[2] + p[6], p[7] + p[5], p[8] - p[4] - p[0] + 1.0, p[1] - p[3]],
        [p[0] - p[4] - p[8] + 1.0, p[3] + p[1], p[2] + p[6], p[5] - p[7]],
        [p[3] + p[1], p[4] - p[0] - p[8] + 1.0, p[7] + p[5], p[6] - p[2]],
    ];
    let q = forms.iter().copied().find(|q| dot4(*q, *q) >= 1.0).unwrap_or(forms[3]);
    quat_normalize(q)
}

fn dot4(a: [f32; 4], b: [f32; 4]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2] + a[3] * b[3]
}

fn quat_normalize(q: [f32; 4]) -> [f32; 4] {
    let len = dot4(q, q).sqrt();
    if len == 0.0 {
        return [0.0, 0.0, 0.0, 1.0];
    }
    let inv = 1.0 / len;
    [q[0] * inv, q[1] * inv, q[2] * inv, q[3] * inv]
}

/// BO2's QuatMultiply(a, b): b turned by a's frame (Hamilton b * a).
fn quat_mul(a: [f32; 4], b: [f32; 4]) -> [f32; 4] {
    [
        a[0] * b[3] + a[3] * b[0] + b[1] * a[2] - b[2] * a[1],
        a[1] * b[3] - a[2] * b[0] + a[3] * b[1] + b[2] * a[0],
        a[1] * b[0] + a[2] * b[3] - b[1] * a[0] + a[3] * b[2],
        a[3] * b[3] - a[0] * b[0] - b[1] * a[1] - b[2] * a[2],
    ]
}

/// BO2's UnitQuatToAxis: forward, left, up.
fn unit_quat_to_axis(q: [f32; 4]) -> [[f32; 3]; 3] {
    let [x, y, z, w] = q;
    [
        [1.0 - 2.0 * (z * z + y * y), 2.0 * w * z + 2.0 * x * y, 2.0 * x * z - 2.0 * w * y],
        [2.0 * x * y - 2.0 * w * z, 1.0 - 2.0 * (z * z + x * x), 2.0 * y * z + 2.0 * w * x],
        [2.0 * w * y + 2.0 * x * z, 2.0 * y * z - 2.0 * w * x, 1.0 - 2.0 * (y * y + x * x)],
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_zero_axis_is_no_turn() {
        assert_eq!(axis_to_quat([[0.0; 3]; 3]), [0.0, 0.0, 0.0, 1.0]);
    }

    #[test]
    fn angles_round_trip_through_the_quat() {
        let angles = [-20.0, 135.0, 0.0];
        let axis = game_math::angles_to_axis(angles);
        let back = unit_quat_to_axis(axis_to_quat(axis));
        for r in 0..3 {
            for c in 0..3 {
                assert!((axis[r][c] - back[r][c]).abs() < 1e-4, "{r},{c}: {axis:?} vs {back:?}");
            }
        }
    }

    #[test]
    fn no_turn_then_a_turn_is_the_turn() {
        let turn = axis_to_quat(game_math::angles_to_axis([10.0, 70.0, 0.0]));
        let q = quat_mul([0.0, 0.0, 0.0, 1.0], turn);
        for k in 0..4 {
            assert!((q[k] - turn[k]).abs() < 1e-6);
        }
    }

    #[test]
    fn curves() {
        // Halfway out (a quarter of the radius squared).
        assert!((influence_weight(CURVE_LINEAR, 4.0, 1.0, 10.0) - 5.0).abs() < 1e-6);
        assert!((influence_weight(CURVE_STEEP, 4.0, 1.0, 10.0) - 2.5).abs() < 1e-6);
        assert!((influence_weight(CURVE_INVERSE_LINEAR, 4.0, 1.0, 10.0) - 5.0).abs() < 1e-6);
        assert!(influence_weight(CURVE_NEG_TO_POS, 4.0, 1.0, 10.0).abs() < 1e-6);
        assert_eq!(influence_weight(0, 4.0, 1.0, 10.0), 10.0);
    }

    #[test]
    fn a_spawn_on_a_body_telefrags() {
        let body = ([-15.0, -15.0, 0.0], [15.0, 15.0, 72.0]);
        assert!(telefrag([20.0, 0.0, 0.0], &[body]));
        assert!(!telefrag([31.0, 0.0, 0.0], &[body]));
        assert!(!telefrag([0.0, 0.0, 73.0], &[body]));
    }
}
