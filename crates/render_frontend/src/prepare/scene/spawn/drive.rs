//! The two systems that move a world spawn along: `spawn_world` runs the
//! early phases a slice at a time, `spawn_level_finish` places the tessellated
//! world once its plans exist.

use std::sync::Arc;
use std::time::{Duration, Instant};

use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use frame::StartupReport;
use render_fx::{EffectsSystemHandle, ReadyFxBook, StagedStreaks};

use super::job::{LevelSpawnJob, LevelSpawnPhase};
use super::look;
use super::shown_frame::{LevelPresentAck, note_first_world_frame};
use crate::assemble::draw_items::LiveLightbakeHandles;
use crate::prepare::scene::world::LevelStage;
use crate::prepare::scene::level_gpu::{LevelGpuReady, check_once};
use crate::prepare::scene::level_pictures::{
    IMAGE_BYTES_PER_TOPMOST_FRAME, IMAGES_PER_TOPMOST_FRAME, MAX_IMAGE_BYTES_PER_SLICE,
    ResidentGpuImages,
};
use crate::prepare::scene::{level_presence, level_plan};

/// How long one frame may spend on spawn work while the loading overlay is up.
const SLICE_BUDGET: Duration = Duration::from_millis(40);

/// A frame at 60 Hz, the unit the per-frame image byte budget is scaled by.
const FRAME_MS: f32 = 16.7;

/// Everything `spawn_world` only reads: what other systems have prepared and
/// what the launch has reported so far.
#[derive(SystemParam)]
pub(crate) struct SpawnReads<'w> {
    gpu_ready: Option<Res<'w, LevelGpuReady>>,
    tracer_defs: Option<Res<'w, StagedStreaks>>,
    effect_defs: Option<Res<'w, ReadyFxBook>>,
    launch: Option<Res<'w, StartupReport>>,
    first_frame: Res<'w, LevelPresentAck>,
    viewmodels: Res<'w, render_anim::PreparedFpv>,
    model_mats: Res<'w, render_anim::PreparedModelMaterials>,
    effect_models: Option<Res<'w, render_fx::ReadyFxProps>>,
}

/// Owned copies of the three handle tables the plan and the placement take:
/// exact material images, reflection probes, lightmaps.
struct HandleTables {
    exact: Vec<Option<Handle<Image>>>,
    probes: Vec<Option<Handle<Image>>>,
    lightmaps: Vec<Option<LiveLightbakeHandles>>,
}

fn handle_tables(job: &LevelSpawnJob) -> HandleTables {
    let up = &job.images;
    HandleTables {
        exact: up.precise_handles().to_vec(),
        probes: up.probe_refs().to_vec(),
        lightmaps: up.lightbake_handles().to_vec(),
    }
}

/// The loading progress of the load that is still running, if any. Pacing
/// follows the load, not the overlay: a run with no overlay spawns the world
/// the same way.
fn running_load(load: Option<&assets::MapLoadProcess>) -> Option<asset_transport::LoadCompletion> {
    let process = load?;
    (!process.is_done()).then(|| process.progress.clone())
}

fn ms_since(start: Instant) -> f32 {
    start.elapsed().as_secs_f32() * 1000.0
}

/// Ends the spawn: the world counts as up from here on.
fn conclude(commands: &mut Commands, stage: &mut LevelStage, job: &mut LevelSpawnJob) {
    job.stage_now = LevelSpawnPhase::Done;
    stage.spawned = true;
    let facts = render_scene::LevelPresentFacts {
        spawned: stage.spawned,
    };
    commands.insert_resource(facts);
    perf::level_ready(1);
}

