//! The state behind the loading overlay: what it announces, how long it has
//! been up, and how the load it covers ended.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use bevy::prelude::Resource;

use asset_transport::progress::LoadCompletion;

/// How the load under the overlay stands.
#[derive(Clone, Debug)]
enum LoadPhase {
    Loading,
    Finished,
    LoadFailed(String),
}

/// The overlay drawn over a map load. It only tracks its own life; what the
/// load itself does on completion is [`crate::MapLoadProcess`]'s job, which
/// carries on whether an overlay exists or not.
#[derive(Resource)]
pub struct LoadBackdrop {
    pub progress: LoadCompletion,
    title_text: String,
    mode_caption: String,
    phase: LoadPhase,
    shown_since: Instant,
    /// The overlay's age as of the last [`Self::tick_elapsed`], so everything
    /// drawn in one frame reads the same number.
    age: Duration,
    backdrop_available: bool,
    /// The screen stays up at least this long after it appeared (BO2's own
    /// loading screen fades its picture in over a few seconds).
    min_hold: Duration,
}

impl LoadBackdrop {
    pub fn new(
        progress: LoadCompletion,
        heading: impl Into<String>,
        mode_caption: impl Into<String>,
    ) -> Self {
        Self {
            progress,
            title_text: heading.into(),
            mode_caption: mode_caption.into(),
            phase: LoadPhase::Loading,
            shown_since: Instant::now(),
            age: Duration::ZERO,
            backdrop_available: false,
            min_hold: Duration::ZERO,
        }
    }

    pub fn title(&self) -> &str {
        &self.title_text
    }

    pub fn mode_caption(&self) -> &str {
        &self.mode_caption
    }

    /// Marks the load done. A load that already ended, either way, stays as it
    /// ended.
    pub fn finish(&mut self) {
        if matches!(self.phase, LoadPhase::Loading) {
            self.phase = LoadPhase::Finished;
        }
    }

    /// Marks the load failed, whatever state it was in.
    pub fn fail(&mut self, why: impl Into<String>) {
        self.phase = LoadPhase::LoadFailed(why.into());
    }

    pub fn failure(&self) -> Option<&str> {
        match &self.phase {
            LoadPhase::LoadFailed(reason) => Some(reason),
            _ => None,
        }
    }

    pub fn is_done(&self) -> bool {
        matches!(self.phase, LoadPhase::Finished)
    }

    /// Keeps the screen up for `d` more from now.
    pub fn hold_for(&mut self, d: Duration) {
        self.min_hold = self.shown_since.elapsed() + d;
    }

    /// [`Self::hold_for`], the first time only.
    pub fn hold_once(&mut self, d: Duration) {
        if self.min_hold.is_zero() {
            self.hold_for(d);
            diag::info!(Ui, "loading: held until {} ms", self.min_hold.as_millis());
        }
    }

    pub fn held(&self) -> bool {
        self.shown_since.elapsed() < self.min_hold
    }

    pub fn flag_showcase_done(&mut self) {
        self.backdrop_available = true;
    }

    pub fn backdrop_available(&self) -> bool {
        self.backdrop_available
    }

    /// Reads the clock once for this frame. The caller's frame delta is not
    /// needed: the age is measured from when the overlay appeared.
    pub fn advance_clock(&mut self, _frame_delta: Duration) {
        self.age = self.shown_since.elapsed();
    }

    pub fn elapsed(&self) -> Duration {
        self.age
    }
}

/// The map file shown as a still behind the overlay while the real load runs.
#[derive(Resource, Clone, Debug)]
pub struct LoadBackdropOrigin {
    pub path: PathBuf,
    pub level_name: String,
    pub ticket: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn screen() -> LoadBackdrop {
        LoadBackdrop::new(LoadCompletion::default(), "Nuketown", "Team Deathmatch")
    }

    #[test]
    fn a_fresh_overlay_is_loading_and_carries_its_labels() {
        let s = screen();
        assert!(!s.is_done());
        assert_eq!(s.failure(), None);
        assert_eq!((s.title(), s.mode_caption()), ("Nuketown", "Team Deathmatch"));
    }

    #[test]
    fn finishing_completes_it_once() {
        let mut s = screen();
        s.finish();
        assert!(s.is_done());
        s.finish();
        assert!(s.is_done());
    }

    #[test]
    fn a_failure_wins_over_finishing_and_over_a_finish_before_it() {
        let mut s = screen();
        s.fail("refused");
        s.finish();
        assert!(!s.is_done());
        assert_eq!(s.failure(), Some("refused"));

        let mut done = screen();
        done.finish();
        done.fail("late");
        assert!(!done.is_done());
        assert_eq!(done.failure(), Some("late"));
    }

    #[test]
    fn the_age_only_moves_when_ticked() {
        let mut s = screen();
        assert_eq!(s.elapsed(), Duration::ZERO);
        std::thread::sleep(Duration::from_millis(2));
        assert_eq!(s.elapsed(), Duration::ZERO);
        s.advance_clock(Duration::ZERO);
        assert!(s.elapsed() >= Duration::from_millis(2));
    }

    #[test]
    fn the_preview_flag_latches() {
        let mut s = screen();
        assert!(!s.backdrop_available());
        s.flag_showcase_done();
        assert!(s.backdrop_available());
    }
}
