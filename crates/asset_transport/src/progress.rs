//! What a map load has done so far, as facts.
//!
//! One [`LoadCompletion`] per load request. The work is cut into stages; a stage
//! opens, may count the units it has handled and the bytes it brought in, and
//! its owner says how it ended. Nothing in here formats a table or decides how
//! many rows fit: a surface reads [`LoadCompletion::snapshot`] and draws it.

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
    /// Names in declaration order, so the discriminant is the index.
    const NAMES: [&'static str; 17] = [
        "map_assets",
        "common_assets",
        "localization",
        "images",
        "preview",
        "install",
        "world_images",
        "programs",
        "program_merge",
        "shaders",
        "gpu_textures",
        "pipelines",
        "render_frames",
        "first_person",
        "audio",
        "navigation",
        "admission",
    ];

    pub fn as_str(self) -> &'static str {
        Self::NAMES[self as usize]
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
    fn whole(id: StageId) -> Self {
        StageKey { id, scope: StageScope::Whole }
    }

    fn scoped(id: StageId, scope: impl Into<StageScope>) -> Self {
        StageKey { id, scope: scope.into() }
    }

    fn label(&self) -> String {
        let head = self.id.as_str();
        match &self.scope {
            StageScope::Named(name) => format!("{head}/{name}"),
            StageScope::Whole => String::from(head),
        }
    }
}

/// How a stage ended, as said by whoever owns it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum StageOutcome {
    /// The owner confirmed the result is ready at its own boundary.
    Done,
    Recycled,
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
        const NAMES: [&str; 6] = ["done", "reused", "skipped", "cancelled", "abandoned", "failed"];
        NAMES[self as usize]
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

impl WorkCount {
    /// `" items=3/5"`, or `" items=3/?"` while the plan is open.
    fn items_text(count: Option<WorkCount>) -> String {
        let Some(WorkCount { completed, total }) = count else {
            return String::new();
        };
        match total {
            Some(total) => format!(" items={completed}/{total}"),
            None => format!(" items={completed}/?"),
        }
    }
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
    pub ask_id: u64,
    pub requested_at: Instant,
    pub now: Instant,
    pub canceled: bool,
    pub stages: Vec<StageSnapshot>,
}

/// Resident set size of this process, where the platform reports it.
pub fn working_set_size() -> Option<u64> {
    status_kib("VmRSS:")
}

/// The highest resident set size this process has reached.
pub fn high_water_memory() -> Option<u64> {
    status_kib("VmHWM:")
}

#[cfg(target_os = "linux")]
fn status_kib(field: &str) -> Option<u64> {
    std::fs::read_to_string("/proc/self/status")
        .ok()?
        .lines()
        .find_map(|row| row.strip_prefix(field))
        .and_then(|tail| tail.split_whitespace().next()?.parse::<u64>().ok())
        .and_then(|kib| kib.checked_mul(1024))
}

#[cfg(not(target_os = "linux"))]
fn status_kib(_field: &str) -> Option<u64> {
    None
}

/// Process memory at one moment.
#[derive(Clone, Copy, Debug, Default)]
struct MemReading([Option<u64>; 2]);

/// Positions in a [`MemReading`], with the label each one is reported under.
const MEM_RESIDENT: usize = 0;
const MEM_HEAP: usize = 1;
const MEM_LABELS: [&str; 2] = ["rss", "heap"];

impl MemReading {
    fn take() -> Self {
        Self([working_set_size(), diag::heap_bytes_in_use()])
    }

    fn gauge(&self, which: usize) -> Option<u64> {
        self.0[which]
    }

    /// `" rss=+12MiB→340MiB heap=..."` for whichever of the two both ends have.
    fn change_text(from: Self, to: Self) -> String {
        let mut text = String::new();
        for (label, (before, after)) in MEM_LABELS.iter().zip(from.0.into_iter().zip(to.0)) {
            if let (Some(before), Some(after)) = (before, after) {
                let (before, after) = (before as f64 / MIB, after as f64 / MIB);
                text += &format!(" {label}={:+.0}MiB→{after:.0}MiB", after - before);
            }
        }
        text
    }
}

