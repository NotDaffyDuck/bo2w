//! The shapes of damage that reach the scripts' world, and the two ways in:
//! what a blast can hurt, and a hit on a map entity.

use bevy_ecs::prelude::World;

use crate::ScriptedModel;
use crate::sim_parts::SeatNo;

/// A shot, blade or blast landing on one of the map's entities.
#[derive(Clone, Debug)]
pub(crate) struct EntityHit {
    pub target: ScriptedModel,
    pub amount: i32,
    pub attacker: Option<SeatNo>,
    pub means: &'static str,
    pub gun: u32,
    pub point: [f32; 3],
    pub way: [f32; 3],
    pub bone: Option<usize>,
    pub bits: i32,
}

/// An explosion's reach and strength, falling from `max` at the centre to
/// `min` at the rim.
#[derive(Clone, Debug)]
pub(crate) struct ScriptBlast {
    pub spot: [f32; 3],
    pub reach: f32,
    pub max: f32,
    pub min: f32,
    pub attacker: Option<SeatNo>,
    pub inflictor: Option<ScriptedModel>,
    pub means: &'static str,
    pub gun: u32,
}

/// What a hit lands on.
#[derive(Clone, Copy, Debug)]
pub(crate) enum HitTarget {
    Player(SeatNo),
    Entity(ScriptedModel),
}

/// One hit as the scripts describe it.
#[derive(Clone, Debug)]
pub(crate) struct ScriptHit {
    pub target: HitTarget,
    pub much: i32,
    pub spot: [f32; 3],
    pub attacker: Option<SeatNo>,
    pub inflictor: Option<ScriptedModel>,
    pub means: &'static str,
    pub gun: u32,
    pub bits: i32,
    pub hit_part: u8,
}

/// The map entities within a blast's radius, each with its centre and its
/// distance from the blast.
pub(crate) fn radius_targets(
    world: &mut World,
    spot: [f32; 3],
    reach: f32,
) -> Vec<(ScriptedModel, [f32; 3], f32)> {
    crate::zombie_rt::radius_targets(world, spot, reach)
}

/// Hand a hit on a map entity to the BO2 map's scripts; whether they took it.
pub(crate) fn damage_entity(world: &mut World, hit: &EntityHit) -> bool {
    crate::zombie_rt::entity_hit(world, hit)
}