pub(crate) fn spawn_level_world(
    mut commands: Commands,
    load: Option<Res<assets::MapLoadProcess>>,
    mut stage: ResMut<LevelStage>,
    mut image_store: ResMut<Assets<Image>>,
    mut common_images: ResMut<ResidentGpuImages>,
    mut job: ResMut<LevelSpawnJob>,
    mut tess: ResMut<render_scene::StitchMaterials>,
    mut effect_host: Option<ResMut<EffectsSystemHandle>>,
    reads: SpawnReads,
) {
    let progress = running_load(load.as_deref());
    let paced = progress.is_some();

    if job.stage_now == LevelSpawnPhase::Gpu {
        if paced {
            wait_for_gpu(&mut commands, &mut stage, &mut job, &tess, &reads);
        } else {
            conclude(&mut commands, &mut stage, &mut job);
        }
        return;
    }
    if stage.spawned {
        note_first_world_frame(
            progress.as_ref(),
            &job,
            &reads.first_frame,
            reads.launch.as_deref(),
        );
        return;
    }
    if stage.batches.is_empty() {
        give_up_on_missing_world(&mut job, paced, reads.launch.as_deref());
        return;
    }

    match job.stage_now {
        LevelSpawnPhase::Done => return,
        LevelSpawnPhase::Unarmed if paced => {
            job.stage_now = LevelSpawnPhase::Yield;
            diag::info!(
                World,
                "world spawn: wait a frame so the overlay ticks once the match is installed"
            );
            return;
        }
        LevelSpawnPhase::Unarmed | LevelSpawnPhase::Yield => {
            job.images.take_from(&mut stage, progress.as_ref());
            job.stage_now = LevelSpawnPhase::Materials;
        }
        LevelSpawnPhase::Materials | LevelSpawnPhase::Images | LevelSpawnPhase::Plan => {}
        LevelSpawnPhase::WorldTess | LevelSpawnPhase::Gpu => return,
    }

    let began = Instant::now();
    let deadline = paced.then(|| began + SLICE_BUDGET);

    // The image hand-over runs every slice while images remain, even while
    // the materials step is still to come.
    let images_done = !job.images.has_slots_left()
        || hand_over_image_slice(
            &mut job,
            &mut image_store,
            &mut common_images,
            began,
            deadline,
            paced,
        );

    loop {
        let next = match job.stage_now {
            LevelSpawnPhase::Materials => {
                look::publish(&mut commands, &stage);
                job.prior_work_ms = ms_since(began);
                diag::info!(
                    World,
                    "world spawn slice: phase=materials install {:.1}ms (budget 40ms)",
                    job.prior_work_ms
                );
                LevelSpawnPhase::Images
            }
            LevelSpawnPhase::Images if images_done => LevelSpawnPhase::Plan,
            LevelSpawnPhase::Plan => {
                let HandleTables { exact, probes, lightmaps } = handle_tables(&job);
                tess.shader_textures = Arc::new(exact.clone());
                level_plan::register_into(
                    &mut commands,
                    &stage,
                    &image_store,
                    exact,
                    probes,
                    lightmaps,
                    reads.tracer_defs.as_deref(),
                    reads.effect_defs.as_deref(),
                );
                let fx_geometry =
                    level_plan::prepare_fx_model_geometry(&stage, reads.effect_models.as_deref());
                commands.insert_resource(fx_geometry.0.clone());
                commands.insert_resource(fx_geometry);
                if let Some(host) = effect_host.as_deref_mut() {
                    host.0.glass = fx::PaneShardOwner::default();
                }
                LevelSpawnPhase::WorldTess
            }
            _ => return,
        };
        job.stage_now = next;
        let over_budget = deadline.is_some_and(|end| Instant::now() >= end);
        if next == LevelSpawnPhase::WorldTess || over_budget {
            return;
        }
    }
}

/// The world drew no batches. A paced load whose launch report says no
/// GfxWorld arrived stops here for good; otherwise the spawn keeps waiting.
fn give_up_on_missing_world(job: &mut LevelSpawnJob, paced: bool, launch: Option<&StartupReport>) {
    if !paced || job.stage_now == LevelSpawnPhase::Done {
        return;
    }
    let no_world = launch.is_some_and(|launch| {
        launch
            .level_digest
            .iter()
            .any(|entry| entry.contains("no GfxWorld reached"))
    });
    if no_world {
        diag::warn!(
            World,
            "world spawn: no batches, so spawn_world leaves without a Camera3d (no GfxWorld, or world_draw gave none)"
        );
        job.stage_now = LevelSpawnPhase::Done;
    }
}

/// The GPU phase of a paced load: the spawn ends once the GPU has taken the
/// images and pipelines and the model and first-person preparations settle.
fn wait_for_gpu(
    commands: &mut Commands,
    scene: &mut LevelStage,
    job: &mut LevelSpawnJob,
    tess: &render_scene::StitchMaterials,
    reads: &SpawnReads,
) {
    // bo2zm: a world with no material images never starts either
    // preparation job, and a match with no weapon catalogs (a BO2 world
    // before weapons are read) never starts the first-person one; there
    // is nothing for them to prepare.
    let catalog = &tess.catalog;
    let viewmodels_settled =
        reads.viewmodels.settled_for(catalog) || reads.viewmodels.inputs_absent();
    let preparations_settled = tess.shader_textures.is_empty()
        || (viewmodels_settled && reads.model_mats.settled_for(catalog));

    let gap_ms = job.slice_gap_millis(Instant::now());
    let epoch = job.spawn;
    let ready = reads.gpu_ready.as_deref();
    if !check_once(&mut job.gpu_wait, epoch, ready, gap_ms) {
        return;
    }
    if !preparations_settled {
        diag::info!(
            World,
            "world spawn: GPU ready; overlay holds for model and first-person preparation"
        );
        return;
    }

    let quiet = job.gpu_wait.steady_frames();
    let waited_ms = job.gpu_wait.elapsed().as_secs_f32() * 1000.0;
    conclude(commands, scene, job);
    diag::info!(
        World,
        "world spawn: GPU is ready, quiet={quiet} images={} pipelines={} after {waited_ms:.1}ms; the overlay waits to be admitted",
        ready.map_or(-1, |r| i32::from(r.images_resident)),
        ready.map_or(-1, |r| i32::from(r.pipelines_warm)),
    );
    log_load_ledger(job, ready, &reads.viewmodels, waited_ms);
}

