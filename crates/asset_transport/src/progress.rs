//! What a map load has done so far, as facts.
//!
//! One [`LoadProgress`] per load request. The work is cut into stages; a stage
//! opens, may count the units it has handled and the bytes it brought in, and
//! its owner says how it ended. Nothing in here formats a table or decides how
//! many rows fit: a surface reads [`LoadProgress::snapshot`] and draws it.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

const MIB: f64 = 1024.0 * 1024.0;

/// Stored in a stage's total while the plan is still being discovered.
const PLAN_OPEN: u64 = u64::MAX;

/// The kinds of stage a load opens. Declaration order is the order a table
/// lists them in, so a stage that has not started still has its place.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum StageId {
    /// Opening the map zone, walking it and building what it described.
    MapAssets,
    /// `common_mp`, the startup zones and the other namespaces.
    CommonAssets,
    /// The localized string zones beside the map.
    Localization,
    /// Decoding material images out of the archives.
    Images,
    /// The loadscreen picture behind the overlay.
    Preview,
    /// Moving the prepared match into the live world on the main thread.
    Install,
    /// Handing world images to the image store. Not the same as GPU-ready.
    WorldImages,
    /// Compiling material programs on the worker pool.
    Programs,
    /// Absorbing the compiled outcomes on the main thread.
    ProgramMerge,
    /// Admitting the absorbed programs as shaders.
    Shaders,
    /// Images resident on the GPU. A gauge: it may fall.
    GpuTextures,
    /// Render pipelines warmed for the spawned world. Also a gauge.
    Pipelines,
    /// Consecutive frames drawn with nothing left compiling.
    RenderFrames,
    FirstPerson,
    /// Converting the clips this match's aliases resolve to.
    Audio,
    /// Baking the bot navigation graph.
    Navigation,
    /// Waiting for the session to let this client in.
    Admission,
}

impl StageId {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::MapAssets => "map_assets",
            Self::CommonAssets => "common_assets",
            Self::Localization => "localization",
            Self::Images => "images",
            Self::Preview => "preview",
            Self::Install => "install",
            Self::WorldImages => "world_images",
            Self::Programs => "programs",
            Self::ProgramMerge => "program_merge",
            Self::Shaders => "shaders",
            Self::GpuTextures => "gpu_textures",
            Self::Pipelines => "pipelines",
            Self::RenderFrames => "render_frames",
            Self::FirstPerson => "first_person",
            Self::Audio => "audio",
            Self::Navigation => "navigation",
            Self::Admission => "admission",
        }
    }
}

/// The same stage can run in several passes (the map zone, `common_mp`, one
/// per namespace). Each pass takes a scope of its own so it gets its own slot.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum StageScope {
    Whole,
    Named(Arc<str>),
}

impl From<&str> for StageScope {
    fn from(name: &str) -> Self {
        Self::Named(name.into())
    }
}

impl From<String> for StageScope {
    fn from(name: String) -> Self {
        Self::Named(name.into())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct StageKey {
    pub id: StageId,
    pub scope: StageScope,
}

impl StageKey {
    fn label(&self) -> String {
        match &self.scope {
            StageScope::Whole => self.id.as_str().to_owned(),
            StageScope::Named(name) => format!("{}/{name}", self.id.as_str()),
        }
    }
}

/// How a stage ended, as said by whoever owns it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum StageOutcome {
    /// The owner confirmed the result is ready at its own boundary.
    Done,
    Reused,
    /// The branch never ran.
    Skipped,
    /// The load was retargeted or aborted first.
    Cancelled,
    /// The handle was dropped without a result. Not a success.
    Abandoned,
    Failed,
}

impl StageOutcome {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Done => "done",
            Self::Reused => "reused",
            Self::Skipped => "skipped",
            Self::Cancelled => "cancelled",
            Self::Abandoned => "abandoned",
            Self::Failed => "failed",
        }
    }
}

