//! bo2mp: the guest weapons in the menu's Zombies Wonder Weapons tab: what
//! the world side draws and plays for the five the rules side runs
//! (`sim::t6::mp_specials`). Their targets are players, bots and bodies.
//!
//! - Half-Life 2's Gravity Gun and its Super: the view model
//!   (`guest_gravgun`), and what it grabs and throws. On a plain map that is
//!   BO2's zombies only: bodies (the Super one live zombies too), carried in
//!   the beam and thrown into the others (Minecraft Zombies' `gravgun_play`
//!   without the blocks and mobs).
//! - Minecraft's bow, enchanted bow and enchanted stick: the item in his
//!   hand as Minecraft draws it (its `firstperson_righthand` display, the
//!   arm transform, the swing and the bow's draw), the bow's pulling
//!   pictures, the enchanted shine, the arrows in the map, and their sounds.
//!   The pictures and sounds are Minecraft's own when its files are on this
//!   PC (`MINECRAFTOSS_ROOT`, or `bo2mp-artifacts/minecraft-26.3`), pixel
//!   stand-ins and no sounds when not. Nothing of Mojang's is in this
//!   repository.
//!
//! `render_gpu`'s guest view draws `GuestWeaponView` (render's extract).

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};

use asset_world::CollisionClipData;
use bevy::input::gamepad::Gamepad;
use bevy::prelude::*;
use bevy::window::PrimaryWindow;
use frame::{ActivePad, GamePaused, ShownStatePosted, EyeSubject};
use net::{HostViewClient, ShownWorldFrame};
use sim::guest_arms::{Ask, Guest, GuestZombie};

pub use crate::guest_gravgun::{GuestTexture, GuestVertex};
use crate::guest_gravgun::{Action, Cue, Gravgun, ViewInput};
use crate::occupancy::debris_phys::PropPhysicsShape;
use crate::occupancy::ragdoll;
use diag::info;

/// BO2's view model (the Ray Gun the five ride on) stays hidden while one is
/// out: it draws itself.
static HIDE: AtomicBool = AtomicBool::new(false);

pub fn hides_viewmodel() -> bool {
    HIDE.load(Ordering::Relaxed)
}

/// This frame's guest weapon for the renderer.
#[derive(Resource, Default, Clone)]
pub struct GuestWeaponView {
    /// The weapon in his hands: `GuestVertex` bytes in view space (x right,
    /// y up, z back), drawn with `clip`; the first `solid` indices are
    /// solid, the rest glow (the Gravity Gun's sprites).
    pub vertices: Vec<u8>,
    pub indices: Vec<u32>,
    pub solid: u32,
    pub to_clip: [f32; 16],
    /// Half-Life 2's sheets (vertices with `pad[1]` 0) and Minecraft's item
    /// pictures (`pad[1]` 1); a vertex's layer is its colour's alpha.
    pub hl2: Option<Arc<GuestTexture>>,
    pub items: Option<Arc<GuestTexture>>,
    /// The arrows in the map: `GuestVertex` bytes in map units.
    pub level_vertex_total: Vec<u8>,
    pub level_indices: Vec<u32>,
    /// Seconds, for the enchanted shine.
    pub clock: f32,
}

pub fn register(application: &mut App) {
    application.init_resource::<GuestWeaponView>()
        .add_systems(bevy::app::Update, guest_weapons.after(ShownStatePosted));
}

/// Map units to a block.
const B: f32 = 36.0;
const HARD_SURFACE_FLAG: u32 = 1;

// ---------------------------------------------------------------------------
// The Gravity Gun in the world (Half-Life 2's `weapon_physcannon`, inches as
// map units).

/// `physcannon_tracelength` 250, the Super one's 850.
const RANGE: f32 = 250.0;
const SUPER_RANGE: f32 = 850.0;
/// `physcannon_cone`: how near the aim a body must be to grab or punt.
const CONE: f32 = 0.97;
/// `physcannon_maxforce`'s throw (units a second), the Super one's half again.
const LAUNCH: f32 = 1500.0;
const SUPER_THROW: f32 = 1.5;
/// Seconds between throws (`physcannon_refire`) and before another grab.
const REFIRE: f32 = 0.5;
/// A held body's pelvis: this far ahead of the eye and below it.
const BODY_AHEAD: f32 = 64.0;
const BODY_BELOW: f32 = 6.0;
/// Half a body's size, kept out of the walls.
const BODY_HALF: f32 = 10.0;
/// A thrown body knocks over what it flies into for this long, at least
/// this fast (units a second).
const BODY_FLIGHT: f32 = 3.0;
const BODY_HIT_SPEED: f32 = 300.0;

/// Half-Life 2's impact damage for a speed (`ImpactDamageTable`).
fn impact(speed: f32) -> f32 {
    const TABLE: [(f32, f32); 5] = [(150.0, 5.0), (250.0, 10.0), (350.0, 50.0), (500.0, 100.0), (1000.0, 500.0)];
    if speed < TABLE[0].0 {
        return 0.0;
    }
    for w in TABLE.windows(2) {
        let ((s0, d0), (s1, d1)) = (w[0], w[1]);
        if speed <= s1 {
            return d0 + (d1 - d0) * (speed - s0) / (s1 - s0);
        }
    }
    TABLE[4].1
}

/// Nothing but the map between `from` and `to`, give or take a hand's
/// width at the far end: no grabbing through a wall or the floor.
fn in_sight(solids: Option<&CollisionClipData>, from: Vec3, to: Vec3) -> bool {
    let Some(solids) = solids else { return true };
    let t = solids.sweep_box(from.to_array(), to.to_array(), [0.0; 3], [0.0; 3], HARD_SURFACE_FLAG);
    t.fraction >= 1.0 || (to - from).length() * (1.0 - t.fraction) < 12.0
}

