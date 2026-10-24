//! The GPU side of loading a world: knowing when its images are resident and
//! its pipelines compiled, and holding the loading screen until the picture
//! has been steady for a couple of frames.

use bevy::asset::RenderAssetUsages;
use bevy::prelude::*;
use frame::WorldGeneration;

use super::spawn::{WorldSpawnJob, WorldSpawnPhase, spawn_world};
use asset_transport::{LoadProgress, StageHandle, StageId};
use std::collections::HashSet;
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Frames the world must stay ready before the loading screen may go.
const QUIET_FRAMES_NEEDED: u32 = 2;

/// The loading progress bars of the GPU phase.
#[derive(Default)]
struct GpuStages {
    images: Option<StageHandle>,
    pipelines: Option<StageHandle>,
    frames: Option<StageHandle>,
}

impl GpuStages {
    fn begin(progress: &LoadProgress) -> Self {
        Self {
            images: Some(progress.begin(StageId::GpuTextures, None)),
            pipelines: Some(progress.begin(StageId::Pipelines, None)),
            frames: Some(progress.begin(
                StageId::RenderFrames,
                Some(u64::from(QUIET_FRAMES_NEEDED)),
            )),
        }
    }

    fn cancel(&mut self) {
        for stage in [
            self.images.take(),
            self.pipelines.take(),
            self.frames.take(),
        ]
        .into_iter()
        .flatten()
        {
            stage.cancel();
        }
    }

    fn finish(&mut self) {
        for stage in [
            self.frames.take(),
            self.images.take(),
            self.pipelines.take(),
        ]
        .into_iter()
        .flatten()
        {
            stage.done();
        }
    }
}

/// How far the GPU phase of one world spawn has got.
#[derive(Default)]
pub struct WorldGpuWait {
    started: Option<Instant>,
    quiet: u32,
    pipelines_at_arm: Option<u32>,
    stages: GpuStages,
}

impl WorldGpuWait {
    /// Starts the wait over. Bars of a wait that never finished are
    /// cancelled: the world changed under it, it did not quietly succeed.
    pub fn arm(&mut self, progress: Option<&LoadProgress>) {
        self.started = Some(Instant::now());
        self.quiet = 0;
        self.pipelines_at_arm = None;
        self.stages.cancel();
        if let Some(progress) = progress {
            self.stages = GpuStages::begin(progress);
        }
    }

    /// Consecutive frames the world has been ready.
    pub(crate) fn quiet(&self) -> u32 {
        self.quiet
    }

    /// Pipelines compiled since the wait was armed, once known.
    pub(crate) fn pipelines_added(&self, now: u32) -> Option<u32> {
        self.pipelines_at_arm.map(|at| now.saturating_sub(at))
    }

    pub(crate) fn elapsed(&self) -> Duration {
        self.started.map_or(Duration::ZERO, |at| at.elapsed())
    }

    /// Moves the image and pipeline bars to what the GPU reports, closing
    /// each one as its gate opens. Residency and warmth are gauges that may
    /// fall between frames, so only the gates say a bar is done.
    fn show_gpu_progress(&mut self, gpu: &WorldGpuReady) {
        self.pipelines_at_arm.get_or_insert(gpu.pipeline_n);
        if let Some(bar) = &self.stages.images {
            bar.set_total(u64::from(gpu.image_need_n));
            bar.set_completed(u64::from(gpu.image_ready_n));
        }
        if gpu.images
            && let Some(bar) = self.stages.images.take()
        {
            bar.done();
        }
        if let Some(bar) = &self.stages.pipelines {
            bar.set_total(u64::from(gpu.pipeline_n));
            bar.set_completed(u64::from(gpu.pipeline_n.saturating_sub(gpu.waiting_n)));
        }
        if gpu.pipelines
            && let Some(bar) = self.stages.pipelines.take()
        {
            bar.done();
        }
    }
}