/// Units a stage has handled. A skipped or refused unit counts as handled;
/// what became of it is the producer's own report.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WorkCount {
    pub completed: u64,
    /// `None` while the plan is open; `Some(0)` is a plan known to be empty.
    pub total: Option<u64>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StageEnd {
    pub at: Instant,
    pub outcome: StageOutcome,
}

#[derive(Clone, Debug, PartialEq)]
pub struct StageSnapshot {
    pub key: StageKey,
    pub started_at: Option<Instant>,
    /// `None` when the stage has no unit worth counting.
    pub count: Option<WorkCount>,
    /// `None` when the producer does not weigh its work.
    pub bytes: Option<u64>,
    pub end: Option<StageEnd>,
    pub rss_delta: Option<i64>,
    pub heap_delta: Option<i64>,
}

impl StageSnapshot {
    pub fn running(&self) -> bool {
        self.started_at.is_some() && self.end.is_none()
    }

    pub fn outcome(&self) -> Option<StageOutcome> {
        self.end.map(|end| end.outcome)
    }

    /// Time from the start to the end, or to `now` while the stage runs.
    fn span(&self, now: Instant) -> Option<Duration> {
        let started = self.started_at?;
        let until = self.end.map_or(now, |end| end.at);
        Some(until.saturating_duration_since(started))
    }
}

/// One consistent read of a load; `now` is sampled once for every stage in it.
#[derive(Clone, Debug)]
pub struct LoadSnapshot {
    pub request_id: u64,
    pub requested_at: Instant,
    pub now: Instant,
    pub canceled: bool,
    pub stages: Vec<StageSnapshot>,
}

/// Resident set size of this process, where the platform reports it.
pub fn process_resident_bytes() -> Option<u64> {
    status_kib("VmRSS:")
}

/// The highest resident set size this process has reached.
pub fn peak_resident_bytes() -> Option<u64> {
    status_kib("VmHWM:")
}

#[cfg(target_os = "linux")]
fn status_kib(field: &str) -> Option<u64> {
    let status = std::fs::read_to_string("/proc/self/status").ok()?;
    let kib: u64 = status
        .lines()
        .find_map(|line| line.strip_prefix(field))?
        .split_whitespace()
        .next()?
        .parse()
        .ok()?;
    kib.checked_mul(1024)
}

#[cfg(not(target_os = "linux"))]
fn status_kib(_field: &str) -> Option<u64> {
    None
}

/// Process memory at one moment.
#[derive(Clone, Copy, Debug, Default)]
struct MemSample {
    rss: Option<u64>,
    heap: Option<u64>,
}

impl MemSample {
    fn take() -> Self {
        Self {
            rss: process_resident_bytes(),
            heap: diag::process_live_heap_bytes(),
        }
    }

    /// `" rss=+12MiB→340MiB heap=..."` for whichever of the two both ends have.
    fn change_text(from: Self, to: Self) -> String {
        let one = |name: &str, from: Option<u64>, to: Option<u64>| match (from, to) {
            (Some(from), Some(to)) => format!(
                " {name}={:+.0}MiB→{:.0}MiB",
                to as f64 / MIB - from as f64 / MIB,
                to as f64 / MIB
            ),
            _ => String::new(),
        };
        one("rss", from.rss, to.rss) + &one("heap", from.heap, to.heap)
    }
}

fn signed_delta(from: Option<u64>, to: Option<u64>) -> Option<i64> {
    Some(i64::try_from(to?).ok()? - i64::try_from(from?).ok()?)
}

/// A stage's window inside a load. `at` is measured from the first stage that
/// started, not from the request.
#[derive(Clone, Debug, PartialEq)]
pub struct LoadLaneTiming {
    pub label: String,
    pub at: Duration,
    pub elapsed: Duration,
    pub done: u64,
    pub total: u64,
    pub running: bool,
    pub outcome: Option<StageOutcome>,
    pub rss_delta: Option<i64>,
    pub heap_delta: Option<i64>,
}