/// Where a held thing `half` units big hangs on the way from `from` to
/// `to`: short of the first wall in the way.
fn short_of_wall(solids: Option<&CollisionClipData>, from: Vec3, to: Vec3, half: f32) -> Vec3 {
    let Some(solids) = solids else { return to };
    let trace = |r: f32| solids.sweep_box(from.to_array(), to.to_array(), [-r; 3], [r; 3], HARD_SURFACE_FLAG);
    let t = trace(half);
    if !t.began_inside {
        return from.lerp(to, t.fraction);
    }
    // Up against a wall: a thin line, backed off by the size.
    let t = trace(0.0);
    if t.fraction < 1.0 {
        let reach = (to - from).length() * t.fraction;
        return from + (to - from).normalize_or_zero() * (reach - half).max(0.0);
    }
    to
}

struct Thrown {
    ent: u32,
    wire: u32,
    age: f32,
    upgraded: bool,
    struck: Vec<u32>,
    /// Where the body was last frame: a fast body crosses a zombie between
    /// frames, so the whole way is checked.
    last: Option<Vec3>,
}

/// Something to grab or punt, and how far.
enum Aim {
    Body(GuestZombie, f32),
    Zombie(GuestZombie),
}

#[derive(Default)]
struct Shown {
    holding: bool,
    open: bool,
    action: Action,
    cues: Vec<Cue>,
}

/// The gun's state between frames.
#[derive(Default)]
struct GravPlay {
    /// The body in the beam: (entity, ragdoll id).
    held: Option<(u32, u32)>,
    thrown: Vec<Thrown>,
    refire: f32,
    regrab: f32,
    body_missing: f32,
    was_open: bool,
    hum: Option<bool>,
}

struct GunFrame<'a> {
    gun: Option<bool>,
    dt: f32,
    eye: Vec3,
    look: Vec3,
    main: bool,
    secondary: bool,
    solids: Option<&'a CollisionClipData>,
}

/// Test aid BO2MP_GRAVGUN_AUTOAIM=1 (a scripted run cannot aim at zombies):
/// the gun takes any body or zombie in reach, not only one in the cone, and
/// throws at the nearest other live zombie.
fn autoaim() -> bool {
    static AUTOAIM: OnceLock<bool> = OnceLock::new();
    *AUTOAIM.get_or_init(|| std::env::var_os("BO2MP_GRAVGUN_AUTOAIM").is_some())
}

fn autoaim_look(from: Vec3, look: Vec3, skip: Option<u32>, zombies: &[GuestZombie]) -> Vec3 {
    if !autoaim() {
        return look;
    }
    zombies
        .iter()
        .filter(|z| z.living && Some(z.ent) != skip)
        .map(|z| Vec3::from_array(z.origin) + Vec3::Z * z.height * 0.5)
        // From what it throws, at one a block or more off it.
        .filter(|at| (*at - from).length() > B)
        .min_by(|lhs, rhs| (*lhs - from).length().total_cmp(&(*rhs - from).length()))
        .map_or(look, |at| (at - from).normalize_or(look))
}

/// A body's middle: its pelvis while it is a ragdoll, else its feet raised.
fn body_point(z: &GuestZombie, bodies: &HashMap<u32, ragdoll::BodySeen>) -> Vec3 {
    let feet = Vec3::from_array(z.origin);
    match bodies.get(&z.wire) {
        Some(b) if !z.living => b.at,
        _ => feet + Vec3::Z * if z.living { z.height * 0.5 } else { 10.0 },
    }
}

fn hit(ent: u32, hurt: i32, at: Vec3, dir: Vec3, launch: Vec3) {
    sim::guest_arms::ask(Ask::GravHit {
        ent,
        harm: hurt,
        means: "MOD_IMPACT".into(),
        at: at.to_array(),
        dir: dir.to_array(),
        launch: launch.to_array(),
    });
}

impl GravPlay {
    fn frame(&mut self, f: &GunFrame) -> Shown {
        let dt = f.dt.max(0.0);
        self.refire = (self.refire - dt).max(0.0);
        self.regrab = (self.regrab - dt).max(0.0);
        let zombies = sim::guest_arms::zombies();
        let mut shown = Shown::default();
        match f.gun {
            None => self.let_go(),
            Some(upgraded) => {
                let range = if upgraded { SUPER_RANGE } else { RANGE };
                // The right button takes a body (the Super one a live
                // zombie too); the left one punts a live zombie as well.
                let aim = self.aim(f, range, upgraded, &zombies);
                let punt_aim = if f.main && !upgraded && self.held.is_none() { self.aim(f, range, true, &zombies) } else { None };
                if f.secondary && self.held.is_some() {
                    self.let_go();
                    self.regrab = REFIRE;
                    shown.action = Action::AltFire;
                    shown.cues.push(Cue::Drop);
                } else if f.secondary && self.regrab <= 0.0 {
                    if let Some(a) = aim.as_ref() {
                        self.grab(f, a);
                        shown.action = Action::AltFire;
                        shown.cues.push(if self.held.is_some() { Cue::Pickup } else { Cue::TooHeavy });
                    }
                } else if f.main && self.refire <= 0.0 {
                    self.refire = REFIRE;
                    shown.action = self.fire(f, punt_aim.as_ref().or(aim.as_ref()), upgraded, &zombies);
                    shown.cues.push(if shown.action == Action::Fire { Cue::Launch } else { Cue::DryFire });
                }
                shown.open = self.held.is_some() || aim.is_some();
                if shown.open != self.was_open {
                    shown.cues.push(if shown.open { Cue::ClawsOpen } else { Cue::ClawsClose });
                }
                self.carry(f, dt, &zombies);
            }
        }
        self.was_open = shown.open;
        shown.holding = self.held.is_some();
        let hum = f.gun.filter(|_| shown.holding);
        if hum != self.hum {
            self.hum = hum;
            shown.cues.push(Cue::Hum(hum));
        }
        self.bodies_fly(dt, &zombies);
        shown
    }

