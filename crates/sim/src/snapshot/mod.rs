//! What the sim publishes after each tick, and the saved form of the area entity sectors that rides along with it.

mod area;
mod capture;
mod restore;
#[cfg(test)]
mod tests;

use playerstate_t6::game::PlayerState;

use crate::ProjectileState;
use crate::match_state::SnapshotMeta;
use crate::world::{ClientId, Tick};

pub use area::{AreaEntityLinkSnapshot, AreaEntityWorldSnapshot, AreaEntityWorldSnapshotError, AreaSectorSnapshot};

/// One tick as published: every player's state, the missiles in flight and the match's side information.
#[derive(Clone, Debug, PartialEq)]
pub struct Snapshot {
    pub tick: Tick,
    pub players: Vec<(ClientId, PlayerState)>,
    pub projectiles: Vec<ProjectileState>,
    pub meta: SnapshotMeta,
}

impl Snapshot {
    /// The snapshot of a tick that has published nothing yet.
    pub fn unpublished(tick: Tick) -> Self {
        Self { tick, players: Vec::new(), projectiles: Vec::new(), meta: SnapshotMeta::default() }
    }
}