impl LoadLaneTiming {
    pub fn end(&self) -> Duration {
        self.at + self.elapsed
    }

    /// A tag for the table: empty for a plain success.
    pub fn outcome_suffix(&self) -> &'static str {
        if self.running {
            return " (running)";
        }
        match self.outcome {
            None | Some(StageOutcome::Done) => "",
            Some(StageOutcome::Reused) => " (reused)",
            Some(StageOutcome::Skipped) => " (skipped)",
            Some(StageOutcome::Cancelled) => " (canceled)",
            Some(StageOutcome::Abandoned) => " (interrupted)",
            Some(StageOutcome::Failed) => " (failed)",
        }
    }

    pub fn mem_suffix(&self) -> String {
        let one = |name: &str, delta: Option<i64>| {
            delta.map_or_else(String::new, |delta| {
                format!(" {name}={:+.0}MiB", delta as f64 / MIB)
            })
        };
        one("rss", self.rss_delta) + &one("heap", self.heap_delta)
    }
}

/// Everything a stage's owner and a reader share. The counters are atomics so
/// that publishing progress from a worker is a single store; only the life
/// cycle takes the lock.
struct Slot {
    key: StageKey,
    done: AtomicU64,
    total: AtomicU64,
    /// Set by the first call that gives the stage a count.
    counted: AtomicBool,
    bytes: AtomicU64,
    /// Set by the first call that gives the stage a byte weight.
    weighed: AtomicBool,
    life: Mutex<Life>,
}

struct Life {
    started_at: Option<Instant>,
    end: Option<StageEnd>,
    /// Frozen under the same lock that records `end`, so a reader of a
    /// finished stage sees outcome, count, bytes and time as one fact.
    frozen: Option<(Option<WorkCount>, Option<u64>)>,
    mem_at_start: MemSample,
    mem_at_end: Option<MemSample>,
}

impl Slot {
    fn new(key: StageKey, total: Option<u64>, life: Life) -> Arc<Self> {
        Arc::new(Self {
            key,
            done: AtomicU64::new(0),
            total: AtomicU64::new(total.unwrap_or(PLAN_OPEN)),
            counted: AtomicBool::new(total.is_some()),
            bytes: AtomicU64::new(0),
            weighed: AtomicBool::new(false),
            life: Mutex::new(life),
        })
    }

    fn lock_life(&self) -> std::sync::MutexGuard<'_, Life> {
        self.life.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn live_count(&self) -> Option<WorkCount> {
        self.counted.load(Ordering::Relaxed).then(|| WorkCount {
            completed: self.done.load(Ordering::Relaxed),
            total: Some(self.total.load(Ordering::Relaxed)).filter(|total| *total != PLAN_OPEN),
        })
    }

    fn live_bytes(&self) -> Option<u64> {
        self.weighed
            .load(Ordering::Relaxed)
            .then(|| self.bytes.load(Ordering::Relaxed))
    }

    fn snapshot(&self) -> StageSnapshot {
        let life = self.lock_life();
        let (count, bytes) = life
            .frozen
            .unwrap_or_else(|| (self.live_count(), self.live_bytes()));
        StageSnapshot {
            key: self.key.clone(),
            started_at: life.started_at,
            count,
            bytes,
            end: life.end,
            rss_delta: life
                .mem_at_end
                .and_then(|end| signed_delta(life.mem_at_start.rss, end.rss)),
            heap_delta: life
                .mem_at_end
                .and_then(|end| signed_delta(life.mem_at_start.heap, end.heap)),
        }
    }
}

/// Facts about the zone open that the bench report reads back.
#[derive(Default)]
struct OpenFacts {
    opened_at: Option<Instant>,
    rss_at_open: Option<u64>,
    peak_at_open: Option<u64>,
    image_bytes: Option<u64>,
    first_frame_ms: Option<f32>,
    rss_peak: Option<u64>,
}

