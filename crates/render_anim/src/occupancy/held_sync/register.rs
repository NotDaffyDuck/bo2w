//! Wiring for the held-weapon sync.

use bevy::prelude::*;
use frame::{LifeEnded, LifeFrontPublished, LifeStarted};
use net::ClientSet;

use super::probe::sync_fpv_status_to_probe;
use super::sync::sync_fpv_to_held_weapon;

/// Add the sync to the app.
///
/// The life messages are declared here too, since this is the system that reads
/// them. The sync runs once the life front is published, so it sees this frame's
/// life changes; the report copy runs with the other diagnostics.
pub(crate) fn register_held_sync_systems(app: &mut App) {
    app.add_message::<LifeStarted>().add_message::<LifeEnded>();
    app.add_systems(Update, sync_fpv_to_held_weapon.in_set(LifeFrontPublished));
    app.add_systems(Update, sync_fpv_status_to_probe.in_set(ClientSet::Diag));
}
