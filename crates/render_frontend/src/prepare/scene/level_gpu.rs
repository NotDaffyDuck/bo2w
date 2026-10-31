//! Waiting on the GPU once a world's CPU side is built: its images must be
//! resident and its pipelines compiled, and that state has to hold for a
//! few frames in a row before the loading screen is allowed to drop.

use std::collections::HashSet;
use std::sync::Arc;
use std::time::{Duration, Instant};

use asset_transport::{LoadCompletion, StageHandle, StageId};
use bevy::asset::{AssetId, RenderAssetUsages};
use bevy::prelude::*;
use bevy::render::MainWorld;
use bevy::render::render_asset::RenderAssets;
use bevy::render::texture::GpuImage;
use frame::LevelGeneration;

use super::spawn::{LevelSpawnJob, LevelSpawnPhase, spawn_level_world};

/// How many consecutive settled frames end the wait.
const STEADY_FRAMES: u32 = 2;

/// How a set of bars is closed.
#[derive(Clone, Copy)]
enum Close {
    Done,
    Cancel,
}

/// The three loading bars owned by the GPU wait.
#[derive(Default)]
struct GpuBars {
    textures: Option<StageHandle>,
    pipelines: Option<StageHandle>,
    steady: Option<StageHandle>,
}

impl GpuBars {
    fn open(progress: &LoadCompletion) -> Self {
        let bar = |id: StageId, total: Option<u64>| Some(progress.begin(id, total));
        Self {
            textures: bar(StageId::GpuTextures, None),
            pipelines: bar(StageId::Pipelines, None),
            steady: bar(StageId::RenderFrames, Some(u64::from(STEADY_FRAMES))),
        }
    }

    /// Closes whichever bars are still open. A successful close reports the
    /// steady-frames bar first.
    fn close_all(&mut self, how: Close) {
        let open = match how {
            Close::Done => [&mut self.steady, &mut self.textures, &mut self.pipelines],
            Close::Cancel => [&mut self.textures, &mut self.pipelines, &mut self.steady],
        };
        for bar in open.into_iter().filter_map(Option::take) {
            match how {
                Close::Done => bar.done(),
                Close::Cancel => bar.cancel(),
            }
        }
    }
}

/// Moves one bar to `completed` of `total`, and closes it for good once
/// `gate_open` says its condition holds. Later dips cannot reopen it.
fn advance_bar(slot: &mut Option<StageHandle>, completed: u32, total: u32, gate_open: bool) {
    if let Some(bar) = slot.as_ref() {
        bar.set_total(u64::from(total));
        bar.set_completed(u64::from(completed));
    }
    if gate_open && let Some(bar) = slot.take() {
        bar.done();
    }
}

/// Per-spawn state of the GPU wait, kept on the spawn job.
#[derive(Default)]
pub struct LevelGpuWait {
    armed_at: Option<Instant>,
    steady: u32,
    pipeline_baseline: Option<u32>,
    bars: GpuBars,
}

impl LevelGpuWait {
    /// Begins a fresh wait. Bars left open by an earlier wait are cancelled,
    /// since that wait was superseded rather than completed.
    pub fn arm_t(&mut self, progress: Option<&LoadCompletion>) {
        self.bars.close_all(Close::Cancel);
        *self = Self {
            armed_at: Some(Instant::now()),
            steady: 0,
            pipeline_baseline: None,
            bars: progress.map(GpuBars::open).unwrap_or_default(),
        };
    }

    /// Settled frames counted so far without a break.
    pub(crate) fn steady_frames(&self) -> u32 {
        self.steady
    }

    /// How many pipelines appeared since the first report of this wait.
    pub(crate) fn pipelines_added(&self, now: u32) -> Option<u32> {
        self.pipeline_baseline.map(|base| now.saturating_sub(base))
    }

    pub(crate) fn elapsed(&self) -> Duration {
        match self.armed_at {
            Some(at) => at.elapsed(),
            None => Duration::ZERO,
        }
    }

