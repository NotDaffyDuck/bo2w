//! bo2mp: the guest weapons in the menu's Zombies Wonder Weapons tab
//! (Half-Life 2's Gravity Gun and its Super, Minecraft's bow, its enchanted
//! bow and the enchanted stick, Portal's portal gun and its Super). The rules side (`t6::mp_specials`)
//! publishes here what the world side draws and plays, and the world side's
//! Gravity Gun asks the rules side to hit players and bots.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicI32, AtomicU32, Ordering};
use std::sync::{Mutex, RwLock};

/// One of the seven, by its script name.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Guest {
    GravityGun,
    SuperGravityGun,
    Bow,
    EnchantedBow,
    Stick,
    PortalGun,
    SuperPortalGun,
}

impl Guest {
    pub fn of(name: &str) -> Option<Self> {
        match name.strip_suffix("_mp").unwrap_or(name) {
            "physcannon_zm" => Some(Self::GravityGun),
            "physcannon_upgraded_zm" => Some(Self::SuperGravityGun),
            "mcbow_zm" => Some(Self::Bow),
            "mcbow_enchanted_zm" => Some(Self::EnchantedBow),
            "mcstick_zm" => Some(Self::Stick),
            "portalgun_zm" => Some(Self::PortalGun),
            "portalgun_upgraded_zm" => Some(Self::SuperPortalGun),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::GravityGun => "physcannon_zm",
            Self::SuperGravityGun => "physcannon_upgraded_zm",
            Self::Bow => "mcbow_zm",
            Self::EnchantedBow => "mcbow_enchanted_zm",
            Self::Stick => "mcstick_zm",
            Self::PortalGun => "portalgun_zm",
            Self::SuperPortalGun => "portalgun_upgraded_zm",
        }
    }

    pub fn is_gravgun(self) -> bool {
        matches!(self, Self::GravityGun | Self::SuperGravityGun)
    }

    pub fn is_portalgun(self) -> bool {
        matches!(self, Self::PortalGun | Self::SuperPortalGun)
    }

    /// The enchanted ones shine as Minecraft's enchanted items do.
    pub fn enchanted(self) -> bool {
        matches!(self, Self::EnchantedBow | Self::Stick)
    }
}

/// What his hands hold, for the world side: the guest weapon out (none
/// while a BO2 weapon is), how far the bow is drawn (seconds), and counters
/// that step on each stick swing, stick hit and arrow let go (and each
/// portal shot, `orange` saying which colour the last one was).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Hand {
    pub weapon: Option<Guest>,
    pub draw: Option<f32>,
    pub swings: u32,
    pub hits: u32,
    pub shots: u32,
    pub orange: bool,
}

/// Each client's hands, by client number.
static HANDS: Mutex<BTreeMap<u32, Hand>> = Mutex::new(BTreeMap::new());

/// The client this PC shows (the world side sets it each frame).
static VIEWER: AtomicU32 = AtomicU32::new(0);

pub fn set_viewer(client: u32) {
    VIEWER.store(client, Ordering::Relaxed);
}

pub fn viewer() -> u32 {
    VIEWER.load(Ordering::Relaxed)
}

pub fn set_hand(client: u32, hand: Hand) {
    HANDS.lock().unwrap().insert(client, hand);
}

/// What the shown client's hands hold.
pub fn hand() -> Hand {
    HANDS.lock().unwrap().get(&viewer()).copied().unwrap_or_default()
}

/// An arrow in flight or stuck, for the world side to draw: map point,
/// velocity (map units a second; its heading once stuck).
#[derive(Clone, Copy, Debug)]
pub struct ArrowView {
    pub at: [f32; 3],
    pub velocity: [f32; 3],
    pub enchanted: bool,
}

static ARROWS: Mutex<Vec<ArrowView>> = Mutex::new(Vec::new());

pub fn set_arrows(arrows: Vec<ArrowView>) {
    *ARROWS.lock().unwrap() = arrows;
}

