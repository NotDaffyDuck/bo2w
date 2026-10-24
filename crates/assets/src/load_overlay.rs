//! The state behind the loading overlay: what it announces, how long it has
//! been up, and how the load it covers ended.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use bevy::prelude::Resource;

use asset_transport::progress::LoadProgress;

/// How the load under the overlay stands.
#[derive(Clone, Debug)]
enum Phase {
    Loading,
    Finished,
    Failed(String),
}

/// The overlay drawn over a map load. It only tracks its own life; what the
/// load itself does on completion is [`crate::MapLoadProcess`]'s job, which
/// carries on whether an overlay exists or not.
#[derive(Resource)]
pub struct LoadingScreen {
    pub progress: LoadProgress,
    title: String,
    mode_label: String,
    phase: Phase,
    shown_since: Instant,
    /// The overlay's age as of the last [`Self::tick_elapsed`], so everything
    /// drawn in one frame reads the same number.
    age: Duration,
    preview_ready: bool,
}

impl LoadingScreen {
    pub fn new(
        progress: LoadProgress,
        title: impl Into<String>,
        mode_label: impl Into<String>,
    ) -> Self {
        Self {
            progress,
            title: title.into(),
            mode_label: mode_label.into(),
            phase: Phase::Loading,
            shown_since: Instant::now(),
            age: Duration::ZERO,
            preview_ready: false,
        }
    }

    pub fn title(&self) -> &str {
        &self.title
    }

    pub fn mode_label(&self) -> &str {
        &self.mode_label
    }

    /// Marks the load done. A load that already ended, either way, stays as it
    /// ended.
    pub fn finish(&mut self) {
        if matches!(self.phase, Phase::Loading) {
            self.phase = Phase::Finished;
        }
    }

    /// Marks the load failed, whatever state it was in.
    pub fn fail(&mut self, reason: impl Into<String>) {
        self.phase = Phase::Failed(reason.into());
    }

    pub fn failure(&self) -> Option<&str> {
        match &self.phase {
            Phase::Failed(reason) => Some(reason),
            _ => None,
        }
    }

    pub fn is_complete(&self) -> bool {
        matches!(self.phase, Phase::Finished)
    }

    pub fn mark_preview_ready(&mut self) {
        self.preview_ready = true;
    }

    pub fn preview_ready(&self) -> bool {
        self.preview_ready
    }

    /// Reads the clock once for this frame. The caller's frame delta is not
    /// needed: the age is measured from when the overlay appeared.
    pub fn tick_elapsed(&mut self, _frame_delta: Duration) {
        self.age = self.shown_since.elapsed();
    }

    pub fn elapsed(&self) -> Duration {
        self.age
    }
}

/// The map file shown as a still behind the overlay while the real load runs.
#[derive(Resource, Clone, Debug)]
pub struct LoadingPreviewSource {
    pub path: PathBuf,
    pub map_name: String,
    pub request_id: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn screen() -> LoadingScreen {
        LoadingScreen::new(LoadProgress::default(), "Nuketown", "Team Deathmatch")
    }

    #[test]
    fn a_fresh_overlay_is_loading_and_carries_its_labels() {
        let s = screen();
        assert!(!s.is_complete());
        assert_eq!(s.failure(), None);
        assert_eq!((s.title(), s.mode_label()), ("Nuketown", "Team Deathmatch"));
    }

    #[test]
    fn finishing_completes_it_once() {
        let mut s = screen();
        s.finish();
        assert!(s.is_complete());
        s.finish();
        assert!(s.is_complete());
    }

    #[test]
    fn a_failure_wins_over_finishing_and_over_a_finish_before_it() {
        let mut s = screen();
        s.fail("refused");
        s.finish();
        assert!(!s.is_complete());
        assert_eq!(s.failure(), Some("refused"));

        let mut done = screen();
        done.finish();
        done.fail("late");
        assert!(!done.is_complete());
        assert_eq!(done.failure(), Some("late"));
    }

    #[test]
    fn the_age_only_moves_when_ticked() {
        let mut s = screen();
        assert_eq!(s.elapsed(), Duration::ZERO);
        std::thread::sleep(Duration::from_millis(2));
        assert_eq!(s.elapsed(), Duration::ZERO);
        s.tick_elapsed(Duration::ZERO);
        assert!(s.elapsed() >= Duration::from_millis(2));
    }

    #[test]
    fn the_preview_flag_latches() {
        let mut s = screen();
        assert!(!s.preview_ready());
        s.mark_preview_ready();
        assert!(s.preview_ready());
    }
}