    /// The nearest body (or, `live`, zombie) in reach and in the cone.
    fn aim(&self, f: &GunFrame, range: f32, live: bool, zombies: &[GuestZombie]) -> Option<Aim> {
        let bodies = ragdoll::bodies_seen();
        let cone = if autoaim() { -1.0 } else { CONE };
        let mut best: Option<Aim> = None;
        let mut best_d = f32::MAX;
        for z in zombies {
            if z.living && !live {
                continue;
            }
            let at = body_point(z, &bodies);
            let to = at - f.eye;
            let d = to.length();
            if d > range + z.reach || to.normalize_or_zero().dot(f.look) < cone || !in_sight(f.solids, f.eye, at) {
                continue;
            }
            if d < best_d {
                best_d = d;
                best = Some(if z.living { Aim::Zombie(*z) } else { Aim::Body(*z, d) });
            }
        }
        best
    }

    fn grab(&mut self, f: &GunFrame, aim: &Aim) {
        match aim {
            Aim::Body(z, _) => {
                // One still in its death animation goes limp, so the beam
                // can carry it.
                hit(z.ent, 0, Vec3::from_array(z.origin), f.look, Vec3::ZERO);
                info!(World, "gravity gun: grabbed body {} (#{})", z.ent, z.wire);
                self.held = Some((z.ent, z.wire));
                self.body_missing = 0.0;
            }
            Aim::Zombie(z) => {
                // The Super one's grab kills it, then holds the body.
                let damage = sim::guest_arms::zombie_health() * 20;
                hit(z.ent, damage, f.eye + f.look * B, f.look, Vec3::ZERO.with_z(40.0));
                info!(World, "gravity gun: grabbed live zombie {} (#{})", z.ent, z.wire);
                self.held = Some((z.ent, z.wire));
                self.body_missing = 0.0;
            }
        }
    }

    fn fire(&mut self, f: &GunFrame, aim: Option<&Aim>, upgraded: bool, zombies: &[GuestZombie]) -> Action {
        let throw = LAUNCH * if upgraded { SUPER_THROW } else { 1.0 };
        self.regrab = REFIRE;
        if let Some((ent, wire)) = self.held.take() {
            let from = ragdoll::bodies_seen().get(&wire).map_or(f.eye, |b| b.at);
            let look = autoaim_look(from, f.look, Some(ent), zombies);
            info!(World, "gravity gun: threw body {ent} (#{wire}) at {throw:.0} u/s");
            ragdoll::beam_throw(wire, look * throw);
            self.thrown.push(Thrown { ent, wire, age: 0.0, upgraded, struck: Vec::new(), last: None });
            return Action::Fire;
        }
        // Nothing held: a punt on what it points at.
        let look = autoaim_look(f.eye, f.look, aim.map(|a| match a {
            Aim::Body(z, _) | Aim::Zombie(z) => z.ent,
        }), zombies);
        match aim {
            Some(Aim::Body(z, d)) if *d <= RANGE || upgraded => {
                info!(World, "gravity gun: punted body {} (#{})", z.ent, z.wire);
                ragdoll::beam_throw(z.wire, look * throw);
                self.thrown.push(Thrown { ent: z.ent, wire: z.wire, age: 0.0, upgraded, struck: Vec::new(), last: None });
                Action::Fire
            }
            Some(Aim::Zombie(z)) => {
                // It hits as a thing thrown at the punt's speed would (Half-
                // Life 2's impact table, 500); one it kills flies off as a
                // body that knocks down others. The Super one kills any.
                let damage = if upgraded { sim::guest_arms::zombie_health() * 20 } else { impact(throw).round() as i32 };
                info!(World, "gravity gun: punted zombie {} (#{}) for {damage}", z.ent, z.wire);
                hit(z.ent, damage, f.eye + f.look * B, look, look * throw);
                self.thrown.push(Thrown { ent: z.ent, wire: z.wire, age: 0.0, upgraded, struck: Vec::new(), last: None });
                Action::Fire
            }
            _ => Action::DryFire,
        }
    }

    /// A held body is pulled along ahead of the eye and kept.
    fn carry(&mut self, f: &GunFrame, dt: f32, zombies: &[GuestZombie]) {
        let Some((ent, mut wire)) = self.held else { return };
        // bo2mp: a player the Super one killed in its grab is a new body
        // (the corpse's own entity): the beam takes that one.
        if let Some(z) = zombies.iter().find(|z| z.ent == ent && !z.living && z.wire != wire) {
            ragdoll::beam_hold(wire, None);
            wire = z.wire;
            self.held = Some((ent, wire));
        }
        // bo2mp: once he respawns his body goes by its own number: the
        // beam keeps it.
        let mut ent = ent;
        if let Some(z) = zombies.iter().find(|z| z.wire == wire && z.ent != ent && !z.living) {
            ent = z.ent;
            self.held = Some((ent, wire));
        }
        let point = f.eye + f.look * BODY_AHEAD - Vec3::Z * BODY_BELOW;
        let point = short_of_wall(f.solids, f.eye, point, BODY_HALF);
        ragdoll::beam_hold(wire, Some(point));
        let there = zombies.iter().any(|z| z.ent == ent);
        self.body_missing = if there { 0.0 } else { self.body_missing + dt };
        if self.body_missing > 1.0 {
            info!(World, "gravity gun: body {ent} gone, let go");
            self.let_go();
        }
    }

    /// Drops what it holds: the body goes limp.
    fn let_go(&mut self) {
        if let Some((_held, wire)) = self.held.take() {
            ragdoll::beam_hold(wire, None);
        }
    }

