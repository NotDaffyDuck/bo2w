//! Arming a spawn when a match installs and undoing it when the match ends.

use bevy::prelude::*;
use frame::{GameSessionArmed, GameSessionClosed, Retiring, LevelGeneration};

use super::job::LevelSpawnJob;
use crate::adapters::motion::debris_piece::DebrisRoomBits;
use crate::MapSunEffects;
use crate::LiveImageHandles;
use crate::PropGpuPlan;
use crate::WorldGpuSubmission;
use crate::mesh_build::sky::SkyDomeDrawPlan;
use crate::prepare::scene::camera::FreeRoamCamera;
use super::super::cull::{DynamicPropModelEntity, ScriptedPropEntity, PropEntity};
use crate::prepare::scene::world::LevelStage;
use crate::prepare::scene::level_gpu::{PipelineDemandTracker, LevelGpuReady};
use crate::prepare::scene::model_illum_atlas::LevelModelIllumAtlas;
use crate::prepare::scene::model_illum_cache::LevelModelIllumCache;
use crate::prepare::scene::prop_geom_memo::LevelPropCache;
use crate::prepare::scene::prop_light_slots::LevelPropIllum;

/// Whether this frame carries a teardown. Reads the messages so they count as seen.
fn torn_down(torn: &mut MessageReader<GameSessionClosed>) -> bool {
    torn.read().count() > 0
}

pub(crate) fn arm_level_spawn_on_install(
    mut installed: MessageReader<GameSessionArmed>,
    mut job: ResMut<LevelSpawnJob>,
    mut gpu: ResMut<LevelGpuReady>,
) {
    let Some(install) = installed.read().last() else {
        return;
    };
    let spawn = LevelGeneration::out_of_setup_dir(install.ask_number);
    *job = LevelSpawnJob {
        spawn,
        ..default()
    };
    *gpu = LevelGpuReady {
        spawn,
        ..default()
    };
}

pub(crate) fn despawn_level_entities_on_teardown(
    mut torn: MessageReader<GameSessionClosed>,
    prop_set: Query<Entity, With<PropEntity>>,
    script_models: Query<Entity, With<ScriptedPropEntity>>,
    debris_pieces: Query<Entity, With<DynamicPropModelEntity>>,
    mut commands: Commands,
) {
    if !torn_down(&mut torn) {
        return;
    }
    for gone in prop_set
        .iter()
        .chain(script_models.iter())
        .chain(debris_pieces.iter())
    {
        commands.entity(gone).despawn();
    }
}

pub(crate) fn despawn_free_cameras_on_teardown(
    mut torn: MessageReader<GameSessionClosed>,
    cam_query: Query<Entity, With<FreeRoamCamera>>,
    mut commands: Commands,
) {
    if !torn_down(&mut torn) {
        return;
    }
    for camera in &cam_query {
        commands.entity(camera).despawn();
    }
}

pub(crate) fn reset_level_spawn_on_teardown(
    mut torn: MessageReader<GameSessionClosed>,
    mut job: ResMut<LevelSpawnJob>,
    mut gpu: ResMut<LevelGpuReady>,
    mut demand: ResMut<PipelineDemandTracker>,
    mut image_handles: Option<ResMut<LiveImageHandles>>,
    mut retiring: ResMut<Retiring>,
    images: Res<Assets<Image>>,
    mut commands: Commands,
) {
    if !torn_down(&mut torn) {
        return;
    }
    let live_before = images.len();
    retiring.hand_over(std::mem::take(&mut *job));
    *gpu = default();
    *demand = default();
    if let Some(handle_list) = image_handles.as_mut() {
        **handle_list = default();
    }
    diag::info!(
        World,
        "world: render side let go of its images ({live_before} live in Assets<Image> at the \
         moment of release; what survives is what another owner still holds)"
    );
    commands.insert_resource(MapSunEffects::default());
    commands.queue(|world: &mut World| {
        frame::retire::retire_resources(world, |batch| {
            batch
                .resource::<WorldGpuSubmission>()
                .resource::<PropGpuPlan>()
                .resource::<SkyDomeDrawPlan>()
                .resource::<LevelPropIllum>()
                .resource::<LevelPropCache>()
                .resource::<LevelModelIllumAtlas>()
                .resource::<LevelModelIllumCache>();
        });
    });
}

pub(crate) fn shutdown_level_on_teardown(
    mut torn: MessageReader<GameSessionClosed>,
    mut retiring: ResMut<Retiring>,
    mut scene: Option<ResMut<LevelStage>>,
    mut membership: Option<ResMut<DebrisRoomBits>>,
    mut present: ResMut<render_scene::LevelPresentFacts>,
    mut tess: ResMut<render_scene::StitchMaterials>,
    mut lookup: ResMut<render_scene::LooseAtSpotLookup>,
    mut cells: ResMut<render_scene::LevelSightRooms>,
) {
    if !torn_down(&mut torn) {
        return;
    }
    if let Some(stage) = scene.as_mut() {
        retiring.hand_over(std::mem::take(&mut **stage));
    }
    // Read after the hand-over: the world left behind reports as not spawned.
    let spawned = scene.as_ref().map_or(0, |scene| i64::from(scene.spawned));
    if let Some(members_ed) = membership.as_mut() {
        **members_ed = default();
    }
    *present = default();
    *tess = default();
    lookup.clear();
    *cells = default();
    perf::level_hold(spawned, 0, 0);
    diag::info!(
        World,
        "world: shut down; leftover geometry, tess plans and model lighting are dropped"
    );
}
