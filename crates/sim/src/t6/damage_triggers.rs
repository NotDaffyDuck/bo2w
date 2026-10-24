//! bo2mp: damage triggers (`trigger_damage`): the billboards on Aftermath and Express that
//! crackle when shot, the drums and cymbals on Encore's stage. A bullet's path or a knife's
//! swing that crosses one, or a blast that reaches it, tells its scripts `damage`; a hit its
//! spawn flags let through fires `trigger` (Black Ops II: G_CheckHitTriggerDamage,
//! G_RadiusDamage, CanDamage, Activate_trigger_damage, Respond_trigger_damage).

use std::collections::BTreeMap;

use bevy_ecs::prelude::{Resource, World};
use gamemode_t6::{
    G_CAN_DAMAGE_CONTENTS_MASK, radius_damage_amount, radius_damage_distance_to_aabb,
};
use gsc_t6::math::{cross, dot, normalize, sub};
use gsc_t6::{ObjRef, Value, Vm};

use super::{Zm, frame, triggers, weapon_text_of, with_vm};
use crate::frame::FrameWorld;
use crate::script::host::triggers::{Volume, t6_volume};
use crate::world::ClientId;

/// The health a damage trigger starts with and goes back to each time it fires.
const FULL_HEALTH: i32 = 32000;
/// Spawn flag: the trigger is gone once it fires (a map's `wait` of 0 or less sets it too).
const FREE_ON_FIRE: i32 = 0x200;
/// The `damage` flags of a blast's hit (iDFlags radius).
const IDFLAGS_RADIUS: i32 = 1;

/// Each damage trigger's health, by entity number, with the object it was kept for (a new
/// entity in the same number starts full).
#[derive(Resource, Default)]
struct Healths(BTreeMap<u32, (ObjRef, i32)>);

/// The means of death's place in BO2's list (`MOD_PISTOL_BULLET` is 1); a name it does not
/// know is `MOD_UNKNOWN`, 0.
fn mod_index(means: &str) -> i32 {
    const MODS: [&str; 21] = [
        "MOD_UNKNOWN",
        "MOD_PISTOL_BULLET",
        "MOD_RIFLE_BULLET",
        "MOD_GRENADE",
        "MOD_GRENADE_SPLASH",
        "MOD_PROJECTILE",
        "MOD_PROJECTILE_SPLASH",
        "MOD_MELEE",
        "MOD_BAYONET",
        "MOD_HEAD_SHOT",
        "MOD_CRUSH",
        "MOD_TELEFRAG",
        "MOD_FALLING",
        "MOD_SUICIDE",
        "MOD_TRIGGER_HURT",
        "MOD_EXPLOSIVE",
        "MOD_IMPACT",
        "MOD_BURNED",
        "MOD_HIT_BY_OBJECT",
        "MOD_DROWN",
        "MOD_GAS",
    ];
    MODS.iter()
        .position(|m| m.eq_ignore_ascii_case(means))
        .map_or(0, |i| i as i32)
}

/// Respond_trigger_damage: does a hit by means `m` get through the trigger's spawn flags?
/// 0x1 shuts out pistol bullets, 0x2 rifle bullets, 0x4 grenades and projectiles (and their
/// splash), 0x8 those and explosives, 0x10 splash, 0x20 the knife, 0x100 the rest (unknown,
/// crush, telefrag, falling, suicide, trigger_hurt).
fn responds(spawnflags: i32, m: i32) -> bool {
    let blocks = |flag: i32, hit: bool| spawnflags & flag != 0 && hit;
    !(blocks(0x1, m == 1)
        || blocks(0x2, m == 2)
        || blocks(0x4, (3..=6).contains(&m))
        || blocks(0x8, (3..=6).contains(&m) || m == 15)
        || blocks(0x10, m == 4 || m == 6)
        || blocks(0x20, m == 7)
        || blocks(0x100, m == 0 || (10..=14).contains(&m)))
}

/// The keys of a trigger its firing reads.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Keys {
    spawnflags: i32,
    threshold: i32,
    accumulate: i32,
    free_on_fire: bool,
}

fn number(v: &Value) -> Option<f32> {
    match *v {
        Value::Int(i) => Some(i as f32),
        Value::Float(f) => Some(f),
        _ => None,
    }
}

