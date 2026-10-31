use std::collections::HashMap;

use bevy::prelude::Resource;
use playerstate_t6::game::SpectatorTarget;
use sim::ClientId;

/// One beholder's killcam in progress.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeathReplayRun {
    /// How far back in the archive the replay is read from.
    pub history_window_millis: i32,
    pub spectated_receiver: ClientId,
    pub focus: SpectatorTarget,
    /// When the killcam is over, on the match clock.
    pub expires_at_millis: i32,
    pub ends_match: bool,
    pub began_millis: i32,
    pub killcamoffset_ms: i32,
    pub predelay_ms: i32,
    pub postdelay_ms: i32,
    pub recalc_pending: bool,
}

/// The killcams running now, one per beholder at most.
#[derive(Resource, Clone, Debug, Default)]
pub struct LiveDeathReplays {
    by_watcher: HashMap<ClientId, DeathReplayRun>,
}

impl LiveDeathReplays {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn run_for(&self, beholder: ClientId) -> Option<&DeathReplayRun> {
        self.by_watcher.get(&beholder)
    }

    /// Start `beholder`'s killcam, replacing any they already had.
    pub fn launch(&mut self, beholder: ClientId, run: DeathReplayRun) {
        self.by_watcher.insert(beholder, run);
    }

    pub fn stop(&mut self, beholder: ClientId) {
        self.by_watcher.remove(&beholder);
    }

    pub fn none_running(&self) -> bool {
        self.by_watcher.is_empty()
    }

    /// Drop every killcam whose end time has come and say whose they were.
    pub fn end_finished(&mut self, clock_millis: i32) -> Vec<ClientId> {
        let mut over = Vec::new();
        self.by_watcher.retain(|watcher, run| {
            let running = clock_millis < run.expires_at_millis;
            if !running {
                over.push(*watcher);
            }
            running
        });
        over
    }

    pub fn watching(&self) -> Vec<ClientId> {
        self.by_watcher.keys().copied().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run_ending(expires_at_millis: i32) -> DeathReplayRun {
        DeathReplayRun {
            history_window_millis: 3000,
            spectated_receiver: ClientId(7),
            focus: SpectatorTarget::hitscan(7),
            expires_at_millis,
            ends_match: false,
            began_millis: 0,
            killcamoffset_ms: 0,
            predelay_ms: 0,
            postdelay_ms: 0,
            recalc_pending: false,
        }
    }

    #[test]
    fn a_viewer_has_one_killcam_and_arming_again_replaces_it() {
        let mut killcams = LiveDeathReplays::new();
        assert!(killcams.none_running());
        killcams.launch(ClientId(1), run_ending(100));
        killcams.launch(ClientId(1), run_ending(200));
        assert_eq!(killcams.run_for(ClientId(1)).unwrap().expires_at_millis, 200);
        assert_eq!(killcams.watching(), [ClientId(1)]);
        assert!(killcams.run_for(ClientId(2)).is_none());
    }

    #[test]
    fn clearing_removes_only_that_viewers_killcam() {
        let mut killcams = LiveDeathReplays::new();
        killcams.launch(ClientId(1), run_ending(100));
        killcams.launch(ClientId(2), run_ending(100));
        killcams.stop(ClientId(1));
        assert!(killcams.run_for(ClientId(1)).is_none());
        assert!(killcams.run_for(ClientId(2)).is_some());
        killcams.stop(ClientId(2));
        assert!(killcams.none_running());
    }

    #[test]
    fn a_killcam_ends_at_its_end_time_not_before() {
        let mut killcams = LiveDeathReplays::new();
        killcams.launch(ClientId(1), run_ending(100));
        killcams.launch(ClientId(2), run_ending(300));
        assert!(killcams.end_finished(99).is_empty());
        assert_eq!(killcams.end_finished(100), [ClientId(1)]);
        assert!(killcams.run_for(ClientId(1)).is_none());
        assert!(killcams.run_for(ClientId(2)).is_some());
        assert_eq!(killcams.end_finished(1000), [ClientId(2)]);
        assert!(killcams.none_running());
    }
}