    /// Feeds one render-side report into the bars. The counts can go down as
    /// well as up; a bar only ends when its readiness flag is set.
    fn follow_report(&mut self, report: &LevelGpuReady) {
        self.pipeline_baseline.get_or_insert(report.pipeline_tally);
        advance_bar(
            &mut self.bars.textures,
            report.image_ready_tally,
            report.image_need_tally,
            report.images_resident,
        );
        let compiled = report.pipeline_tally.saturating_sub(report.waiting_tally);
        advance_bar(
            &mut self.bars.pipelines,
            compiled,
            report.pipeline_tally,
            report.pipelines_warm,
        );
    }
}

/// The frontend's view of how far the render side is with the current world.
#[derive(Resource, Debug, Default, Clone, Copy)]
pub struct LevelGpuReady {
    pub spawn: LevelGeneration,
    pub images_resident: bool,
    pub pipelines_warm: bool,
    pub waiting_tally: u32,
    pub pipeline_tally: u32,
    pub image_ready_tally: u32,
    pub image_need_tally: u32,
}

impl LevelGpuReady {
    /// Whether this report says `spawn` is fully on the GPU. A report with no
    /// spawn attached covers nothing.
    fn covers(&self, spawn: LevelGeneration) -> bool {
        let same_world = self.spawn.0.is_some() && self.spawn == spawn;
        same_world && self.images_resident && self.pipelines_warm
    }
}

/// Requests the frontend hands to the render side each frame.
#[derive(Resource, Clone, Debug, Default)]
pub struct GpuSubmitNeed {
    pub level_generation: LevelGeneration,
    /// Build pipelines before anything draws with them.
    pub prewarmed_pipelines: bool,
    /// The loading screen is blocked on the GPU wait.
    pub topmost_gpu_wait: bool,
    /// Report how many images are resident.
    pub wants_presence: bool,
    pub pipeline_level_materials: Arc<HashSet<u16>>,
    pub pipeline_prop_materials: Arc<HashSet<u16>>,
    pub pipeline_demand_revision: u64,
}

/// The material sets whose pipelines should be built, plus a counter that
/// moves whenever a spawn hands over a new allocation of either set.
#[derive(Resource, Clone, Debug, Default)]
pub struct PipelineDemandTracker {
    world: Arc<HashSet<u16>>,
    props: Arc<HashSet<u16>>,
    bumps: u64,
}

impl PipelineDemandTracker {
    fn take_sets(&mut self, level_set: Arc<HashSet<u16>>, props: Arc<HashSet<u16>>) {
        let unchanged = Arc::ptr_eq(&self.world, &level_set) && Arc::ptr_eq(&self.props, &props);
        if !unchanged {
            self.world = level_set;
            self.props = props;
            self.bumps = self.bumps.wrapping_add(1);
        }
    }

    pub fn snapshot(&self) -> (Arc<HashSet<u16>>, Arc<HashSet<u16>>, u64) {
        (self.world.clone(), self.props.clone(), self.bumps)
    }
}

/// Raw numbers the render app writes back into the main world.
#[derive(Resource, Clone, Debug, Default)]
pub struct GpuLoadStatus {
    pub waiting_tally: u32,
    pub pipeline_tally: u32,
    pub queued_shaders: Vec<AssetId<bevy::shader::Shader>>,
    pub residency: Option<GpuImagePresence>,
}

/// Images the world wants on the GPU, and how many of them are there.
#[derive(Clone, Copy, Debug, Default)]
pub struct GpuImagePresence {
    pub wanted: u32,
    pub resident: u32,
}

/// Whether the loading screen is currently blocked on the GPU side of the
/// spawn that both the report and the job belong to.
pub fn gpu_phase_is_live(
    report_spawn: LevelGeneration,
    job_spawn: Option<LevelGeneration>,
    loading_waits: bool,
    current: LevelGeneration,
) -> bool {
    if current.0.is_none() || !loading_waits {
        return false;
    }
    report_spawn == current && job_spawn == Some(current)
}