fn keys(vm: &mut Vm<World>, obj: ObjRef) -> Keys {
    let mut int = |name: &str| {
        let f = vm.intern(name);
        number(&vm.raw_field(obj, f)).map_or(0, |n| n as i32)
    };
    let (spawnflags, threshold, accumulate) =
        (int("spawnflags"), int("threshold"), int("accumulate"));
    let wait = vm.intern("wait");
    let wait = number(&vm.raw_field(obj, wait));
    Keys {
        spawnflags,
        threshold,
        accumulate,
        free_on_fire: spawnflags & FREE_ON_FIRE != 0 || wait.is_some_and(|w| w <= 0.0),
    }
}

/// Activate_trigger_damage: a hit of `damage` by means `m` the trigger lets through takes
/// from its health; it fires once the damage taken reaches its `accumulate` (at once without
/// one) and its health goes back to full. `Some(true)` when it fires and is then gone.
fn activate(health: &mut i32, keys: &Keys, damage: i32, m: i32) -> Option<bool> {
    if !(keys.threshold < 1 || keys.threshold <= damage) || !responds(keys.spawnflags, m) {
        return None;
    }
    *health -= damage;
    if keys.accumulate == 0 || keys.accumulate <= FULL_HEALTH - *health {
        *health = FULL_HEALTH;
        return Some(keys.free_on_fire);
    }
    None
}

/// Does the line `start..end` meet the inside of every plane `(n, d)` (inside where
/// `n · p <= d`) at once?
fn clip(planes: impl Iterator<Item = ([f32; 3], f32)>, start: [f32; 3], end: [f32; 3]) -> bool {
    let (mut t0, mut t1) = (0.0f32, 1.0f32);
    for (n, d) in planes {
        let (ds, de) = (dot(n, start) - d, dot(n, end) - d);
        if ds > 0.0 && de > 0.0 {
            return false;
        }
        if ds > 0.0 {
            t0 = t0.max(ds / (ds - de));
        } else if de > 0.0 {
            t1 = t1.min(ds / (ds - de));
        }
        if t0 > t1 {
            return false;
        }
    }
    true
}

fn box_planes(mins: [f32; 3], maxs: [f32; 3]) -> [([f32; 3], f32); 6] {
    [
        ([-1.0, 0.0, 0.0], -mins[0]),
        ([1.0, 0.0, 0.0], maxs[0]),
        ([0.0, -1.0, 0.0], -mins[1]),
        ([0.0, 1.0, 0.0], maxs[1]),
        ([0.0, 0.0, -1.0], -mins[2]),
        ([0.0, 0.0, 1.0], maxs[2]),
    ]
}

/// Does the line `start..end` cross the trigger's volume?
fn crosses(volume: &Volume, start: [f32; 3], end: [f32; 3]) -> bool {
    match *volume {
        Volume::Brushes {
            origin,
            ids,
            brushes,
        } => {
            let (s, e) = (sub(start, origin), sub(end, origin));
            ids.iter()
                .filter_map(|&id| brushes.get(usize::from(id)))
                .any(|b| clip(b.planes.iter().map(|p| ([p[0], p[1], p[2]], p[3])), s, e))
        }
        Volume::Box { mins, maxs } => clip(box_planes(mins, maxs).into_iter(), start, end),
        Volume::Hulls { origin, hulls } => {
            let (s, e) = (sub(start, origin), sub(end, origin));
            hulls.iter().any(|h| {
                let lo = std::array::from_fn(|i| h.mid[i] - h.half[i]);
                let hi = std::array::from_fn(|i| h.mid[i] + h.half[i]);
                let slabs = h.slabs.iter().flat_map(|&(dir, at, width)| {
                    [(dir, at + width), ([-dir[0], -dir[1], -dir[2]], width - at)]
                });
                clip(box_planes(lo, hi).into_iter().chain(slabs), s, e)
            })
        }
        Volume::Cylinder {
            origin,
            radius,
            height,
        } => {
            // The height first, then the circle across the part of the line within it.
            let (mut t0, mut t1) = (0.0f32, 1.0f32);
            for (bound, below) in [(origin[2], true), (origin[2] + height, false)] {
                let ds = if below {
                    bound - start[2]
                } else {
                    start[2] - bound
                };
                let de = if below {
                    bound - end[2]
                } else {
                    end[2] - bound
                };
                if ds > 0.0 && de > 0.0 {
                    return false;
                }
                if ds > 0.0 {
                    t0 = t0.max(ds / (ds - de));
                } else if de > 0.0 {
                    t1 = t1.min(ds / (ds - de));
                }
            }
            if t0 > t1 {
                return false;
            }
            let (fx, fy) = (start[0] - origin[0], start[1] - origin[1]);
            let (dx, dy) = (end[0] - start[0], end[1] - start[1]);
            let a = dx * dx + dy * dy;
            let c = fx * fx + fy * fy - radius * radius;
            if a < 1e-6 {
                return c <= 0.0;
            }
            let b = 2.0 * (fx * dx + fy * dy);
            let disc = b * b - 4.0 * a * c;
            if disc < 0.0 {
                return false;
            }
            let root = disc.sqrt();
            let (ta, tb) = ((-b - root) / (2.0 * a), (-b + root) / (2.0 * a));
            t0.max(ta) <= t1.min(tb)
        }
    }
}

