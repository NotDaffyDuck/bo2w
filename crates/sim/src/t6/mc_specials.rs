//! Minecraft Zombies' guest weapons on a plain Black Ops II map, "just for
//! fun" in the Special Weapons tab: Half-Life 2's Gravity Gun and its Super
//! (Pack-a-Punched), Minecraft's bow and enchanted bow (Pack-a-Punched), and
//! the enchanted stick. Each rides a copy of the Ray Gun
//! (`assets::lane::t6_m2`, `GUEST_WEAPONS`) so BO2 holds, switches and
//! reloads it; its trigger fires no Ray Gun shots (`owns_trigger`):
//! - the bow draws while fire is held and lets an arrow go on release, as
//!   Minecraft's (`BowItem.getPowerForTime`); the plain bow spends a round
//!   of its clip a shot, the enchanted one none (Infinity) and its arrows
//!   set zombies alight (Flame);
//! - the stick swings on each press: Sharpness X, Knockback X, a kill flies;
//! - the Gravity Gun is the world side's (`render_anim` guest weapons): it
//!   grabs and throws bodies, and the Super grabs and throws live zombies
//!   too. No blocks on a plain map. What it does to the zombies comes back
//!   here (`guest::Ask::GravHit`).

use bevy_ecs::prelude::{Resource, World};
use gsc_t6::{ObjRef, Value};

use super::{Zm, frame, with_vm};
use crate::guest::{self, ArrowView, Ask, Guest, GuestZombie, Hand};
use crate::world::ClientId;

/// The tab's entries: (script name, shown name).
const SPECIALS: [(&str, &str); 5] = [
    ("physcannon_zm", "Gravity Gun"),
    ("physcannon_upgraded_zm", "Super Gravity Gun"),
    ("mcbow_zm", "Bow"),
    ("mcbow_enchanted_zm", "Enchanted Bow"),
    ("mcstick_zm", "Enchanted Stick"),
];

/// Into the weapon table: (gun, its Pack-a-Punched one, shown name).
const TABLE: [(&str, Option<&str>, &str); 3] = [
    ("physcannon_zm", Some("physcannon_upgraded_zm"), "Gravity Gun"),
    ("mcbow_zm", Some("mcbow_enchanted_zm"), "Bow"),
    ("mcstick_zm", None, "Enchanted Stick"),
];

/// Map units per Minecraft block.
const B: f32 = 36.0;
/// A Minecraft tick in seconds and in ms.
const TICK: f32 = 0.05;
const TICK_MS: i64 = 50;
/// Minecraft's arrow: 3 blocks a tick at full draw, its drag and gravity.
const ARROW_SPEED: f32 = 3.0 * B;
const ARROW_DRAG: f32 = 0.99;
const ARROW_GRAVITY: f32 = 0.05 * B;
/// Minecraft's arrow damage (2) and Power V's (+3): times its speed in
/// blocks a tick, rounded up (`AbstractArrow.onHitEntity`).
const ARROW_BASE: f32 = 2.0;
const POWER_V: f32 = 3.0;
/// BO2 damage per Minecraft point: the plain bow's full draw (6) is 300,
/// the enchanted one's (15) 1500, the Ray Gun's league.
const PLAIN_PER_POINT: f32 = 50.0;
const ENCHANTED_PER_POINT: f32 = 100.0;
/// A head strike's multiplier (BO2's rifles: twice).
const HEAD: f32 = 2.0;
/// Flame: the zombie burns 4 s, BO2 damage every half second.
const BURN_MS: i64 = 4000;
const BURN_EVERY_MS: i64 = 500;
const BURN_DAMAGE: i32 = 60;
/// An arrow in a wall stays this long; one in flight at most this long.
const STUCK_MS: i64 = 10_000;
const FLIGHT_TICKS: u32 = 1200;
const MAX_ARROWS: usize = 48;
/// A BO2 zombie's body for an arrow: a standing cylinder, the head above.
const ZOMBIE_RADIUS: f32 = 16.0;
const ZOMBIE_HEIGHT: f32 = 76.0;
const ZOMBIE_HEAD: f32 = 60.0;
/// Minecraft's full draw: 20 ticks.
const FULL_DRAW_S: f32 = 1.0;
/// The enchanted stick: BO2 damage a full swing deals (a round-20 zombie's
/// health), the throw of what it kills (units a second), its reach (three
/// blocks) and Minecraft's attack cooldown for a plain item (4 a second).
const STICK_DAMAGE: f32 = 2750.0;
const STICK_FLING: f32 = 220.0;
const STICK_REACH: f32 = 3.0 * B;
const STICK_COOLDOWN_S: f32 = 0.25;
/// A body's throw: BO2's `launchragdoll` is 61 units a second per unit.
const LAUNCH_UNIT: f32 = 0.5 / 0.0082;

