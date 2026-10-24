//! bo2mp: the hack menu's guest weapons ("Zombies Wonder Weapons", with the
//! Ray Gun): Half-Life 2's Gravity Gun and its Super, Minecraft's bow and
//! enchanted bow and the enchanted stick. Each rides a copy of the Ray Gun
//! (`assets::lane::t6_m2`, `GUEST_WEAPONS`) so the game holds, switches and
//! reloads it; its trigger fires no Ray Gun shots (`owns_trigger`). Here
//! they hit players and bots, Minecraft's numbers at one Minecraft health
//! point to five of ours (20 points is a full 100):
//! - the bow draws while fire is held and lets an arrow go on release
//!   (`BowItem.getPowerForTime`); a full draw is a critical arrow; the plain
//!   bow spends a round of its clip a shot, the enchanted one none
//!   (Infinity), hits harder (Power V) and sets the player alight (Flame);
//! - the stick swings on each press: Sharpness X, Knockback X;
//! - the Gravity Gun is the world side's (`render_anim` guest weapons): it
//!   grabs and throws bodies, and the Super one grabs and throws live
//!   players too. What it does to them comes back here
//!   (`guest::Ask::GravHit`).
//!
//! Every client's hands are read, bots' too. Nothing of Minecraft runs:
//! these are its numbers only.

use std::collections::BTreeMap;

use bevy_ecs::prelude::{Resource, World};
use gsc_t6::Value;

use super::{Zm, frame, with_vm};
use crate::guest::{self, ArrowView, Ask, Guest, GuestZombie, Hand};
use crate::world::ClientId;

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
/// Our health per Minecraft health point (a player's 20 is our 100).
const PER_POINT: f32 = 5.0;
/// A head strike's multiplier (the game's rifles: twice).
const HEAD: f32 = 2.0;
/// Flame: 5 seconds alight, a point a second.
const BURN_MS: i64 = 5000;
const BURN_EVERY_MS: i64 = 1000;
const BURN_POINTS: f32 = 1.0;
/// An arrow in a wall stays this long; one in flight at most this long.
const STUCK_MS: i64 = 10_000;
const FLIGHT_TICKS: u32 = 1200;
const MAX_ARROWS: usize = 64;
/// A player's body for an arrow: a standing cylinder this wide, as tall as
/// his eye and a little more; the top of it is his head.
const PLAYER_RADIUS: f32 = 15.0;
const PLAYER_ABOVE_EYE: f32 = 8.0;
const PLAYER_HEAD: f32 = 14.0;
/// Minecraft's full draw: 20 ticks.
const FULL_DRAW_S: f32 = 1.0;
/// The enchanted stick: a stick's 1 point and Sharpness X's 5.5, its reach
/// (three blocks) and Minecraft's attack cooldown for a plain item (4 a
/// second).
const STICK_POINTS: f32 = 1.0 + 5.5;
const STICK_REACH: f32 = 3.0 * B;
const STICK_COOLDOWN_S: f32 = 0.25;
/// Knockback X: Minecraft throws the struck 5 blocks a tick along the
/// swing and 0.4 up, and the air barely slows him (about 35 blocks). Ours
/// has no air drag, so the push that carries him as far.
const KNOCK_FLAT: f32 = 1400.0;
const KNOCK_UP: f32 = 0.4 * B / TICK;
/// A body's throw: the clients' ragdoll takes `launchragdoll`'s vector
/// times 4 as its speed.
const LAUNCH_SCALE: f32 = 4.0;
/// A bot (or the test player) holds fire as it likes: one not seen pressing
/// it this long has let go, and its bow lets go at a full draw.
const BOT_HOLD_MS: i64 = 300;
/// MC Zombies' plain bow (`bo2mc::BOW_CLIP`, `BOW_RELOAD_TICKS`): an empty
/// clip fills again from the stock by itself, a second after the last arrow.
const BOW_CLIP: i32 = 20;
const BOW_REFILL_MS: i64 = 1000;