/// Spawn phases during which pipelines are built ahead of drawing.
fn warms_pipelines(phase: LevelSpawnPhase) -> bool {
    matches!(phase, LevelSpawnPhase::Images | LevelSpawnPhase::Plan | LevelSpawnPhase::WorldTess | LevelSpawnPhase::Gpu)
}

pub(crate) fn post_gpu_demand(
    job: Option<Res<LevelSpawnJob>>,
    generation: Option<Res<LevelGeneration>>,
    report: Res<LevelGpuReady>,
    mut tracker: ResMut<PipelineDemandTracker>,
    mut need: ResMut<GpuSubmitNeed>,
) {
    let job = job.as_deref();
    let current = generation.as_deref().copied().unwrap_or_default();
    let phase = job.map(|j| j.stage_now);
    let loading_waits = phase == Some(LevelSpawnPhase::Gpu);

    let (world_set, prop_set) = job.map_or_else(Default::default, |j| {
        (
            j.images.pipeline_level_materials.clone(),
            j.images.pipeline_prop_materials.clone(),
        )
    });
    tracker.take_sets(world_set, prop_set);
    let (level_set, props_set, revision) = tracker.snapshot();

    *need = GpuSubmitNeed {
        level_generation: current,
        prewarmed_pipelines: phase.is_some_and(warms_pipelines),
        topmost_gpu_wait: loading_waits,
        wants_presence: gpu_phase_is_live(
            report.spawn,
            job.map(|j| j.spawn),
            loading_waits,
            current,
        ),
        pipeline_level_materials: level_set,
        pipeline_prop_materials: props_set,
        pipeline_demand_revision: revision,
    };
}

/// Advances the wait by one frame. Returns true when the world has been
/// settled for `STEADY_FRAMES` frames running, closing the bars as it does.
pub(crate) fn check_once(
    wait: &mut LevelGpuWait,
    spawn: LevelGeneration,
    report: Option<&LevelGpuReady>,
    frame_gap_ms: f32,
) -> bool {
    let settled = match report {
        Some(r) => {
            wait.follow_report(r);
            r.covers(spawn)
        }
        None => false,
    };
    wait.steady = match settled {
        true => wait.steady.saturating_add(1),
        false => 0,
    };
    if let Some(bar) = wait.bars.steady.as_ref() {
        bar.set_completed(u64::from(STEADY_FRAMES.min(wait.steady)));
    }
    log_gpu_slice(wait.steady, report, frame_gap_ms);

    let done = settled && wait.steady >= STEADY_FRAMES;
    if done {
        wait.bars.close_all(Close::Done);
    }
    done
}

/// One diagnostics line per frame of the wait; missing reports print as -1.
fn log_gpu_slice(steady: u32, report: Option<&LevelGpuReady>, frame_gap_ms: f32) {
    let field = |read: fn(&LevelGpuReady) -> i64| report.map_or(-1, read);
    diag::info!(
        World,
        "world spawn slice: phase=gpu quiet={}/{} images={} pipelines={} waiting={} total={} img={}/{} gap={:.1}ms",
        steady,
        STEADY_FRAMES,
        field(|r| i64::from(r.images_resident)),
        field(|r| i64::from(r.pipelines_warm)),
        field(|r| i64::from(r.waiting_tally)),
        field(|r| i64::from(r.pipeline_tally)),
        field(|r| i64::from(r.image_ready_tally)),
        field(|r| i64::from(r.image_need_tally)),
        frame_gap_ms,
    );
}

pub(crate) fn install(app: &mut App) {
    app.init_resource::<LevelGpuReady>();
    app.init_resource::<GpuSubmitNeed>();
    app.init_resource::<PipelineDemandTracker>();
    app.init_resource::<GpuLoadStatus>();
    app.add_systems(
        Update,
        (
            absorb_gpu_report.before(spawn_level_world),
            post_gpu_demand.after(spawn_level_world),
        ),
    );
}

