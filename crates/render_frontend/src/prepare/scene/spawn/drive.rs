//! The two systems that move a world spawn along: `spawn_world` runs the
//! early phases a slice at a time, `spawn_world_finish` places the tessellated
//! world once its plans exist.

use std::time::{Duration, Instant};

use bevy::prelude::*;
use frame::LaunchReport;
use render_fx::{HostFxSystem, PreparedFxCatalog, PreparedTracers};

use super::job::{WorldSpawnJob, WorldSpawnPhase};
use super::look;
use super::present::{WorldPresentAck, record_first_world_frame};
use crate::prepare::scene::world::WorldScene;
use crate::prepare::scene::world_gpu::{self, WorldGpuReady};
use crate::prepare::scene::world_images::{
    IMAGE_BYTES_PER_OVERLAY_FRAME, IMAGES_PER_OVERLAY_FRAME, MAX_IMAGE_BYTES_PER_SLICE,
    ResidentGpuImages,
};
use crate::prepare::scene::{world_occupancy, world_plan};

/// How long one frame may spend on spawn work while the loading overlay is up.
const SLICE_BUDGET: Duration = Duration::from_millis(40);

/// A frame at 60 Hz, the unit the per-frame image byte budget is scaled by.
const FRAME_MS: f32 = 16.7;

/// The loading progress of the load that is still running, if any. Pacing
/// follows the load, not the overlay: a run with no overlay spawns the world
/// the same way.
fn running_load(load: Option<&assets::MapLoadProcess>) -> Option<asset_transport::LoadProgress> {
    load.filter(|process| !process.is_complete())
        .map(|process| process.progress.clone())
}

fn mark_done(scene: &mut WorldScene, job: &mut WorldSpawnJob, commands: &mut Commands) {
    job.phase = WorldSpawnPhase::Done;
    scene.spawned = true;
    commands.insert_resource(render_scene::WorldPresentFacts { spawned: true });
    perf::world_ready(1);
}