#[derive(Clone, Copy, Debug)]
struct Arrow {
    owner: u32,
    weapon: Guest,
    at: [f32; 3],
    /// Map units a tick.
    velocity: [f32; 3],
    crit: bool,
    stuck: Option<i64>,
    ticks: u32,
}

#[derive(Clone, Copy, Debug, Default)]
struct Hands {
    attack_was: bool,
    /// When a bot last pressed fire.
    pressed: Option<i64>,
    draw_since: Option<i64>,
    /// When the plain bow's empty clip fills again.
    refill_at: Option<i64>,
    last_swing: i64,
    hand: Hand,
}

/// Someone alight: who, who set him alight and with what, until, next burn.
#[derive(Clone, Copy, Debug)]
struct Burn {
    victim: u32,
    by: u32,
    weapon: Guest,
    until: i64,
    next: i64,
}

#[derive(Resource, Default)]
pub(super) struct McSpecials {
    arrows: Vec<Arrow>,
    clock_ms: Option<i64>,
    burning: Vec<Burn>,
    hands: BTreeMap<u32, Hands>,
    /// A body to throw once it shows: whose, his body before, the throw
    /// (`launchragdoll`'s units), until when.
    throws: Vec<(u32, Option<i32>, [f32; 3], i64)>,
    rng: u32,
    given: BTreeMap<u32, i64>,
}

fn state(world: &mut World) -> bevy_ecs::world::Mut<'_, McSpecials> {
    if !world.contains_resource::<McSpecials>() {
        world.insert_resource(McSpecials { rng: 0x9e37_79b9, ..Default::default() });
    }
    world.resource_mut::<McSpecials>()
}

/// The weapon step leaves their trigger alone: they fire no Ray Gun shots.
pub(crate) fn owns_trigger(weapon_name: &str) -> bool {
    Guest::of(weapon_name).is_some()
}

pub(super) fn tick(world: &mut World) {
    let now = world.resource::<Zm>().now_ms;
    if world.resource::<Zm>().players.is_empty() {
        return;
    }
    // The Super one's grab or punt kills any player (20 x 100).
    guest::set_zombie_health(100);
    test_give(world, now);
    asks(world, now);
    let clients: Vec<u32> = world.resource::<Zm>().players.keys().copied().collect();
    for c in clients {
        hands(world, c, now);
    }
    arrows(world, now);
    burning(world, now);
    throws(world, now);
    super::mp_portals::tick(world, now);
    publish(world);
}

/// A player who can fight: playing and alive.
fn fighting(world: &mut World, c: u32) -> bool {
    let playing = world.resource::<Zm>().players.get(&c).is_some_and(|p| p.sessionstate == "playing");
    playing && frame(world).player(ClientId(c)).is_some_and(|ps| ps.health > 0)
}

/// His eye and view direction, while he can fight.
fn aim(world: &mut World, c: u32) -> Option<([f32; 3], [f32; 3])> {
    if !fighting(world, c) {
        return None;
    }
    frame(world).player(ClientId(c)).map(|ps| {
        (
            [ps.origin[0], ps.origin[1], ps.origin[2] + ps.view_height_current],
            crate::bullet::angles_to_forward(ps.viewangles),
        )
    })
}