    /// Thrown bodies knock down the live zombies they fly into.
    fn bodies_fly(&mut self, dt: f32, zombies: &[GuestZombie]) {
        if self.thrown.is_empty() {
            return;
        }
        let bodies = ragdoll::bodies_seen();
        for t in &mut self.thrown {
            t.age += dt;
            let Some(seen) = bodies.get(&t.wire) else { continue };
            let now = seen.at;
            let from = t.last.replace(now).unwrap_or(now);
            let speed = seen.speed.length();
            if speed < BODY_HIT_SPEED {
                continue;
            }
            let damage = impact(speed).min(500.0) * 2.0 * if t.upgraded { 2.0 } else { 1.0 };
            for z in zombies {
                if !z.living || z.ent == t.ent || t.struck.contains(&z.ent) {
                    continue;
                }
                let middle = Vec3::from_array(z.origin) + Vec3::Z * z.height * 0.5;
                let way = now - from;
                let at = from + way * ((middle - from).dot(way) / way.length_squared().max(1e-3)).clamp(0.0, 1.0);
                let near = Vec2::new(at.x - z.origin[0], at.y - z.origin[1]).length() < z.reach + 24.0
                    && at.z > z.origin[2] - 12.0
                    && at.z < z.origin[2] + z.height + 12.0;
                if !near {
                    continue;
                }
                t.struck.push(z.ent);
                info!(World, "gravity gun: body {} hits zombie {} at {speed:.0} u/s for {damage:.0}", t.ent, z.ent);
                hit(z.ent, damage.round() as i32, seen.at, seen.speed.normalize_or_zero(), seen.speed * 0.7);
            }
        }
        self.thrown.retain(|t| t.age < BODY_FLIGHT);
    }
}

// ---------------------------------------------------------------------------
// Minecraft's bow and stick.

/// The item pictures' layers.
const BOW: u8 = 0;
const BOW_PULLING: &[u8; 3] = &[1, 2, 3];
const STICK: u8 = 4;
const ARROW: u8 = 5;
const PICTURES: [&str; 6] = ["bow", "bow_pulling_0", "bow_pulling_1", "bow_pulling_2", "stick", "arrow"];
const PIXELS: u32 = 16;
/// Ticks of a swing (`LivingEntity.getCurrentSwingDuration`).
const SWING_TICKS: f32 = 6.0;
/// The sounds read (`sounds/` paths, without `.ogg`).
const MC_SOUNDS: [&str; 22] = [
    "random/bow",
    "random/bowhit1",
    "random/bowhit2",
    "random/bowhit3",
    "random/bowhit4",
    "entity/player/attack/sweep1",
    "entity/player/attack/sweep2",
    "entity/player/attack/sweep3",
    "entity/player/attack/sweep4",
    "entity/player/attack/sweep5",
    "entity/player/attack/sweep6",
    "entity/player/attack/sweep7",
    "entity/player/attack/strong1",
    "entity/player/attack/strong2",
    "entity/player/attack/strong3",
    "entity/player/attack/strong4",
    "entity/player/attack/strong5",
    "entity/player/attack/strong6",
    "entity/player/attack/knockback1",
    "entity/player/attack/knockback2",
    "entity/player/attack/knockback3",
    "entity/player/attack/knockback4",
];

/// One face of an extruded picture, in the item's own block (0..1).
struct Quad {
    corners: [Vec3; 4],
    uv: [[f32; 2]; 4],
    shade: f32,
}

struct Items {
    texture: Arc<GuestTexture>,
    /// Each picture's faces (Minecraft's `item/generated`: front, back and
    /// an edge round every pixel).
    quads: Vec<Vec<Quad>>,
    sounds: HashMap<&'static str, Arc<[u8]>>,
}

/// Minecraft's files on this PC: `MINECRAFTOSS_ROOT`, or the copy the game
/// keeps beside it (or where it runs from).
fn minecraft_assets() -> Option<PathBuf> {
    let under = |root: PathBuf| root.join("resourcepacks/local/minecraft-26.3/assets/minecraft");
    let mut roots = Vec::new();
    if let Some(r) = std::env::var_os("MINECRAFTOSS_ROOT") {
        roots.push(PathBuf::from(r));
    }
    if let Some(dir) = std::env::current_exe().ok().and_then(|p| p.parent().map(PathBuf::from)) {
        roots.push(dir.join("bo2mp-artifacts/minecraft-26.3"));
    }
    roots.push(PathBuf::from("bo2mp-artifacts/minecraft-26.3"));
    roots.into_iter().map(under).find(|p| p.join("textures/item/bow.png").is_file())
}

/// A 16-pixel picture: Minecraft's file (its top square, kept sharp), or
/// the stand-in.
fn picture(assets: Option<&PathBuf>, name: &str) -> Vec<u8> {
    let read = assets.and_then(|a| std::fs::read(a.join(format!("textures/item/{name}.png"))).ok());
    let decoded = read.and_then(|bytes| image::load_from_memory(&bytes).ok()).map(|i| i.to_rgba8());
    match decoded {
        Some(img) if img.width() > 0 => {
            let side = img.width().min(img.height());
            let mut out = Vec::with_capacity((PIXELS * PIXELS * 4) as usize);
            for y in 0..PIXELS {
                for x in 0..PIXELS {
                    out.extend_from_slice(&img.get_pixel(x * side / PIXELS, y * side / PIXELS).0);
                }
            }
            out
        }
        _ => standin(name),
    }
}