pub(crate) fn spawn_world(
    mut commands: Commands,
    load: Option<Res<assets::MapLoadProcess>>,
    mut scene: ResMut<WorldScene>,
    mut images: ResMut<Assets<Image>>,
    mut common_images: ResMut<ResidentGpuImages>,
    mut job: ResMut<WorldSpawnJob>,
    mut tess: ResMut<render_scene::TessMaterials>,
    mut fx_host: Option<ResMut<HostFxSystem>>,
    gpu: Option<Res<WorldGpuReady>>,
    tracers: Option<Res<PreparedTracers>>,
    fx_catalog: Option<Res<PreparedFxCatalog>>,
    report: Option<Res<LaunchReport>>,
    present_ack: Res<WorldPresentAck>,
    prepared: (
        Res<render_anim::PreparedFpv>,
        Res<render_anim::PreparedModelMaterials>,
        Option<Res<render_fx::PreparedFxModels>>,
    ),
) {
    let (fpv, model_materials, fx_models) = prepared;
    let progress = running_load(load.as_deref());
    let paced = progress.is_some();

    if job.phase == WorldSpawnPhase::Gpu {
        if !paced {
            mark_done(&mut scene, &mut job, &mut commands);
            return;
        }
        // bo2zm: a world with no material images never starts either
        // preparation job, and a match with no weapon catalogs (a BO2 world
        // before weapons are read) never starts the first-person one; there
        // is nothing for them to prepare.
        let fpv_ready = ((fpv.settled_for(&tess.catalog) || fpv.inputs_absent())
            && model_materials.settled_for(&tess.catalog))
            || tess.material_images.is_empty();
        let gap_ms = job.slice_gap_ms(Instant::now());
        let spawn = job.spawn;
        let gpu_ready = world_gpu::poll(&mut job.gpu_wait, spawn, gpu.as_deref(), gap_ms);
        if gpu_ready && !fpv_ready {
            diag::info!(
                World,
                "world spawn: GPU ready; overlay holds for model and first-person preparation"
            );
        }
        if gpu_ready && fpv_ready {
            let quiet = job.gpu_wait.quiet();
            let waited_ms = job.gpu_wait.elapsed().as_secs_f32() * 1000.0;
            mark_done(&mut scene, &mut job, &mut commands);
            let gpu = gpu.as_deref();
            diag::info!(
                World,
                "world spawn: GPU ready quiet={quiet} images={} pipelines={} elapsed={waited_ms:.1}ms; overlay waits for admission",
                gpu.map_or(-1, |g| i32::from(g.images)),
                gpu.map_or(-1, |g| i32::from(g.pipelines)),
            );
            log_load_ledger(&job, gpu, &fpv, waited_ms);
        }
        return;
    }

    if scene.spawned {
        record_first_world_frame(progress.as_ref(), &job, &present_ack, report.as_deref());
        return;
    }

    if scene.batches.is_empty() {
        let no_gfx_world = report.as_ref().is_some_and(|report| {
            report
                .world_report
                .iter()
                .any(|line| line.contains("no GfxWorld reached"))
        });
        if paced && job.phase != WorldSpawnPhase::Done && no_gfx_world {
            diag::warn!(
                World,
                "world spawn: empty batches — spawn_world returns without Camera3d (no GfxWorld or world_draw produced none)"
            );
            job.phase = WorldSpawnPhase::Done;
        }
        return;
    }
    if job.phase == WorldSpawnPhase::Done {
        return;
    }

    if paced && job.phase == WorldSpawnPhase::Unarmed {
        job.phase = WorldSpawnPhase::Yield;
        diag::info!(
            World,
            "world spawn: yield one frame so the overlay can tick after match install"
        );
        return;
    }

    let started = Instant::now();
    let deadline = paced.then(|| started + SLICE_BUDGET);
    let out_of_time = || deadline.is_some_and(|end| Instant::now() >= end);

    match job.phase {
        WorldSpawnPhase::Unarmed | WorldSpawnPhase::Yield => {
            job.images.arm(&mut scene, progress.as_ref());
            job.phase = WorldSpawnPhase::Materials;
        }
        WorldSpawnPhase::Materials | WorldSpawnPhase::Images | WorldSpawnPhase::Plan => {}
        _ => return,
    }

    let images_done = if job.images.unfinished() {
        hand_over_image_slice(
            &mut job,
            &mut images,
            &mut common_images,
            started,
            deadline,
            paced,
        )
    } else {
        true
    };

    if job.phase == WorldSpawnPhase::Materials {
        look::publish(&mut commands, &scene);
        job.phase = WorldSpawnPhase::Images;
        job.last_work_ms = started.elapsed().as_secs_f32() * 1000.0;
        diag::info!(
            World,
            "world spawn slice: phase=materials install {:.1}ms (budget 40ms)",
            job.last_work_ms
        );
        if out_of_time() {
            return;
        }
    }

    if job.phase == WorldSpawnPhase::Images {
        if !images_done {
            return;
        }
        job.phase = WorldSpawnPhase::Plan;
        if out_of_time() {
            return;
        }
    }

    if job.phase == WorldSpawnPhase::Plan {
        world_plan::install(
            &mut commands,
            &scene,
            &images,
            job.images.exact_handles().to_vec(),
            job.images.probe_handles().to_vec(),
            job.images.lightmap_handles().to_vec(),
            tracers.as_deref(),
            fx_catalog.as_deref(),
        );
        let fx_geometry = world_plan::prepare_fx_model_geometry(&scene, fx_models.as_deref());
        commands.insert_resource(fx_geometry.0.clone());
        commands.insert_resource(fx_geometry);
        tess.material_images = std::sync::Arc::new(job.images.exact_handles().to_vec());
        if let Some(host) = fx_host.as_mut() {
            host.0.glass = fx::FxGlassSystemHost::default();
        }
        job.phase = WorldSpawnPhase::WorldTess;
    }
}