/// What the render side has reported about the world being loaded.
#[derive(Resource, Debug, Default, Clone, Copy)]
pub struct WorldGpuReady {
    pub spawn: WorldGeneration,
    pub images: bool,
    pub pipelines: bool,
    pub waiting_n: u32,
    pub pipeline_n: u32,
    pub image_ready_n: u32,
    pub image_need_n: u32,
}

impl WorldGpuReady {
    fn is_ready_for(&self, spawn: WorldGeneration) -> bool {
        self.spawn.0.is_some() && self.spawn == spawn && self.images && self.pipelines
    }
}

/// What the frontend asks the render side to prepare.
#[derive(Resource, Clone, Debug, Default)]
pub struct GpuSubmitDemand {
    pub world_generation: WorldGeneration,
    /// Compile pipelines ahead of need.
    pub warm_pipelines: bool,
    /// The loading screen is waiting on the GPU.
    pub overlay_gpu_wait: bool,
    /// Count the images that are resident.
    pub wants_residency: bool,
    pub pipeline_world_materials: Arc<HashSet<u16>>,
    pub pipeline_smodel_materials: Arc<HashSet<u16>>,
    pub pipeline_demand_revision: u64,
}

/// Holds the working set of materials whose pipelines are wanted. A spawn's
/// snapshot is adopted when it is a different allocation from the held one.
#[derive(Resource, Clone, Debug, Default)]
pub struct PipelineDemandTracker {
    spawn_world: Arc<HashSet<u16>>,
    spawn_smodel: Arc<HashSet<u16>>,
    revision: u64,
}

impl PipelineDemandTracker {
    fn adopt(&mut self, world: Arc<HashSet<u16>>, smodel: Arc<HashSet<u16>>) {
        if Arc::ptr_eq(&self.spawn_world, &world) && Arc::ptr_eq(&self.spawn_smodel, &smodel) {
            return;
        }
        self.spawn_world = world;
        self.spawn_smodel = smodel;
        self.revision = self.revision.wrapping_add(1);
    }

    pub fn snapshot(&self) -> (Arc<HashSet<u16>>, Arc<HashSet<u16>>, u64) {
        (
            Arc::clone(&self.spawn_world),
            Arc::clone(&self.spawn_smodel),
            self.revision,
        )
    }
}