/// Test aid BO2MP_GUEST_NEAR=1: every 1.5 s client 0's nearest living enemy
/// is stood 70 units in front of him (for the stick and the Gravity Gun,
/// which only reach a few steps); with a bow, 300 units out (past a knife)
/// every quarter second, so an arrow has someone to meet.
fn test_near(world: &mut World, now: i64) {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    static LAST: std::sync::atomic::AtomicI64 = std::sync::atomic::AtomicI64::new(0);
    if !*ON.get_or_init(|| std::env::var("BO2MP_GUEST_NEAR").is_ok_and(|v| v == "1")) || !fighting(world, 0) {
        return;
    }
    let Some((origin, yaw, weapon)) = frame(world).player(ClientId(0)).map(|ps| (ps.origin, ps.viewangles[1].to_radians(), ps.weapon)) else {
        return;
    };
    let bow = matches!(Guest::of(&super::weapon_text(world, weapon)), Some(Guest::Bow | Guest::EnchantedBow));
    let (every, out) = if bow { (250, 300.0) } else { (1500, 70.0) };
    if now - LAST.load(std::sync::atomic::Ordering::Relaxed) < every {
        return;
    }
    let enemies = super::with_vm(world, |vm, world| super::bots::enemies_of(vm, world, 0, true)).unwrap_or_default();
    let near = enemies
        .into_iter()
        .filter_map(|e| {
            if !fighting(world, e) {
                return None;
            }
            frame(world).player(ClientId(e)).map(|ps| (e, length(std::array::from_fn(|k| ps.origin[k] - origin[k]))))
        })
        .min_by(|a, b| a.1.total_cmp(&b.1));
    let Some((enemy, _)) = near else { return };
    LAST.store(now, std::sync::atomic::Ordering::Relaxed);
    let spot = [origin[0] + yaw.cos() * out, origin[1] + yaw.sin() * out, origin[2]];
    super::teleport_player(world, ClientId(enemy), spot);
    if !bow {
        diag::info!(Sim, "bo2mp guest weapons: test stands client {enemy} in front of client 0");
    }
}

/// Test aid BO2MP_GUEST=<weapon>[,bots]: client 0 (and with `,bots` every
/// bot) is given that guest weapon in each life, a second after it starts.
fn test_give(world: &mut World, now: i64) {
    test_near(world, now);
    static GIVE: std::sync::OnceLock<Option<(String, bool)>> = std::sync::OnceLock::new();
    let Some((name, bots)) = GIVE
        .get_or_init(|| {
            let v = std::env::var("BO2MP_GUEST").ok()?;
            let (w, rest) = v.split_once(',').unwrap_or((&v, ""));
            Some((w.trim().to_owned(), rest.trim() == "bots"))
        })
        .clone()
    else {
        return;
    };
    let clients: Vec<u32> = world.resource::<Zm>().players.keys().copied().collect();
    for c in clients {
        if c != 0 && !(bots && super::is_bot(world, c)) {
            continue;
        }
        if !fighting(world, c) {
            state(world).given.remove(&c);
            continue;
        }
        let since = *state(world).given.entry(c).or_insert(now);
        if since < 0 || now - since < 1000 {
            continue;
        }
        let Ok(w) = super::weapon(world, &name) else {
            diag::warn!(Sim, "bo2mp guest weapons: test weapon {name} is not loaded");
            state(world).given.insert(c, -1);
            continue;
        };
        let id = ClientId(c);
        let mut f = frame(world);
        let given = crate::script_player::give_weapon(&mut f, id, w, false);
        crate::script_player::give_max_ammo(&mut f, id, w);
        crate::script_player::switch_to_weapon(&mut f, id, w);
        drop(f);
        state(world).given.insert(c, -1);
        diag::info!(Sim, "bo2mp guest weapons: test gave client {c} {name} ({given:?})");
    }
}

/// What the world side's Gravity Gun asks.
fn asks(world: &mut World, now: i64) {
    for a in guest::take_asks() {
        match a {
            Ask::GravHit { ent, damage, means, at, dir, launch } => grav_hit(world, now, ent, damage, &means, at, dir, launch),
        }
    }
}