/// Tallies render-world images against what the GPU already holds.
fn tally_residency(
    store: &Assets<Image>,
    on_gpu: impl Fn(AssetId<Image>) -> bool,
) -> GpuImagePresence {
    store
        .iter()
        .filter(|entry| entry.1.asset_usage.contains(RenderAssetUsages::RENDER_WORLD))
        .fold(GpuImagePresence::default(), |tally, (id, _)| GpuImagePresence {
            wanted: tally.wanted.saturating_add(1),
            resident: tally.resident + u32::from(on_gpu(id)),
        })
}

/// Runs in the render app: when the frontend asks for it, writes the image
/// residency tally into the main world's `GpuLoadStatus`; otherwise clears it.
pub(crate) fn count_resident_images(
    mut main: ResMut<MainWorld>,
    uploaded: Res<RenderAssets<GpuImage>>,
) {
    let asked = main
        .get_resource::<GpuSubmitNeed>()
        .is_some_and(|need| need.wants_presence);
    let tally = if asked {
        let images = main.resource::<Assets<Image>>();
        Some(tally_residency(images, |id| uploaded.get(id).is_some()))
    } else {
        None
    };
    if let Some(mut status) = main.get_resource_mut::<GpuLoadStatus>() {
        status.residency = tally;
    }
}

/// Folds the render app's raw status into `LevelGpuReady`. Pipelines count
/// as warm only while this spawn's GPU phase is live and the cache has
/// nothing left compiling.
pub(crate) fn absorb_gpu_report(
    status: Res<GpuLoadStatus>,
    generation: Option<Res<LevelGeneration>>,
    need: Res<GpuSubmitNeed>,
    job: Option<Res<LevelSpawnJob>>,
    mut report: ResMut<LevelGpuReady>,
) {
    let current = generation
        .as_deref()
        .copied()
        .unwrap_or(LevelGeneration(None));
    report.waiting_tally = status.waiting_tally;
    report.pipeline_tally = status.pipeline_tally;
    report.images_resident = false;
    if let Some(GpuImagePresence { wanted, resident }) = status.residency {
        report.image_need_tally = wanted;
        report.image_ready_tally = resident;
        report.images_resident = wanted != 0 && resident == wanted;
    }
    let live = gpu_phase_is_live(
        report.spawn,
        job.map(|j| j.spawn),
        need.topmost_gpu_wait,
        current,
    );
    report.pipelines_warm = live && status.waiting_tally == 0;
}

#[cfg(test)]
mod tests {
    use asset_transport::{StageOutcome, StageSnapshot, WorkCount};
    use bevy::ecs::system::RunSystemOnce;

    use super::*;

    fn gen_of(id: u64) -> LevelGeneration {
        LevelGeneration::out_of_setup_dir(id)
    }

    fn ready_for(id: u64) -> LevelGpuReady {
        LevelGpuReady {
            spawn: gen_of(id),
            images_resident: true,
            pipelines_warm: true,
            ..LevelGpuReady::default()
        }
    }

    fn stage(progress: &LoadCompletion, id: StageId) -> Vec<StageSnapshot> {
        let snap = progress.snapshot();
        snap.stages
            .into_iter()
            .filter(|stage| stage.key.id == id)
            .collect()
    }

    #[test]
    fn a_report_is_ready_only_for_its_own_spawn_with_images_and_pipelines() {
        let ready = ready_for(4);
        assert!(ready.covers(gen_of(4)));
        assert!(!ready.covers(gen_of(5)));
        assert!(!ready.covers(LevelGeneration(None)));
        assert!(
            !LevelGpuReady {
                images_resident: false,
                ..ready
            }
            .covers(gen_of(4))
        );
        assert!(
            !LevelGpuReady {
                pipelines_warm: false,
                ..ready
            }
            .covers(gen_of(4))
        );
        // A report that names no spawn is ready for nobody, itself included.
        assert!(
            !LevelGpuReady {
                spawn: LevelGeneration(None),
                ..ready
            }
            .covers(LevelGeneration(None))
        );
    }

