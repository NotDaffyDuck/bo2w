//! Minecraft Zombies' guest weapons on a plain Black Ops II map (the
//! Special Weapons tab: Half-Life 2's Gravity Gun and its Super, Minecraft's
//! bow, its enchanted bow and the enchanted stick). The rules side
//! (`t6::mc_specials`) publishes here what the world side draws and plays,
//! and the world side's Gravity Gun asks the rules side to hit zombies.

use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::{Mutex, RwLock};

/// One of the five, by its script name.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Guest {
    GravityGun,
    SuperGravityGun,
    Bow,
    EnchantedBow,
    Stick,
}

impl Guest {
    pub fn of(name: &str) -> Option<Self> {
        match name.strip_suffix("_mp").unwrap_or(name) {
            "physcannon_zm" => Some(Self::GravityGun),
            "physcannon_upgraded_zm" => Some(Self::SuperGravityGun),
            "mcbow_zm" => Some(Self::Bow),
            "mcbow_enchanted_zm" => Some(Self::EnchantedBow),
            "mcstick_zm" => Some(Self::Stick),
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
        }
    }

    pub fn is_gravgun(self) -> bool {
        matches!(self, Self::GravityGun | Self::SuperGravityGun)
    }

    /// The enchanted ones shine as Minecraft's enchanted items do.
    pub fn enchanted(self) -> bool {
        matches!(self, Self::EnchantedBow | Self::Stick)
    }
}

/// What his hands hold, for the world side: the guest weapon out (none
/// while a BO2 weapon is), how far the bow is drawn (seconds), and counters
/// that step on each stick swing, stick hit and arrow let go.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Hand {
    pub weapon: Option<Guest>,
    pub draw: Option<f32>,
    pub swings: u32,
    pub hits: u32,
    pub shots: u32,
}

static HAND: Mutex<Hand> = Mutex::new(Hand {
    weapon: None,
    draw: None,
    swings: 0,
    hits: 0,
    shots: 0,
});

pub fn set_hand(hand: Hand) {
    *HAND.lock().unwrap() = hand;
}

pub fn hand() -> Hand {
    *HAND.lock().unwrap()
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

/// A BO2 zombie as the Gravity Gun sees it: its entity, feet, alive or a
/// body, its standing box, and its drawn model's id (the ragdoll's).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GuestZombie {
    pub ent: u32,
    pub origin: [f32; 3],
    pub alive: bool,
    pub radius: f32,
    pub height: f32,
    pub wire: u32,
}

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

pub fn held_bodies() -> Vec<u32> {
    HELD_BODIES.lock().unwrap().clone()
}

/// World side -> rules.
#[derive(Clone, Debug, PartialEq)]
pub enum Ask {
    /// The Gravity Gun's hit on BO2 zombie `ent` (a thrown body, a punt):
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

/// A BO2 zombie's health this round (`level.zombie_health`).
static ZOMBIE_HEALTH: AtomicI32 = AtomicI32::new(150);

pub fn set_zombie_health(health: i32) {
    ZOMBIE_HEALTH.store(health.max(1), Ordering::Relaxed);
}

pub fn zombie_health() -> i32 {
    ZOMBIE_HEALTH.load(Ordering::Relaxed).max(1)
}
