//! Turns a load snapshot into the rows of the loading table.
//!
//! This is presentation only: it decides which rows exist, their order and
//! what their cells say, and it never influences the load itself.

use std::time::{Duration, Instant};

use asset_transport::{LoadSnapshot, StageId, StageOutcome, StageSnapshot};
use bevy::prelude::Color;

/// Where a row stands once all the scopes of its stage are folded together.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum RowState {
    Running,
    Done,
    Reused,
    Skipped,
    Cancelled,
    Abandoned,
    Failed,
}

impl RowState {
    /// Tag appended to the stage name for states that need explaining.
    fn tag(self) -> &'static str {
        match self {
            Self::Running | Self::Done => "",
            Self::Reused => " (reused)",
            Self::Skipped => " (skipped)",
            Self::Cancelled => " (canceled)",
            Self::Abandoned => " (interrupted)",
            Self::Failed => " (failed)",
        }
    }

    pub(crate) fn color(self) -> Color {
        match self {
            Self::Running => Color::srgb(1.0, 0.72, 0.28),
            Self::Done => Color::srgb(0.62, 0.92, 0.68),
            Self::Reused | Self::Skipped => Color::srgb(0.52, 0.56, 0.62),
            Self::Cancelled | Self::Abandoned => Color::srgb(0.86, 0.72, 0.42),
            Self::Failed => Color::srgb(1.0, 0.35, 0.25),
        }
    }
}

/// A number and the unit printed next to it.
pub(crate) struct Metric {
    pub number: String,
    pub unit: String,
}

impl Metric {
    fn new(number: impl Into<String>, unit: impl Into<String>) -> Self {
        Self {
            number: number.into(),
            unit: unit.into(),
        }
    }
}

pub(crate) struct LoadRow {
    pub id: StageId,
    pub name: String,
    /// Bytes when the stage can weigh itself, otherwise its own count.
    pub value: Metric,
    pub state: RowState,
}

/// `running` lists stages in the order they opened, `ended` in the order they
/// finished, so each block only ever grows at the bottom.
pub(crate) struct LoadTable {
    pub running: Vec<LoadRow>,
    pub ended: Vec<LoadRow>,
}

impl LoadTable {
    pub fn row(&self, id: StageId) -> Option<&LoadRow> {
        self.running.iter().chain(&self.ended).find(|row| row.id == id)
    }
}

/// The only place a stage turns into English.
fn stage_label(id: StageId) -> &'static str {
    match id {
        StageId::MapAssets => "Map assets",
        StageId::CommonAssets => "Common assets",
        StageId::Localization => "Localization",
        StageId::Images => "Images",
        StageId::Preview => "Loadscreen",
        StageId::Install => "Match install",
        StageId::WorldImages => "World images",
        StageId::Programs => "Programs",
        StageId::ProgramMerge => "Program merge",
        StageId::Shaders => "Shaders",
        StageId::GpuTextures => "GPU textures",
        StageId::Pipelines => "Pipelines",
        StageId::RenderFrames => "Render frames",
        StageId::FirstPerson => "First person",
        StageId::Audio => "Audio",
        StageId::Navigation => "Navigation",
        StageId::Admission => "Admission",
    }
}

/// What a stage's count counts; empty when the bare number says enough.
fn count_noun(id: StageId) -> &'static str {
    match id {
        StageId::Navigation => "edges",
        StageId::RenderFrames => "frames",
        StageId::FirstPerson => "models",
        StageId::Images | StageId::WorldImages | StageId::GpuTextures => "images",
        StageId::Pipelines => "pipelines",
        StageId::Shaders | StageId::Programs | StageId::ProgramMerge => "programs",
        _ => "",
    }
}

/// Divides `value` down by `step` until it fits, returning the scaled value
/// and how many steps were taken (never past the last unit).
fn scale_down(value: u64, step: f64, unit_count: usize) -> (f64, usize) {
    let mut scaled = value as f64;
    let mut at = 0;
    while scaled >= step && at + 1 < unit_count {
        scaled /= step;
        at += 1;
    }
    (scaled, at)
}

/// One decimal below ten, none from ten up.
fn short_decimal(value: f64) -> String {
    if value < 10.0 {
        format!("{value:.1}")
    } else {
        format!("{value:.0}")
    }
}

pub(crate) fn format_elapsed(span: Duration) -> Metric {
    if span < Duration::from_secs(1) {
        Metric::new(span.as_millis().to_string(), "ms")
    } else {
        Metric::new(format!("{:.1}", span.as_secs_f64()), "s")
    }
}

fn format_bytes(bytes: u64) -> Metric {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    match scale_down(bytes, 1024.0, UNITS.len()) {
        (_, 0) => Metric::new(bytes.to_string(), "B"),
        (scaled, at) => Metric::new(short_decimal(scaled), UNITS[at]),
    }
}