    #[test]
    fn the_gpu_extract_is_live_only_for_the_spawn_the_loading_screen_waits_on() {
        let g = gen_of(3);
        assert!(gpu_phase_is_live(g, Some(g), true, g));
        assert!(!gpu_phase_is_live(g, Some(g), false, g));
        assert!(!gpu_phase_is_live(gen_of(2), Some(g), true, g));
        assert!(!gpu_phase_is_live(g, Some(gen_of(2)), true, g));
        assert!(!gpu_phase_is_live(g, None, true, g));
        let none = LevelGeneration(None);
        assert!(!gpu_phase_is_live(none, Some(none), true, none));
    }

    #[test]
    fn a_wait_that_was_never_armed_has_no_time_and_no_quiet_frames() {
        let wait = LevelGpuWait::default();
        assert_eq!(wait.elapsed(), Duration::ZERO);
        assert_eq!(wait.steady_frames(), 0);
        assert_eq!(wait.pipelines_added(50), None);
    }

    #[test]
    fn arming_starts_the_clock_and_forgets_the_old_wait() {
        let mut wait = LevelGpuWait::default();
        wait.steady = 5;
        wait.pipeline_baseline = Some(3);
        wait.arm_t(None);
        assert_eq!(wait.steady_frames(), 0);
        assert_eq!(wait.pipelines_added(9), None);
        assert!(wait.armed_at.is_some());
    }

    #[test]
    fn nothing_is_ready_until_the_render_side_reports() {
        let mut wait = LevelGpuWait::default();
        wait.arm_t(None);
        for _ in 0..4 {
            assert!(!check_once(&mut wait, gen_of(1), None, 0.0));
        }
        assert_eq!(wait.steady_frames(), 0);
    }

    #[test]
    fn the_world_must_stay_ready_for_two_frames_in_a_row() {
        let mut wait = LevelGpuWait::default();
        wait.arm_t(None);
        let gpu = ready_for(1);
        assert!(!check_once(&mut wait, gen_of(1), Some(&gpu), 0.0));
        assert_eq!(wait.steady_frames(), 1);
        assert!(check_once(&mut wait, gen_of(1), Some(&gpu), 0.0));
        assert_eq!(wait.steady_frames(), 2);
    }

    #[test]
    fn a_frame_that_is_not_ready_starts_the_count_again() {
        let mut wait = LevelGpuWait::default();
        wait.arm_t(None);
        let gpu = ready_for(1);
        let busy = LevelGpuReady {
            pipelines_warm: false,
            ..gpu
        };
        assert!(!check_once(&mut wait, gen_of(1), Some(&gpu), 0.0));
        assert!(!check_once(&mut wait, gen_of(1), Some(&busy), 0.0));
        assert_eq!(wait.steady_frames(), 0);
        assert!(!check_once(&mut wait, gen_of(1), Some(&gpu), 0.0));
        assert!(check_once(&mut wait, gen_of(1), Some(&gpu), 0.0));
    }

    #[test]
    fn a_report_for_another_spawn_never_counts() {
        let mut wait = LevelGpuWait::default();
        wait.arm_t(None);
        let stale = ready_for(1);
        for _ in 0..5 {
            assert!(!check_once(&mut wait, gen_of(2), Some(&stale), 0.0));
        }
        assert_eq!(wait.steady_frames(), 0);
    }