/// Pixel stand-ins in Minecraft's style when its files are not here.
fn standin(name: &str) -> Vec<u8> {
    let mut px = vec![0u8; (PIXELS * PIXELS * 4) as usize];
    let mut put = |x: i32, y: i32, c: [u8; 4]| {
        if (0..PIXELS as i32).contains(&x) && (0..PIXELS as i32).contains(&y) {
            let at = ((y as u32 * PIXELS + x as u32) * 4) as usize;
            px[at..at + 4].copy_from_slice(&c);
        }
    };
    const WOOD: [u8; 4] = [137, 103, 39, 255];
    const DARK: [u8; 4] = [73, 54, 21, 255];
    const STRING: [u8; 4] = [220, 220, 220, 255];
    const STONE: [u8; 4] = [150, 150, 150, 255];
    const FEATHER: [u8; 4] = [240, 240, 240, 255];
    let pull = match name {
        "bow_pulling_0" => Some(1),
        "bow_pulling_1" => Some(2),
        "bow_pulling_2" => Some(3),
        _ => None,
    };
    match name {
        "stick" => {
            for x in 3..13 {
                put(x, 15 - x, WOOD);
                put(x + 1, 15 - x, DARK);
            }
        }
        "arrow" => {
            for x in 3..12 {
                put(x, 15 - x, WOOD);
            }
            for (x, y) in [(11, 2), (12, 2), (13, 2), (13, 3), (13, 4), (12, 3), (11, 3), (12, 4)] {
                put(x, y, STONE);
            }
            for (x, y) in [(2, 11), (2, 12), (2, 13), (3, 13), (4, 13), (3, 12), (1, 13), (2, 14)] {
                put(x, y, FEATHER);
            }
        }
        _ => {
            // The limbs: a quarter circle from top right to bottom left.
            for y in 0..PIXELS as i32 {
                for x in 0..PIXELS as i32 {
                    let d = (((x - 2) * (x - 2) + (y - 2) * (y - 2)) as f32).sqrt();
                    if x >= 2 && y >= 2 && (d - 11.0).abs() < 0.75 {
                        put(x, y, WOOD);
                    }
                }
            }
            // The string, pulled back toward the grip as it is drawn, and the
            // arrow on it.
            let p = pull.unwrap_or(0);
            for i in 0..=11i32 {
                let (x, y) = (13 - i, 2 + i);
                let bend = p - (i - 5).abs().min(p);
                put(x - bend, y - bend, STRING);
            }
            if pull.is_some() {
                for i in 3 + p..13 {
                    put(i, i, WOOD);
                }
                put(13, 13, STONE);
                put(12, 13, STONE);
                put(13, 12, STONE);
            }
        }
    }
    px
}

/// Minecraft's `item/generated` faces for a picture (a pixel deep).
fn extrude(rgba: &[u8]) -> Vec<Quad> {
    let n = PIXELS as i32;
    let opaque = |x: i32, y: i32| x >= 0 && y >= 0 && x < n && y < n && rgba[((y * n + x) * 4 + 3) as usize] != 0;
    let (z0, z1) = (7.5 / 16.0, 8.5 / 16.0);
    let mut quads = vec![
        // South (front) and north (back): the whole picture.
        Quad {
            corners: [Vec3::ZERO.with_z(z1), Vec3::X.with_z(z1), Vec3::ONE.with_z(z1), Vec3::Y.with_z(z1)],
            uv: [[0.0, 1.0], [1.0, 1.0], [1.0, 0.0], [0.0, 0.0]],
            shade: 1.0,
        },
        Quad {
            corners: [Vec3::X.with_z(z0), Vec3::ZERO.with_z(z0), Vec3::Y.with_z(z0), Vec3::ONE.with_z(z0)],
            uv: [[1.0, 1.0], [0.0, 1.0], [0.0, 0.0], [1.0, 0.0]],
            shade: 1.0,
        },
    ];
    let w = n as f32;
    for y in 0..n {
        for x in 0..n {
            if !opaque(x, y) {
                continue;
            }
            let (left, right) = (x as f32 / w, (x + 1) as f32 / w);
            let (top, bottom) = (1.0 - y as f32 / w, 1.0 - (y + 1) as f32 / w);
            let uv = [(x as f32 + 0.5) / w, (y as f32 + 0.5) / w];
            let mut side = |corners: [Vec3; 4], shade: f32| quads.push(Quad { corners, uv: [uv; 4], shade });
            if !opaque(x, y - 1) {
                side([Vec3::new(left, top, z1), Vec3::new(right, top, z1), Vec3::new(right, top, z0), Vec3::new(left, top, z0)], 1.0);
            }
            if !opaque(x, y + 1) {
                side([Vec3::new(left, bottom, z0), Vec3::new(right, bottom, z0), Vec3::new(right, bottom, z1), Vec3::new(left, bottom, z1)], 0.5);
            }
            if !opaque(x - 1, y) {
                side([Vec3::new(left, bottom, z0), Vec3::new(left, bottom, z1), Vec3::new(left, top, z1), Vec3::new(left, top, z0)], 0.6);
            }
            if !opaque(x + 1, y) {
                side([Vec3::new(right, bottom, z1), Vec3::new(right, bottom, z0), Vec3::new(right, top, z0), Vec3::new(right, top, z1)], 0.6);
            }
        }
    }
    quads
}

fn load_items() -> Items {
    let assets = minecraft_assets();
    match &assets {
        Some(a) => diag::info!(World, "guest bow/stick: Minecraft's files at {}", a.display()),
        None => diag::info!(World, "guest bow/stick: no Minecraft files; pixel stand-ins, no sounds"),
    }
    let pictures: Vec<Vec<u8>> = PICTURES.iter().map(|name| picture(assets.as_ref(), name)).collect();
    let quads = pictures.iter().map(|p| extrude(p)).collect();
    let mut sound_list = HashMap::new();
    if let Some(a) = &assets {
        for name in MC_SOUNDS {
            if let Ok(bytes) = std::fs::read(a.join(format!("sounds/{name}.ogg"))) {
                sound_list.insert(name, Arc::from(bytes));
            }
        }
    }
    let layers = pictures.into_iter().map(|p| vec![p]).collect();
    Items { texture: Arc::new(GuestTexture { size: PIXELS, layers }), quads, sounds: sound_list }
}

/// The item's `firstperson_righthand` display (every one of the three:
/// `item/generated`, `handheld`, `bow`).
fn display() -> Mat4 {
    Mat4::from_translation(Vec3::new(1.13, 3.2, 1.13) / 16.0)
        * Mat4::from_rotation_y((-90.0f32).to_radians())
        * Mat4::from_rotation_z(25.0f32.to_radians())
        * Mat4::from_scale(Vec3::splat(0.68))
        * Mat4::from_translation(Vec3::splat(-0.5))
}

