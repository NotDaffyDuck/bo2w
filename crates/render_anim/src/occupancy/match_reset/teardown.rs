//! The reset only a teardown needs.

use bevy::prelude::*;
use frame::MatchTornDown;

use crate::occupancy::dyn_ent::{DynEntPhysClip, DynEntPhysWorld};

/// After a match is torn down, start the loose-piece physics from scratch.
///
/// The collision shapes belong to the old map and the awake pieces to its
/// entities, so both resources are replaced with empty ones. An install alone
/// does not do this; the next install fills them in.
pub(super) fn reset_dyn_ent_phys_on_teardown(
    mut torn_down: MessageReader<MatchTornDown>,
    mut commands: Commands,
) {
    if torn_down.read().count() == 0 {
        return;
    }
    commands.insert_resource(DynEntPhysClip::default());
    commands.insert_resource(DynEntPhysWorld::default());
}