/// A damage trigger as the hits see it.
struct Target {
    n: u32,
    obj: ObjRef,
    origin: [f32; 3],
    cylinder: Option<(f32, f32)>,
    brush: Option<u32>,
}

/// The map's damage triggers that are on.
fn targets(world: &World) -> Vec<Target> {
    let Some(zm) = world.get_resource::<Zm>() else {
        return Vec::new();
    };
    zm.ents
        .iter()
        .filter(|(_, e)| e.classname == "trigger_damage" && !e.trigger_off)
        .filter_map(|(&n, e)| {
            Some(Target {
                n,
                obj: e.obj?,
                origin: e.origin,
                cylinder: (e.radius > 0.0).then_some((e.radius, e.height.max(1.0))),
                brush: triggers::brush_index(e),
            })
        })
        .collect()
}

/// A trigger's bounds (absmin / absmax).
fn bounds(f: &FrameWorld, t: &Target) -> Option<([f32; 3], [f32; 3])> {
    if let Some((r, h)) = t.cylinder {
        let o = t.origin;
        return Some(([o[0] - r, o[1] - r, o[2]], [o[0] + r, o[1] + r, o[2] + h]));
    }
    let m = f.clip_cmodels().models.get(t.brush? as usize)?;
    Some((
        std::array::from_fn(|i| t.origin[i] + m.mins[i]),
        std::array::from_fn(|i| t.origin[i] + m.maxs[i]),
    ))
}

/// CanDamage (an entity that is not a player): can the blast at `origin` see the box's middle,
/// or one of four points out from it across the box as the blast faces it? A point outside a
/// blast that only goes one way (`cone`) does not count.
fn can_damage(
    f: &FrameWorld,
    origin: [f32; 3],
    mins: [f32; 3],
    maxs: [f32; 3],
    cone: &dyn Fn([f32; 3]) -> bool,
) -> bool {
    let mid: [f32; 3] = std::array::from_fn(|i| (mins[i] + maxs[i]) * 0.5);
    let half: [f32; 3] = std::array::from_fn(|i| (maxs[i] - mins[i]) * 0.5);
    let fwd = normalize(sub(origin, mid));
    let right = if fwd[0] * fwd[0] + fwd[1] * fwd[1] < 0.001 {
        [1.0, 0.0, 0.0]
    } else {
        normalize([fwd[1], -fwd[0], 0.0])
    };
    let up = cross(fwd, right);
    let reach_r = (half[0] * right[0]).abs() + (half[1] * right[1]).abs();
    let reach_u = (up[0] * half[0]).abs() + (up[1] * half[1]).abs() + (up[2] * half[2]).abs();
    let at = |r: f32, u: f32| -> [f32; 3] {
        std::array::from_fn(|i| mid[i] + right[i] * reach_r * r + up[i] * reach_u * u)
    };
    [
        at(0.0, 0.0),
        at(1.0, 1.0),
        at(1.0, -1.0),
        at(-1.0, 1.0),
        at(-1.0, -1.0),
    ]
    .into_iter()
    .filter(|&p| cone(p))
    .any(|p| {
        let t = f.trace_world(origin, p, [0.0; 3], [0.0; 3], G_CAN_DAMAGE_CONTENTS_MASK);
        t.fraction >= 1.0 && !t.startsolid
    })
}

