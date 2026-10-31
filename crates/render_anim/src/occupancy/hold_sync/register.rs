//! Wiring for the held-weapon sync.

use bevy::prelude::*;
use frame::{SpawnLifeOver, LifeStatePosted, SpawnLifeBegun};
use net::ClientStep;

use super::probe::mirror_fpv_status_probe;
use super::sync::match_fpv_to_held;

/// Add the sync to the app.
///
/// The life messages are declared here too, since this is the system that reads
/// them. The sync runs once the life front is published, so it sees this frame's
/// life changes; the report copy runs with the other diagnostics.
pub(crate) fn add_held_sync_systems(application: &mut App) {
    application.add_message::<SpawnLifeBegun>().add_message::<SpawnLifeOver>();
    application.add_systems(Update, match_fpv_to_held.in_set(LifeStatePosted));
    application.add_systems(Update, mirror_fpv_status_probe.in_set(ClientStep::Diag));
}