/// The arm's place (`applyItemArmTransform`) and the swing.
fn item_pose(swing: f32) -> Mat4 {
    let root = swing.sqrt();
    let x = -0.4 * (root * std::f32::consts::PI).sin();
    let y = 0.2 * (root * std::f32::consts::TAU).sin();
    let z = -0.2 * (swing * std::f32::consts::PI).sin();
    let y_rotation = (swing * swing * std::f32::consts::PI).sin();
    let xz_rotation = (root * std::f32::consts::PI).sin();
    Mat4::from_translation(Vec3::new(0.56, -0.52, -0.72))
        * Mat4::from_translation(Vec3::new(x, y, z))
        * Mat4::from_rotation_y((45.0 - 20.0 * y_rotation).to_radians())
        * Mat4::from_rotation_z((-20.0 * xz_rotation).to_radians())
        * Mat4::from_rotation_x((-80.0 * xz_rotation).to_radians())
        * Mat4::from_rotation_y((-45.0f32).to_radians())
        * display()
}

/// Vanilla's `BOW` use pose: turned across, pulled back and stretched with
/// the draw, trembling past a tenth of full. `ticks` drawn.
fn bow_pose(ticks: f32) -> Mat4 {
    let mut pose = Mat4::from_translation(Vec3::new(0.56, -0.52, -0.72))
        * Mat4::from_translation(Vec3::new(-0.278_568_2, 0.183_443_87, 0.157_315_31))
        * Mat4::from_rotation_x((-13.935f32).to_radians())
        * Mat4::from_rotation_y(35.3f32.to_radians())
        * Mat4::from_rotation_z((-9.785f32).to_radians());
    let f = ticks / 20.0;
    let f = ((f * f + f * 2.0) / 3.0).min(1.0);
    if f > 0.1 {
        let shake = ((ticks - 0.1) * 1.3).sin() * (f - 0.1);
        pose *= Mat4::from_translation(Vec3::new(0.0, shake * 0.004, 0.0));
    }
    pose * Mat4::from_translation(Vec3::new(0.0, 0.0, f * 0.04))
        * Mat4::from_scale(Vec3::new(1.0, 1.0, 1.0 + f * 0.2))
        * Mat4::from_rotation_y((-45.0f32).to_radians())
        * display()
}

fn push_quads(verts: &mut Vec<GuestVertex>, indices_in: &mut Vec<u32>, quads_in: &[Quad], pose: Mat4, layer: u8, enchanted: bool) {
    for q in quads_in {
        let base = verts.len() as u32;
        let c = (q.shade * 255.0).round() as u8;
        for k in 0..4 {
            verts.push(GuestVertex {
                position: pose.transform_point3(q.corners[k]).to_array(),
                tex_uv: q.uv[k],
                tint: [c, c, c, layer],
                light: [240, 240],
                pad: [if enchanted { 255 } else { 0 }, 255],
            });
        }
        indices_in.extend([base, base + 1, base + 2, base, base + 2, base + 3]);
    }
}

/// Minecraft's first-person projection (70 degrees up, reverse-Z, no far
/// plane), in blocks.
fn hand_clip(aspect: f32) -> [f32; 16] {
    let f = 1.0 / 35.0f32.to_radians().tan();
    let near = 0.05;
    [
        f / aspect.max(0.1), 0.0, 0.0, 0.0, //
        0.0, f, 0.0, 0.0, //
        0.0, 0.0, 0.0, -1.0, //
        0.0, 0.0, near, 0.0,
    ]
}

/// A portal on a wall or a floor: one quad the shader cuts to an oval
/// (`pad.y` 128), blue or orange, darker inside once the pair is linked.
fn push_portal(vertices_v: &mut Vec<GuestVertex>, indices_val: &mut Vec<u32>, p: &sim::guest_arms::GuestPortal) {
    let n = Vec3::from_array(p.face_dir).normalize_or(Vec3::Z);
    let up = Vec3::from_array(p.up_dir).normalize_or(Vec3::Y);
    let right = up.cross(n).normalize_or(Vec3::X);
    let [half_w, half_h] = sim::guest_arms::PORTAL_HALF;
    // Just off the surface, so the wall does not cover it.
    let centre = Vec3::from_array(p.centre) + n * 0.5;
    let colour = if p.orange { [255, 140, 20] } else { [40, 140, 255] };
    // MC Zombies' (`portal_mesh`): it grows from a dot in a fifth of a
    // second; the Super one's halo reaches 1.45 times past the oval.
    let grow = (p.age / 0.2).clamp(0.05, 1.0);
    let reach = if p.upgraded { 1.45 } else { 1.0 };
    let base = vertices_v.len() as u32;
    for (u, v) in [(-1.0f32, -1.0f32), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)] {
        let (u, v) = (u * reach, v * reach);
        vertices_v.push(GuestVertex {
            position: (centre + right * u * half_w + up * v * half_h).to_array(),
            tex_uv: [u, v],
            tint: [colour[0], colour[1], colour[2], 0],
            // The Super bit, and which gun's pair it is: the oval shows the
            // view through its partner (`portal_view`) when it is that pair.
            light: [(grow * 255.0).round() as u8, if p.upgraded { 128 } else { 0 } | (p.pair & 127) as u8],
            // Linked: 160 the blue end, 200 the orange.
            pad: [match (p.paired, p.orange) { (false, _) => 0, (true, false) => 160, (true, true) => 200 }, 128],
        });
    }
    indices_val.extend([base, base + 1, base + 2, base, base + 2, base + 3]);
}