fn signed_delta(from: Option<u64>, to: Option<u64>) -> Option<i64> {
    Some(i64::try_from(to?).ok()? - i64::try_from(from?).ok()?)
}

/// A stage's window inside a load. `at` is measured from the first stage that
/// started, not from the request.
#[derive(Clone, Debug, PartialEq)]
pub struct LoadLaneTiming {
    pub caption: String,
    pub at: Duration,
    pub elapsed: Duration,
    pub done: u64,
    pub all_count: u64,
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
        use StageOutcome::*;
        match (self.running, self.outcome) {
            (true, _) => " (running)",
            (false, Some(Recycled)) => " (reused)",
            (false, Some(Skipped)) => " (skipped)",
            (false, Some(Cancelled)) => " (canceled)",
            (false, Some(Abandoned)) => " (interrupted)",
            (false, Some(Failed)) => " (failed)",
            (false, Some(Done) | None) => "",
        }
    }

    pub fn mem_suffix(&self) -> String {
        [("rss", self.rss_delta), ("heap", self.heap_delta)]
            .into_iter()
            .filter_map(|(label, change)| Some(format!(" {label}={:+.0}MiB", change? as f64 / MIB)))
            .collect()
    }

    /// The lane of a stage that has started, placed relative to `origin`.
    fn of_stage(stage: &StageSnapshot, origin: Instant, now: Instant) -> Option<Self> {
        let started = stage.started_at?;
        let WorkCount { completed, total } = stage.count.unwrap_or(WorkCount { completed: 0, total: None });
        Some(LoadLaneTiming {
            caption: stage.key.label(),
            at: started.saturating_duration_since(origin),
            elapsed: stage.span(now).unwrap_or_default(),
            done: completed,
            all_count: total.unwrap_or(0),
            running: stage.running(),
            outcome: stage.outcome(),
            rss_delta: stage.rss_delta,
            heap_delta: stage.heap_delta,
        })
    }

    /// The text row of the timing report.
    fn report_line(&self) -> String {
        let items = match (self.done, self.all_count) {
            (0, 0) => String::new(),
            (done, 0) => format!(" items={done}"),
            (done, total) => format!(" items={done}/{total}"),
        };
        let ending = match self.outcome {
            Some(other) if other != StageOutcome::Done => format!(" {}", other.as_str()),
            _ => String::new(),
        };
        let still = if self.running { " (still running)" } else { "" };
        let (at_ms, took_ms) = (self.at.as_secs_f32() * 1000.0, self.elapsed.as_secs_f32() * 1000.0);
        format!(
            "load stage: {} at=+{at_ms:.0}ms {took_ms:.1}ms{items}{}{still}{ending}{}",
            self.caption,
            self.mem_suffix(),
            "",
        )
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
    /// Memory when the stage opened and, once it has ended, when it did.
    mem: (MemReading, Option<MemReading>),
}

impl Life {
    /// A stage that opens now and has not ended.
    fn opened_now() -> Self {
        Life {
            started_at: Some(Instant::now()),
            end: None,
            frozen: None,
            mem: (MemReading::take(), None),
        }
    }

    /// A stage registered already ended; only a reused one counts as started.
    fn registered_ended(outcome: StageOutcome) -> Self {
        let at = Instant::now();
        Life {
            started_at: (outcome == StageOutcome::Recycled).then_some(at),
            end: Some(StageEnd { at, outcome }),
            frozen: None,
            mem: (MemReading::default(), None),
        }
    }
}

impl Slot {
    fn new(key: StageKey, total: Option<u64>, life: Life) -> Arc<Self> {
        Arc::new(Slot {
            key,
            done: AtomicU64::default(),
            total: AtomicU64::new(total.unwrap_or(PLAN_OPEN)),
            counted: AtomicBool::new(total.is_some()),
            bytes: AtomicU64::default(),
            weighed: AtomicBool::default(),
            life: Mutex::new(life),
        })
    }

    /// Change the count, then note that the stage has one.
    fn update_count(&self, apply: impl FnOnce(&Slot)) {
        apply(self);
        self.counted.store(true, Ordering::Relaxed);
    }

