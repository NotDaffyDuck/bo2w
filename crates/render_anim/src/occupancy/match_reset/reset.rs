//! Dropping every per-match animation cache.

use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use frame::{MatchInstalled, MatchTornDown};

use crate::anim::fpv_prepared::PreparedFpv;
use crate::anim::model_materials::PreparedModelMaterials;
use crate::anim::scene_submission::AnimDObjSceneSkels;
use crate::occupancy::fpv_present::{
    FpvHeldLife, FpvHeldSettled, FpvPlacementRoot, FpvPresentCursor, LocalSpawnArmed,
    PendingFpvSpawn, SessionViewmodel,
};

/// All the state that belongs to one match's animation, gathered so it can be
/// emptied in one call.
#[derive(SystemParam)]
pub struct MatchAnimState<'w, 's> {
    commands: Commands<'w, 's>,
    roots: Query<'w, 's, Entity, With<FpvPlacementRoot>>,
    spawn_armed: ResMut<'w, LocalSpawnArmed>,
    cursor: ResMut<'w, FpvPresentCursor>,
    pending: ResMut<'w, PendingFpvSpawn>,
    viewmodel: ResMut<'w, SessionViewmodel>,
    settled: ResMut<'w, FpvHeldSettled>,
    held_life: ResMut<'w, FpvHeldLife>,
    fpv: ResMut<'w, PreparedFpv>,
    materials: ResMut<'w, PreparedModelMaterials>,
    scene_skels: ResMut<'w, AnimDObjSceneSkels>,
}

impl MatchAnimState<'_, '_> {
    /// Forget everything: disarm the spawn, rewind the present cursor, drop the
    /// queued and drawn first-person weapon, release the prepared tables, and
    /// despawn the placement roots.
    fn clear(&mut self) {
        self.spawn_armed.0 = false;
        self.cursor.0.clear();
        self.pending.0 = None;
        *self.viewmodel = SessionViewmodel::default();
        self.fpv.clear();
        self.materials.clear();
        self.settled.0 = None;
        self.held_life.0 = None;
        *self.scene_skels = AnimDObjSceneSkels::default();

        for root in &self.roots {
            self.commands.entity(root).try_despawn();
        }
    }
}

/// On a match install or teardown, empty the per-match animation state.
///
/// Both message queues are drained every frame; with neither message this
/// frame the state is left alone.
pub fn reset_anim_for_match(
    mut state: MatchAnimState,
    mut installed: MessageReader<MatchInstalled>,
    mut torn_down: MessageReader<MatchTornDown>,
) {
    let edges = installed.read().count() + torn_down.read().count();
    if edges > 0 {
        state.clear();
    }
}