/// The render side's latest report, copied over by the render app.
#[derive(Resource, Clone, Debug, Default)]
pub struct GpuLoadProgress {
    pub waiting_n: u32,
    pub pipeline_n: u32,
    pub pending_shaders: Vec<bevy::asset::AssetId<bevy::shader::Shader>>,
    pub residency: Option<GpuImageResidency>,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct GpuImageResidency {
    pub need: u32,
    pub ready: u32,
}

/// True while the GPU phase of this very spawn is what the loading screen
/// waits for.
pub fn spawn_gpu_extract_live(
    gpu_spawn: WorldGeneration,
    job_spawn: Option<WorldGeneration>,
    overlay_gpu_wait: bool,
    world_gen: WorldGeneration,
) -> bool {
    world_gen.0.is_some()
        && gpu_spawn == world_gen
        && job_spawn == Some(world_gen)
        && overlay_gpu_wait
}

pub(crate) fn publish_gpu_submit_demand(
    spawn: Option<Res<WorldSpawnJob>>,
    generation: Option<Res<WorldGeneration>>,
    ready: Res<WorldGpuReady>,
    mut tracker: ResMut<PipelineDemandTracker>,
    mut demand: ResMut<GpuSubmitDemand>,
) {
    let job = spawn.as_deref();
    let phase = job.map(|job| job.phase);
    let world_generation = generation.map(|g| *g).unwrap_or_default();
    let overlay_gpu_wait = phase == Some(WorldSpawnPhase::Gpu);
    let warm_pipelines = matches!(
        phase,
        Some(
            WorldSpawnPhase::Images
                | WorldSpawnPhase::Plan
                | WorldSpawnPhase::WorldTess
                | WorldSpawnPhase::Gpu
        )
    );
    tracker.adopt(
        job.map(|job| job.images.pipeline_world_materials.clone())
            .unwrap_or_default(),
        job.map(|job| job.images.pipeline_smodel_materials.clone())
            .unwrap_or_default(),
    );
    let (pipeline_world_materials, pipeline_smodel_materials, pipeline_demand_revision) =
        tracker.snapshot();
    *demand = GpuSubmitDemand {
        world_generation,
        warm_pipelines,
        overlay_gpu_wait,
        wants_residency: spawn_gpu_extract_live(
            ready.spawn,
            job.map(|job| job.spawn),
            overlay_gpu_wait,
            world_generation,
        ),
        pipeline_world_materials,
        pipeline_smodel_materials,
        pipeline_demand_revision,
    };
}

/// One frame of the GPU wait. True once the world has been ready for
/// `QUIET_FRAMES_NEEDED` frames in a row; the bars are closed then.
pub(crate) fn poll(
    wait: &mut WorldGpuWait,
    spawn: WorldGeneration,
    gpu: Option<&WorldGpuReady>,
    gap_ms: f32,
) -> bool {
    if let Some(gpu) = gpu {
        wait.show_gpu_progress(gpu);
    }
    let ready = gpu.is_some_and(|gpu| gpu.is_ready_for(spawn));
    wait.quiet = if ready { wait.quiet.saturating_add(1) } else { 0 };
    if let Some(bar) = &wait.stages.frames {
        bar.set_completed(u64::from(wait.quiet.min(QUIET_FRAMES_NEEDED)));
    }

    // -1 in the log stands for "no report yet".
    let or_unknown = |pick: fn(&WorldGpuReady) -> i64| gpu.map_or(-1, pick);
    diag::info!(
        World,
        "world spawn slice: phase=gpu quiet={}/{QUIET_FRAMES_NEEDED} images={} pipelines={} waiting={} total={} img={}/{} gap={gap_ms:.1}ms",
        wait.quiet,
        or_unknown(|g| i64::from(g.images)),
        or_unknown(|g| i64::from(g.pipelines)),
        or_unknown(|g| i64::from(g.waiting_n)),
        or_unknown(|g| i64::from(g.pipeline_n)),
        or_unknown(|g| i64::from(g.image_ready_n)),
        or_unknown(|g| i64::from(g.image_need_n)),
    );

    if ready && wait.quiet >= QUIET_FRAMES_NEEDED {
        wait.stages.finish();
        return true;
    }
    false
}

pub(crate) fn register_resources(app: &mut App) {
    app.init_resource::<WorldGpuReady>()
        .init_resource::<GpuSubmitDemand>()
        .init_resource::<PipelineDemandTracker>()
        .init_resource::<GpuLoadProgress>()
        .add_systems(
            Update,
            (
                consume_gpu_load_progress.before(spawn_world),
                publish_gpu_submit_demand.after(spawn_world),
            ),
        );
}

/// Render-world system: counts the images that are resident on the GPU and
/// leaves the count in the main world's `GpuLoadProgress`. Only counts while
/// the frontend wants it.
pub(crate) fn extract_images_ready(
    mut main_world: ResMut<bevy::render::MainWorld>,
    gpu_images: Res<bevy::render::render_asset::RenderAssets<bevy::render::texture::GpuImage>>,
) {
    let counting = main_world
        .get_resource::<GpuSubmitDemand>()
        .is_some_and(|demand| demand.wants_residency);
    let residency = counting.then(|| {
        let images = main_world.resource::<Assets<Image>>();
        let mut counts = GpuImageResidency::default();
        for (id, image) in images.iter() {
            if image.asset_usage.contains(RenderAssetUsages::RENDER_WORLD) {
                counts.need = counts.need.saturating_add(1);
                counts.ready += u32::from(gpu_images.get(id).is_some());
            }
        }
        counts
    });
    if let Some(mut progress) = main_world.get_resource_mut::<GpuLoadProgress>() {
        progress.residency = residency;
    }
}

pub(crate) fn consume_gpu_load_progress(
    progress: Res<GpuLoadProgress>,
    generation: Option<Res<WorldGeneration>>,
    demand: Res<GpuSubmitDemand>,
    job: Option<Res<WorldSpawnJob>>,
    mut ready: ResMut<WorldGpuReady>,
) {
    let world_gen = generation.map_or(WorldGeneration(None), |g| *g);
    ready.waiting_n = progress.waiting_n;
    ready.pipeline_n = progress.pipeline_n;
    match progress.residency {
        Some(counts) => {
            ready.image_need_n = counts.need;
            ready.image_ready_n = counts.ready;
            ready.images = counts.need > 0 && counts.ready == counts.need;
        }
        None => ready.images = false,
    }
    let gpu_phase_live = spawn_gpu_extract_live(
        ready.spawn,
        job.as_ref().map(|job| job.spawn),
        demand.overlay_gpu_wait,
        world_gen,
    );
    // The pipelines pass through the cache; the world is warm when none are
    // still compiling.
    ready.pipelines = gpu_phase_live && progress.waiting_n == 0;
}

#[cfg(test)]
mod tests {
    use asset_transport::{StageOutcome, StageSnapshot, WorkCount};
    use bevy::ecs::system::RunSystemOnce;

