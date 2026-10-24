//! The shapes of damage that reach the scripts' world, and the two ways in:
//! what a blast can hurt, and a hit on a map entity.

use bevy_ecs::prelude::World;

use crate::ScriptModelId;
use crate::world::ClientId;

/// A shot, blade or blast landing on one of the map's entities.
#[derive(Clone, Debug)]
pub(crate) struct EntityHit {
    pub target: ScriptModelId,
    pub amount: i32,
    pub attacker: Option<ClientId>,
    pub means: &'static str,
    pub weapon: u32,
    pub point: [f32; 3],
    pub dir: [f32; 3],
    pub bone: Option<usize>,
    pub flags: i32,
}

/// An explosion's reach and strength, falling from `max` at the centre to
/// `min` at the rim.
#[derive(Clone, Debug)]
pub(crate) struct ScriptBlast {
    pub origin: [f32; 3],
    pub radius: f32,
    pub max: f32,
    pub min: f32,
    pub attacker: Option<ClientId>,
    pub inflictor: Option<ScriptModelId>,
    pub means: &'static str,
    pub weapon: u32,
}

/// What a hit lands on.
#[derive(Clone, Copy, Debug)]
pub(crate) enum HitTarget {
    Player(ClientId),
    Entity(ScriptModelId),
}

/// One hit as the scripts describe it.
#[derive(Clone, Debug)]
pub(crate) struct ScriptHit {
    pub target: HitTarget,
    pub amount: i32,
    pub origin: [f32; 3],
    pub attacker: Option<ClientId>,
    pub inflictor: Option<ScriptModelId>,
    pub means: &'static str,
    pub weapon: u32,
    pub flags: i32,
    pub hitloc: u8,
}

/// The map entities within a blast's radius, each with its centre and its
/// distance from the blast.
pub(crate) fn radius_targets(
    world: &mut World,
    origin: [f32; 3],
    radius: f32,
) -> Vec<(ScriptModelId, [f32; 3], f32)> {
    crate::t6::radius_targets(world, origin, radius)
}

/// Hand a hit on a map entity to the BO2 map's scripts; whether they took it.
pub(crate) fn damage_entity(world: &mut World, hit: &EntityHit) -> bool {
    crate::t6::entity_hit(world, hit)
}