struct Inner {
    request_id: u64,
    requested_at: Instant,
    slots: Mutex<Vec<Arc<Slot>>>,
    open: Mutex<OpenFacts>,
    canceled: Arc<AtomicBool>,
}

/// The progress of one load request.
#[derive(Clone)]
pub struct LoadProgress(Arc<Inner>);

impl Default for LoadProgress {
    fn default() -> Self {
        Self::new(0)
    }
}

impl LoadProgress {
    /// A request gets a fresh one instead of clearing the last: tasks of the
    /// map before still hold their own slots and must not write into the next
    /// map's rows.
    pub fn new(request_id: u64) -> Self {
        Self(Arc::new(Inner {
            request_id,
            requested_at: Instant::now(),
            slots: Mutex::new(Vec::new()),
            open: Mutex::new(OpenFacts::default()),
            canceled: Arc::new(AtomicBool::new(false)),
        }))
    }

    pub fn request_id(&self) -> u64 {
        self.0.request_id
    }

    /// The instant the request was made; milestones are offsets from it.
    pub fn requested_at(&self) -> Instant {
        self.0.requested_at
    }

    pub fn cancel(&self) {
        self.0.canceled.store(true, Ordering::Relaxed);
    }

    pub fn is_canceled(&self) -> bool {
        self.0.canceled.load(Ordering::Relaxed)
    }

    /// Open a stage. `total` is the plan size if the producer knows it yet.
    pub fn begin(&self, id: StageId, total: Option<u64>) -> StageHandle {
        self.open_stage(
            StageKey {
                id,
                scope: StageScope::Whole,
            },
            total,
        )
    }

    pub fn begin_scoped(
        &self,
        id: StageId,
        scope: impl Into<StageScope>,
        total: Option<u64>,
    ) -> StageHandle {
        self.open_stage(
            StageKey {
                id,
                scope: scope.into(),
            },
            total,
        )
    }

    fn open_stage(&self, key: StageKey, total: Option<u64>) -> StageHandle {
        let slot = Slot::new(
            key,
            total,
            Life {
                started_at: Some(Instant::now()),
                end: None,
                frozen: None,
                mem_at_start: MemSample::take(),
                mem_at_end: None,
            },
        );
        self.register(Arc::clone(&slot));
        StageHandle {
            slot,
            canceled: Arc::clone(&self.0.canceled),
            published: false,
        }
    }

    pub fn record_skipped(&self, id: StageId) {
        self.record_ended(
            StageKey {
                id,
                scope: StageScope::Whole,
            },
            StageOutcome::Skipped,
        );
    }

    pub fn record_skipped_scoped(&self, id: StageId, scope: impl Into<StageScope>) {
        self.record_ended(
            StageKey {
                id,
                scope: scope.into(),
            },
            StageOutcome::Skipped,
        );
    }

    pub fn record_reused_scoped(&self, id: StageId, scope: impl Into<StageScope>) {
        let key = StageKey {
            id,
            scope: scope.into(),
        };
        diag::info!(World, "load stage: {} reused", key.label());
        self.record_ended(key, StageOutcome::Reused);
    }

    /// A stage that never ran, or that was satisfied without running, is
    /// registered already ended. Only a reused one counts as having started.
    fn record_ended(&self, key: StageKey, outcome: StageOutcome) {
        let at = Instant::now();
        let slot = Slot::new(
            key,
            None,
            Life {
                started_at: (outcome == StageOutcome::Reused).then_some(at),
                end: Some(StageEnd { at, outcome }),
                frozen: None,
                mem_at_start: MemSample::default(),
                mem_at_end: None,
            },
        );
        self.register(slot);
    }

    fn register(&self, slot: Arc<Slot>) {
        self.0
            .slots
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(slot);
    }

    /// Every stage opened so far, in the order they opened.
    pub fn snapshot(&self) -> LoadSnapshot {
        let stages = self
            .0
            .slots
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .map(|slot| slot.snapshot())
            .collect();
        LoadSnapshot {
            request_id: self.0.request_id,
            requested_at: self.0.requested_at,
            now: Instant::now(),
            canceled: self.is_canceled(),
            stages,
        }
    }