fn health_of(world: &World, n: u32, obj: ObjRef) -> i32 {
    world
        .get_resource::<Healths>()
        .and_then(|h| h.0.get(&n))
        .filter(|(o, _)| *o == obj)
        .map_or(FULL_HEALTH, |&(_, h)| h)
}

fn set_health(world: &mut World, n: u32, obj: ObjRef, health: i32) {
    world
        .get_resource_or_insert_with(Healths::default)
        .0
        .insert(n, (obj, health));
}

/// The entity goes, as `delete` takes it (its waiters hear `death` first).
fn free(vm: &mut Vm<World>, world: &mut World, n: u32, obj: ObjRef) {
    let item = {
        let mut zm = world.resource_mut::<Zm>();
        zm.movers.list.remove(&n);
        zm.ents.remove(&n).and_then(|e| e.item)
    };
    if let Some(number) = item {
        super::items::forget(world, number);
    }
    if let Some(mut h) = world.get_resource_mut::<Healths>() {
        h.0.remove(&n);
    }
    if world.resource::<Zm>().mp && vm.alive(obj) {
        vm.notify_str(world, obj, "death", &[]);
    }
    vm.free_object(world, obj);
}

fn attacker_value(world: &World, attacker: Option<ClientId>) -> Value {
    attacker
        .and_then(|a| {
            world
                .resource::<Zm>()
                .players
                .get(&a.0)
                .map(|p| Value::Object(p.obj))
        })
        .unwrap_or(Value::Undefined)
}

/// After a trigger let a hit through: it fires (`trigger`, with who hit it) and, if it is to,
/// goes.
fn fire(vm: &mut Vm<World>, world: &mut World, n: u32, obj: ObjRef, by: &Value, gone: bool) {
    if vm.alive(obj) {
        vm.notify_str(world, obj, "trigger", std::slice::from_ref(by));
    }
    if gone
        && world
            .resource::<Zm>()
            .ents
            .get(&n)
            .is_some_and(|e| e.obj == Some(obj))
    {
        free(vm, world, n, obj);
    }
}

/// G_CheckHitTriggerDamage: a bullet's path (or the first line of a knife's swing) from
/// `start` to `end`, of `damage` by `means`. Every damage trigger it crosses hears `damage`
/// (amount, attacker, the line's way, no point, means), then fires if the hit gets through.
pub(crate) fn bullet_path(
    world: &mut World,
    attacker: ClientId,
    start: [f32; 3],
    end: [f32; 3],
    damage: i32,
    means: &str,
) {
    with_vm(world, |vm, world| {
        let by = attacker_value(world, Some(attacker));
        bullet_path_vm(vm, world, by, start, end, damage, means);
    });
}

/// `bullet_path` from inside the scripts (a turret's or a vehicle's gun, fired by a native),
/// hit by `by` (a player, or the turret or vehicle itself).
pub(crate) fn bullet_path_vm(
    vm: &mut Vm<World>,
    world: &mut World,
    by: Value,
    start: [f32; 3],
    end: [f32; 3],
    damage: i32,
    means: &str,
) {
    let list = targets(world);
    if list.is_empty() || damage <= 0 {
        return;
    }
    let struck: Vec<(u32, ObjRef)> = {
        let f = frame(world);
        list.iter()
            .filter(|t| {
                t6_volume(&f, t.origin, t.cylinder, t.brush)
                    .is_some_and(|v| crosses(&v, start, end))
            })
            .map(|t| (t.n, t.obj))
            .collect()
    };
    if struck.is_empty() {
        return;
    }
    let m = mod_index(means);
    let dir = normalize(sub(end, start));
    let means = vm.string(means);
    for (n, obj) in struck {
        if !vm.alive(obj) {
            continue;
        }
        let args = [
            Value::Int(damage),
            by.clone(),
            Value::Vec3(dir),
            Value::Vec3([0.0; 3]),
            means.clone(),
        ];
        vm.notify_str(world, obj, "damage", &args);
        if !vm.alive(obj) {
            continue;
        }
        let keys = keys(vm, obj);
        let mut health = health_of(world, n, obj);
        let fired = activate(&mut health, &keys, damage, m);
        if keys.accumulate == 0 {
            health = FULL_HEALTH;
        }
        set_health(world, n, obj, health);
        if let Some(gone) = fired {
            fire(vm, world, n, obj, &by, gone);
        }
    }
}

