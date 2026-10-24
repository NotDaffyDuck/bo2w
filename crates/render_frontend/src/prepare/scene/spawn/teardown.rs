//! Arming a spawn when a match installs and undoing it when the match ends.

use bevy::prelude::*;
use frame::{MatchInstalled, MatchTornDown, Retiring, WorldGeneration};

use super::job::WorldSpawnJob;
use crate::adapters::anim::dyn_ent::DynEntCellBits;
use crate::assemble::drawsurf::{
    MapSunEffects, RuntimeImageHandles, SmodelGpuPlan, WorldDrawGpuPlan, tess::sky::SkyModelDrawPlan,
};
use crate::prepare::scene::camera::FlyCamera;
use crate::prepare::scene::cull::{DynEntModelEntity, ScriptModelEntity, StaticModelEntity};
use crate::prepare::scene::world::WorldScene;
use crate::prepare::scene::world_gpu::{PipelineDemandTracker, WorldGpuReady};
use crate::prepare::scene::{
    model_lighting_atlas::WorldModelLightingAtlas, model_lighting_cache::WorldModelLightingCache,
    smodel_geom_cache::WorldStaticModelCache, smodel_lighting::WorldSmodelLighting,
};

/// Whether this frame carries a teardown. Reads the messages so they count as seen.
fn torn_down(torn: &mut MessageReader<MatchTornDown>) -> bool {
    torn.read().count() > 0
}

pub(crate) fn arm_world_spawn_on_install(
    mut installed: MessageReader<MatchInstalled>,
    mut job: ResMut<WorldSpawnJob>,
    mut gpu: ResMut<WorldGpuReady>,
) {
    let Some(install) = installed.read().last() else {
        return;
    };
    let spawn = WorldGeneration::from_install(install.request_id);
    *job = WorldSpawnJob {
        spawn,
        ..default()
    };
    *gpu = WorldGpuReady {
        spawn,
        ..default()
    };
}

pub(crate) fn despawn_world_entities_on_teardown(
    mut torn: MessageReader<MatchTornDown>,
    static_models: Query<Entity, With<StaticModelEntity>>,
    script_models: Query<Entity, With<ScriptModelEntity>>,
    dyn_ents: Query<Entity, With<DynEntModelEntity>>,
    mut commands: Commands,
) {
    if !torn_down(&mut torn) {
        return;
    }
    for entity in static_models
        .iter()
        .chain(script_models.iter())
        .chain(dyn_ents.iter())
    {
        commands.entity(entity).despawn();
    }
}

pub(crate) fn despawn_fly_cameras_on_teardown(
    mut torn: MessageReader<MatchTornDown>,
    cameras: Query<Entity, With<FlyCamera>>,
    mut commands: Commands,
) {
    if !torn_down(&mut torn) {
        return;
    }
    for camera in &cameras {
        commands.entity(camera).despawn();
    }
}

pub(crate) fn reset_world_spawn_on_teardown(
    mut torn: MessageReader<MatchTornDown>,
    mut job: ResMut<WorldSpawnJob>,
    mut gpu: ResMut<WorldGpuReady>,
    mut demand: ResMut<PipelineDemandTracker>,
    mut image_handles: Option<ResMut<RuntimeImageHandles>>,
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
    if let Some(handles) = image_handles.as_mut() {
        **handles = default();
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
                .resource::<WorldDrawGpuPlan>()
                .resource::<SmodelGpuPlan>()
                .resource::<SkyModelDrawPlan>()
                .resource::<WorldSmodelLighting>()
                .resource::<WorldStaticModelCache>()
                .resource::<WorldModelLightingAtlas>()
                .resource::<WorldModelLightingCache>();
        });
    });
}

pub(crate) fn shutdown_world_on_teardown(
    mut torn: MessageReader<MatchTornDown>,
    mut retiring: ResMut<Retiring>,
    mut scene: Option<ResMut<WorldScene>>,
    mut membership: Option<ResMut<DynEntCellBits>>,
    mut present: ResMut<render_scene::WorldPresentFacts>,
    mut tess: ResMut<render_scene::TessMaterials>,
    mut lookup: ResMut<render_scene::DynAtPointLookup>,
    mut cells: ResMut<render_scene::WorldDpvsCells>,
) {
    if !torn_down(&mut torn) {
        return;
    }
    if let Some(scene) = scene.as_mut() {
        retiring.hand_over(std::mem::take(&mut **scene));
    }
    // Read after the hand-over: the world left behind reports as not spawned.
    let spawned = scene.as_ref().map_or(0, |scene| i64::from(scene.spawned));
    if let Some(membership) = membership.as_mut() {
        **membership = default();
    }
    *present = default();
    *tess = default();
    lookup.clear();
    *cells = default();
    perf::world_hold(spawned, 0, 0);
    diag::info!(
        World,
        "world: shutdown — leftover geometry, tess plans, and model lighting dropped"
    );
}