    fn open_facts(&self) -> std::sync::MutexGuard<'_, OpenFacts> {
        self.0.open.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Mark the start of the zone open. Only the first call counts.
    pub fn begin_zone_open(&self) {
        let (rss, peak) = (process_resident_bytes(), peak_resident_bytes());
        let mut facts = self.open_facts();
        if facts.opened_at.is_none() {
            *facts = OpenFacts {
                opened_at: Some(Instant::now()),
                rss_at_open: rss,
                peak_at_open: peak,
                ..OpenFacts::default()
            };
        }
    }

    pub fn record_zone_image_bytes(&self, bytes: usize) {
        self.open_facts().image_bytes = u64::try_from(bytes).ok();
    }

    /// Record when the first frame of the loaded map was presented. Returns
    /// whether this call was the one that recorded it.
    pub fn record_first_frame(&self, presented_at: Instant) -> bool {
        let peak_now = peak_resident_bytes();
        let mut facts = self.open_facts();
        let Some(opened_at) = facts.opened_at else {
            return false;
        };
        if facts.first_frame_ms.is_some() {
            return false;
        }
        facts.first_frame_ms =
            Some(presented_at.saturating_duration_since(opened_at).as_secs_f32() * 1000.0);
        // The peak counts for this load only if the high-water mark moved
        // during it, or the process was already at its mark when it began.
        facts.rss_peak = match (facts.rss_at_open, facts.peak_at_open, peak_now) {
            (Some(rss), Some(before), Some(now)) if now > before || rss == before => Some(now),
            _ => None,
        };
        true
    }

    pub fn zone_image_bytes(&self) -> Option<u64> {
        self.open_facts().image_bytes
    }

    pub fn first_frame_ms(&self) -> Option<f32> {
        self.open_facts().first_frame_ms
    }

    pub fn load_rss_peak_bytes(&self) -> Option<u64> {
        self.open_facts().rss_peak
    }

    pub fn rss_at_open_bytes(&self) -> Option<u64> {
        self.open_facts().rss_at_open
    }

    /// The window each started stage occupied. Stages overlap on the load
    /// pool, so the offsets say which ones did.
    pub fn lane_timings(&self) -> Vec<LoadLaneTiming> {
        let snapshot = self.snapshot();
        let origin = snapshot
            .stages
            .iter()
            .filter_map(|stage| stage.started_at)
            .min()
            .unwrap_or(snapshot.requested_at);
        snapshot
            .stages
            .iter()
            .filter_map(|stage| {
                let started = stage.started_at?;
                let count = stage.count.unwrap_or(WorkCount {
                    completed: 0,
                    total: Some(0),
                });
                Some(LoadLaneTiming {
                    label: stage.key.label(),
                    at: started.saturating_duration_since(origin),
                    elapsed: stage.span(snapshot.now).unwrap_or_default(),
                    done: count.completed,
                    total: count.total.unwrap_or(0),
                    running: stage.running(),
                    outcome: stage.outcome(),
                    rss_delta: stage.rss_delta,
                    heap_delta: stage.heap_delta,
                })
            })
            .collect()
    }