/// G_RadiusDamage on the damage triggers: a blast at `origin` reaching `radius`, `inner` at
/// its heart down to `outer` at its edge, measured to each trigger's bounds. A trigger it
/// can see (CanDamage) takes the damage and hears `damage` (amount, attacker, the way from
/// the blast, the blast's point, means, weapon, radius flag), then fires if the hit gets
/// through (Activate_trigger_damage); one the damage took to nothing fires no more.
#[allow(clippy::too_many_arguments)]
pub(crate) fn blast(
    world: &mut World,
    origin: [f32; 3],
    radius: f32,
    inner: f32,
    outer: f32,
    attacker: Option<ClientId>,
    means: &str,
    weapon: u32,
    cone: &dyn Fn([f32; 3]) -> bool,
) {
    if radius <= 0.0 || targets(world).is_empty() {
        return;
    }
    let gun = weapon_text_of(world, attacker.map(|a| a.0), weapon);
    with_vm(world, |vm, world| {
        let by = attacker_value(world, attacker);
        blast_in(vm, world, origin, radius, inner, outer, &by, means, &gun, None, cone);
    });
}

/// `blast` from inside the scripts (`radiusdamage`, a streak's blast), by `by` (a player, or
/// undefined), with the weapon's name; `inflictor`, the thing that went up, takes none of its
/// own blast.
#[allow(clippy::too_many_arguments)]
pub(super) fn blast_in(
    vm: &mut Vm<World>,
    world: &mut World,
    origin: [f32; 3],
    radius: f32,
    inner: f32,
    outer: f32,
    by: &Value,
    means: &str,
    gun: &str,
    inflictor: Option<ObjRef>,
    cone: &dyn Fn([f32; 3]) -> bool,
) {
    let list = targets(world);
    if list.is_empty() || radius <= 0.0 {
        return;
    }
    let hits: Vec<(u32, ObjRef, i32, [f32; 3])> = {
        let f = frame(world);
        list.iter()
            .filter(|t| Some(t.obj) != inflictor)
            .filter_map(|t| {
                let (mins, maxs) = bounds(&f, t)?;
                let mid: [f32; 3] = std::array::from_fn(|i| (mins[i] + maxs[i]) * 0.5);
                let half: [f32; 3] = std::array::from_fn(|i| (maxs[i] - mins[i]) * 0.5);
                let dist = radius_damage_distance_to_aabb(origin, mid, half);
                if dist >= radius {
                    return None;
                }
                let amount = radius_damage_amount(inner, outer, radius, dist, 1.0);
                (amount > 0 && can_damage(&f, origin, mins, maxs, cone))
                    .then_some((t.n, t.obj, amount, mid))
            })
            .collect()
    };
    if hits.is_empty() {
        return;
    }
    let m = mod_index(means);
    let (means, gun, none) = (vm.string(means), vm.string(gun), vm.string(""));
    for (n, obj, amount, mid) in hits {
        if !vm.alive(obj) {
            continue;
        }
        let mut health = health_of(world, n, obj) - amount;
        set_health(world, n, obj, health);
        let args = [
            Value::Int(amount),
            by.clone(),
            Value::Vec3(normalize(sub(mid, origin))),
            Value::Vec3(origin),
            means.clone(),
            none.clone(),
            none.clone(),
            none.clone(),
            gun.clone(),
            Value::Int(IDFLAGS_RADIUS),
        ];
        vm.notify_str(world, obj, "damage", &args);
        if health <= 0 || !vm.alive(obj) {
            continue;
        }
        let keys = keys(vm, obj);
        let fired = activate(&mut health, &keys, amount, m);
        set_health(world, n, obj, health);
        if let Some(gone) = fired {
            fire(vm, world, n, obj, by, gone);
        }
    }
}

/// The damage triggers a grenade moving through damages (Black Ops II's entity flag that
/// `enablegrenadetouchdamage` sets and `disablegrenadetouchdamage` clears).
#[derive(Resource, Default)]
struct GrenadeTouch(BTreeMap<u32, ObjRef>);

/// `enablegrenadetouchdamage` (`on`) or `disablegrenadetouchdamage` on entity `n`.
pub(super) fn grenade_touch(world: &mut World, n: u32, obj: ObjRef, on: bool) {
    let mut touch = world.get_resource_or_insert_with(GrenadeTouch::default);
    if on {
        touch.0.insert(n, obj);
    } else {
        touch.0.remove(&n);
    }
}