    #[test]
    fn the_first_report_fixes_the_pipeline_count_the_wait_began_with() {
        let mut wait = LevelGpuWait::default();
        wait.arm_t(None);
        let first = LevelGpuReady {
            pipeline_tally: 10,
            ..LevelGpuReady::default()
        };
        check_once(&mut wait, gen_of(1), Some(&first), 0.0);
        let later = LevelGpuReady {
            pipeline_tally: 25,
            ..LevelGpuReady::default()
        };
        check_once(&mut wait, gen_of(1), Some(&later), 0.0);
        assert_eq!(wait.pipelines_added(25), Some(15));
        // The count never goes negative if the cache was trimmed meanwhile.
        assert_eq!(wait.pipelines_added(4), Some(0));
    }

    #[test]
    fn the_bars_follow_what_the_gpu_reports() {
        let progress = LoadCompletion::new(1);
        let mut wait = LevelGpuWait::default();
        wait.arm_t(Some(&progress));
        let gpu = LevelGpuReady {
            image_need_tally: 10,
            image_ready_tally: 4,
            pipeline_tally: 8,
            waiting_tally: 3,
            ..LevelGpuReady::default()
        };
        assert!(!check_once(&mut wait, gen_of(1), Some(&gpu), 0.0));

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
        let progress = LoadCompletion::new(1);
        let mut wait = LevelGpuWait::default();
        wait.arm_t(Some(&progress));
        let images_only = LevelGpuReady {
            spawn: gen_of(1),
            images_resident: true,
            image_need_tally: 6,
            image_ready_tally: 6,
            ..LevelGpuReady::default()
        };
        check_once(&mut wait, gen_of(1), Some(&images_only), 0.0);
        assert_eq!(
            stage(&progress, StageId::GpuTextures)[0].outcome(),
            Some(StageOutcome::Done)
        );
        assert!(stage(&progress, StageId::Pipelines)[0].running());

        // A gauge falling back afterwards does not reopen a closed bar.
        let fallen = LevelGpuReady {
            images_resident: false,
            image_ready_tally: 2,
            ..images_only
        };
        check_once(&mut wait, gen_of(1), Some(&fallen), 0.0);
        assert_eq!(
            stage(&progress, StageId::GpuTextures)[0].outcome(),
            Some(StageOutcome::Done)
        );
    }

    #[test]
    fn the_quiet_frames_bar_never_reads_past_its_total() {
        let progress = LoadCompletion::new(1);
        let mut wait = LevelGpuWait::default();
        wait.arm_t(Some(&progress));
        let gpu = ready_for(1);
        check_once(&mut wait, gen_of(1), Some(&gpu), 0.0);
        let frames = &stage(&progress, StageId::RenderFrames)[0];
        assert_eq!(frames.count.map(|c| c.completed), Some(1));
        assert!(check_once(&mut wait, gen_of(1), Some(&gpu), 0.0));
    }

    #[test]
    fn finishing_the_wait_closes_every_bar_as_done() {
        let progress = LoadCompletion::new(1);
        let mut wait = LevelGpuWait::default();
        wait.arm_t(Some(&progress));
        let gpu = ready_for(1);
        check_once(&mut wait, gen_of(1), Some(&gpu), 0.0);
        assert!(check_once(&mut wait, gen_of(1), Some(&gpu), 0.0));
        for id in [StageId::GpuTextures, StageId::Pipelines, StageId::RenderFrames] {
            let stages = stage(&progress, id);
            assert_eq!(stages.len(), 1);
            assert_eq!(stages[0].outcome(), Some(StageOutcome::Done), "{id:?}");
        }
    }

    #[test]
    fn arming_again_cancels_the_bars_of_a_wait_that_never_finished() {
        let progress = LoadCompletion::new(1);
        let mut wait = LevelGpuWait::default();
        wait.arm_t(Some(&progress));
        wait.arm_t(Some(&progress));
        let snap = progress.snapshot();
        let first_round: Vec<_> = snap
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

        tracker.take_sets(world.clone(), smodel.clone());
        let (w, s, rev) = tracker.snapshot();
        assert!(Arc::ptr_eq(&w, &world) && Arc::ptr_eq(&s, &smodel));
        assert_eq!(rev, 1);

        // The same allocations again: nothing changed.
        tracker.take_sets(world.clone(), smodel.clone());
        assert_eq!(tracker.snapshot().2, 1);

        // Equal contents in a different allocation still count as new: the
        // render side compares revisions, not sets.
        tracker.take_sets(Arc::new((*world).clone()), smodel.clone());
        assert_eq!(tracker.snapshot().2, 2);
        tracker.take_sets(world.clone(), Arc::new(HashSet::new()));
        assert_eq!(tracker.snapshot().2, 3);
    }