#[derive(Clone, Copy, Debug)]
struct Arrow {
    at: [f32; 3],
    /// Map units a tick.
    velocity: [f32; 3],
    enchanted: bool,
    stuck: Option<i64>,
    ticks: u32,
}

#[derive(Resource, Default)]
pub(super) struct McSpecials {
    tabled: bool,
    arrows: Vec<Arrow>,
    clock_ms: Option<i64>,
    /// Zombies on fire: entity, until, next burn.
    burning: Vec<(u32, i64, i64)>,
    /// Fire held last tick, and since when (ms) the bow has been drawn.
    attack_was: bool,
    draw_since: Option<i64>,
    last_swing: i64,
    hand: Hand,
}

fn state(world: &mut World) -> bevy_ecs::world::Mut<'_, McSpecials> {
    if !world.contains_resource::<McSpecials>() {
        world.insert_resource(McSpecials::default());
    }
    world.resource_mut::<McSpecials>()
}

/// The weapon step leaves their trigger alone: they fire no Ray Gun shots.
pub(crate) fn owns_trigger(weapon_name: &str) -> bool {
    Guest::of(weapon_name).is_some()
}

/// The Special Weapons tab lists the five (on a map that loaded them).
pub(super) fn add_to_cheats() {
    super::add_special_weapons(&SPECIALS);
}

pub(super) fn tick(world: &mut World) {
    let now = world.resource::<Zm>().now_ms;
    if world.resource::<Zm>().players.is_empty() {
        return;
    }
    table(world);
    zombie_health(world);
    asks(world);
    hands(world, now);
    arrows(world, now);
    burning(world, now);
    publish_zombies(world, now);
}

/// Once the scripts have their weapon table: the three in it (none in the
/// box), with their Pack-a-Punch and their buy prompt, so the scripts'
/// lookups by name find them.
fn table(world: &mut World) {
    if world.get_resource::<McSpecials>().is_some_and(|s| s.tabled) {
        return;
    }
    let ready = with_vm(world, |vm, _| {
        let f = vm.intern("zombie_weapons");
        let g = vm.intern("zombie_include_weapons");
        !matches!(vm.raw_field(vm.level, f), Value::Undefined) && !matches!(vm.raw_field(vm.level, g), Value::Undefined)
    })
    .unwrap_or(false);
    if !ready {
        return;
    }
    state(world).tabled = true;
    let m14 = world.resource::<Zm>().strings.get("ZOMBIE_WEAPON_M14").filter(|t| t.contains("M14")).cloned();
    for (gun, up, label) in TABLE {
        if super::weapon(world, gun).is_err() {
            diag::warn!(Sim, "bo2zm guest weapons: {gun} is not loaded");
            continue;
        }
        let key = format!("BO2MC_WEAPON_{}", gun.to_ascii_uppercase());
        let key = match &m14 {
            Some(t) => {
                world.resource_mut::<Zm>().strings.insert(key.clone(), t.replace("M14", label));
                key
            }
            None => label.to_owned(),
        };
        let up = up.filter(|u| super::weapon(world, u).is_ok());
        with_vm(world, |vm, world| {
            let level = Value::Object(vm.level);
            let w = vm.string(gun);
            vm.spawn_named(world, "maps/mp/zombies/_zm_utility", "include_weapon", level.clone(), vec![w.clone(), Value::Int(0)]);
            let up = match up {
                Some(u) => {
                    let u = vm.string(u);
                    vm.spawn_named(world, "maps/mp/zombies/_zm_utility", "include_weapon", level.clone(), vec![u.clone(), Value::Int(0)]);
                    u
                }
                None => Value::Undefined,
            };
            let hint = Value::IStr(vm.intern(&key));
            let empty = vm.string("");
            let args = vec![w, up, hint, Value::Int(10000), empty.clone(), empty, Value::Undefined];
            if vm.spawn_named(world, "maps/mp/zombies/_zm_weapons", "add_zombie_weapon", level, args).is_none() {
                diag::warn!(Sim, "bo2zm guest weapons: no add_zombie_weapon");
            }
        });
    }
    diag::info!(Sim, "bo2zm guest weapons: in the weapon table");
}