    use super::*;

    fn gen_of(id: u64) -> WorldGeneration {
        WorldGeneration::from_install(id)
    }

    fn ready_for(id: u64) -> WorldGpuReady {
        WorldGpuReady {
            spawn: gen_of(id),
            images: true,
            pipelines: true,
            ..WorldGpuReady::default()
        }
    }

    fn stage(progress: &LoadProgress, id: StageId) -> Vec<StageSnapshot> {
        progress
            .snapshot()
            .stages
            .into_iter()
            .filter(|stage| stage.key.id == id)
            .collect()
    }

    #[test]
    fn a_report_is_ready_only_for_its_own_spawn_with_images_and_pipelines() {
        let ready = ready_for(4);
        assert!(ready.is_ready_for(gen_of(4)));
        assert!(!ready.is_ready_for(gen_of(5)));
        assert!(!ready.is_ready_for(WorldGeneration(None)));
        assert!(
            !WorldGpuReady {
                images: false,
                ..ready
            }
            .is_ready_for(gen_of(4))
        );
        assert!(
            !WorldGpuReady {
                pipelines: false,
                ..ready
            }
            .is_ready_for(gen_of(4))
        );
        // A report that names no spawn is ready for nobody, itself included.
        assert!(
            !WorldGpuReady {
                spawn: WorldGeneration(None),
                ..ready
            }
            .is_ready_for(WorldGeneration(None))
        );
    }

    #[test]
    fn the_gpu_extract_is_live_only_for_the_spawn_the_loading_screen_waits_on() {
        let g = gen_of(3);
        assert!(spawn_gpu_extract_live(g, Some(g), true, g));
        assert!(!spawn_gpu_extract_live(g, Some(g), false, g));
        assert!(!spawn_gpu_extract_live(gen_of(2), Some(g), true, g));
        assert!(!spawn_gpu_extract_live(g, Some(gen_of(2)), true, g));
        assert!(!spawn_gpu_extract_live(g, None, true, g));
        let none = WorldGeneration(None);
        assert!(!spawn_gpu_extract_live(none, Some(none), true, none));
    }

    #[test]
    fn a_wait_that_was_never_armed_has_no_time_and_no_quiet_frames() {
        let wait = WorldGpuWait::default();
        assert_eq!(wait.elapsed(), Duration::ZERO);
        assert_eq!(wait.quiet(), 0);
        assert_eq!(wait.pipelines_added(50), None);
    }