    /// One text line per stage, then a summary line.
    pub fn timing_report(&self) -> Vec<String> {
        let lanes = self.lane_timings();
        let mut rows: Vec<String> = lanes
            .iter()
            .map(|lane| {
                let items = match (lane.done, lane.total) {
                    (0, 0) => String::new(),
                    (done, 0) => format!(" items={done}"),
                    (done, total) => format!(" items={done}/{total}"),
                };
                let outcome = match lane.outcome {
                    None | Some(StageOutcome::Done) => String::new(),
                    Some(other) => format!(" {}", other.as_str()),
                };
                format!(
                    "load stage: {} at=+{:.0}ms {:.1}ms{items}{}{outcome}{}",
                    lane.label,
                    lane.at.as_secs_f32() * 1000.0,
                    lane.elapsed.as_secs_f32() * 1000.0,
                    lane.mem_suffix(),
                    if lane.running { " (still running)" } else { "" },
                )
            })
            .collect();
        let wall: Duration = lanes.iter().map(|lane| lane.elapsed).sum();
        let peak = peak_resident_bytes()
            .map(|peak| format!(" rss_peak={:.0}MiB", peak as f64 / MIB))
            .unwrap_or_default();
        rows.push(format!(
            "load stages: {} stages, {:.1}ms of stage wall time (parallel stages overlap){peak}",
            lanes.len(),
            wall.as_secs_f32() * 1000.0,
        ));
        rows
    }
}

/// The write end of one stage.
///
/// Counting is an atomic store and nothing more: no formatting, no logging, no
/// directory walk. The owner publishes the result with [`finish`](Self::finish)
/// or one of its shorthands; a handle dropped without one is recorded as
/// abandoned (or cancelled, when the load was retargeted), never as success.
pub struct StageHandle {
    slot: Arc<Slot>,
    canceled: Arc<AtomicBool>,
    published: bool,
}

impl StageHandle {
    /// The plan is closed at this size. `0` is a known empty plan.
    pub fn set_total(&self, total: u64) {
        self.slot.total.store(total, Ordering::Relaxed);
        self.slot.counted.store(true, Ordering::Relaxed);
    }

    /// Units finished since the last call. Call it after the work.
    pub fn advance(&self, delta: u64) {
        self.slot.done.fetch_add(delta, Ordering::Relaxed);
        self.slot.counted.store(true, Ordering::Relaxed);
    }

    /// Set the count outright, for a producer that already holds the number or
    /// reports a gauge that may fall.
    pub fn set_completed(&self, completed: u64) {
        self.slot.done.store(completed, Ordering::Relaxed);
        self.slot.counted.store(true, Ordering::Relaxed);
    }

    /// Bytes just brought in. Independent of the count.
    pub fn add_bytes(&self, delta: u64) {
        self.slot.bytes.fetch_add(delta, Ordering::Relaxed);
        self.slot.weighed.store(true, Ordering::Relaxed);
    }

    pub fn set_bytes(&self, bytes: u64) {
        self.slot.bytes.store(bytes, Ordering::Relaxed);
        self.slot.weighed.store(true, Ordering::Relaxed);
    }

    pub fn is_canceled(&self) -> bool {
        self.canceled.load(Ordering::Relaxed)
    }

    /// Publish how the stage ended. Reaching the planned total is not this:
    /// only the owner knows the result is ready.
    pub fn finish(mut self, outcome: StageOutcome) {
        self.publish(outcome);
    }

    pub fn done(self) {
        self.finish(StageOutcome::Done);
    }

    pub fn fail(self) {
        self.finish(StageOutcome::Failed);
    }

    pub fn cancel(self) {
        self.finish(StageOutcome::Cancelled);
    }

    pub fn skip(self) {
        self.finish(StageOutcome::Skipped);
    }

    pub fn reuse(self) {
        self.finish(StageOutcome::Reused);
    }

    pub fn finish_from<T, E>(self, result: &Result<T, E>) {
        self.finish(if result.is_ok() {
            StageOutcome::Done
        } else {
            StageOutcome::Failed
        });
    }

