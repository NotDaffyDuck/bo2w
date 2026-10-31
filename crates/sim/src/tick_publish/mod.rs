//! What the sim publishes after each tick, and the saved form of the area entity sectors that rides along with it.

mod area;
mod capture;
mod restore;
#[cfg(test)]
mod tests;

use playerstate_t6::game::AvatarState;

use crate::FlyingProjectile;
use crate::player_ledger::TickLedgerNotes;
use crate::sim_parts::{SeatNo, StepNo};

pub use area::{ZoneLinkRow, AreaEntityLinkSnapshot, ZoneThingTable, AreaEntityWorldSnapshot, RegionSnapshotFault, ZoneCellRow, AreaSectorSnapshot};

/// One tick as published: every player's state, the missiles in flight and the match's side information.
#[derive(Clone, Debug, PartialEq)]
pub struct TickSnapshot {
    pub tick: StepNo,
    pub players: Vec<(SeatNo, AvatarState)>,
    pub projectiles: Vec<FlyingProjectile>,
    pub meta: TickLedgerNotes,
}
/// Older spelling Snapshot still calls; the rename sweep moves it over.
pub use self::TickSnapshot as Snapshot;

impl TickSnapshot {
    /// The snapshot of a tick that has published nothing yet.
    pub fn unpublished(tick: StepNo) -> Self {
        Self { tick, players: Vec::new(), projectiles: Vec::new(), meta: TickLedgerNotes::default() }
    }
}
