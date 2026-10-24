//! The row table behind `load_jobs.csv`.
//!
//! Transitional: nothing in the engine records a load job any more, so the
//! table is always empty. It stays only because the bench report in
//! `bootstrap` still reads it; once that reading goes, this file goes too.

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

/// The environment variable that asks for one row per asset.
const MODE_ENV: &str = "BO2MP_BENCH_JOBS";

/// The class of work a row describes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JobKind {
    ZoneOpen,
    ZoneWalk,
    ImagePlan,
    ImageDecode,
    Image,
    AudioPrep,
    AudioClip,
}

impl JobKind {
    /// The name written to the `kind` column.
    pub const fn name(self) -> &'static str {
        match self {
            Self::ZoneOpen => "zone_open",
            Self::ZoneWalk => "zone_walk",
            Self::ImagePlan => "image_plan",
            Self::ImageDecode => "image_decode",
            Self::Image => "image",
            Self::AudioPrep => "audio_prep",
            Self::AudioClip => "audio_clip",
        }
    }
}

/// Whether a prepared result came out of the artifact cache.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CacheResult {
    Hit,
    Miss,
    Stored,
    Failed,
}

impl CacheResult {
    /// The name written to the `cache_result` column.
    pub const fn name(self) -> &'static str {
        match self {
            Self::Hit => "hit",
            Self::Miss => "miss",
            Self::Stored => "stored",
            Self::Failed => "failed",
        }
    }
}

/// One recorded job. A missing stamp or size is `None`, never zero.
#[derive(Clone, Debug)]
pub struct JobRow {
    pub id: u64,
    /// Ids of the jobs this one waited on.
    pub deps: Vec<u64>,
    pub kind: JobKind,
    pub namespace: Option<&'static str>,
    pub canonical: Option<String>,
    pub discovered_at: Instant,
    pub plan_ready_at: Option<Instant>,
    pub enqueued_at: Option<Instant>,
    pub started_at: Option<Instant>,
    pub decode_started_at: Option<Instant>,
    pub finished_at: Option<Instant>,
    pub joined_at: Option<Instant>,
    pub outstanding_at_decode: Option<u64>,
    pub waited_for_budget: bool,
    pub source_bytes: Option<u64>,
    pub prepared_bytes: Option<u64>,
    pub reused_bytes: Option<u64>,
    pub output_bytes: Option<u64>,
    pub retained_bytes: Option<u64>,
    pub discarded_bytes: Option<u64>,
    pub rss_at_finish: Option<u64>,
    pub items: Option<u64>,
    pub cache_result: Option<CacheResult>,
    pub discard_reason: Option<String>,
}

/// A copy of the table and the mode it was kept in.
pub struct Jobs {
    pub rows: Vec<JobRow>,
    pub dropped: u64,
    pub per_asset: bool,
    pub capacity: usize,
}

static PER_ASSET: AtomicBool = AtomicBool::new(false);

/// Start recording when `on`. The mode comes from `BO2MP_BENCH_JOBS`:
/// `assets` keeps a row per asset, anything else a row per class.
pub fn arm(on: bool) {
    if !on {
        return;
    }
    let per_asset = std::env::var(MODE_ENV)
        .is_ok_and(|mode| mode.eq_ignore_ascii_case("assets") || mode.eq_ignore_ascii_case("asset"));
    PER_ASSET.store(per_asset, Ordering::Relaxed);
}

/// The rows recorded so far, in the order the jobs were opened.
pub fn snapshot() -> Jobs {
    Jobs {
        rows: Vec::new(),
        dropped: 0,
        per_asset: PER_ASSET.load(Ordering::Relaxed),
        capacity: 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nothing_records_rows_yet() {
        arm(true);
        let jobs = snapshot();
        assert!(jobs.rows.is_empty());
        assert_eq!(jobs.dropped, 0);
    }

    #[test]
    fn column_names_are_stable() {
        assert_eq!(JobKind::ImageDecode.name(), "image_decode");
        assert_eq!(JobKind::AudioClip.name(), "audio_clip");
        assert_eq!(CacheResult::Stored.name(), "stored");
    }
}