    /// Change the byte weight, then note that the stage has one.
    fn update_weight(&self, apply: impl FnOnce(&Slot)) {
        apply(self);
        self.weighed.store(true, Ordering::Relaxed);
    }

    fn lock_life(&self) -> std::sync::MutexGuard<'_, Life> {
        self.life.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn active_total(&self) -> Option<WorkCount> {
        if !self.counted.load(Ordering::Relaxed) {
            return None;
        }
        let planned = self.total.load(Ordering::Relaxed);
        Some(WorkCount {
            completed: self.done.load(Ordering::Relaxed),
            total: (planned != PLAN_OPEN).then_some(planned),
        })
    }

    fn live_bytes(&self) -> Option<u64> {
        if self.weighed.load(Ordering::Relaxed) {
            Some(self.bytes.load(Ordering::Relaxed))
        } else {
            None
        }
    }

    fn snapshot(&self) -> StageSnapshot {
        let life = self.lock_life();
        let (count, bytes) = match life.frozen {
            Some(frozen) => frozen,
            None => (self.active_total(), self.live_bytes()),
        };
        let drift = |which: usize| {
            let (start, end) = (&life.mem.0, life.mem.1.as_ref()?);
            signed_delta(start.gauge(which), end.gauge(which))
        };
        StageSnapshot {
            key: self.key.clone(),
            started_at: life.started_at,
            count,
            bytes,
            end: life.end,
            rss_delta: drift(MEM_RESIDENT),
            heap_delta: drift(MEM_HEAP),
        }
    }
}

/// What was sampled when the zone open began.
struct OpenSample {
    at: Instant,
    resident_bytes: Option<u64>,
    peak: Option<u64>,
}

/// What was recorded when the first frame of the loaded map went up.
struct FirstFrame {
    after_ms: f32,
    rss_high: Option<u64>,
}

/// Facts about the zone open that the bench report reads back.
#[derive(Default)]
struct OpenFacts {
    sample: Option<OpenSample>,
    inflated: Option<u64>,
    frame: Option<FirstFrame>,
}

struct LoadState {
    ask_id: u64,
    requested_at: Instant,
    stage_slots: Mutex<Vec<Arc<Slot>>>,
    zone_open: Mutex<OpenFacts>,
    canceled: Arc<AtomicBool>,
}

/// The progress of one load request.
#[derive(Clone)]
pub struct LoadCompletion(Arc<LoadState>);

impl Default for LoadCompletion {
    fn default() -> Self {
        Self::new(0)
    }
}

impl LoadCompletion {
    /// A request gets a fresh one instead of clearing the last: tasks of the
    /// map before still hold their own slots and must not write into the next
    /// map's rows.
    pub fn new(request_id: u64) -> Self {
        Self(Arc::new(LoadState {
            ask_id: request_id,
            requested_at: Instant::now(),
            stage_slots: Mutex::new(Vec::new()),
            zone_open: Mutex::new(OpenFacts::default()),
            canceled: Arc::new(AtomicBool::new(false)),
        }))
    }

    pub fn request_id(&self) -> u64 {
        self.0.ask_id
    }

    /// The instant the request was made; milestones are offsets from it.
    pub fn requested_at(&self) -> Instant {
        self.0.requested_at
    }

    pub fn cancel(&self) {
        self.0.canceled.store(true, Ordering::Relaxed);
    }

    pub fn was_aborted(&self) -> bool {
        self.0.canceled.load(Ordering::Relaxed)
    }

    /// Open a stage. `total` is the plan size if the producer knows it yet.
    pub fn begin(&self, id: StageId, total: Option<u64>) -> StageHandle {
        self.open_stage(StageKey::whole(id), total)
    }

    pub fn begin_scoped(&self, id: StageId, scope: impl Into<StageScope>, total: Option<u64>) -> StageHandle {
        self.open_stage(StageKey::scoped(id, scope), total)
    }

    fn open_stage(&self, key: StageKey, total: Option<u64>) -> StageHandle {
        let slot = Slot::new(key, total, Life::opened_now());
        self.slots().push(Arc::clone(&slot));
        StageHandle {
            slot,
            canceled: Arc::clone(&self.0.canceled),
            published: false,
        }
    }