    fn publish(&mut self, outcome: StageOutcome) {
        if std::mem::replace(&mut self.published, true) {
            return;
        }
        let at = Instant::now();
        let mem_at_end = MemSample::take();
        let (count, span, mem_at_start) = {
            let mut life = self.slot.lock_life();
            if life.end.is_some() {
                return;
            }
            let count = self.slot.live_count();
            life.frozen = Some((count, self.slot.live_bytes()));
            life.mem_at_end = Some(mem_at_end);
            life.end = Some(StageEnd { at, outcome });
            let span = life
                .started_at
                .map_or(Duration::ZERO, |started| at.saturating_duration_since(started));
            (count, span, life.mem_at_start)
        };
        let items = match count {
            None => String::new(),
            Some(WorkCount {
                completed,
                total: None,
            }) => format!(" items={completed}/?"),
            Some(WorkCount {
                completed,
                total: Some(total),
            }) => format!(" items={completed}/{total}"),
        };
        diag::info!(
            World,
            "load stage: {} {} {:.1}ms{items}{}",
            self.slot.key.label(),
            outcome.as_str(),
            span.as_secs_f32() * 1000.0,
            MemSample::change_text(mem_at_start, mem_at_end),
        );
    }
}

impl Drop for StageHandle {
    fn drop(&mut self) {
        // Unwinding out of a retargeted load is an interruption, not a stage
        // whose owner forgot to report.
        self.publish(if self.is_canceled() {
            StageOutcome::Cancelled
        } else {
            StageOutcome::Abandoned
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_dropped_handle_is_abandoned_and_a_canceled_one_is_cancelled() {
        let progress = LoadProgress::new(7);
        drop(progress.begin(StageId::Images, None));
        progress.cancel();
        drop(progress.begin(StageId::Audio, Some(2)));

        let snapshot = progress.snapshot();
        assert_eq!(snapshot.request_id, 7);
        assert!(snapshot.canceled);
        assert_eq!(snapshot.stages[0].outcome(), Some(StageOutcome::Abandoned));
        assert_eq!(snapshot.stages[1].outcome(), Some(StageOutcome::Cancelled));
    }

    #[test]
    fn counts_and_bytes_freeze_when_the_stage_ends() {
        let progress = LoadProgress::default();
        let stage = progress.begin_scoped(StageId::MapAssets, "mp_x", None);
        assert_eq!(progress.snapshot().stages[0].count.unwrap_or(WorkCount {
            completed: 9,
            total: None
        }).completed, 9, "nothing counted yet");
        stage.advance(3);
        stage.set_total(5);
        stage.add_bytes(100);
        let live = progress.snapshot().stages[0].clone();
        assert!(live.running());
        assert_eq!(live.count, Some(WorkCount { completed: 3, total: Some(5) }));
        assert_eq!(live.bytes, Some(100));

        stage.done();
        let ended = progress.snapshot().stages[0].clone();
        assert_eq!(ended.outcome(), Some(StageOutcome::Done));
        assert_eq!(ended.count, live.count);
        assert_eq!(ended.key.label(), "map_assets/mp_x");
    }

    #[test]
    fn skipped_stages_never_start_and_reused_ones_do() {
        let progress = LoadProgress::default();
        progress.record_skipped(StageId::Preview);
        progress.record_reused_scoped(StageId::CommonAssets, "common_mp");
        let stages = progress.snapshot().stages;
        assert!(stages[0].started_at.is_none());
        assert!(stages[1].started_at.is_some());
        assert_eq!(progress.lane_timings().len(), 1);
    }

    #[test]
    fn an_open_plan_reads_as_no_total() {
        let progress = LoadProgress::default();
        let stage = progress.begin(StageId::Navigation, None);
        stage.advance(1);
        assert_eq!(
            progress.snapshot().stages[0].count,
            Some(WorkCount { completed: 1, total: None })
        );
        stage.finish_from(&Err::<(), _>("no"));
        assert_eq!(
            progress.snapshot().stages[0].outcome(),
            Some(StageOutcome::Failed)
        );
    }

    #[test]
    fn first_frame_needs_an_open_and_counts_once() {
        let progress = LoadProgress::default();
        assert!(!progress.record_first_frame(Instant::now()));
        progress.begin_zone_open();
        assert!(progress.record_first_frame(Instant::now()));
        assert!(!progress.record_first_frame(Instant::now()));
        assert!(progress.first_frame_ms().is_some());
    }
}
