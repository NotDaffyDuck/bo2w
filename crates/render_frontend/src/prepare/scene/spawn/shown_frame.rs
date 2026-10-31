//! Telling the main world when the render world has actually drawn the spawned
//! world, so the first-frame time of a load is measured at the real frame.

use std::sync::{Arc, Mutex};
use std::time::Instant;

use bevy::prelude::*;
use bevy::render::ExtractSchedule;
use bevy::render::MainWorld;
use bevy::render::Render;
use bevy::render::RenderApp;
use bevy::render::RenderSystems;
use frame::{StartupReport, LevelGeneration};

use super::job::LevelSpawnJob;
use crate::prepare::scene::world::LevelStage;
use crate::prepare::scene::level_gpu;
use crate::prepare::scene::level_pictures;

/// The generation of the world the render world last presented, and when.
/// One handle is shared by both worlds.
#[derive(Resource, Clone, Default)]
pub(crate) struct LevelPresentAck(Arc<Mutex<Option<Presented>>>);

#[derive(Clone, Copy)]
struct Presented {
    generation: LevelGeneration,
    at: Instant,
}


impl LevelPresentAck {
    fn shown_at(&self, generation: LevelGeneration) -> Option<Instant> {
        let latest = self.0.lock().ok()?;
        match *latest {
            Some(Presented { generation: seen, at }) if seen == generation => Some(at),
            _ => None,
        }
    }

    fn record(&self, generation: LevelGeneration) {
        if let Ok(mut latest) = self.0.lock() {
            *latest = Some(Presented { generation, at: Instant::now() });
        }
    }
}

/// Render-world copy of "the main world has a spawned world of this generation".
#[derive(Resource, Default)]
struct ExtractedWorldShown(Option<LevelGeneration>);

pub(crate) fn register_level_gpu_ready(app: &mut App) {
    level_gpu::install(app);
    app.init_resource::<level_pictures::ResidentGpuImages>();
    let confirm_receipt = LevelPresentAck::default();
    app.insert_resource(confirm_receipt.clone());

    let Some(render_sub) = app.get_sub_app_mut(RenderApp) else {
        return;
    };
    render_sub
        .insert_resource(confirm_receipt)
        .init_resource::<ExtractedWorldShown>()
        .add_systems(
            ExtractSchedule,
            (level_gpu::count_resident_images, extract_world_present),
        )
        .add_systems(
            Render,
            acknowledge_level_present
                .after(RenderSystems::Render)
                .before(RenderSystems::Cleanup),
        );
}

fn extract_world_present(main: Res<MainWorld>, mut pulled: ResMut<ExtractedWorldShown>) {
    let epoch = main
        .get_resource::<LevelGeneration>()
        .copied()
        .unwrap_or(LevelGeneration(None));
    let spawned = main
        .get_resource::<LevelStage>()
        .is_some_and(|stage| stage.spawned);
    pulled.0 = (spawned && epoch.0.is_some()).then_some(epoch);
}

fn acknowledge_level_present(extracted: Res<ExtractedWorldShown>, confirm_receipt: Res<LevelPresentAck>) {
    if let Some(generation) = extracted.0 {
        confirm_receipt.record(generation);
    }
}

/// Once the render world has presented this job's world, records the load's
/// first-frame measurement (once) and logs it.
pub(super) fn note_first_world_frame(
    progress: Option<&asset_transport::LoadCompletion>,
    job: &LevelSpawnJob,
    confirm_receipt: &LevelPresentAck,
    report: Option<&StartupReport>,
) {
    let Some(progress) = progress else { return };
    let Some(presented_at) = confirm_receipt.shown_at(job.spawn) else {
        return;
    };
    if !progress.note_first_frame(presented_at) {
        return;
    }
    let or_null = |value: Option<String>| value.unwrap_or_else(|| "NULL".to_owned());
    let zone = report.map_or("<unknown>", |report| report.zone.as_str());
    let image = or_null(progress.zone_texture_bytes().map(|bytes| bytes.to_string()));
    let first_frame = or_null(progress.opening_tick_millis().map(|ms| format!("{ms:.1}")));
    let rss_peak = or_null(progress.read_resident_high_water().map(|bytes| bytes.to_string()));
    let rss_open = or_null(progress.memory_on_start().map(|bytes| bytes.to_string()));
    diag::info!(
        World,
        "load measurement: zone={zone} zone_image_bytes={image} first_frame_ms={first_frame} rss_peak_bytes={rss_peak} rss_at_open_bytes={rss_open}"
    );
}