/// His guest weapon out, read from his fire button: the bow draws and lets
/// go, the stick swings, the Portal Gun shoots its portals. The world side learns what his hands do.
fn hands(world: &mut World, c: u32, now: i64) {
    let id = ClientId(c);
    let Some(ps) = frame(world).player(id).copied() else {
        return;
    };
    let name = super::weapon_text(world, ps.weapon);
    let held = Guest::of(&name);
    let mut h = state(world).hands.get(&c).copied().unwrap_or_default();
    if held.is_none() && h.hand.weapon.is_none() && h.draw_since.is_none() {
        return;
    }
    let down = !fighting(world, c);
    let raw = if down { 0 } else { crate::script_player::buttons(&mut frame(world), id) };
    let pressed = raw & playerstate_t6::game::buttons::ATTACK != 0;
    // Bots and the test player tap fire on and off: held is pressed lately.
    let tapper = super::is_bot(world, c) || (c == 0 && super::autoplay::mp_enabled());
    if pressed {
        h.pressed = Some(now);
    }
    let attack = if tapper { !down && h.pressed.is_some_and(|t| now - t < BOT_HOLD_MS) } else { pressed };
    // TODO(fact): ready as our old MW2 mirror read it.
    let ready = weapon_t6::state::is_ready_as_mirrored(ps.weapon_state_t6) || weapon_t6::state::is_any_firing(ps.weapon_state_t6);
    h.hand.weapon = held;
    let mut draw_since = None;
    match held {
        Some(bow @ (Guest::Bow | Guest::EnchantedBow)) => {
            let enchanted = bow == Guest::EnchantedBow;
            let mut clip = crate::script_player::ammo_clip(&frame(world), id, ps.weapon);
            // The last arrow out starts the Ray Gun's own reload under the
            // bow (its 3 s): the bow's 1 s refill runs through it and ends it.
            let reloading = weapon_t6::state::is_any_reload(ps.weapon_state_t6);
            if !enchanted && clip == 0 && (ready || reloading) {
                let stock = crate::script_player::ammo_stock(&frame(world), id, ps.weapon);
                match h.refill_at {
                    _ if stock <= 0 => h.refill_at = None,
                    None => h.refill_at = Some(now + BOW_REFILL_MS),
                    Some(t) if now >= t => {
                        clip = stock.min(BOW_CLIP);
                        crate::script_player::set_ammo_stock(&mut frame(world), id, ps.weapon, stock - clip);
                        crate::script_player::set_ammo_clip(&mut frame(world), id, ps.weapon, clip);
                        if reloading {
                            if let Some(p) = frame(world).player_mut(id) {
                                p.weapon_time = 0;
                            }
                        }
                        h.refill_at = None;
                        diag::info!(Sim, "bo2mp guest bow: client {c}'s clip fills again ({clip}, {} left)", stock - clip);
                    }
                    Some(_) => {}
                }
            } else {
                h.refill_at = None;
            }
            // MC Zombies draws on either button; so does a player here.
            let attack = if tapper { attack } else { raw & (playerstate_t6::game::buttons::ATTACK | playerstate_t6::game::buttons::ADS) != 0 };
            let can = ready && (enchanted || clip > 0);
            let full = h.draw_since.is_some_and(|t| (now - t) as f32 / 1000.0 >= FULL_DRAW_S);
            if attack && can && !(tapper && full) {
                draw_since = Some(h.draw_since.unwrap_or(now));
            } else if let Some(t) = h.draw_since {
                // Let go (or the bow went away): Minecraft's power for the
                // draw, `(f² + 2f) / 3` of a full second's draw.
                let f = ((now - t) as f32 / 1000.0 / FULL_DRAW_S).min(1.0);
                let power = ((f * f + 2.0 * f) / 3.0).min(1.0);
                if power >= 0.1 && can {
                    if !enchanted {
                        crate::script_player::set_ammo_clip(&mut frame(world), id, ps.weapon, clip - 1);
                    }
                    fire(world, c, bow, power);
                    h.hand.shots = h.hand.shots.wrapping_add(1);
                }
            }
        }
        Some(Guest::Stick) if attack && (!h.attack_was || tapper) && ready => {
            let since = (now - h.last_swing) as f32 / 1000.0;
            // A bot swings once the stick is ready again.
            if !tapper || since >= STICK_COOLDOWN_S * 2.0 {
                // Minecraft's attack strength: 0.2 + 0.8 f², f the cooldown's share.
                let f = ((since + 0.025) / STICK_COOLDOWN_S).clamp(0.0, 1.0);
                let strength = 0.2 + 0.8 * f * f;
                h.last_swing = now;
                h.hand.swings = h.hand.swings.wrapping_add(1);
                if stick(world, c, strength, now) {
                    h.hand.hits = h.hand.hits.wrapping_add(1);
                }
            }
        }
        Some(gun) if gun.is_portalgun() && !down && ready => {
            let mut buttons = crate::script_player::buttons(&mut frame(world), id);
            if tapper {
                buttons = if attack { buttons | playerstate_t6::game::buttons::ATTACK } else { buttons & !playerstate_t6::game::buttons::ATTACK };
            }
            if let Some(orange) = super::mp_portals::hands(world, c, gun, buttons, tapper, now) {
                h.hand.orange = orange;
                h.hand.shots = h.hand.shots.wrapping_add(1);
            }
        }
        _ => {}
    }
    h.hand.draw = draw_since.map(|t| (now - t) as f32 / 1000.0);
    h.attack_was = attack;
    h.draw_since = draw_since;
    state(world).hands.insert(c, h);
    guest::set_hand(c, h.hand);
}