/// An arrow in the map: Minecraft's arrow picture on two crossed planes
/// along its flight, its tip at `at`.
fn push_arrow(vertices_in: &mut Vec<GuestVertex>, indices_v: &mut Vec<u32>, at: Vec3, heading: Vec3, enchanted: bool) {
    let d = heading.normalize_or(Vec3::X);
    let s1 = d.cross(Vec3::Z).normalize_or(Vec3::Y);
    let s2 = d.cross(s1).normalize_or(Vec3::Z);
    // Half its length: Minecraft's arrow is a little under a block.
    let k = 14.0;
    let centre = at - d * k * 0.8;
    for side in [s1, s2] {
        let base = vertices_in.len() as u32;
        for (u, v) in [(0.0f32, 1.0f32), (1.0, 1.0), (1.0, 0.0), (0.0, 0.0)] {
            // The picture's diagonal (fletching bottom left, tip top right)
            // lies along the flight.
            let p = centre + d * (u - v) * k + side * (u + v - 1.0) * k;
            vertices_in.push(GuestVertex {
                position: p.to_array(),
                tex_uv: [u, v],
                tint: [255, 255, 255, ARROW],
                light: [240, 240],
                pad: [if enchanted { 255 } else { 0 }, 255],
            });
        }
        indices_v.extend([base, base + 1, base + 2, base, base + 2, base + 3]);
    }
}

// ---------------------------------------------------------------------------

#[derive(Default)]
struct State {
    gravgun: Gravgun,
    play: GravPlay,
    items: Option<Items>,
    /// The hand's counters last frame, and the draw.
    last: sim::guest_arms::Hand,
    /// Seconds into a swing.
    swing: Option<f32>,
    clock: f32,
    rng: u32,
}

impl State {
    fn random(&mut self) -> f32 {
        self.rng = self.rng.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        (self.rng >> 8) as f32 / (1u32 << 24) as f32
    }

    /// One of Minecraft's sounds (none without its files).
    fn mc_sound(&mut self, queue: &mut Vec<(audio::GuestSoundRequest, Option<Vec3>)>, sound_name: &'static str, vol: f32, pitch: f32, at: Option<Vec3>) {
        let Some(bytes) = self.items.as_ref().and_then(|i| i.sounds.get(sound_name)).cloned() else { return };
        if std::env::var_os("BO2MP_SOUND_LOG").is_some() {
            info!(World, "guest sound: Minecraft {sound_name}");
        }
        queue.push((audio::GuestSoundRequest { key: format!("mc:{sound_name}"), bytes, volume: vol, pitch_mul: pitch, hold: None }, at));
    }