fn zombie_health(world: &mut World) {
    let health = with_vm(world, |vm, _| {
        let h = vm.intern("zombie_health");
        vm.raw_field(vm.level, h).as_int()
    })
    .flatten();
    if let Some(h) = health {
        guest::set_zombie_health(h);
    }
}

/// What the world side's Gravity Gun asks.
fn asks(world: &mut World) {
    for a in guest::take_asks() {
        match a {
            Ask::GravHit { ent, damage, means, at, dir, launch } => hit(world, ent, damage, &means, at, dir, launch),
        }
    }
}

/// His eye and view direction, while he can fight.
fn aim(world: &mut World) -> Option<(ObjRef, [f32; 3], [f32; 3])> {
    let obj = world.resource::<Zm>().players.get(&0).filter(|p| !p.laststand).map(|p| p.obj)?;
    let (eye, fwd) = frame(world).player(ClientId(0)).filter(|ps| ps.health > 0).map(|ps| {
        (
            [ps.origin[0], ps.origin[1], ps.origin[2] + ps.view_height_current],
            crate::bullet::angles_to_forward(ps.viewangles),
        )
    })?;
    Some((obj, eye, fwd))
}

/// His guest weapon out, read from his fire button: the bow draws and lets
/// go, the stick swings. The world side learns what his hands do.
fn hands(world: &mut World, now: i64) {
    use weapon_iw4::WeaponState as S;
    let id = ClientId(0);
    let Some(ps) = frame(world).player(id).copied() else {
        return;
    };
    let name = super::weapon_text(world, ps.weapon);
    let held = Guest::of(&name);
    let down = world.resource::<Zm>().players.get(&0).is_none_or(|p| p.laststand) || ps.health <= 0;
    let attack = !down && crate::script_player::buttons(&mut frame(world), id) & playerstate_iw4::buttons::ATTACK != 0;
    let ready = matches!(S::from_i32(ps.weaponstate_primary).ok(), Some(S::Ready | S::Firing));
    let (was, since, last_swing, mut hand) = {
        let s = state(world);
        (s.attack_was, s.draw_since, s.last_swing, s.hand)
    };
    hand.weapon = held;
    let mut draw_since = None;
    match held {
        Some(bow @ (Guest::Bow | Guest::EnchantedBow)) => {
            let enchanted = bow == Guest::EnchantedBow;
            let clip = crate::script_player::ammo_clip(&frame(world), id, ps.weapon);
            let can = ready && (enchanted || clip > 0);
            if attack && can {
                draw_since = Some(since.unwrap_or(now));
            } else if let Some(t) = since {
                // Let go (or the bow went away): Minecraft's power for the
                // draw, `(f² + 2f) / 3` of a full second's draw.
                let f = ((now - t) as f32 / 1000.0 / FULL_DRAW_S).min(1.0);
                let power = ((f * f + 2.0 * f) / 3.0).min(1.0);
                if power >= 0.1 && can {
                    if !enchanted {
                        crate::script_player::set_ammo_clip(&mut frame(world), id, ps.weapon, clip - 1);
                    }
                    fire(world, power, enchanted);
                    hand.shots = hand.shots.wrapping_add(1);
                }
            }
        }
        Some(Guest::Stick) if attack && !was && ready => {
            let since = (now - last_swing) as f32 / 1000.0;
            // Minecraft's attack strength: 0.2 + 0.8 f², f the cooldown's share.
            let f = ((since + 0.025) / STICK_COOLDOWN_S).clamp(0.0, 1.0);
            let strength = 0.2 + 0.8 * f * f;
            state(world).last_swing = now;
            hand.swings = hand.swings.wrapping_add(1);
            if stick(world, strength) {
                hand.hits = hand.hits.wrapping_add(1);
            }
        }
        _ => {}
    }
    hand.draw = draw_since.map(|t| (now - t) as f32 / 1000.0);
    let mut s = state(world);
    s.attack_was = attack;
    s.draw_since = draw_since;
    s.hand = hand;
    guest::set_hand(hand);
}

