//! Loose map pieces (bottles, trays, a mannequin's head) once something
//! shoves them: a small rigid-body step with gravity, sweeps against the
//! world's clip, a bounce with friction, and sleep when they settle.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use asset_world::CollisionClipData;
use bevy::prelude::*;
use net::FrameClock;
use render_scene::{DynamicPropModelEntity, LevelDebrisInstance};

use crate::occupancy::shard_piece::PieceModels;

mod body;
mod run;
mod step;
use self::body::*;
use self::run::*;
pub use self::step::*;

mod tests;

/// Units per second squared.
const GRAVITY: f32 = -800.0;
/// The physics step, in milliseconds; a frame runs at most `MAX_STEPS` of them.
const STEP_MS: i32 = 17;
const MAX_STEPS: i32 = 2;
/// Below this speed, standing on ground, a piece counts as settled.
const SETTLE_SPEED: f32 = 4.0;
/// A surface whose normal points up at least this much is ground.
const GROUND_NORMAL_Z: f32 = 0.7;
/// A piece is left this far off the surface it touched.
const SKIN: f32 = 0.125;
/// Settled for this long and the piece goes to sleep.
const SLEEP_AFTER_MS: i32 = 200;
/// A contact whose approach speed is under this many steps of gravity is
/// resting, and does not bounce.
const RESTING_STEPS: f32 = 2.0;
/// The world mask loose pieces collide with.
const PIECE_CLIP_MASK: u32 = 0x280_ec93;
/// How far a piece found inside solid searches for open space, and how
/// finely.
const ESCAPE_REACH: f32 = 96.0;
const ESCAPE_STEP: f32 = 2.0;
/// Hard speed limit. Light props thrown by a blast otherwise leave the map
/// through windows at several thousand units a second; pieces that landed
/// sensibly all stayed under this.
const SPEED_LIMIT: f32 = 2600.0;
/// Speed above which a piece is noted in the hit log.
const FAST_NOTE: f32 = 3000.0;

/// Diagnostics: where the last piece came to rest (hit log only).
pub static LAST_REST: Mutex<Option<[f32; 3]>> = Mutex::new(None);

/// The world geometry pieces collide with.
#[derive(Resource, Clone, Default)]
pub struct PropPhysicsShape(pub Option<Arc<CollisionClipData>>);

/// Pieces that are currently moving.
#[derive(Resource, Default)]
pub struct DebrisPhysLevel {
    clock_ms: Option<i32>,
    awake: HashMap<Entity, PhysBody>,
}

impl DebrisPhysLevel {
    pub fn still_moving(&self, entity_in: Entity) -> bool {
        self.awake.contains_key(&entity_in)
    }

    pub fn remove_physics_body(&mut self, ent: Entity) {
        self.awake.remove(&ent);
    }
}

/// A shove on a piece, as momentum.
#[derive(Clone, Copy, Debug)]
#[derive(Message)]
pub struct PropPhysicsNudge {
    pub entity: Entity,
    pub impulse: Vec3,
}

/// One awake piece: where it is, how it moves, and what has been logged.
#[derive(Clone, Copy, Debug)]
struct PhysBody {
    motion_vec: Vec3,
    pos: Vec3,
    maxs: [f32; 3],
    mins: [f32; 3],
    drag: f32,
    rebound: f32,
    still_ms: i32,
    grounded: bool,
    noted_stuck: bool,
    noted_fast: bool,
}

pub(crate) fn add_dyn_ent_physics(app: &mut App) {
    app.add_message::<PropPhysicsNudge>();
    app.init_resource::<DebrisPhysLevel>();
    app.init_resource::<PropPhysicsShape>();
}

fn hit_log_on() -> bool {
    const HITLOG_VAR: &str = "BO2MP_T6_HITLOG";
    std::env::var_os(HITLOG_VAR).is_some()
}
