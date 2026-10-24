//! Telling the main world when the render world has actually drawn the spawned
//! world, so the first-frame time of a load is measured at the real frame.

use std::sync::{Arc, Mutex};
use std::time::Instant;

use bevy::prelude::*;
use bevy::render::{ExtractSchedule, MainWorld, Render, RenderApp, RenderSystems};
use frame::{LaunchReport, WorldGeneration};

use super::job::WorldSpawnJob;
use crate::prepare::scene::world::WorldScene;
use crate::prepare::scene::{world_gpu, world_images};

/// The generation of the world the render world last presented, and when.
/// One handle is shared by both worlds.
#[derive(Resource, Clone, Default)]
pub(crate) struct WorldPresentAck(Arc<Mutex<Option<(WorldGeneration, Instant)>>>);

impl WorldPresentAck {
    fn presented_at(&self, generation: WorldGeneration) -> Option<Instant> {
        let latest = self.0.lock().ok()?;
        match *latest {
            Some((seen, at)) if seen == generation => Some(at),
            _ => None,
        }
    }

    fn record(&self, generation: WorldGeneration) {
        if let Ok(mut latest) = self.0.lock() {
            *latest = Some((generation, Instant::now()));
        }
    }
}

/// Render-world copy of "the main world has a spawned world of this generation".
#[derive(Resource, Default)]
struct ExtractedWorldPresent(Option<WorldGeneration>);

pub(crate) fn register_world_gpu_ready(app: &mut App) {
    world_gpu::register_resources(app);
    app.init_resource::<world_images::ResidentGpuImages>();
    let ack = WorldPresentAck::default();
    app.insert_resource(ack.clone());

    let Some(render_app) = app.get_sub_app_mut(RenderApp) else {
        return;
    };
    render_app
        .insert_resource(ack)
        .init_resource::<ExtractedWorldPresent>()
        .add_systems(
            ExtractSchedule,
            (world_gpu::extract_images_ready, extract_world_present),
        )
        .add_systems(
            Render,
            acknowledge_world_present
                .after(RenderSystems::Render)
                .before(RenderSystems::Cleanup),
        );
}

fn extract_world_present(main: Res<MainWorld>, mut extracted: ResMut<ExtractedWorldPresent>) {
    let generation = main
        .get_resource::<WorldGeneration>()
        .copied()
        .unwrap_or(WorldGeneration(None));
    let spawned = main
        .get_resource::<WorldScene>()
        .is_some_and(|scene| scene.spawned);
    extracted.0 = (spawned && generation.0.is_some()).then_some(generation);
}

fn acknowledge_world_present(extracted: Res<ExtractedWorldPresent>, ack: Res<WorldPresentAck>) {
    if let Some(generation) = extracted.0 {
        ack.record(generation);
    }
}

/// Once the render world has presented this job's world, records the load's
/// first-frame measurement (once) and logs it.
pub(super) fn record_first_world_frame(
    progress: Option<&asset_transport::LoadProgress>,
    job: &WorldSpawnJob,
    ack: &WorldPresentAck,
    report: Option<&LaunchReport>,
) {
    let Some(progress) = progress else { return };
    let Some(presented_at) = ack.presented_at(job.spawn) else {
        return;
    };
    if !progress.record_first_frame(presented_at) {
        return;
    }
    let or_null = |value: Option<String>| value.unwrap_or_else(|| "NULL".to_owned());
    let zone = report.map_or("<unknown>", |report| report.zone.as_str());
    let image = or_null(progress.zone_image_bytes().map(|bytes| bytes.to_string()));
    let first_frame = or_null(progress.first_frame_ms().map(|ms| format!("{ms:.1}")));
    let rss_peak = or_null(progress.load_rss_peak_bytes().map(|bytes| bytes.to_string()));
    let rss_open = or_null(progress.rss_at_open_bytes().map(|bytes| bytes.to_string()));
    diag::info!(
        World,
        "load measurement: zone={zone} zone_image_bytes={image} first_frame_ms={first_frame} rss_peak_bytes={rss_peak} rss_at_open_bytes={rss_open}"
    );
}