    #[test]
    fn arming_starts_the_clock_and_forgets_the_old_wait() {
        let mut wait = WorldGpuWait::default();
        wait.quiet = 5;
        wait.pipelines_at_arm = Some(3);
        wait.arm(None);
        assert_eq!(wait.quiet(), 0);
        assert_eq!(wait.pipelines_added(9), None);
        assert!(wait.started.is_some());
    }

    #[test]
    fn nothing_is_ready_until_the_render_side_reports() {
        let mut wait = WorldGpuWait::default();
        wait.arm(None);
        for _ in 0..4 {
            assert!(!poll(&mut wait, gen_of(1), None, 0.0));
        }
        assert_eq!(wait.quiet(), 0);
    }

    #[test]
    fn the_world_must_stay_ready_for_two_frames_in_a_row() {
        let mut wait = WorldGpuWait::default();
        wait.arm(None);
        let gpu = ready_for(1);
        assert!(!poll(&mut wait, gen_of(1), Some(&gpu), 0.0));
        assert_eq!(wait.quiet(), 1);
        assert!(poll(&mut wait, gen_of(1), Some(&gpu), 0.0));
        assert_eq!(wait.quiet(), 2);
    }

    #[test]
    fn a_frame_that_is_not_ready_starts_the_count_again() {
        let mut wait = WorldGpuWait::default();
        wait.arm(None);
        let gpu = ready_for(1);
        let busy = WorldGpuReady {
            pipelines: false,
            ..gpu
        };
        assert!(!poll(&mut wait, gen_of(1), Some(&gpu), 0.0));
        assert!(!poll(&mut wait, gen_of(1), Some(&busy), 0.0));
        assert_eq!(wait.quiet(), 0);
        assert!(!poll(&mut wait, gen_of(1), Some(&gpu), 0.0));
        assert!(poll(&mut wait, gen_of(1), Some(&gpu), 0.0));
    }

    #[test]
    fn a_report_for_another_spawn_never_counts() {
        let mut wait = WorldGpuWait::default();
        wait.arm(None);
        let stale = ready_for(1);
        for _ in 0..5 {
            assert!(!poll(&mut wait, gen_of(2), Some(&stale), 0.0));
        }
        assert_eq!(wait.quiet(), 0);
    }

    #[test]
    fn the_first_report_fixes_the_pipeline_count_the_wait_began_with() {
        let mut wait = WorldGpuWait::default();
        wait.arm(None);
        let first = WorldGpuReady {
            pipeline_n: 10,
            ..WorldGpuReady::default()
        };
        poll(&mut wait, gen_of(1), Some(&first), 0.0);
        let later = WorldGpuReady {
            pipeline_n: 25,
            ..WorldGpuReady::default()
        };
        poll(&mut wait, gen_of(1), Some(&later), 0.0);
        assert_eq!(wait.pipelines_added(25), Some(15));
        // The count never goes negative if the cache was trimmed meanwhile.
        assert_eq!(wait.pipelines_added(4), Some(0));
    }

    #[test]
    fn the_bars_follow_what_the_gpu_reports() {
        let progress = LoadProgress::new(1);
        let mut wait = WorldGpuWait::default();
        wait.arm(Some(&progress));
        let gpu = WorldGpuReady {
            image_need_n: 10,
            image_ready_n: 4,
            pipeline_n: 8,
            waiting_n: 3,
            ..WorldGpuReady::default()
        };
        assert!(!poll(&mut wait, gen_of(1), Some(&gpu), 0.0));

        let images = &stage(&progress, StageId::GpuTextures)[0];
        assert!(images.running());
        assert_eq!(
            images.count,
            Some(WorkCount {
                completed: 4,
                total: Some(10)
            })
        );
        let pipelines = &stage(&progress, StageId::Pipelines)[0];
        assert!(pipelines.running());
        assert_eq!(
            pipelines.count,
            Some(WorkCount {
                completed: 5,
                total: Some(8)
            })
        );
        let frames = &stage(&progress, StageId::RenderFrames)[0];
        assert_eq!(
            frames.count,
            Some(WorkCount {
                completed: 0,
                total: Some(2)
            })
        );
    }

