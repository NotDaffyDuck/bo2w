//! The buffer of recent snapshots that remote players are shown from, a fixed delay behind the
//! newest.

mod blend;
mod sample;
mod shot;
mod types;

use std::collections::VecDeque;
use std::sync::Arc;

use playerstate_t6::game::AvatarState;
use sim::{ClientId, Snapshot};

pub use types::{
    PoseFillIn, CONSTANT_LAG_POLICY_GENERATION, SMOOTHER_FRAME_CAPACITY, STANDIN_LAG_MILLIS,
    SmoothingSampleVerdict, SmoothingSampleOrigin, SmoothingInstant, SmootherStrategy,
    SmootherPolicyGeneration, SmootherReading, RemoteBufferShortfall,
};

use crate::host_tick::peer_mail::HOST_STEP_MILLIS;

#[derive(Clone, Debug)]
/// One buffered snapshot and the game time it stands for.
struct HeldFrame { clock_stamp_ms: i32, frame: Arc<Snapshot> }

/// Snapshots in time order, oldest first, at most [`SMOOTHER_FRAME_CAPACITY`] of them.
#[derive(Clone, Debug, Default)]
pub struct OtherPlayerSmoother {
    history: VecDeque<HeldFrame>,
}

impl OtherPlayerSmoother {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn mode(&self) -> SmootherStrategy {
        SmootherStrategy::ConstantLag
    }

    pub fn take_copy(&mut self, latest: &Snapshot) {
        let shared = Arc::new(latest.clone());
        self.enqueue_shared_item(shared);
    }

    /// Add a snapshot. One that is not newer than the newest is dropped. One where any player
    /// changed between live and archived frames, or changed whom the killcam follows, starts
    /// the buffer over: samples must not blend across that change.
    pub fn enqueue_shared_item(&mut self, arrived: Arc<Snapshot>) {
        let clock_stamp_ms = state_clock_millis(&arrived);
        let start_over = match self.history.back() {
            Some(newest) if clock_stamp_ms <= newest.clock_stamp_ms => return,
            Some(newest) => view_changed_between(&newest.frame, &arrived),
            None => false,
        };
        if start_over {
            self.history.truncate(0);
        }
        self.history.push_back(HeldFrame { clock_stamp_ms, frame: arrived });
        // Over capacity: the oldest go.
        let excess = self.history.len().saturating_sub(SMOOTHER_FRAME_CAPACITY);
        self.history.drain(..excess);
    }

    /// The newest snapshot at or before the delayed render time, else the oldest one.
    pub fn blended_frame_at(&self, draw_clock_millis: i32) -> Option<Arc<Snapshot>> {
        let effective = lagged_clock(draw_clock_millis);
        let shown = self.history.iter().rev().find(|held| held.clock_stamp_ms <= effective);
        shown.or(self.history.front()).map(|held| held.frame.clone())
    }

    /// The first snapshot after the delayed render time.
    pub(crate) fn snapshot_after(&self, draw_clock_millis: i32) -> Option<Arc<Snapshot>> {
        let effective = lagged_clock(draw_clock_millis);
        let next = self.history.iter().find(|held| held.clock_stamp_ms > effective);
        next.map(|held| held.frame.clone())
    }

    /// The buffered entries on each side of `clock_stamp_ms`: the last at or before it and the first at
    /// or after it. They are the same entry when one is exactly at `clock_stamp_ms`.
    fn bracket(&self, clock_stamp_ms: i32) -> (Option<&HeldFrame>, Option<&HeldFrame>) {
        // The first entry at or after the time; the search for the one before stops there too.
        let split = self.history.iter().position(|held| held.clock_stamp_ms >= clock_stamp_ms);
        let searched = split.map_or(self.history.len(), |at| at + 1);
        let before = self.history.iter().take(searched).filter(|held| held.clock_stamp_ms <= clock_stamp_ms).last();
        (before, split.map(|at| &self.history[at]))
    }