    pub fn record_skipped(&self, id: StageId) {
        self.record_ended(StageKey::whole(id), StageOutcome::Skipped);
    }

    pub fn record_skipped_scoped(&self, id: StageId, scope: impl Into<StageScope>) {
        self.record_ended(StageKey::scoped(id, scope), StageOutcome::Skipped);
    }

    pub fn record_reused_scoped(&self, id: StageId, scope: impl Into<StageScope>) {
        let key = StageKey::scoped(id, scope);
        diag::info!(World, "load stage: {} reused", key.label());
        self.record_ended(key, StageOutcome::Recycled);
    }

    /// A stage that never ran, or that was satisfied without running, is
    /// registered already ended.
    fn record_ended(&self, key: StageKey, outcome: StageOutcome) {
        let slot = Slot::new(key, None, Life::registered_ended(outcome));
        self.slots().push(slot);
    }

    fn slots(&self) -> std::sync::MutexGuard<'_, Vec<Arc<Slot>>> {
        self.0.stage_slots.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Every stage opened so far, in the order they opened.
    pub fn snapshot(&self) -> LoadSnapshot {
        let stages: Vec<StageSnapshot> = self.slots().iter().map(|each| each.snapshot()).collect();
        LoadSnapshot {
            ask_id: self.0.ask_id,
            requested_at: self.0.requested_at,
            now: Instant::now(),
            canceled: self.was_aborted(),
            stages,
        }
    }

    fn open_facts(&self) -> std::sync::MutexGuard<'_, OpenFacts> {
        self.0.zone_open.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Mark the start of the zone open. Only the first call counts.
    pub fn start_zone_unpack(&self) {
        let sample = OpenSample {
            at: Instant::now(),
            resident_bytes: working_set_size(),
            peak: high_water_memory(),
        };
        self.open_facts().sample.get_or_insert(sample);
    }

    pub fn log_zone_texture_sizes(&self, byte_count: usize) {
        self.open_facts().inflated = u64::try_from(byte_count).ok();
    }

    /// Record when the first frame of the loaded map was presented. Returns
    /// whether this call was the one that recorded it.
    pub fn note_first_frame(&self, shown_at: Instant) -> bool {
        let peak_now = high_water_memory();
        let mut facts = self.open_facts();
        if facts.frame.is_some() {
            return false;
        }
        let Some(sample) = facts.sample.as_ref() else {
            return false;
        };
        let after = shown_at.saturating_duration_since(sample.at);
        // The peak counts for this load only if the high-water mark moved
        // during it, or the process was already at its mark when it began.
        let rss_high = match (sample.resident_bytes, sample.peak, peak_now) {
            (Some(resident), Some(before), Some(now)) if now > before || resident == before => Some(now),
            _ => None,
        };
        facts.frame = Some(FirstFrame { after_ms: after.as_secs_f32() * 1000.0, rss_high });
        true
    }

    pub fn zone_texture_bytes(&self) -> Option<u64> {
        self.open_facts().inflated
    }

    pub fn opening_tick_millis(&self) -> Option<f32> {
        self.open_facts().frame.as_ref().map(|frame| frame.after_ms)
    }

    pub fn read_resident_high_water(&self) -> Option<u64> {
        self.open_facts().frame.as_ref().and_then(|frame| frame.rss_high)
    }

    pub fn memory_on_start(&self) -> Option<u64> {
        self.open_facts().sample.as_ref().and_then(|sample| sample.resident_bytes)
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
        let now = snapshot.now;
        let lanes = snapshot.stages.iter().filter_map(|stage| LoadLaneTiming::of_stage(stage, origin, now));
        lanes.collect()
    }