/// He let the bow go at this power: an arrow from his eye.
fn fire(world: &mut World, power: f32, enchanted: bool) {
    let Some((_, eye, fwd)) = aim(world) else { return };
    let speed = ARROW_SPEED * power;
    let arrow = Arrow { at: eye, velocity: fwd.map(|v| v * speed), enchanted, stuck: None, ticks: 0 };
    let mut s = state(world);
    s.arrows.push(arrow);
    if s.arrows.len() > MAX_ARROWS {
        s.arrows.remove(0);
    }
    diag::info!(Sim, "bo2zm guest bow: arrow at power {power:.2}{}", if enchanted { " (enchanted)" } else { "" });
}

/// The nearest living, shown BO2 zombie on segment `a`..`b`: its entity,
/// the fraction along, the point and whether it is the head.
fn zombie_on_segment(world: &World, a: [f32; 3], b: [f32; 3]) -> Option<(u32, f32, [f32; 3], bool)> {
    let zm = world.resource::<Zm>();
    let d: [f32; 3] = std::array::from_fn(|k| b[k] - a[k]);
    let flat = d[0] * d[0] + d[1] * d[1];
    let mut best: Option<(u32, f32, [f32; 3], bool)> = None;
    for (n, actor) in &zm.actors {
        let Some(e) = zm.ents.get(n).filter(|e| actor.alive && !e.hidden) else {
            continue;
        };
        let o = e.origin;
        let t = if flat > 1e-6 { (((o[0] - a[0]) * d[0] + (o[1] - a[1]) * d[1]) / flat).clamp(0.0, 1.0) } else { 0.0 };
        let p: [f32; 3] = std::array::from_fn(|k| a[k] + d[k] * t);
        let (dx, dy) = (p[0] - o[0], p[1] - o[1]);
        if dx * dx + dy * dy <= ZOMBIE_RADIUS * ZOMBIE_RADIUS
            && (o[2] - 4.0..=o[2] + ZOMBIE_HEIGHT).contains(&p[2])
            && best.is_none_or(|(_, bt, _, _)| t < bt)
        {
            best = Some((*n, t, p, p[2] > o[2] + ZOMBIE_HEAD));
        }
    }
    best
}