    fn job_in(phase: LevelSpawnPhase, spawn: u64) -> LevelSpawnJob {
        let mut job = LevelSpawnJob::default();
        job.stage_now = phase;
        job.spawn = gen_of(spawn);
        job
    }

    fn demand_world(phase: Option<LevelSpawnPhase>, spawn: u64, ready_spawn: u64) -> World {
        let mut world = World::new();
        world.insert_resource(gen_of(spawn));
        world.insert_resource(LevelGpuReady {
            spawn: gen_of(ready_spawn),
            ..LevelGpuReady::default()
        });
        world.insert_resource(PipelineDemandTracker::default());
        world.insert_resource(GpuSubmitNeed::default());
        if let Some(phase) = phase {
            world.insert_resource(job_in(phase, spawn));
        }
        world
    }

    fn demand_after(world: &mut World) -> GpuSubmitNeed {
        world.run_system_once(post_gpu_demand).unwrap();
        world.resource::<GpuSubmitNeed>().clone()
    }

    #[test]
    fn with_no_spawn_running_the_render_side_is_asked_for_nothing() {
        let mut world = demand_world(None, 1, 1);
        let demand = demand_after(&mut world);
        assert_eq!(demand.level_generation, gen_of(1));
        assert!(!demand.prewarmed_pipelines);
        assert!(!demand.topmost_gpu_wait);
        assert!(!demand.wants_presence);
        assert!(demand.pipeline_level_materials.is_empty());
    }

    #[test]
    fn pipelines_are_warmed_from_the_images_phase_until_the_gpu_phase_ends() {
        for (phase, warm, overlay) in [
            (LevelSpawnPhase::Unarmed, false, false),
            (LevelSpawnPhase::Yield, false, false),
            (LevelSpawnPhase::Materials, false, false),
            (LevelSpawnPhase::Images, true, false),
            (LevelSpawnPhase::Plan, true, false),
            (LevelSpawnPhase::WorldTess, true, false),
            (LevelSpawnPhase::Gpu, true, true),
            (LevelSpawnPhase::Done, false, false),
        ] {
            let mut world = demand_world(Some(phase), 1, 1);
            let demand = demand_after(&mut world);
            assert_eq!(demand.prewarmed_pipelines, warm, "{phase:?}");
            assert_eq!(demand.topmost_gpu_wait, overlay, "{phase:?}");
        }
    }

    #[test]
    fn residency_is_counted_only_while_the_gpu_phase_of_this_spawn_is_live() {
        let mut world = demand_world(Some(LevelSpawnPhase::Gpu), 1, 1);
        assert!(demand_after(&mut world).wants_presence);

        // The render side still reports on an older world.
        let mut world = demand_world(Some(LevelSpawnPhase::Gpu), 2, 1);
        assert!(!demand_after(&mut world).wants_presence);

        let mut world = demand_world(Some(LevelSpawnPhase::Plan), 1, 1);
        assert!(!demand_after(&mut world).wants_presence);
    }