    /// `client`'s newest buffered state, with the tick it was in.
    fn latest_reading(&self, peer: ClientId) -> Option<(sim::Tick, AvatarState)> {
        let reading = |held: &HeldFrame| pose_of_peer(&held.frame.players, peer).map(|state| (held.frame.tick, *state));
        self.history.iter().rev().find_map(reading)
    }
}

fn lagged_clock(draw_clock_millis: i32) -> i32 {
    draw_clock_millis.saturating_sub(STANDIN_LAG_MILLIS)
}

fn state_clock_millis(snapshot: &Snapshot) -> i32 {
    snapshot.tick.0 as i32 * HOST_STEP_MILLIS
}

fn pose_of_peer(players: &[(ClientId, AvatarState)], peer: ClientId) -> Option<&AvatarState> {
    players.iter().find_map(|(id, state)| (*id == peer).then_some(state))
}

/// Whether some player is live in one snapshot and archived in the other, or follows a different
/// killcam target.
fn view_changed_between(old: &Snapshot, new: &Snapshot) -> bool {
    new.players.iter().any(|(id, state)| {
        old.players.iter().any(|(prior_id, old_state)| {
            id == prior_id
                && (old_state.is_live_frame() != state.is_live_frame()
                    || old_state.death_replay_player != state.death_replay_player)
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at_tick(tick: u32) -> Snapshot {
        let mut snapshot = Snapshot::unpublished(sim::Tick(0));
        snapshot.tick = sim::Tick(tick);
        snapshot
    }

    fn times(proxy: &OtherPlayerSmoother) -> Vec<i32> {
        proxy.history.iter().map(|entry| entry.clock_stamp_ms).collect()
    }

    #[test]
    fn snapshots_that_are_not_newer_are_dropped() {
        let mut proxy = OtherPlayerSmoother::new();
        proxy.take_copy(&at_tick(5));
        proxy.take_copy(&at_tick(5));
        proxy.take_copy(&at_tick(4));
        proxy.take_copy(&at_tick(6));
        assert_eq!(times(&proxy), vec![5 * HOST_STEP_MILLIS, 6 * HOST_STEP_MILLIS]);
    }

    #[test]
    fn the_buffer_keeps_only_the_newest_ticks() {
        let mut proxy = OtherPlayerSmoother::new();
        for tick in 1..=(SMOOTHER_FRAME_CAPACITY as u32 + 5) {
            proxy.take_copy(&at_tick(tick));
        }
        assert_eq!(proxy.history.len(), SMOOTHER_FRAME_CAPACITY);
        assert_eq!(proxy.history.front().map(|entry| entry.frame.tick), Some(sim::Tick(6)));
    }

    #[test]
    fn snapshot_at_takes_the_newest_not_after_the_delayed_time_else_the_oldest() {
        let mut proxy = OtherPlayerSmoother::new();
        for tick in [10, 20, 30] {
            proxy.take_copy(&at_tick(tick));
        }
        let ms = |tick: i32| tick * HOST_STEP_MILLIS;
        let pick = |render: i32| proxy.blended_frame_at(render).map(|snapshot| snapshot.tick.0);
        assert_eq!(pick(ms(25) + STANDIN_LAG_MILLIS), Some(20));
        assert_eq!(pick(ms(30) + STANDIN_LAG_MILLIS), Some(30));
        assert_eq!(pick(0), Some(10));
        assert_eq!(
            proxy
                .snapshot_after(ms(20) + STANDIN_LAG_MILLIS)
                .map(|snapshot| snapshot.tick.0),
            Some(30)
        );
        assert!(proxy.snapshot_after(ms(30) + STANDIN_LAG_MILLIS).is_none());
    }

    #[test]
    fn an_empty_proxy_has_nothing_to_show() {
        let proxy = OtherPlayerSmoother::new();
        assert!(proxy.blended_frame_at(1000).is_none());
        assert!(proxy.snapshot_after(1000).is_none());
        assert_eq!(proxy.mode(), SmootherStrategy::ConstantLag);
    }
}