/// Runs one slice of the image hand-over and logs it. True once every image
/// has been handed over.
fn hand_over_image_slice(
    job: &mut LevelSpawnJob,
    gpu_images: &mut Assets<Image>,
    common_images: &mut ResidentGpuImages,
    slice_start: Instant,
    due_at: Option<Instant>,
    paced: bool,
) -> bool {
    let gap_ms = job.slice_gap_millis(slice_start);
    let image_cap = if paced { IMAGES_PER_TOPMOST_FRAME } else { u32::MAX };
    // A slow frame earns a proportionally larger byte allowance.
    let frames_elapsed = gap_ms.max(FRAME_MS) / FRAME_MS;
    let byte_budget = ((IMAGE_BYTES_PER_TOPMOST_FRAME as f32 * frames_elapsed).round() as u64)
        .min(MAX_IMAGE_BYTES_PER_SLICE);
    let handed_before = job.images.handed_bytes;

    let finished = job
        .images
        .hand_over_until(gpu_images, common_images, due_at, image_cap, byte_budget);

    job.prior_work_ms = ms_since(slice_start);
    let up = &job.images;
    diag::info!(
        World,
        "world spawn slice: phase=images done={}/{} resident={} skipped={} reused_handles={}/{}B common_reused={}/{}B {:.1}ms gap={gap_ms:.1}ms handed_bytes={} total_handed={} byte_budget={byte_budget} largest_step={:.1}ms/{}B (budget 40ms; bytes are handed to Assets<Image>, not copied to the GPU; a reused handle is a slot whose variant was already an asset)",
        up.finished,
        up.expected,
        up.resident,
        up.passed_over,
        up.reused_handles,
        up.reused_handle_bytes,
        up.reused_common_handles,
        up.reused_common_bytes,
        job.prior_work_ms,
        up.handed_bytes - handed_before,
        up.handed_bytes,
        up.largest_step_ns as f64 / 1.0e6,
        up.largest_step_bytes,
    );
    finished
}

pub(crate) fn spawn_level_finish(
    mut commands: Commands,
    load: Option<Res<assets::MapLoadProcess>>,
    mut stage: ResMut<LevelStage>,
    mut gpu_images: ResMut<Assets<Image>>,
    mut job: ResMut<LevelSpawnJob>,
    mut tess: ResMut<render_scene::StitchMaterials>,
) {
    if job.stage_now != LevelSpawnPhase::WorldTess {
        return;
    }
    let progress = running_load(load.as_deref());
    let began = Instant::now();

    let HandleTables { exact, probes, lightmaps } = handle_tables(&job);
    let shared = Arc::new(exact.clone());
    level_presence::populate_world(
        &mut commands,
        &mut stage,
        &mut gpu_images,
        level_presence::RegistryHandles {
            precise_materials: exact,
            env_probes: probes,
            lightbakes: lightmaps,
        },
    );
    tess.shader_textures = shared;
    job.prior_work_ms = ms_since(began);

    let slice_ms = job.prior_work_ms;
    let (done, total) = (job.images.finished, job.images.expected);
    match progress {
        Some(progress) => {
            job.stage_now = LevelSpawnPhase::Gpu;
            job.gpu_wait.arm_t(Some(&progress));
            diag::info!(
                World,
                "world spawn: Camera3d is up; the overlay waits on GPU images/pipelines, last_slice={slice_ms:.1}ms images={done}/{total}"
            );
        }
        None => {
            conclude(&mut commands, &mut stage, &mut job);
            diag::info!(
                World,
                "world spawn: complete last_slice={slice_ms:.1}ms images={done}/{total} (no overlay)"
            );
        }
    }
}

/// One line summing up what the load cost and reused, written when the GPU
/// side reports ready.
fn log_load_ledger(
    spawn_job: &LevelSpawnJob,
    ready: Option<&LevelGpuReady>,
    viewmodels: &render_anim::PreparedFpv,
    gpu_wait_ms: f32,
) {
    let up = &spawn_job.images;
    let unknown = || "?".to_owned();
    let compiled = ready.and_then(|r| spawn_job.gpu_wait.pipelines_added(r.pipeline_tally));
    let census = viewmodels.table().map(|table| table.census());
    diag::info!(
        World,
        "load ledger: images handed={} handed_bytes={} reused_slot={} reused_slot_bytes={} reused_common={} reused_common_bytes={} resident_map={} | pipelines cached={} compiled_this_load={} | fpv models={} rigs={} materials={} built_ms={} | gpu_wait_ms={gpu_wait_ms:.1}",
        up.finished.saturating_sub(up.passed_over).saturating_sub(up.reused_handles),
        up.handed_bytes,
        up.reused_handles,
        up.reused_handle_bytes,
        up.reused_common_handles,
        up.reused_common_bytes,
        up.resident,
        ready.map_or_else(unknown, |r| r.pipeline_tally.to_string()),
        compiled.map_or_else(unknown, |count| count.to_string()),
        census.map_or_else(unknown, |c| c.models.to_string()),
        census.map_or_else(unknown, |c| c.rigs.to_string()),
        census.map_or_else(unknown, |c| c.materials.to_string()),
        census.map_or_else(unknown, |c| format!("{:.1}", c.elapsed_ms)),
    );
}