    #[test]
    fn the_jobs_pipeline_sets_reach_the_demand_with_one_revision_per_new_snapshot() {
        let mut world = demand_world(Some(LevelSpawnPhase::Images), 1, 1);
        let first = demand_after(&mut world);
        let again = demand_after(&mut world);
        assert_eq!(first.pipeline_demand_revision, again.pipeline_demand_revision);
        assert!(Arc::ptr_eq(
            &first.pipeline_level_materials,
            &again.pipeline_level_materials
        ));

        world.resource_mut::<LevelSpawnJob>().images.pipeline_level_materials =
            Arc::new(HashSet::from([7u16]));
        let changed = demand_after(&mut world);
        assert_eq!(
            changed.pipeline_demand_revision,
            again.pipeline_demand_revision + 1
        );
        assert!(changed.pipeline_level_materials.contains(&7));
    }

    fn consume_world(progress: GpuLoadStatus) -> World {
        let mut world = World::new();
        world.insert_resource(progress);
        world.insert_resource(gen_of(7));
        world.insert_resource(GpuSubmitNeed::default());
        world.insert_resource(LevelGpuReady {
            spawn: gen_of(7),
            ..LevelGpuReady::default()
        });
        world
    }

    fn consumed(world: &mut World) -> LevelGpuReady {
        world.run_system_once(absorb_gpu_report).unwrap();
        *world.resource::<LevelGpuReady>()
    }

    #[test]
    fn the_pipeline_counts_are_copied_over_and_no_residency_means_no_images() {
        let mut world = consume_world(GpuLoadStatus {
            waiting_tally: 3,
            pipeline_tally: 12,
            residency: None,
            ..GpuLoadStatus::default()
        });
        world.resource_mut::<LevelGpuReady>().images_resident = true;
        let ready = consumed(&mut world);
        assert_eq!((ready.waiting_tally, ready.pipeline_tally), (3, 12));
        assert!(!ready.images_resident);
    }

    #[test]
    fn images_are_ready_when_every_one_that_is_needed_is_resident() {
        let cases = [
            (GpuImagePresence { wanted: 5, resident: 5 }, true),
            (GpuImagePresence { wanted: 5, resident: 4 }, false),
            // A world that needs nothing has not been counted yet.
            (GpuImagePresence { wanted: 0, resident: 0 }, false),
        ];
        for (counts, expected) in cases {
            let mut world = consume_world(GpuLoadStatus {
                residency: Some(counts),
                ..GpuLoadStatus::default()
            });
            let ready = consumed(&mut world);
            assert_eq!(ready.images_resident, expected, "{counts:?}");
            assert_eq!(
                (ready.image_need_tally, ready.image_ready_tally),
                (counts.wanted, counts.resident)
            );
        }
    }

    #[test]
    fn pipelines_are_warm_once_nothing_is_still_compiling_in_the_live_gpu_phase() {
        let mut world = consume_world(GpuLoadStatus {
            waiting_tally: 0,
            pipeline_tally: 4,
            ..GpuLoadStatus::default()
        });
        world.insert_resource(job_in(LevelSpawnPhase::Gpu, 7));
        world.resource_mut::<GpuSubmitNeed>().topmost_gpu_wait = true;
        assert!(consumed(&mut world).pipelines_warm);

        world.resource_mut::<GpuLoadStatus>().waiting_tally = 2;
        assert!(!consumed(&mut world).pipelines_warm);
    }

    #[test]
    fn pipelines_are_never_warm_outside_the_live_gpu_phase() {
        // No spawn job at all.
        let mut world = consume_world(GpuLoadStatus::default());
        world.resource_mut::<GpuSubmitNeed>().topmost_gpu_wait = true;
        assert!(!consumed(&mut world).pipelines_warm);

        // A job for another spawn.
        let mut world = consume_world(GpuLoadStatus::default());
        world.insert_resource(job_in(LevelSpawnPhase::Gpu, 6));
        world.resource_mut::<GpuSubmitNeed>().topmost_gpu_wait = true;
        assert!(!consumed(&mut world).pipelines_warm);

        // The loading screen is not waiting on the GPU.
        let mut world = consume_world(GpuLoadStatus::default());
        world.insert_resource(job_in(LevelSpawnPhase::Gpu, 7));
        assert!(!consumed(&mut world).pipelines_warm);
    }
}