/// Runs one slice of the image hand-over and logs it. True once every image
/// has been handed over.
fn hand_over_image_slice(
    job: &mut WorldSpawnJob,
    images: &mut Assets<Image>,
    common_images: &mut ResidentGpuImages,
    started: Instant,
    deadline: Option<Instant>,
    paced: bool,
) -> bool {
    let gap_ms = job.slice_gap_ms(started);
    let max_images = if paced { IMAGES_PER_OVERLAY_FRAME } else { u32::MAX };
    // A slow frame earns a proportionally larger byte allowance.
    let byte_budget = (IMAGE_BYTES_PER_OVERLAY_FRAME as f32 * (gap_ms.max(FRAME_MS) / FRAME_MS))
        .round() as u64;
    let byte_budget = byte_budget.min(MAX_IMAGE_BYTES_PER_SLICE);
    let bytes_before = job.images.handed_bytes;

    let finished = job
        .images
        .until(images, common_images, deadline, max_images, byte_budget);

    job.last_work_ms = started.elapsed().as_secs_f32() * 1000.0;
    let up = &job.images;
    diag::info!(
        World,
        "world spawn slice: phase=images done={}/{} resident={} skipped={} reused_handles={}/{}B common_reused={}/{}B {:.1}ms gap={gap_ms:.1}ms handed_bytes={} total_handed={} byte_budget={byte_budget} largest_step={:.1}ms/{}B (budget 40ms; bytes are handed to Assets<Image>, not copied to the GPU; a reused handle is a slot whose variant was already an asset)",
        up.done,
        up.total,
        up.resident,
        up.skipped,
        up.reused_handles,
        up.reused_handle_bytes,
        up.reused_common_handles,
        up.reused_common_bytes,
        job.last_work_ms,
        up.handed_bytes - bytes_before,
        up.handed_bytes,
        up.largest_step_ns as f64 / 1.0e6,
        up.largest_step_bytes,
    );
    finished
}

pub(crate) fn spawn_world_finish(
    mut commands: Commands,
    load: Option<Res<assets::MapLoadProcess>>,
    mut scene: ResMut<WorldScene>,
    mut images: ResMut<Assets<Image>>,
    mut job: ResMut<WorldSpawnJob>,
    mut tess: ResMut<render_scene::TessMaterials>,
) {
    if job.phase != WorldSpawnPhase::WorldTess {
        return;
    }
    let progress = running_load(load.as_deref());
    let started = Instant::now();

    world_occupancy::place(
        &mut commands,
        &mut scene,
        &mut images,
        job.images.exact_handles().to_vec(),
        job.images.probe_handles().to_vec(),
        job.images.lightmap_handles().to_vec(),
    );
    tess.material_images = std::sync::Arc::new(job.images.exact_handles().to_vec());
    job.last_work_ms = started.elapsed().as_secs_f32() * 1000.0;

    let (done, total) = (job.images.done, job.images.total);
    if progress.is_some() {
        job.phase = WorldSpawnPhase::Gpu;
        job.gpu_wait.arm(progress.as_ref());
        diag::info!(
            World,
            "world spawn: Camera3d up; overlay holds for GPU images/pipelines last_slice={:.1}ms images={done}/{total}",
            job.last_work_ms
        );
        return;
    }
    mark_done(&mut scene, &mut job, &mut commands);
    diag::info!(
        World,
        "world spawn: complete last_slice={:.1}ms images={done}/{total} (no overlay)",
        job.last_work_ms
    );
}

/// One line summing up what the load cost and reused, written when the GPU
/// side reports ready.
fn log_load_ledger(
    job: &WorldSpawnJob,
    gpu: Option<&WorldGpuReady>,
    fpv: &render_anim::PreparedFpv,
    gpu_wait_ms: f32,
) {
    let up = &job.images;
    let compiled = gpu.and_then(|gpu| job.gpu_wait.pipelines_added(gpu.pipeline_n));
    let census = fpv.table().map(|table| table.census());
    diag::info!(
        World,
        "load ledger: images handed={} handed_bytes={} reused_slot={} reused_slot_bytes={} reused_common={} reused_common_bytes={} resident_map={} | pipelines cached={} compiled_this_load={} | fpv models={} rigs={} materials={} built_ms={} | gpu_wait_ms={gpu_wait_ms:.1}",
        up.done.saturating_sub(up.skipped).saturating_sub(up.reused_handles),
        up.handed_bytes,
        up.reused_handles,
        up.reused_handle_bytes,
        up.reused_common_handles,
        up.reused_common_bytes,
        up.resident,
        gpu.map_or_else(|| "?".to_owned(), |gpu| gpu.pipeline_n.to_string()),
        compiled.map_or_else(|| "?".to_owned(), |n| n.to_string()),
        census.map_or_else(|| "?".to_owned(), |c| c.models.to_string()),
        census.map_or_else(|| "?".to_owned(), |c| c.rigs.to_string()),
        census.map_or_else(|| "?".to_owned(), |c| c.materials.to_string()),
        census.map_or_else(|| "?".to_owned(), |c| format!("{:.1}", c.elapsed_ms)),
    );
}