    #[test]
    fn each_bar_closes_when_its_own_gate_opens() {
        let progress = LoadProgress::new(1);
        let mut wait = WorldGpuWait::default();
        wait.arm(Some(&progress));
        let images_only = WorldGpuReady {
            spawn: gen_of(1),
            images: true,
            image_need_n: 6,
            image_ready_n: 6,
            ..WorldGpuReady::default()
        };
        poll(&mut wait, gen_of(1), Some(&images_only), 0.0);
        assert_eq!(
            stage(&progress, StageId::GpuTextures)[0].outcome(),
            Some(StageOutcome::Done)
        );
        assert!(stage(&progress, StageId::Pipelines)[0].running());

        // A gauge falling back afterwards does not reopen a closed bar.
        let fallen = WorldGpuReady {
            images: false,
            image_ready_n: 2,
            ..images_only
        };
        poll(&mut wait, gen_of(1), Some(&fallen), 0.0);
        assert_eq!(
            stage(&progress, StageId::GpuTextures)[0].outcome(),
            Some(StageOutcome::Done)
        );
    }

    #[test]
    fn the_quiet_frames_bar_never_reads_past_its_total() {
        let progress = LoadProgress::new(1);
        let mut wait = WorldGpuWait::default();
        wait.arm(Some(&progress));
        let gpu = ready_for(1);
        poll(&mut wait, gen_of(1), Some(&gpu), 0.0);
        let frames = &stage(&progress, StageId::RenderFrames)[0];
        assert_eq!(frames.count.map(|c| c.completed), Some(1));
        assert!(poll(&mut wait, gen_of(1), Some(&gpu), 0.0));
    }

    #[test]
    fn finishing_the_wait_closes_every_bar_as_done() {
        let progress = LoadProgress::new(1);
        let mut wait = WorldGpuWait::default();
        wait.arm(Some(&progress));
        let gpu = ready_for(1);
        poll(&mut wait, gen_of(1), Some(&gpu), 0.0);
        assert!(poll(&mut wait, gen_of(1), Some(&gpu), 0.0));
        for id in [StageId::GpuTextures, StageId::Pipelines, StageId::RenderFrames] {
            let stages = stage(&progress, id);
            assert_eq!(stages.len(), 1);
            assert_eq!(stages[0].outcome(), Some(StageOutcome::Done), "{id:?}");
        }
    }

    #[test]
    fn arming_again_cancels_the_bars_of_a_wait_that_never_finished() {
        let progress = LoadProgress::new(1);
        let mut wait = WorldGpuWait::default();
        wait.arm(Some(&progress));
        wait.arm(Some(&progress));
        let first_round: Vec<_> = progress
            .snapshot()
            .stages
            .into_iter()
            .filter(|stage| stage.outcome() == Some(StageOutcome::Cancelled))
            .collect();
        assert_eq!(first_round.len(), 3);
        for id in [StageId::GpuTextures, StageId::Pipelines, StageId::RenderFrames] {
            let stages = stage(&progress, id);
            assert_eq!(stages.len(), 2);
            assert_eq!(stages[0].outcome(), Some(StageOutcome::Cancelled));
            assert!(stages[1].running());
        }
    }

