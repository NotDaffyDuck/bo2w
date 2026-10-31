//! The reset only a teardown needs.

use bevy::prelude::*;
use frame::GameSessionClosed;

use crate::occupancy::shard_piece::{PropPhysicsShape, DebrisPhysLevel};

/// After a match is torn down, start the loose-piece physics from scratch.
///
/// The collision shapes belong to the old map and the awake pieces to its
/// entities, so both resources are replaced with empty ones. An install alone
/// does not do this; the next install fills them in.
pub(super) fn clear_dyn_ent_phys_at_teardown(
    mut torn_down: MessageReader<GameSessionClosed>,
    mut cmds: Commands,
) {
    if torn_down.read().count() == 0 {
        return;
    }
    cmds.insert_resource(PropPhysicsShape::default());
    cmds.insert_resource(DebrisPhysLevel::default());
}
