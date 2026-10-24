//! Counts of what the collision world holds, for the `hitvol` console commands: the map's clip data, the player bodies
//! shots can hit, and the entities with something to hit.

mod entities;
mod model;
mod players;
mod world;

pub use entities::EntityClipCensus;
pub use model::ModelCollisionCensus;
pub use players::PlayerClipCensus;
pub use world::WorldClipCensus;

pub(crate) use entities::entity_clip_census;
pub(crate) use players::player_clip_census;

/// Everything the census counts, in one report.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CollisionCensus {
    pub world: WorldClipCensus,
    pub players: PlayerClipCensus,
    pub entities: EntityClipCensus,
    /// One row per model the match's kit uses (weapons, bodies, equipment).
    pub kits: Vec<ModelCollisionCensus>,
}