fn length(v: [f32; 3]) -> f32 {
    (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt()
}

/// Each arrow a Minecraft tick at a time: into a wall it sticks, into a
/// zombie it strikes and goes.
fn arrows(world: &mut World, now: i64) {
    let mut clock = world.get_resource::<McSpecials>().and_then(|s| s.clock_ms).unwrap_or(now);
    let mut arrows = std::mem::take(&mut state(world).arrows);
    let pobj = world.resource::<Zm>().players.get(&0).map(|p| p.obj);
    let mut steps = 0;
    while clock + TICK_MS <= now && steps < 10 {
        clock += TICK_MS;
        steps += 1;
        let mut keep = Vec::with_capacity(arrows.len());
        for mut arrow in arrows {
            if let Some(at) = arrow.stuck {
                if now - at < STUCK_MS {
                    keep.push(arrow);
                }
                continue;
            }
            arrow.ticks += 1;
            let end: [f32; 3] = std::array::from_fn(|k| arrow.at[k] + arrow.velocity[k]);
            let tr = frame(world).trace_world(arrow.at, end, [0.0; 3], [0.0; 3], crate::bullet_collision::MASK_SHOT);
            let wall_end: [f32; 3] = std::array::from_fn(|k| arrow.at[k] + (end[k] - arrow.at[k]) * tr.fraction);
            let speed_blocks = length(arrow.velocity) / B;
            let points = (speed_blocks * (ARROW_BASE + if arrow.enchanted { POWER_V } else { 0.0 })).ceil();
            let per_point = if arrow.enchanted { ENCHANTED_PER_POINT } else { PLAIN_PER_POINT };
            let damage = points * per_point;
            match zombie_on_segment(world, arrow.at, wall_end) {
                Some((n, _, point, head)) => {
                    let amount = (damage * if head { HEAD } else { 1.0 }).round() as i32;
                    let l = length(arrow.velocity).max(1e-3);
                    let dir = arrow.velocity.map(|v| v / l);
                    if let Some(p) = pobj {
                        with_vm(world, |vm, world| {
                            super::actors::damage(
                                vm,
                                world,
                                n,
                                Value::Object(p),
                                Value::Object(p),
                                amount,
                                0,
                                "MOD_RIFLE_BULLET",
                                "m14_zm",
                                point,
                                dir,
                                if head { "head" } else { "torso_upper" },
                            );
                        });
                    }
                    guest::arrow_hit(point);
                    if arrow.enchanted {
                        state(world).burning.push((n, now + BURN_MS, now + BURN_EVERY_MS));
                    }
                    diag::info!(Sim, "bo2zm guest bow: arrow on zombie {n} for {amount}{}", if head { " (head)" } else { "" });
                }
                None if tr.fraction < 1.0 => {
                    arrow.at = wall_end;
                    arrow.stuck = Some(now);
                    guest::arrow_hit(wall_end);
                    keep.push(arrow);
                }
                None => {
                    arrow.at = end;
                    arrow.velocity = arrow.velocity.map(|v| v * ARROW_DRAG);
                    arrow.velocity[2] -= ARROW_GRAVITY;
                    if arrow.ticks < FLIGHT_TICKS {
                        keep.push(arrow);
                    }
                }
            }
        }
        arrows = keep;
    }
    if clock + TICK_MS * 10 < now {
        clock = now;
    }
    guest::set_arrows(
        arrows
            .iter()
            .map(|a| ArrowView { at: a.at, velocity: a.velocity.map(|v| v / TICK), enchanted: a.enchanted })
            .collect(),
    );
    let mut s = state(world);
    s.arrows = arrows;
    s.clock_ms = Some(clock);
}

/// Flame on the zombies the enchanted bow struck.
fn burning(world: &mut World, now: i64) {
    let Some(p) = world.resource::<Zm>().players.get(&0).map(|p| p.obj) else {
        return;
    };
    let mut fire = std::mem::take(&mut state(world).burning);
    fire.retain(|&(n, until, _)| now < until && world.resource::<Zm>().actors.get(&n).is_some_and(|a| a.alive));
    for burn in &mut fire {
        if now < burn.2 {
            continue;
        }
        burn.2 = now + BURN_EVERY_MS;
        let n = burn.0;
        let Some(at) = world.resource::<Zm>().ents.get(&n).map(|e| e.origin) else {
            continue;
        };
        with_vm(world, |vm, world| {
            super::actors::damage(vm, world, n, Value::Object(p), Value::Object(p), BURN_DAMAGE, 0, "MOD_BURNED", "m14_zm", at, [0.0, 0.0, 1.0], "torso_upper");
        });
    }
    state(world).burning = fire;
}

/// The stick's swing: the nearest zombie on his line of sight within reach
/// takes `STICK_DAMAGE` (cut by the cooldown) and, killed, flies along his
/// look. Whether it struck one.
fn stick(world: &mut World, strength: f32) -> bool {
    let Some((obj, eye, fwd)) = aim(world) else { return false };
    let end: [f32; 3] = std::array::from_fn(|k| eye[k] + fwd[k] * STICK_REACH);
    let tr = frame(world).trace_world(eye, end, [0.0; 3], [0.0; 3], crate::bullet_collision::MASK_SHOT);
    let end: [f32; 3] = std::array::from_fn(|k| eye[k] + (end[k] - eye[k]) * tr.fraction);
    let Some((n, _, point, _)) = zombie_on_segment(world, eye, end) else {
        return false;
    };
    let amount = (STICK_DAMAGE * strength.clamp(0.0, 1.0)).round().max(1.0) as i32;
    with_vm(world, |vm, world| {
        super::actors::damage(vm, world, n, Value::Object(obj), Value::Object(obj), amount, 0, "MOD_MELEE", "knife_zm", point, fwd, "torso_upper");
    });
    let dead = world.resource::<Zm>().actors.get(&n).is_none_or(|a| !a.alive);
    if dead {
        // The Thunder Gun's throw: along his look, up off the ground.
        let flat = (fwd[0] * fwd[0] + fwd[1] * fwd[1]).sqrt().max(1e-3);
        let throw = [fwd[0] / flat, fwd[1] / flat, 0.6].map(|v| v * STICK_FLING * strength.clamp(0.3, 1.0));
        super::natives_ent::go_limp(world, n);
        let zm = world.resource_mut::<Zm>().into_inner();
        if let Some(r) = zm.ents.get_mut(&n).and_then(|e| e.ragdoll.as_mut()) {
            r.launch = throw;
            r.launches = r.launches.wrapping_add(1);
        }
    }
    diag::info!(Sim, "bo2zm guest stick: zombie {n} takes {amount}{}", if dead { ", flies" } else { "" });
    true
}

/// The Gravity Gun's hit on BO2 zombie `ent`: `damage` (BO2 health) by
/// `means` at `at` going `dir`, as his Gravity Gun. One it kills (or one
/// already down) flies at `launch` (units a second).
fn hit(world: &mut World, ent: u32, damage: i32, means: &str, at: [f32; 3], dir: [f32; 3], launch: [f32; 3]) {
    let Some(obj) = world.resource::<Zm>().players.get(&0).map(|p| p.obj) else {
        return;
    };
    let alive = world.resource::<Zm>().actors.get(&ent).is_some_and(|a| a.alive);
    let weapon = match guest::hand().weapon {
        Some(Guest::SuperGravityGun) => Guest::SuperGravityGun.name(),
        _ => Guest::GravityGun.name(),
    };
    if alive && damage > 0 {
        with_vm(world, |vm, world| {
            super::actors::damage(vm, world, ent, Value::Object(obj), Value::Object(obj), damage, 0, means, weapon, at, dir, "torso_upper");
        });
    }
    let dead = world.resource::<Zm>().actors.get(&ent).is_some_and(|a| !a.alive);
    if dead {
        // A limp body the gun can carry (one in its death animation too).
        super::natives_ent::go_limp(world, ent);
    }
    if !dead || launch.iter().all(|v| v.abs() < 1.0) {
        diag::info!(Sim, "bo2zm guest gravity gun: zombie {ent} takes {damage} ({means})");
        return;
    }
    let mut zm = world.resource_mut::<Zm>();
    if let Some(ragdoll) = zm.ents.get_mut(&ent).and_then(|e| e.ragdoll.as_mut()) {
        ragdoll.launch = launch.map(|v| v / LAUNCH_UNIT);
        if alive {
            ragdoll.launches = ragdoll.launches.max(1);
        } else {
            ragdoll.launches = ragdoll.launches.wrapping_add(1);
        }
    }
    diag::info!(
        Sim,
        "bo2zm guest gravity gun: zombie {ent} {} ({damage}, {means}), thrown {:.0} u/s",
        if alive { "killed" } else { "thrown again" },
        length(launch)
    );
}

/// The bodies he holds stay (their clock starts again), and the world side
/// learns where the zombies are.
fn publish_zombies(world: &mut World, now: i64) {
    let held = guest::held_bodies();
    let mut zm = world.resource_mut::<Zm>();
    for n in &held {
        if let Some(a) = zm.actors.get_mut(n)
            && !a.alive
        {
            a.dead_since = Some(now);
        }
    }
    let zm = zm.into_inner();
    let list: Vec<GuestZombie> = zm
        .actors
        .iter()
        .filter_map(|(n, a)| {
            let e = zm.ents.get(n).filter(|e| !e.hidden)?;
            a.obj?;
            Some(GuestZombie {
                ent: *n,
                origin: e.origin,
                alive: a.alive,
                radius: a.radius.max(12.0),
                height: a.height.max(48.0),
                wire: zm.presences.by_ent.get(n).map_or(0x2000_0000 | (n & 0x0fff_ffff), |s| s.id.to_wire()),
            })
        })
        .collect();
    guest::set_zombies(list);
}
