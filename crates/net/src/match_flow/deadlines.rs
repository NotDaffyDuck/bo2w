//! Deadlines for loading and for a silent authority, and the failure record they produce.

use frame::GameSessionStamp;

/// How long a host may take to load a match.
pub const LOAD_TIMEOUT_MILLIS: u64 = 120_000;
/// How long a running authority may go without a sign of life.
pub const ACTIVE_QUIET_LIMIT_MILLIS: u64 = 60_000;
/// How long a peer may take to be admitted once the match runs.
pub const PEER_ENTRY_TIMEOUT_MILLIS: u64 = 30_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LoadStageKind {
    Idle,
    Loading,
    Running,
}

/// Watches one match for progress: loading is held to a deadline from its start, running to a
/// limit on silence since the last beat.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LoadStageTimer {
    pub phase: LoadStageKind,
    stage_entered_stamp: u64,
    latest_advance_millis: u64,
}

impl Default for LoadStageTimer {
    fn default() -> Self {
        Self {
            phase: LoadStageKind::Idle,
            stage_entered_stamp: 0,
            latest_advance_millis: 0,
        }
    }
}

impl LoadStageTimer {
    /// A sign of life. An older timestamp than the last one is ignored.
    pub fn mark_alive(&mut self, clock_millis: u64) {
        self.latest_advance_millis = self.latest_advance_millis.max(clock_millis);
    }

    pub fn enter_phase(&mut self, kind: LoadStageKind, clock_millis: u64) {
        self.phase = kind;
        self.stage_entered_stamp = clock_millis;
        self.latest_advance_millis = clock_millis;
    }

    pub fn stage_entered_stamp(&self) -> u64 {
        self.stage_entered_stamp
    }

    pub fn timed_out(&self, clock_millis: u64) -> bool {
        let (since, limit) = match self.phase {
            LoadStageKind::Idle => return false,
            LoadStageKind::Loading => (self.stage_entered_stamp, LOAD_TIMEOUT_MILLIS),
            LoadStageKind::Running => (self.latest_advance_millis, ACTIVE_QUIET_LIMIT_MILLIS),
        };
        clock_millis.saturating_sub(since) >= limit
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AbortStep {
    Transport,
    Load,
    Authority,
    Admission,
    ListingTenancy,
    Overload,
}

impl AbortStep {
    fn tag(self) -> &'static str {
        match self {
            Self::Transport => "transport",
            Self::Load => "load",
            Self::Authority => "authority",
            Self::Admission => "admission",
            Self::ListingTenancy => "advert-lease",
            Self::Overload => "overload",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SessionAbortRecord {
    pub stage: AbortStep,
    pub source: String,
    pub game_session_key: GameSessionStamp,
}

impl core::fmt::Display for SessionAbortRecord {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(
            f,
            "{} ({:?}, epoch {}): {}",
            self.stage.tag(),
            self.stage,
            self.game_session_key.game_generation,
            self.source
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_idle_watch_never_expires() {
        let watch = LoadStageTimer::default();
        assert!(!watch.timed_out(0));
        assert!(!watch.timed_out(u64::MAX));
    }

    #[test]
    fn loading_expires_a_fixed_time_after_it_started_whatever_the_beats() {
        let mut watch = LoadStageTimer::default();
        watch.enter_phase(LoadStageKind::Loading, 1_000);
        watch.mark_alive(100_000);
        assert!(!watch.timed_out(1_000 + LOAD_TIMEOUT_MILLIS - 1));
        assert!(watch.timed_out(1_000 + LOAD_TIMEOUT_MILLIS));
        assert_eq!(watch.stage_entered_stamp(), 1_000);
    }

    #[test]
    fn running_expires_after_silence_and_a_beat_resets_the_count() {
        let mut watch = LoadStageTimer::default();
        watch.enter_phase(LoadStageKind::Running, 5_000);
        assert!(watch.timed_out(5_000 + ACTIVE_QUIET_LIMIT_MILLIS));
        watch.mark_alive(30_000);
        assert!(!watch.timed_out(5_000 + ACTIVE_QUIET_LIMIT_MILLIS));
        assert!(watch.timed_out(30_000 + ACTIVE_QUIET_LIMIT_MILLIS));
    }

    #[test]
    fn an_old_beat_does_not_move_the_clock_back() {
        let mut watch = LoadStageTimer::default();
        watch.enter_phase(LoadStageKind::Running, 50_000);
        watch.mark_alive(10_000);
        assert!(!watch.timed_out(50_000 + ACTIVE_QUIET_LIMIT_MILLIS - 1));
        assert!(watch.timed_out(50_000 + ACTIVE_QUIET_LIMIT_MILLIS));
    }

    #[test]
    fn a_clock_that_runs_backwards_does_not_expire_anything() {
        let mut watch = LoadStageTimer::default();
        watch.enter_phase(LoadStageKind::Loading, 9_000);
        assert!(!watch.timed_out(1));
    }

    #[test]
    fn the_failure_reads_stage_label_debug_name_epoch_and_source() {
        let fail = SessionAbortRecord {
            stage: AbortStep::ListingTenancy,
            source: "lease lost".into(),
            game_session_key: GameSessionStamp::new([1; 16], 4),
        };
        assert_eq!(fail.to_string(), "advert-lease (ListingTenancy, epoch 4): lease lost");
        let fail = SessionAbortRecord {
            stage: AbortStep::Load,
            source: "x".into(),
            game_session_key: GameSessionStamp::NONE,
        };
        assert_eq!(fail.to_string(), "load (Load, epoch 0): x");
    }

    #[test]
    fn every_stage_has_its_own_label() {
        let labels: Vec<_> = [
            AbortStep::Transport,
            AbortStep::Load,
            AbortStep::Authority,
            AbortStep::Admission,
            AbortStep::ListingTenancy,
            AbortStep::Overload,
        ]
        .into_iter()
        .map(AbortStep::tag)
        .collect();
        assert_eq!(
            labels,
            ["transport", "load", "authority", "admission", "advert-lease", "overload"]
        );
    }
}