pub fn arrows() -> Vec<ArrowView> {
    ARROWS.lock().unwrap().clone()
}

/// Where arrows struck this frame (a zombie or a wall), for their sound.
static ARROW_HITS: Mutex<Vec<[f32; 3]>> = Mutex::new(Vec::new());

pub fn arrow_hit(at: [f32; 3]) {
    let mut hits = ARROW_HITS.lock().unwrap();
    if hits.len() < 64 {
        hits.push(at);
    }
}

pub fn take_arrow_hits() -> Vec<[f32; 3]> {
    std::mem::take(&mut *ARROW_HITS.lock().unwrap())
}

/// A player, bot or body as the Gravity Gun sees it: its entity (a client
/// number; a body no longer its player's newest is `BODY | entity`), feet,
/// alive or a body, its standing box, and its drawn body's id (the
/// ragdoll's).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GuestZombie {
    pub ent: u32,
    pub origin: [f32; 3],
    pub alive: bool,
    pub radius: f32,
    pub height: f32,
    pub wire: u32,
}

/// The `ent` of a body that is not its player's newest.
pub const BODY: u32 = 0x1000_0000;

static ZOMBIES: RwLock<Vec<GuestZombie>> = RwLock::new(Vec::new());

pub fn set_zombies(list: Vec<GuestZombie>) {
    *ZOMBIES.write().unwrap() = list;
}

pub fn zombies() -> Vec<GuestZombie> {
    ZOMBIES.read().unwrap().clone()
}

/// The bodies the Gravity Gun holds: the rules side keeps them.
static HELD_BODIES: Mutex<Vec<u32>> = Mutex::new(Vec::new());

pub fn set_held_bodies(ents: Vec<u32>) {
    *HELD_BODIES.lock().unwrap() = ents;
}

/// World side -> rules.
#[derive(Clone, Debug, PartialEq)]
pub enum Ask {
    /// The Gravity Gun's hit on target `ent` (a thrown body, a punt):
    /// `damage` (BO2 health) by `means` (`MOD_*`) at `at` going `dir`; a body
    /// it kills, or one already down, flies at `launch` (units a second).
    GravHit { ent: u32, damage: i32, means: String, at: [f32; 3], dir: [f32; 3], launch: [f32; 3] },
}

static ASKS: Mutex<Vec<Ask>> = Mutex::new(Vec::new());

pub fn ask(a: Ask) {
    ASKS.lock().unwrap().push(a);
}

pub fn take_asks() -> Vec<Ask> {
    std::mem::take(&mut *ASKS.lock().unwrap())
}

/// A full-health player's health (the Gravity Gun's kill is twenty of it).
static ZOMBIE_HEALTH: AtomicI32 = AtomicI32::new(100);

pub fn set_zombie_health(health: i32) {
    ZOMBIE_HEALTH.store(health.max(1), Ordering::Relaxed);
}

pub fn zombie_health() -> i32 {
    ZOMBIE_HEALTH.load(Ordering::Relaxed).max(1)
}

/// A portal on a wall, floor or ceiling, for the world side to draw: its
/// centre (map units), the way its face looks out, its up, its colour, and
/// whether its other end is open (a linked portal shows its swirl).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PortalView {
    pub centre: [f32; 3],
    pub normal: [f32; 3],
    pub up: [f32; 3],
    pub orange: bool,
    pub linked: bool,
    /// Seconds since it opened, and whether the Super Portal Gun shot it.
    pub age: f32,
    pub upgraded: bool,
    /// Which pair it is (its shooter): the blue and orange of one gun.
    pub pair: u32,
}

/// A portal's half width and half height (map units: Portal's 64 by 108).
pub const PORTAL_HALF: [f32; 2] = [32.0, 54.0];

static PORTALS: Mutex<Vec<PortalView>> = Mutex::new(Vec::new());

pub fn set_portals(list: Vec<PortalView>) {
    *PORTALS.lock().unwrap() = list;
}

pub fn portals() -> Vec<PortalView> {
    PORTALS.lock().unwrap().clone()
}