    #[test]
    fn the_pipeline_demand_changes_revision_only_for_a_new_allocation() {
        let world = Arc::new(HashSet::from([1u16, 2]));
        let smodel = Arc::new(HashSet::from([9u16]));
        let mut tracker = PipelineDemandTracker::default();

        tracker.adopt(world.clone(), smodel.clone());
        let (w, s, rev) = tracker.snapshot();
        assert!(Arc::ptr_eq(&w, &world) && Arc::ptr_eq(&s, &smodel));
        assert_eq!(rev, 1);

        // The same allocations again: nothing changed.
        tracker.adopt(world.clone(), smodel.clone());
        assert_eq!(tracker.snapshot().2, 1);

        // Equal contents in a different allocation still count as new: the
        // render side compares revisions, not sets.
        tracker.adopt(Arc::new((*world).clone()), smodel.clone());
        assert_eq!(tracker.snapshot().2, 2);
        tracker.adopt(world.clone(), Arc::new(HashSet::new()));
        assert_eq!(tracker.snapshot().2, 3);
    }

    fn job_in(phase: WorldSpawnPhase, spawn: u64) -> WorldSpawnJob {
        let mut job = WorldSpawnJob::default();
        job.phase = phase;
        job.spawn = gen_of(spawn);
        job
    }

    fn demand_world(phase: Option<WorldSpawnPhase>, spawn: u64, ready_spawn: u64) -> World {
        let mut world = World::new();
        world.insert_resource(gen_of(spawn));
        world.insert_resource(WorldGpuReady {
            spawn: gen_of(ready_spawn),
            ..WorldGpuReady::default()
        });
        world.insert_resource(PipelineDemandTracker::default());
        world.insert_resource(GpuSubmitDemand::default());
        if let Some(phase) = phase {
            world.insert_resource(job_in(phase, spawn));
        }
        world
    }

    fn demand_after(world: &mut World) -> GpuSubmitDemand {
        world.run_system_once(publish_gpu_submit_demand).unwrap();
        world.resource::<GpuSubmitDemand>().clone()
    }

    #[test]
    fn with_no_spawn_running_the_render_side_is_asked_for_nothing() {
        let mut world = demand_world(None, 1, 1);
        let demand = demand_after(&mut world);
        assert_eq!(demand.world_generation, gen_of(1));
        assert!(!demand.warm_pipelines);
        assert!(!demand.overlay_gpu_wait);
        assert!(!demand.wants_residency);
        assert!(demand.pipeline_world_materials.is_empty());
    }

    #[test]
    fn pipelines_are_warmed_from_the_images_phase_until_the_gpu_phase_ends() {
        for (phase, warm, overlay) in [
            (WorldSpawnPhase::Unarmed, false, false),
            (WorldSpawnPhase::Yield, false, false),
            (WorldSpawnPhase::Materials, false, false),
            (WorldSpawnPhase::Images, true, false),
            (WorldSpawnPhase::Plan, true, false),
            (WorldSpawnPhase::WorldTess, true, false),
            (WorldSpawnPhase::Gpu, true, true),
            (WorldSpawnPhase::Done, false, false),
        ] {
            let mut world = demand_world(Some(phase), 1, 1);
            let demand = demand_after(&mut world);
            assert_eq!(demand.warm_pipelines, warm, "{phase:?}");
            assert_eq!(demand.overlay_gpu_wait, overlay, "{phase:?}");
        }
    }

    #[test]
    fn residency_is_counted_only_while_the_gpu_phase_of_this_spawn_is_live() {
        let mut world = demand_world(Some(WorldSpawnPhase::Gpu), 1, 1);
        assert!(demand_after(&mut world).wants_residency);

        // The render side still reports on an older world.
        let mut world = demand_world(Some(WorldSpawnPhase::Gpu), 2, 1);
        assert!(!demand_after(&mut world).wants_residency);

        let mut world = demand_world(Some(WorldSpawnPhase::Plan), 1, 1);
        assert!(!demand_after(&mut world).wants_residency);
    }

