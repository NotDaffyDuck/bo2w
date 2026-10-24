//! Deadlines for loading and for a silent authority, and the failure record they produce.

use frame::MatchKey;

/// How long a host may take to load a match.
pub const LOADING_DEADLINE_MS: u64 = 120_000;
/// How long a running authority may go without a sign of life.
pub const RUNNING_SILENCE_MS: u64 = 60_000;
/// How long a peer may take to be admitted once the match runs.
pub const PEER_ADMISSION_DEADLINE_MS: u64 = 30_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProgressPhase {
    Idle,
    Loading,
    Running,
}

/// Watches one match for progress: loading is held to a deadline from its start, running to a
/// limit on silence since the last beat.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProgressWatch {
    pub phase: ProgressPhase,
    phase_started_at: u64,
    last_progress_ms: u64,
}

impl Default for ProgressWatch {
    fn default() -> Self {
        Self {
            phase: ProgressPhase::Idle,
            phase_started_at: 0,
            last_progress_ms: 0,
        }
    }
}

impl ProgressWatch {
    /// A sign of life. An older timestamp than the last one is ignored.
    pub fn beat(&mut self, now_ms: u64) {
        self.last_progress_ms = self.last_progress_ms.max(now_ms);
    }

    pub fn set_phase(&mut self, phase: ProgressPhase, now_ms: u64) {
        self.phase = phase;
        self.phase_started_at = now_ms;
        self.last_progress_ms = now_ms;
    }

    pub fn phase_started_at(&self) -> u64 {
        self.phase_started_at
    }

    pub fn expired(&self, now_ms: u64) -> bool {
        let (since, limit) = match self.phase {
            ProgressPhase::Idle => return false,
            ProgressPhase::Loading => (self.phase_started_at, LOADING_DEADLINE_MS),
            ProgressPhase::Running => (self.last_progress_ms, RUNNING_SILENCE_MS),
        };
        now_ms.saturating_sub(since) >= limit
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FailStage {
    Transport,
    Load,
    Authority,
    Admission,
    AdvertLease,
    Overload,
}

impl FailStage {
    fn label(self) -> &'static str {
        match self {
            Self::Transport => "transport",
            Self::Load => "load",
            Self::Authority => "authority",
            Self::Admission => "admission",
            Self::AdvertLease => "advert-lease",
            Self::Overload => "overload",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SessionFail {
    pub stage: FailStage,
    pub source: String,
    pub match_key: MatchKey,
}

impl core::fmt::Display for SessionFail {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(
            f,
            "{} at {:?} epoch={}: {}",
            self.stage.label(),
            self.stage,
            self.match_key.match_epoch,
            self.source
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_idle_watch_never_expires() {
        let watch = ProgressWatch::default();
        assert!(!watch.expired(0));
        assert!(!watch.expired(u64::MAX));
    }

    #[test]
    fn loading_expires_a_fixed_time_after_it_started_whatever_the_beats() {
        let mut watch = ProgressWatch::default();
        watch.set_phase(ProgressPhase::Loading, 1_000);
        watch.beat(100_000);
        assert!(!watch.expired(1_000 + LOADING_DEADLINE_MS - 1));
        assert!(watch.expired(1_000 + LOADING_DEADLINE_MS));
        assert_eq!(watch.phase_started_at(), 1_000);
    }

    #[test]
    fn running_expires_after_silence_and_a_beat_resets_the_count() {
        let mut watch = ProgressWatch::default();
        watch.set_phase(ProgressPhase::Running, 5_000);
        assert!(watch.expired(5_000 + RUNNING_SILENCE_MS));
        watch.beat(30_000);
        assert!(!watch.expired(5_000 + RUNNING_SILENCE_MS));
        assert!(watch.expired(30_000 + RUNNING_SILENCE_MS));
    }

    #[test]
    fn an_old_beat_does_not_move_the_clock_back() {
        let mut watch = ProgressWatch::default();
        watch.set_phase(ProgressPhase::Running, 50_000);
        watch.beat(10_000);
        assert!(!watch.expired(50_000 + RUNNING_SILENCE_MS - 1));
        assert!(watch.expired(50_000 + RUNNING_SILENCE_MS));
    }

    #[test]
    fn a_clock_that_runs_backwards_does_not_expire_anything() {
        let mut watch = ProgressWatch::default();
        watch.set_phase(ProgressPhase::Loading, 9_000);
        assert!(!watch.expired(1));
    }

    #[test]
    fn the_failure_reads_stage_label_debug_name_epoch_and_source() {
        let fail = SessionFail {
            stage: FailStage::AdvertLease,
            source: "lease lost".into(),
            match_key: MatchKey::new([1; 16], 4),
        };
        assert_eq!(fail.to_string(), "advert-lease at AdvertLease epoch=4: lease lost");
        let fail = SessionFail {
            stage: FailStage::Load,
            source: "x".into(),
            match_key: MatchKey::NONE,
        };
        assert_eq!(fail.to_string(), "load at Load epoch=0: x");
    }

    #[test]
    fn every_stage_has_its_own_label() {
        let labels: Vec<_> = [
            FailStage::Transport,
            FailStage::Load,
            FailStage::Authority,
            FailStage::Admission,
            FailStage::AdvertLease,
            FailStage::Overload,
        ]
        .into_iter()
        .map(FailStage::label)
        .collect();
        assert_eq!(
            labels,
            ["transport", "load", "authority", "admission", "advert-lease", "overload"]
        );
    }
}