fn format_count(count: u64) -> String {
    const SUFFIX: [&str; 4] = ["", "k", "M", "G"];
    match scale_down(count, 1000.0, SUFFIX.len()) {
        (_, 0) => count.to_string(),
        (scaled, at) => format!("{}{}", short_decimal(scaled), SUFFIX[at]),
    }
}

/// Every scope the load opened under one stage id.
struct Group<'a> {
    id: StageId,
    scopes: Vec<&'a StageSnapshot>,
}

impl Group<'_> {
    fn first_start(&self) -> Option<Instant> {
        self.scopes.iter().filter_map(|s| s.started_at).min()
    }

    fn last_end(&self) -> Option<Instant> {
        self.scopes.iter().filter_map(|s| s.end.as_ref().map(|e| e.at)).max()
    }

    fn running(&self) -> bool {
        self.scopes.iter().any(|s| s.running())
    }

    /// First start to last end. Scopes overlap, so this is not their sum.
    fn elapsed(&self, now: Instant) -> Option<Duration> {
        let first_end = self.scopes.iter().filter_map(|s| s.end.as_ref().map(|e| e.at)).min();
        let from = self.first_start().or(first_end)?;
        let to = match self.last_end() {
            Some(at) if !self.running() => at,
            _ => now,
        };
        Some(to.saturating_duration_since(from))
    }

    fn state(&self) -> RowState {
        if self.running() {
            return RowState::Running;
        }
        let outcomes: Vec<StageOutcome> = self.scopes.iter().filter_map(|s| s.outcome()).collect();
        let has = |wanted| outcomes.contains(&wanted);
        if has(StageOutcome::Failed) {
            RowState::Failed
        } else if has(StageOutcome::Abandoned) {
            RowState::Abandoned
        } else if has(StageOutcome::Cancelled) {
            RowState::Cancelled
        } else if outcomes.iter().all(|o| *o == StageOutcome::Skipped) {
            RowState::Skipped
        } else if outcomes
            .iter()
            .all(|o| matches!(o, StageOutcome::Reused | StageOutcome::Skipped))
        {
            RowState::Reused
        } else {
            RowState::Done
        }
    }

    /// Bytes if any scope weighed itself; otherwise the completed count,
    /// summed only within this one stage because units differ across stages.
    fn value(&self) -> Option<Metric> {
        let weights: Vec<u64> = self.scopes.iter().filter_map(|s| s.bytes).collect();
        if !weights.is_empty() {
            return Some(format_bytes(weights.iter().sum()));
        }
        let mut counts = self.scopes.iter().filter_map(|s| s.count).peekable();
        counts.peek()?;
        let completed: u64 = counts.map(|c| c.completed).sum();
        Some(Metric::new(format_count(completed), count_noun(self.id)))
    }
}

/// One row for each stage the load has opened and can say something about.
pub(crate) fn project(snapshot: &LoadSnapshot) -> LoadTable {
    let mut order: Vec<StageId> = Vec::new();
    for scope in &snapshot.stages {
        if !order.contains(&scope.key.id) {
            order.push(scope.key.id);
        }
    }

    let mut running: Vec<(Instant, LoadRow)> = Vec::new();
    let mut ended: Vec<(Instant, LoadRow)> = Vec::new();
    for id in order {
        let group = Group {
            id,
            scopes: snapshot.stages.iter().filter(|s| s.key.id == id).collect(),
        };
        let Some(span) = group.elapsed(snapshot.now) else {
            continue;
        };
        let state = group.state();
        let mut value = group.value().unwrap_or_else(|| Metric::new("", ""));
        let time = format_elapsed(span);
        if !value.unit.is_empty() {
            value.unit.push_str(" · ");
        }
        value.unit.push_str(&time.number);
        value.unit.push_str(&time.unit);
        let row = LoadRow {
            id,
            name: format!("{}{}", stage_label(id), state.tag()),
            value,
            state,
        };
        // Rows sit where their group first opened, so a later scope of the
        // same stage cannot make the row jump.
        if state == RowState::Running {
            running.push((group.first_start().unwrap_or(snapshot.requested_at), row));
        } else {
            ended.push((group.last_end().unwrap_or(snapshot.requested_at), row));
        }
    }

    // A stable sort keeps registration order for stages that tie on time.
    running.sort_by_key(|(at, _)| *at);
    ended.sort_by_key(|(at, _)| *at);
    LoadTable {
        running: running.into_iter().map(|(_, row)| row).collect(),
        ended: ended.into_iter().map(|(_, row)| row).collect(),
    }
}

/// Wall time from the request to now, or to the last end once nothing is
/// open. Stages overlap, so it is not the sum of the rows.
pub(crate) fn total_elapsed(snapshot: &LoadSnapshot) -> Duration {
    let busy = snapshot.stages.iter().any(|s| s.running());
    let last_end = snapshot.stages.iter().filter_map(|s| s.end.as_ref().map(|e| e.at)).max();
    let to = match last_end {
        Some(at) if !busy => at,
        _ => snapshot.now,
    };
    to.saturating_duration_since(snapshot.requested_at)
}