    #[test]
    fn the_jobs_pipeline_sets_reach_the_demand_with_one_revision_per_new_snapshot() {
        let mut world = demand_world(Some(WorldSpawnPhase::Images), 1, 1);
        let first = demand_after(&mut world);
        let again = demand_after(&mut world);
        assert_eq!(first.pipeline_demand_revision, again.pipeline_demand_revision);
        assert!(Arc::ptr_eq(
            &first.pipeline_world_materials,
            &again.pipeline_world_materials
        ));

        world.resource_mut::<WorldSpawnJob>().images.pipeline_world_materials =
            Arc::new(HashSet::from([7u16]));
        let changed = demand_after(&mut world);
        assert_eq!(
            changed.pipeline_demand_revision,
            again.pipeline_demand_revision + 1
        );
        assert!(changed.pipeline_world_materials.contains(&7));
    }

    fn consume_world(progress: GpuLoadProgress) -> World {
        let mut world = World::new();
        world.insert_resource(progress);
        world.insert_resource(gen_of(7));
        world.insert_resource(GpuSubmitDemand::default());
        world.insert_resource(WorldGpuReady {
            spawn: gen_of(7),
            ..WorldGpuReady::default()
        });
        world
    }

    fn consumed(world: &mut World) -> WorldGpuReady {
        world.run_system_once(consume_gpu_load_progress).unwrap();
        *world.resource::<WorldGpuReady>()
    }

    #[test]
    fn the_pipeline_counts_are_copied_over_and_no_residency_means_no_images() {
        let mut world = consume_world(GpuLoadProgress {
            waiting_n: 3,
            pipeline_n: 12,
            residency: None,
            ..GpuLoadProgress::default()
        });
        world.resource_mut::<WorldGpuReady>().images = true;
        let ready = consumed(&mut world);
        assert_eq!((ready.waiting_n, ready.pipeline_n), (3, 12));
        assert!(!ready.images);
    }

    #[test]
    fn images_are_ready_when_every_one_that_is_needed_is_resident() {
        let cases = [
            (GpuImageResidency { need: 5, ready: 5 }, true),
            (GpuImageResidency { need: 5, ready: 4 }, false),
            // A world that needs nothing has not been counted yet.
            (GpuImageResidency { need: 0, ready: 0 }, false),
        ];
        for (counts, expected) in cases {
            let mut world = consume_world(GpuLoadProgress {
                residency: Some(counts),
                ..GpuLoadProgress::default()
            });
            let ready = consumed(&mut world);
            assert_eq!(ready.images, expected, "{counts:?}");
            assert_eq!(
                (ready.image_need_n, ready.image_ready_n),
                (counts.need, counts.ready)
            );
        }
    }

    #[test]
    fn pipelines_are_warm_once_nothing_is_still_compiling_in_the_live_gpu_phase() {
        let mut world = consume_world(GpuLoadProgress {
            waiting_n: 0,
            pipeline_n: 4,
            ..GpuLoadProgress::default()
        });
        world.insert_resource(job_in(WorldSpawnPhase::Gpu, 7));
        world.resource_mut::<GpuSubmitDemand>().overlay_gpu_wait = true;
        assert!(consumed(&mut world).pipelines);

        world.resource_mut::<GpuLoadProgress>().waiting_n = 2;
        assert!(!consumed(&mut world).pipelines);
    }

    #[test]
    fn pipelines_are_never_warm_outside_the_live_gpu_phase() {
        // No spawn job at all.
        let mut world = consume_world(GpuLoadProgress::default());
        world.resource_mut::<GpuSubmitDemand>().overlay_gpu_wait = true;
        assert!(!consumed(&mut world).pipelines);

        // A job for another spawn.
        let mut world = consume_world(GpuLoadProgress::default());
        world.insert_resource(job_in(WorldSpawnPhase::Gpu, 6));
        world.resource_mut::<GpuSubmitDemand>().overlay_gpu_wait = true;
        assert!(!consumed(&mut world).pipelines);

        // The loading screen is not waiting on the GPU.
        let mut world = consume_world(GpuLoadProgress::default());
        world.insert_resource(job_in(WorldSpawnPhase::Gpu, 7));
        assert!(!consumed(&mut world).pipelines);
    }
}