    fn pick(&mut self, names: &[&'static str]) -> &'static str {
        names[((self.random() * names.len() as f32) as usize).min(names.len() - 1)]
    }
}

#[allow(clippy::too_many_arguments, reason = "kept as written")]
fn guest_weapons(
    presented: Res<ShownWorldFrame>,
    local: Res<HostViewClient>,
    subject: Res<EyeSubject>,
    time_in: Res<Time>,
    paused: Option<Res<GamePaused>>,
    buttons: Option<Res<ButtonInput<MouseButton>>>,
    pad: Option<Res<ActivePad>>,
    pads: Query<&Gamepad>,
    windows_in: Query<&Window, With<PrimaryWindow>>,
    solids: Option<Res<PropPhysicsShape>>,
    sounds: Option<ResMut<audio::GuestSoundQueue>>,
    mut view: ResMut<GuestWeaponView>,
    mut state: Local<State>,
) {
    let state = &mut *state;
    let paused = paused.is_some_and(|p| p.0);
    let dt = if paused { 0.0 } else { time_in.delta_secs() };
    state.clock += dt;
    sim::guest_arms::set_viewer(local.0.0);
    let hand = sim::guest_arms::hand();
    let pstate = presented.player(local.0).copied();
    let alive = pstate.is_some_and(|pstate| pstate.movement_mode == 0 || pstate.movement_mode == 2);
    let third = crate::occupancy::other_view::shown_in_third_person(&presented, local.0, subject.replaying_final_death());
    let weapon = hand.weapon.filter(|_| alive && !third);
    HIDE.store(hand.weapon.is_some(), Ordering::Relaxed);

    let mut queue = Vec::new();
    let mut out = GuestWeaponView { clock: state.clock, ..Default::default() };
    let aspect = windows_in.iter().next().map_or(16.0 / 9.0, |w| w.width() / w.height().max(1.0));
    let (eye, look, pitch) = match pstate {
        Some(pstate) => {
            let (p, y) = (pstate.look_angles[0].to_radians(), pstate.look_angles[1].to_radians());
            let look = Vec3::new(p.cos() * y.cos(), p.cos() * y.sin(), -p.sin());
            (Vec3::from_array(pstate.origin) + Vec3::Z * pstate.eye_height_now, look, pstate.look_angles[0])
        }
        None => (Vec3::ZERO, Vec3::X, 0.0),
    };

    // The Gravity Gun: what it does, then its view model and sounds.
    let pad = pad.and_then(|p| p.0).and_then(|e| pads.get(e).ok());
    let trigger = |button: GamepadButton| pad.is_some_and(|p| p.just_pressed(button));
    let free = !paused;
    let gun = weapon.filter(|w| w.is_gravgun()).map(|w| w == Guest::SuperGravityGun);
    let primary = free && (buttons.as_ref().is_some_and(|b| b.just_pressed(MouseButton::Left)) || trigger(GamepadButton::RightTrigger2));
    let secondary = free && (buttons.as_ref().is_some_and(|b| b.just_pressed(MouseButton::Right)) || trigger(GamepadButton::LeftTrigger2));
    let clip_ref = solids.as_ref().and_then(|c| c.0.as_deref());
    let shown = state.play.frame(&GunFrame { gun, dt, eye, look, main: primary, secondary, solids: clip_ref });
    let mut cue_sounds = Vec::new();
    state.gravgun.play_cues(&shown.cues, &mut cue_sounds);
    queue.extend(cue_sounds.into_iter().map(|s| (s, None)));
    // Minecraft's axes for the view model: x, then up, then south.
    let mc = |v: Vec3| [v.x, v.z, -v.y];
    let feet = pstate.map_or([0.0; 3], |pstate| [pstate.origin[0] as f64 / 36.0, pstate.origin[2] as f64 / 36.0, -pstate.origin[1] as f64 / 36.0]);
    // The Portal Gun shows in the same view (Half-Life 2's hand): each
    // portal shot is its fire.
    let portal = weapon.filter(|w| w.is_portalgun());
    let portal_shot = portal.is_some() && hand.shots != state.last.shots;
    let gun_name = match (gun, portal) {
        (Some(true), _) => Some("physcannon_upgraded_zm"),
        (Some(false), _) => Some("physcannon_zm"),
        (None, Some(Guest::SuperPortalGun)) => Some("portalgun_upgraded_zm"),
        (None, Some(_)) => Some("portalgun_zm"),
        (None, None) => None,
    };
    let gun_view = state.gravgun.view(&ViewInput {
        weapon: gun_name.map(str::to_owned),
        dt,
        look: mc(look),
        tilt: pitch,
        feet,
        aspect_ratio: aspect,
        eye_light: [15.0, 0.0],
        holding: shown.holding,
        open: shown.open,
        action: if portal_shot { Action::Fire } else if portal.is_some() { Action::None } else { shown.action },
        orange: hand.orange,
    });
    if let Some(mesh) = gun_view {
        out.vertices = mesh.verts;
        out.indices = mesh.idx_list;
        out.solid = mesh.solid_count;
        out.to_clip = mesh.to_clip;
        out.hl2 = Some(mesh.texture);
    }

    // The bow and the stick.
    let items_needed = weapon.is_some_and(|w| !w.is_gravgun() && !w.is_portalgun()) || !sim::guest_arms::arrows().is_empty();
    if items_needed && state.items.is_none() {
        state.items = Some(load_items());
    }
    if hand.swings != state.last.swings {
        state.swing = Some(0.0);
        if hand.struck != state.last.struck {
            let name = state.pick(&[
                "entity/player/attack/strong1",
                "entity/player/attack/strong2",
                "entity/player/attack/strong3",
                "entity/player/attack/strong4",
                "entity/player/attack/strong5",
                "entity/player/attack/strong6",
            ]);
            state.mc_sound(&mut queue, name, 1.0, 1.0, None);
            let name = state.pick(&[
                "entity/player/attack/knockback1",
                "entity/player/attack/knockback2",
                "entity/player/attack/knockback3",
                "entity/player/attack/knockback4",
            ]);
            state.mc_sound(&mut queue, name, 1.0, 1.0, None);
        } else {
            let name = state.pick(&[
                "entity/player/attack/sweep1",
                "entity/player/attack/sweep2",
                "entity/player/attack/sweep3",
                "entity/player/attack/sweep4",
                "entity/player/attack/sweep5",
                "entity/player/attack/sweep6",
                "entity/player/attack/sweep7",
            ]);
            state.mc_sound(&mut queue, name, 1.0, 1.0, None);
        }
    }
    if hand.shots != state.last.shots && !portal_shot {
        // `BowItem.releaseUsing`: the shot's pitch rises with its power.
        let f = state.last.draw.unwrap_or(1.0).min(1.0);
        let power = ((f * f + 2.0 * f) / 3.0).min(1.0);
        let pitch = 1.0 / (state.random() * 0.4 + 1.2) + power * 0.5;
        state.mc_sound(&mut queue, "random/bow", 1.0, pitch, None);
    }
    for at in sim::guest_arms::take_arrow_hits() {
        let name = state.pick(&["random/bowhit1", "random/bowhit2", "random/bowhit3", "random/bowhit4"]);
        let pitch = 1.2 / (state.random() * 0.2 + 0.9);
        state.mc_sound(&mut queue, name, 1.0, pitch, Some(Vec3::from_array(at)));
    }
    if let Some(t) = state.swing.as_mut() {
        *t += dt;
        if *t >= SWING_TICKS / 20.0 {
            state.swing = None;
        }
    }
    state.last = hand;
    if let (Some(w), Some(items)) = (weapon.filter(|w| !w.is_gravgun() && !w.is_portalgun()), state.items.as_ref()) {
        let mut vertices = Vec::new();
        let mut index_list = Vec::new();
        let (layer, pose) = match (w, hand.draw) {
            (Guest::Bow | Guest::EnchantedBow, Some(t)) => {
                // Minecraft's pulling pictures: 0, then 1 from 0.65 of a
                // full draw, 2 from 0.9.
                let stage = if t >= 0.9 { 2 } else if t >= 0.65 { 1 } else { 0 };
                (BOW_PULLING[stage], bow_pose(t * 20.0))
            }
            (Guest::Stick, _) => (STICK, item_pose(state.swing.map_or(0.0, |t| (t * 20.0 / SWING_TICKS).clamp(0.0, 1.0)))),
            _ => (BOW, item_pose(0.0)),
        };
        push_quads(&mut vertices, &mut index_list, &items.quads[layer as usize], pose, layer, w.enchanted());
        out.solid = index_list.len() as u32;
        out.vertices = bytemuck::cast_slice(&vertices).to_vec();
        out.indices = index_list;
        out.to_clip = hand_clip(aspect);
    }
    let mut vertices = Vec::new();
    let mut index_list = Vec::new();
    if let Some(items) = state.items.as_ref() {
        out.items = Some(items.texture.clone());
        for a in sim::guest_arms::arrows() {
            push_arrow(&mut vertices, &mut index_list, Vec3::from_array(a.at), Vec3::from_array(a.motion), a.enchanted);
        }
    }
    for p in sim::guest_arms::portals() {
        push_portal(&mut vertices, &mut index_list, &p);
    }
    out.level_vertex_total = bytemuck::cast_slice(&vertices).to_vec();
    out.level_indices = index_list;
    if let Some(mut sounds) = sounds {
        sounds.0.extend(queue);
    }
    *view = out;
}