/// G_GrenadeTouchTriggerDamage: a thrown grenade's move this tick, from `start` to `end`.
/// Every damage trigger set to feel it that the move touches hears `damage` (`damage`, the
/// grenade, the move's way, no point, MOD_GRENADE), then fires if the hit gets through, the
/// grenade as the one who hit it.
pub(crate) fn grenade_path(
    world: &mut World,
    projectile: u32,
    start: [f32; 3],
    end: [f32; 3],
    damage: i32,
) {
    let Some(touch) = world.get_resource::<GrenadeTouch>() else {
        return;
    };
    let felt: Vec<Target> = targets(world)
        .into_iter()
        .filter(|t| touch.0.get(&t.n) == Some(&t.obj))
        .collect();
    if felt.is_empty() {
        return;
    }
    let struck: Vec<(u32, ObjRef)> = {
        let f = frame(world);
        felt.iter()
            .filter(|t| {
                t6_volume(&f, t.origin, t.cylinder, t.brush)
                    .is_some_and(|v| crosses(&v, start, end))
            })
            .map(|t| (t.n, t.obj))
            .collect()
    };
    if struck.is_empty() {
        return;
    }
    // The scripts' stand-in for the grenade (made the pass after the throw).
    let Some(grenade) = super::killstreaks::thrown_ent(world, projectile)
        .and_then(|n| world.resource::<Zm>().ents.get(&n)?.obj)
    else {
        return;
    };
    let m = mod_index("MOD_GRENADE");
    let dir = normalize(sub(end, start));
    with_vm(world, |vm, world| {
        let by = Value::Object(grenade);
        let means = vm.string("MOD_GRENADE");
        for (n, obj) in struck {
            if !vm.alive(obj) || !vm.alive(grenade) {
                continue;
            }
            let args = [
                Value::Int(damage),
                by.clone(),
                Value::Vec3(dir),
                Value::Vec3([0.0; 3]),
                means.clone(),
            ];
            vm.notify_str(world, obj, "damage", &args);
            if !vm.alive(obj) {
                continue;
            }
            let keys = keys(vm, obj);
            let mut health = health_of(world, n, obj);
            let fired = activate(&mut health, &keys, damage, m);
            if keys.accumulate == 0 {
                health = FULL_HEALTH;
            }
            set_health(world, n, obj, health);
            if let Some(gone) = fired {
                fire(vm, world, n, obj, &by, gone);
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cube(min: [f32; 3], max: [f32; 3]) -> crate::SimBrush {
        crate::SimBrush {
            planes: box_planes(min, max)
                .iter()
                .map(|(n, d)| [n[0], n[1], n[2], *d])
                .collect(),
            contents: 0x4000_0000,
            plane_surface_flags: vec![0; 6],
            glass_encoded: 0,
            verts: Vec::new(),
            hit_surface_flags: 0,
        }
    }

    #[test]
    fn a_line_through_a_brush_crosses_it_and_one_beside_it_does_not() {
        let brushes = [cube([-10.0, -10.0, -10.0], [10.0, 10.0, 10.0])];
        let v = Volume::Brushes {
            origin: [100.0, 0.0, 0.0],
            ids: &[0],
            brushes: &brushes,
        };
        assert!(crosses(&v, [0.0, 0.0, 0.0], [200.0, 0.0, 0.0]));
        assert!(!crosses(&v, [0.0, 20.0, 0.0], [200.0, 20.0, 0.0]));
        // Stops short of it, starts inside it, runs along beside it.
        assert!(!crosses(&v, [0.0, 0.0, 0.0], [85.0, 0.0, 0.0]));
        assert!(crosses(&v, [100.0, 0.0, 0.0], [100.0, 0.0, 50.0]));
        assert!(!crosses(&v, [80.0, 11.0, 0.0], [120.0, 11.0, 0.0]));
    }

    #[test]
    fn a_line_across_a_box_corner_crosses_it() {
        let v = Volume::Box {
            mins: [0.0, 0.0, 0.0],
            maxs: [10.0, 10.0, 10.0],
        };
        assert!(crosses(&v, [-5.0, 8.0, 5.0], [8.0, -5.0, 5.0]));
        assert!(!crosses(&v, [-5.0, 2.0, 5.0], [2.0, -5.0, 5.0]));
    }

    #[test]
    fn a_line_crosses_a_cylinder_within_its_height() {
        let v = Volume::Cylinder {
            origin: [0.0, 0.0, 0.0],
            radius: 10.0,
            height: 50.0,
        };
        assert!(crosses(&v, [-20.0, 0.0, 25.0], [20.0, 0.0, 25.0]));
        assert!(!crosses(&v, [-20.0, 0.0, 60.0], [20.0, 0.0, 60.0]));
        assert!(!crosses(&v, [-20.0, 11.0, 25.0], [20.0, 11.0, 25.0]));
        assert!(crosses(&v, [5.0, 0.0, -10.0], [5.0, 0.0, 10.0]));
        assert!(!crosses(&v, [-20.0, 0.0, 25.0], [-15.0, 0.0, 25.0]));
    }

    #[test]
    fn encores_instruments_let_bullets_and_knives_through_and_not_explosions() {
        let flags = 344;
        for means in [
            "MOD_PISTOL_BULLET",
            "MOD_RIFLE_BULLET",
            "MOD_MELEE",
            "MOD_HEAD_SHOT",
            "MOD_IMPACT",
        ] {
            assert!(responds(flags, mod_index(means)), "{means}");
        }
        for means in [
            "MOD_GRENADE",
            "MOD_GRENADE_SPLASH",
            "MOD_PROJECTILE_SPLASH",
            "MOD_EXPLOSIVE",
            "MOD_CRUSH",
            "MOD_SOMETHING_ELSE",
        ] {
            assert!(!responds(flags, mod_index(means)), "{means}");
        }
        assert!(responds(0, mod_index("MOD_EXPLOSIVE")));
        // BO2's list: the bayonet is 8, so the head shot is 9 and the explosive 15.
        assert_eq!(mod_index("MOD_HEAD_SHOT"), 9);
        assert_eq!(mod_index("MOD_EXPLOSIVE"), 15);
        assert_eq!(mod_index("MOD_GAS"), 20);
    }

    #[test]
    fn a_trigger_fires_at_once_without_accumulate_and_after_it_with_one() {
        let mut health = FULL_HEALTH;
        let keys = Keys::default();
        assert_eq!(activate(&mut health, &keys, 30, 1), Some(false));
        assert_eq!(health, FULL_HEALTH);

        let keys = Keys {
            accumulate: 100,
            ..Keys::default()
        };
        assert_eq!(activate(&mut health, &keys, 60, 1), None);
        assert_eq!(health, FULL_HEALTH - 60);
        assert_eq!(activate(&mut health, &keys, 60, 1), Some(false));
        assert_eq!(health, FULL_HEALTH);
    }

    #[test]
    fn a_hit_under_the_threshold_or_shut_out_does_nothing() {
        let mut health = FULL_HEALTH;
        let keys = Keys {
            threshold: 50,
            ..Keys::default()
        };
        assert_eq!(activate(&mut health, &keys, 49, 1), None);
        assert_eq!(activate(&mut health, &keys, 50, 1), Some(false));
        let keys = Keys {
            spawnflags: 0x20,
            free_on_fire: true,
            ..Keys::default()
        };
        assert_eq!(activate(&mut health, &keys, 50, 7), None);
        assert_eq!(health, FULL_HEALTH);
        assert_eq!(activate(&mut health, &keys, 50, 1), Some(true));
    }

    #[test]
    fn grenade_touch_damage_is_switched_on_and_off_per_trigger() {
        let mut world = World::new();
        // Nothing switched on: a grenade's move touches nothing.
        grenade_path(&mut world, 1, [0.0; 3], [10.0, 0.0, 0.0], 100);
        let a = ObjRef {
            index: 4,
            generation: 1,
        };
        let b = ObjRef {
            index: 5,
            generation: 1,
        };
        grenade_touch(&mut world, 40, a, true);
        grenade_touch(&mut world, 41, b, true);
        grenade_touch(&mut world, 40, a, false);
        let touch = world.resource::<GrenadeTouch>();
        assert_eq!(touch.0.get(&40), None);
        assert_eq!(touch.0.get(&41), Some(&b));
    }
}