/// He let the bow go at this power: an arrow from his eye. A full draw is
/// critical.
fn fire(world: &mut World, c: u32, bow: Guest, power: f32) {
    let Some((eye, fwd)) = aim(world, c) else { return };
    let speed = ARROW_SPEED * power;
    let crit = power >= 1.0;
    let arrow = Arrow { owner: c, weapon: bow, at: eye, velocity: fwd.map(|v| v * speed), crit, stuck: None, ticks: 0 };
    let mut s = state(world);
    s.arrows.push(arrow);
    if s.arrows.len() > MAX_ARROWS {
        s.arrows.remove(0);
    }
    diag::info!(Sim, "bo2mp guest bow: client {c} looses an arrow at power {power:.2}{}", if bow.enchanted() { " (enchanted)" } else { "" });
}

/// The nearest fighting player but `skip` on segment `a`..`b`: his client,
/// the fraction along, the point and whether it is his head.
fn player_on_segment(world: &mut World, skip: u32, a: [f32; 3], b: [f32; 3]) -> Option<(u32, f32, [f32; 3], bool)> {
    let clients: Vec<u32> = world.resource::<Zm>().players.keys().copied().filter(|c| *c != skip).collect();
    let d: [f32; 3] = std::array::from_fn(|k| b[k] - a[k]);
    let flat = d[0] * d[0] + d[1] * d[1];
    let mut best: Option<(u32, f32, [f32; 3], bool)> = None;
    for c in clients {
        if !fighting(world, c) {
            continue;
        }
        let Some((o, top)) = frame(world).player(ClientId(c)).map(|ps| (ps.origin, ps.view_height_current + PLAYER_ABOVE_EYE)) else {
            continue;
        };
        let t = if flat > 1e-6 { (((o[0] - a[0]) * d[0] + (o[1] - a[1]) * d[1]) / flat).clamp(0.0, 1.0) } else { 0.0 };
        let p: [f32; 3] = std::array::from_fn(|k| a[k] + d[k] * t);
        let (dx, dy) = (p[0] - o[0], p[1] - o[1]);
        if dx * dx + dy * dy <= PLAYER_RADIUS * PLAYER_RADIUS
            && (o[2] - 4.0..=o[2] + top).contains(&p[2])
            && best.is_none_or(|(_, bt, _, _)| t < bt)
        {
            best = Some((c, t, p, p[2] > o[2] + top - PLAYER_HEAD));
        }
    }
    best
}

