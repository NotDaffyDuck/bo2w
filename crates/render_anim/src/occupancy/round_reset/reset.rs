//! Dropping every per-match animation cache.

use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use frame::{GameSessionArmed, GameSessionClosed};

use crate::motion::fpv_prepared::PreparedFpv;
use crate::motion::model_materials::PreparedModelMaterials;
use crate::motion::scene_feed::SceneBonePoseSet;
use crate::occupancy::gun_present::{
    HoldLifeStamp, HoldSettledLife, GunMountAnchor, FpvShownCursor, OwnSpawnPrimed,
    WaitingFpvSpawn, ActiveGunMeshes,
};

/// All the state that belongs to one match's animation, gathered so it can be
/// emptied in one call.
#[derive(SystemParam)]
pub struct MatchAnimState<'w, 's> {
    commands: Commands<'w, 's>,
    roots: Query<'w, 's, Entity, With<GunMountAnchor>>,
    spawn_armed: ResMut<'w, OwnSpawnPrimed>,
    cursor: ResMut<'w, FpvShownCursor>,
    waiting: ResMut<'w, WaitingFpvSpawn>,
    viewmodel: ResMut<'w, ActiveGunMeshes>,
    settled: ResMut<'w, HoldSettledLife>,
    held_life: ResMut<'w, HoldLifeStamp>,
    fpv: ResMut<'w, PreparedFpv>,
    mats: ResMut<'w, PreparedModelMaterials>,
    skel_cache: ResMut<'w, SceneBonePoseSet>,
}

impl MatchAnimState<'_, '_> {
    /// Forget everything: disarm the spawn, rewind the present cursor, drop the
    /// queued and drawn first-person weapon, release the prepared tables, and
    /// despawn the placement roots.
    fn clear(&mut self) {
        self.spawn_armed.0 = false;
        self.cursor.0.clear();
        self.waiting.0 = None;
        *self.viewmodel = ActiveGunMeshes::default();
        self.fpv.clear();
        self.mats.clear();
        self.settled.0 = None;
        self.held_life.0 = None;
        *self.skel_cache = SceneBonePoseSet::default();

        for root in &self.roots {
            self.commands.entity(root).try_despawn();
        }
    }
}

/// On a match install or teardown, empty the per-match animation state.
///
/// Both message queues are drained every frame; with neither message this
/// frame the state is left alone.
pub fn reset_anim_at_match_start(
    mut state: MatchAnimState,
    mut installed: MessageReader<GameSessionArmed>,
    mut torn_down: MessageReader<GameSessionClosed>,
) {
    let edges = installed.read().count() + torn_down.read().count();
    if edges > 0 {
        state.clear();
    }
}