    /// One text line per stage, then a summary line.
    pub fn duration_digest(&self) -> Vec<String> {
        let lanes = self.lane_timings();
        let wall_ms = lanes.iter().map(|lane| lane.elapsed).sum::<Duration>().as_secs_f32() * 1000.0;
        let high = high_water_memory().map_or_else(String::new, |peak| {
            format!(" rss_high={:.0}MiB", peak as f64 / MIB)
        });
        let summary = format!(
            "load stages: {} stages, {wall_ms:.1}ms of stage wall time (parallel stages overlap){high}",
            lanes.len(),
        );
        let mut report: Vec<String> = lanes.iter().map(LoadLaneTiming::report_line).collect();
        report.push(summary);
        report
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
    pub fn set_total(&self, sum: u64) {
        self.slot.update_count(|slot| slot.total.store(sum, Ordering::Relaxed));
    }

    /// Units finished since the last call. Call it after the work.
    pub fn advance(&self, step: u64) {
        self.slot.update_count(|slot| {
            slot.done.fetch_add(step, Ordering::Relaxed);
        });
    }

    /// Set the count outright, for a producer that already holds the number or
    /// reports a gauge that may fall.
    pub fn set_completed(&self, completed: u64) {
        self.slot.update_count(|slot| slot.done.store(completed, Ordering::Relaxed));
    }

    /// Bytes just brought in. Independent of the count.
    pub fn add_bytes(&self, step: u64) {
        self.slot.update_weight(|slot| {
            slot.bytes.fetch_add(step, Ordering::Relaxed);
        });
    }

    pub fn set_bytes(&self, bytes: u64) {
        self.slot.update_weight(|slot| slot.bytes.store(bytes, Ordering::Relaxed));
    }

    pub fn was_aborted(&self) -> bool {
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
        self.finish(StageOutcome::Recycled);
    }

    pub fn finish_from<T, E>(self, result: &Result<T, E>) {
        let outcome = match result {
            Ok(_) => StageOutcome::Done,
            Err(_) => StageOutcome::Failed,
        };
        self.finish(outcome);
    }

    /// Record the end under the lock: what the stage had counted, how long it
    /// took and what memory did. `None` if it had already ended.
    fn seal(&self, outcome: StageOutcome) -> Option<(Option<WorkCount>, Duration, [MemReading; 2])> {
        let at = Instant::now();
        let mem_at_end = MemReading::take();
        let mut life = self.slot.lock_life();
        if life.end.is_some() {
            return None;
        }
        let count = self.slot.active_total();
        life.frozen = Some((count, self.slot.live_bytes()));
        life.mem.1 = Some(mem_at_end);
        life.end = Some(StageEnd { at, outcome });
        let span = life.started_at.map_or(Duration::ZERO, |began| at.saturating_duration_since(began));
        Some((count, span, [life.mem.0, mem_at_end]))
    }

    fn publish(&mut self, outcome: StageOutcome) {
        if std::mem::replace(&mut self.published, true) {
            return;
        }
        let Some((count, span, [mem_before, mem_after])) = self.seal(outcome) else {
            return;
        };
        diag::info!(
            World,
            "load stage: {} {} {:.1}ms{}{}",
            self.slot.key.label(),
            outcome.as_str(),
            span.as_secs_f32() * 1000.0,
            WorkCount::items_text(count),
            MemReading::change_text(mem_before, mem_after),
        );
    }
}

impl Drop for StageHandle {
    fn drop(&mut self) {
        // Unwinding out of a retargeted load is an interruption, not a stage
        // whose owner forgot to report.
        self.publish(if self.was_aborted() {
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
    fn the_lane_suffix_tells_how_the_stage_ended() {
        let lane = |running, outcome| LoadLaneTiming {
            caption: String::new(),
            at: Duration::ZERO,
            elapsed: Duration::ZERO,
            done: 0,
            all_count: 0,
            running,
            outcome,
            rss_delta: None,
            heap_delta: None,
        };
        assert_eq!(lane(true, Some(StageOutcome::Failed)).outcome_suffix(), " (running)");
        assert_eq!(lane(false, Some(StageOutcome::Cancelled)).outcome_suffix(), " (canceled)");
        assert_eq!(lane(false, Some(StageOutcome::Abandoned)).outcome_suffix(), " (interrupted)");
        assert_eq!(lane(false, Some(StageOutcome::Recycled)).outcome_suffix(), " (reused)");
        assert_eq!(lane(false, Some(StageOutcome::Skipped)).outcome_suffix(), " (skipped)");
        assert_eq!(lane(false, Some(StageOutcome::Failed)).outcome_suffix(), " (failed)");
        assert_eq!(lane(false, Some(StageOutcome::Done)).outcome_suffix(), "");
        assert_eq!(lane(false, None).outcome_suffix(), "");
    }

    /// The first stage of a run, as a snapshot would show it now.
    fn first_stage(run: &LoadCompletion) -> StageSnapshot {
        run.snapshot().stages.swap_remove(0)
    }

    fn count_of(completed: u64, total: Option<u64>) -> Option<WorkCount> {
        Some(WorkCount { completed, total })
    }

    #[test]
    fn names_follow_declaration_order() {
        let cases = [
            (StageId::MapAssets.as_str(), "map_assets"),
            (StageId::Audio.as_str(), "audio"),
            (StageId::Admission.as_str(), "admission"),
            (StageOutcome::Done.as_str(), "done"),
            (StageOutcome::Failed.as_str(), "failed"),
            (StageOutcome::Cancelled.as_str(), "cancelled"),
        ];
        for (got, want) in cases {
            assert_eq!(got, want);
        }
    }

    #[test]
    fn a_dropped_handle_is_abandoned_and_a_canceled_one_is_cancelled() {
        let run = LoadCompletion::new(7);
        let (first, second) = (run.begin(StageId::Images, None), run.begin(StageId::Audio, Some(2)));
        drop(first);
        run.cancel();
        drop(second);

        let seen = run.snapshot();
        assert_eq!((seen.ask_id, seen.canceled), (7, true));
        let outcomes: Vec<_> = seen.stages.iter().map(StageSnapshot::outcome).collect();
        assert_eq!(outcomes, [Some(StageOutcome::Abandoned), Some(StageOutcome::Cancelled)]);
    }

    #[test]
    fn counts_and_bytes_freeze_when_the_stage_ends() {
        let run = LoadCompletion::default();
        let handle = run.begin_scoped(StageId::MapAssets, "mp_x", None);
        assert!(first_stage(&run).count.is_none_or(|n| n.completed == 0), "nothing counted yet");

        handle.advance(3);
        handle.set_total(5);
        handle.add_bytes(100);
        let live = first_stage(&run);
        assert!(live.running());
        assert_eq!((live.count, live.bytes), (count_of(3, Some(5)), Some(100)));

        handle.done();
        let ended = first_stage(&run);
        assert_eq!(ended.count, live.count);
        assert_eq!(ended.outcome(), Some(StageOutcome::Done));
        assert_eq!(ended.key.label(), "map_assets/mp_x");
    }

    #[test]
    fn skipped_stages_never_start_and_reused_ones_do() {
        let run = LoadCompletion::default();
        run.record_skipped(StageId::Preview);
        run.record_reused_scoped(StageId::CommonAssets, "common_mp");
        let started: Vec<bool> = run.snapshot().stages.iter().map(|s| s.started_at.is_some()).collect();
        assert_eq!(started, [false, true]);
        assert_eq!(run.lane_timings().len(), 1);
    }

    #[test]
    fn an_open_plan_reads_as_no_total() {
        let run = LoadCompletion::default();
        let handle = run.begin(StageId::Navigation, None);
        handle.advance(1);
        assert_eq!(first_stage(&run).count, count_of(1, None));
        handle.finish_from(&Err::<(), _>("no"));
        assert_eq!(first_stage(&run).outcome(), Some(StageOutcome::Failed));
    }

    #[test]
    fn first_frame_needs_an_open_and_counts_once() {
        let run = LoadCompletion::default();
        let now = Instant::now();
        assert!(!run.note_first_frame(now));
        run.start_zone_unpack();
        let recorded = [run.note_first_frame(now), run.note_first_frame(now)];
        assert_eq!(recorded, [true, false]);
        assert!(run.opening_tick_millis().is_some());
    }

    #[test]
    fn memory_change_lists_only_gauges_both_ends_have() {
        let mib = 1u64 << 20;
        let from = MemReading([Some(100 * mib), None]);
        let to = MemReading([Some(112 * mib), Some(5 * mib)]);
        assert_eq!(MemReading::change_text(from, to), " rss=+12MiB→112MiB");
        assert_eq!(MemReading::change_text(MemReading::default(), to), "");
    }
}