fn length(v: [f32; 3]) -> f32 {
    (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt()
}

/// Player `victim` takes `amount` from `by`'s guest weapon, through the
/// game's own damage callback (teams, kill feed, score). Whether he died.
#[allow(clippy::too_many_arguments)]
fn hurt(world: &mut World, victim: u32, by: u32, weapon: Guest, amount: i32, means: &str, point: [f32; 3], dir: [f32; 3], head: bool) -> bool {
    let Some(obj) = world.resource::<Zm>().players.get(&by).map(|p| p.obj) else {
        return false;
    };
    let name = format!("{}_mp", weapon.name());
    let name = if super::weapon(world, &name).is_ok() { name } else { weapon.name().to_owned() };
    with_vm(world, |vm, world| {
        super::natives_ai::player_damage(
            vm,
            world,
            victim,
            Value::Object(obj),
            Value::Object(obj),
            amount,
            0,
            means,
            &name,
            point,
            dir,
            if head { "head" } else { "torso_upper" },
        );
    });
    !fighting(world, victim)
}

/// Each arrow a Minecraft tick at a time: into a wall it sticks, into a
/// player it strikes and goes.
fn arrows(world: &mut World, now: i64) {
    let mut clock = world.get_resource::<McSpecials>().and_then(|s| s.clock_ms).unwrap_or(now);
    let mut arrows = std::mem::take(&mut state(world).arrows);
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
            let tr = super::shot_trace(world, arrow.at, end, crate::bullet_collision::MASK_SHOT);
            let wall_end: [f32; 3] = std::array::from_fn(|k| arrow.at[k] + (end[k] - arrow.at[k]) * tr.fraction);
            match player_on_segment(world, arrow.owner, arrow.at, wall_end) {
                Some((victim, _, point, head)) => {
                    let speed_blocks = length(arrow.velocity) / B;
                    let base = ARROW_BASE + if arrow.weapon.enchanted() { POWER_V } else { 0.0 };
                    let mut points = (speed_blocks * base).ceil();
                    if arrow.crit {
                        // Minecraft's critical arrow: up to half again and 2.
                        let s = &mut state(world).rng;
                        *s ^= *s << 13;
                        *s ^= *s >> 17;
                        *s ^= *s << 5;
                        points += (*s % (points as u32 / 2 + 2)) as f32;
                    }
                    let amount = (points * PER_POINT * if head { HEAD } else { 1.0 }).round() as i32;
                    let l = length(arrow.velocity).max(1e-3);
                    let dir = arrow.velocity.map(|v| v / l);
                    let means = if head { "MOD_HEAD_SHOT" } else { "MOD_RIFLE_BULLET" };
                    let died = hurt(world, victim, arrow.owner, arrow.weapon, amount, means, point, dir, head);
                    guest::arrow_hit(point);
                    if arrow.weapon.enchanted() && !died {
                        let burn = Burn { victim, by: arrow.owner, weapon: arrow.weapon, until: now + BURN_MS, next: now + BURN_EVERY_MS };
                        let mut s = state(world);
                        s.burning.retain(|b| b.victim != victim);
                        s.burning.push(burn);
                    }
                    diag::info!(
                        Sim,
                        "bo2mp guest bow: client {}'s arrow hits client {victim} for {amount}{}{}{}",
                        arrow.owner,
                        if head { " (head)" } else { "" },
                        if arrow.crit { " (critical)" } else { "" },
                        if died { ", killed" } else { "" }
                    );
                }
                None if tr.fraction < 1.0 => {
                    arrow.at = wall_end;
                    arrow.stuck = Some(now);
                    diag::info!(Sim, "bo2mp guest bow: client {}'s arrow sticks in a wall after {} ticks", arrow.owner, arrow.ticks);
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
            .map(|a| ArrowView { at: a.at, velocity: a.velocity.map(|v| v / TICK), enchanted: a.weapon.enchanted() })
            .collect(),
    );
    let mut s = state(world);
    s.arrows = arrows;
    s.clock_ms = Some(clock);
}

/// Flame on the players the enchanted bow struck: a point a second.
fn burning(world: &mut World, now: i64) {
    let mut fire = std::mem::take(&mut state(world).burning);
    let mut kept = Vec::with_capacity(fire.len());
    for mut burn in fire.drain(..) {
        if now >= burn.until || !fighting(world, burn.victim) {
            continue;
        }
        if now >= burn.next {
            burn.next = now + BURN_EVERY_MS;
            let at = frame(world).player(ClientId(burn.victim)).map_or([0.0; 3], |ps| ps.origin);
            let amount = (BURN_POINTS * PER_POINT).round() as i32;
            if hurt(world, burn.victim, burn.by, burn.weapon, amount, "MOD_BURNED", at, [0.0, 0.0, 1.0], false) {
                diag::info!(Sim, "bo2mp guest bow: client {} burns to death", burn.victim);
                continue;
            }
        }
        kept.push(burn);
    }
    state(world).burning = kept;
}

/// The stick's swing: the nearest player on his line of sight within reach
/// takes Sharpness X (cut by the cooldown) and Knockback X throws him (a
/// kill throws his body). Whether it struck one.
fn stick(world: &mut World, c: u32, strength: f32, now: i64) -> bool {
    let Some((eye, fwd)) = aim(world, c) else { return false };
    let end: [f32; 3] = std::array::from_fn(|k| eye[k] + fwd[k] * STICK_REACH);
    let tr = super::shot_trace(world, eye, end, crate::bullet_collision::MASK_SHOT);
    let end: [f32; 3] = std::array::from_fn(|k| eye[k] + (end[k] - eye[k]) * tr.fraction);
    let Some((victim, _, point, _)) = player_on_segment(world, c, eye, end) else {
        return false;
    };
    let strength = strength.clamp(0.0, 1.0);
    let amount = (STICK_POINTS * strength * PER_POINT).round().max(1.0) as i32;
    let before = newest_body(world, victim);
    let died = hurt(world, victim, c, Guest::Stick, amount, "MOD_MELEE", point, fwd, false);
    // Knockback X along the swing, a weak swing a weaker throw.
    let flat = (fwd[0] * fwd[0] + fwd[1] * fwd[1]).sqrt().max(1e-3);
    let knock = [fwd[0] / flat * KNOCK_FLAT, fwd[1] / flat * KNOCK_FLAT, KNOCK_UP].map(|v| v * strength.max(0.3));
    if died {
        throw_body(world, victim, before, knock.map(|v| v / LAUNCH_SCALE), now);
    } else if let Some(ps) = frame(world).player_mut(ClientId(victim)) {
        ps.velocity = [ps.velocity[0] * 0.5 + knock[0], ps.velocity[1] * 0.5 + knock[1], knock[2].max(ps.velocity[2] * 0.5 + knock[2])];
        ps.ground_entity_num = playerstate_t6::game::ENTITYNUM_NONE;
    }
    diag::info!(Sim, "bo2mp guest stick: client {c} hits client {victim} for {amount}{}", if died { ", killed, flies" } else { ", knocked back" });
    true
}

/// His newest body's entity number, if he has one.
fn newest_body(world: &mut World, client: u32) -> Option<i32> {
    super::playeranim::corpses_seen(world)
        .into_iter()
        .find(|(_, _, victim, newest)| *victim == client && *newest)
        .map(|(_, entnum, _, _)| entnum)
}

/// The body a kill leaves flies at `launch` (`launchragdoll`'s units) as
/// soon as it shows (his death may clone it a moment later).
fn throw_body(world: &mut World, client: u32, before: Option<i32>, launch: [f32; 3], now: i64) {
    state(world).throws.push((client, before, launch, now + 2000));
    throws(world, now);
}

fn throws(world: &mut World, now: i64) {
    let pending = std::mem::take(&mut state(world).throws);
    if pending.is_empty() {
        return;
    }
    let bodies = super::playeranim::corpses_seen(world);
    let mut kept = Vec::new();
    for (client, before, launch, until) in pending {
        let body = bodies.iter().find(|(_, entnum, victim, newest)| *victim == client && *newest && Some(*entnum) != before);
        match body {
            Some(&(slot, entnum, _, _)) => {
                super::playeranim::limp_corpse(world, slot, entnum, Some(launch));
                diag::info!(Sim, "bo2mp guest weapons: client {client}'s body #{entnum} thrown {:.0} u/s", length(launch) * LAUNCH_SCALE);
            }
            None if now < until => kept.push((client, before, launch, until)),
            None => diag::info!(Sim, "bo2mp guest weapons: client {client}'s body never showed to throw"),
        }
    }
    state(world).throws.extend(kept);
}

/// The Gravity Gun's hit on `ent` (a player's client, or `guest::BODY` and
/// a body's entity number): `damage` by `means`, from the player this PC
/// shows. A player it kills (or a body) flies at `launch` (units a second).
#[allow(clippy::too_many_arguments)]
fn grav_hit(world: &mut World, now: i64, ent: u32, damage: i32, means: &str, at: [f32; 3], dir: [f32; 3], launch: [f32; 3]) {
    let by = guest::viewer();
    let weapon = match state(world).hands.get(&by).and_then(|h| h.hand.weapon) {
        Some(Guest::SuperGravityGun) => Guest::SuperGravityGun,
        _ => Guest::GravityGun,
    };
    let thrown = launch.iter().any(|v| v.abs() >= 1.0);
    let launch = thrown.then(|| launch.map(|v| v / LAUNCH_SCALE));
    if ent & guest::BODY != 0 {
        let entnum = (ent & !guest::BODY) as i32;
        let slot = super::playeranim::corpses_seen(world).into_iter().find(|(_, e, _, _)| *e == entnum).map(|(s, ..)| s);
        if let Some(slot) = slot {
            super::playeranim::limp_corpse(world, slot, entnum, launch);
        }
        diag::info!(Sim, "bo2mp guest gravity gun: body #{entnum} goes limp");
        return;
    }
    if damage > 0 && fighting(world, ent) {
        let before = newest_body(world, ent);
        let died = hurt(world, ent, by, weapon, damage, means, at, dir, false);
        if died {
            throw_body(world, ent, before, launch.unwrap_or([0.0; 3]), now);
        }
        diag::info!(Sim, "bo2mp guest gravity gun: client {ent} takes {damage} ({means}){}", if died { ", killed" } else { "" });
        return;
    }
    // His body (the newest): limp, so the beam can carry it.
    let body = super::playeranim::corpses_seen(world).into_iter().find(|(_, _, v, newest)| *v == ent && *newest);
    if let Some((slot, entnum, _, _)) = body {
        super::playeranim::limp_corpse(world, slot, entnum, launch);
        diag::info!(Sim, "bo2mp guest gravity gun: client {ent}'s body #{entnum} goes limp");
    }
}

/// The world side learns where the players and bodies are: every fighting
/// player but the one this PC shows, and every body (a dead player's newest
/// one as him, so the Super one's grab carries on with his body).
fn publish(world: &mut World) {
    let viewer = guest::viewer();
    let clients: Vec<u32> = world.resource::<Zm>().players.keys().copied().collect();
    let mut list = Vec::new();
    let mut dead = Vec::new();
    for c in clients {
        if !fighting(world, c) {
            dead.push(c);
            continue;
        }
        if c == viewer {
            continue;
        }
        let Some(ps) = frame(world).player(ClientId(c)).copied() else { continue };
        list.push(GuestZombie {
            ent: c,
            origin: ps.origin,
            alive: true,
            radius: PLAYER_RADIUS,
            height: ps.view_height_current + PLAYER_ABOVE_EYE,
            wire: 0x4000_0000 | c,
        });
    }
    for (slot, entnum, victim, newest) in super::playeranim::corpses_seen(world) {
        let origin = frame(world).corpses().slots[slot].origin;
        let ent = if newest && dead.contains(&victim) { victim } else { guest::BODY | entnum as u32 };
        list.push(GuestZombie { ent, origin, alive: false, radius: 16.0, height: 20.0, wire: entnum as u32 });
    }
    guest::set_zombies(list);
}
