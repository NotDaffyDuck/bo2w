//! The first-person state that the held-weapon sync reads and rewrites.

use bevy::ecs::system::SystemParam;
use bevy::prelude::*;

use crate::occupancy::fpv_present::{
    FpvHeldLife, FpvHeldSettled, FpvPlacementRoot, FpvState, FpvStatusGap, PendingFpvSpawn,
    SessionViewmodel,
};

/// Everything about "what the first-person view is showing" in one parameter.
///
/// A weapon can be in one of three places: already drawn (`session`), asked
/// for but not drawn yet (`pending`), or decided on and left alone (`settled`,
/// which also records "nothing to draw" as weapon 0). `life` remembers which
/// life of the local player the view belongs to, and `status` is the last
/// outcome worth reporting.
#[derive(SystemParam)]
pub struct FpvHeldSlots<'w, 's> {
    commands: Commands<'w, 's>,
    roots: Query<'w, 's, Entity, With<FpvPlacementRoot>>,
    pub(super) pending: ResMut<'w, PendingFpvSpawn>,
    pub(super) session: ResMut<'w, SessionViewmodel>,
    pub(super) settled: ResMut<'w, FpvHeldSettled>,
    pub(super) life: ResMut<'w, FpvHeldLife>,
    pub(super) status: ResMut<'w, FpvStatusGap>,
}

impl FpvHeldSlots<'_, '_> {
    /// True when any of the three places still holds something.
    pub(super) fn is_occupied(&self) -> bool {
        self.session.0.is_some() || self.pending.0.is_some() || self.settled.0.is_some()
    }

    /// Remove the drawn placement entities (the roots) from the world.
    pub(super) fn despawn_roots(&mut self) {
        for root in &self.roots {
            self.commands.entity(root).try_despawn();
        }
    }

    /// Throw away everything: despawn the roots and empty all three places.
    pub(super) fn wipe(&mut self) {
        self.despawn_roots();
        self.pending.0 = None;
        self.session.0 = None;
        self.settled.0 = None;
    }

    /// Wipe, but only when there is something to wipe. Returns whether it did.
    pub(super) fn wipe_if_occupied(&mut self) -> bool {
        let occupied = self.is_occupied();
        if occupied {
            self.wipe();
        }
        occupied
    }

    /// Wipe because the player is not in a state to hold a weapon (dead,
    /// between matches, not spawned) and say so in the status.
    pub(super) fn retire_not_alive(&mut self) {
        if self.wipe_if_occupied() {
            self.status.0 = Some(FpvState::ClearedNotAlive);
        }
    }
}
