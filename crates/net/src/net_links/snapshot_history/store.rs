//! The archive itself: frames in tick order, oldest dropped first.

use std::collections::VecDeque;

use bevy::prelude::Resource;
use sim::{ClientId, PlayerSessionStage, Snapshot, Tick};

use super::*;

/// Running totals, kept across [`SnapshotVault::clear`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct HistoryStoreStats {
    /// Frames ever accepted.
    pub accepted: u64,
    /// Frames dropped to stay within the limits.
    pub dropped: u64,
    /// Most frames held at once.
    pub high_water_steps: usize,
    /// Most player records held at once.
    pub high_water_entries: usize,
    /// Most estimated bytes held at once.
    pub high_water_bytes: usize,
}

impl HistoryStoreStats {
    /// Counts a frame taken in and raises the peaks to what is now held.
    fn took(&mut self, frames_held: usize, load: Load) {
        self.accepted = self.accepted.saturating_add(1);
        self.high_water_steps = self.high_water_steps.max(frames_held);
        self.high_water_entries = self.high_water_entries.max(load.players);
        self.high_water_bytes = self.high_water_bytes.max(load.weight);
    }

    fn gave_up_one(&mut self) {
        self.dropped = self.dropped.saturating_add(1);
    }
}

/// What the held frames add up to.
#[derive(Clone, Copy, Debug, Default)]
struct Load {
    players: usize,
    weight: usize,
}

impl Load {
    fn include(&mut self, held: &StoredHistoryFrame) {
        self.players += held.row_total();
        self.weight += held.bytes();
    }

    fn exclude(&mut self, held: &StoredHistoryFrame) {
        self.players -= held.row_total();
        self.weight -= held.bytes();
    }
}

#[derive(Resource, Clone, Debug, Default)]
pub struct SnapshotVault {
    pub(super) kept: VecDeque<StoredHistoryFrame>,
    load: Load,
    tally: HistoryStoreStats,
}

impl SnapshotVault {
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a frame newer than every frame held; returns whether it was taken. Older frames go
    /// when the archive passes its frame or byte limit, but the newest one always stays.
    pub fn append(&mut self, entry: StoredHistoryFrame) -> bool {
        let newest_tick = self.kept.back().map(|held| held.at_tick);
        let newer = newest_tick.is_none_or(|tick| tick < entry.at_tick);
        if newer {
            self.load.include(&entry);
            self.kept.push_back(entry);
            self.drop_oldest_while_over();
            self.tally.took(self.kept.len(), self.load);
        }
        newer
    }

    /// Archive a snapshot. Spectators, and clients that are not alive, dead or waiting to respawn,
    /// are recorded as having no player state.
    pub fn append_snapshot(&mut self, view: &Snapshot, spectators: &[ClientId]) -> bool {
        let playing = |lifecycle: &PlayerSessionStage| {
            matches!(
                lifecycle,
                PlayerSessionStage::Alive | PlayerSessionStage::Dead | PlayerSessionStage::AwaitingNextLife
            )
        };
        let no_player_state = view
            .meta
            .clients
            .iter()
            .filter(|(cid, meta)| spectators.contains(cid) || !playing(&meta.lifecycle))
            .map(|(cid, _)| *cid)
            .collect();
        self.append(StoredHistoryFrame {
            at_tick: view.tick,
            state: view.clone(),
            no_player_state,
        })
    }

    /// Past a limit, with more than the newest frame left to give up.
    fn over_limits(&self) -> bool {
        let held = self.kept.len();
        held > 1 && (held > HISTORY_FRAME_CAPACITY || self.load.weight > HISTORY_SIZE_LIMIT_BYTES)
    }

    fn drop_oldest_while_over(&mut self) {
        while self.over_limits() {
            let Some(gone) = self.kept.pop_front() else {
                return;
            };
            self.load.exclude(&gone);
            self.tally.gave_up_one();
        }
    }

    /// Player records held.
    pub fn row_total(&self) -> usize {
        self.load.players
    }

    /// Estimated bytes held.
    pub fn bytes(&self) -> usize {
        self.load.weight
    }

    pub fn is_empty(&self) -> bool {
        self.kept.is_empty()
    }

    pub fn stats(&self) -> HistoryStoreStats {
        self.tally
    }

    pub fn oldest(&self) -> Option<&StoredHistoryFrame> {
        self.kept.front()
    }

    pub fn newest(&self) -> Option<&StoredHistoryFrame> {
        self.kept.back()
    }

    /// The frame archived for exactly `tick`.
    pub fn held_at(&self, at: Tick) -> Option<&StoredHistoryFrame> {
        self.kept.iter().rev().find(|entry| entry.at_tick == at)
    }

    /// Drop every frame. The metrics keep counting.
    pub fn clear(&mut self) {
        self.kept.clear();
        self.load = Load::default();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use playerstate_t6::game::AvatarState;
    use sim::ClientSnapshotMeta;

    fn snapshot_at(tick: u32, players: u32) -> Snapshot {
        let mut snapshot = Snapshot::unpublished(Tick(tick));
        for id in 0..players {
            snapshot.players.push((ClientId(id), AvatarState::ZERO));
        }
        snapshot
    }

    fn frame_at(tick: u32, players: u32) -> StoredHistoryFrame {
        StoredHistoryFrame {
            at_tick: Tick(tick),
            state: snapshot_at(tick, players),
            no_player_state: Vec::new(),
        }
    }

    #[test]
    fn frames_must_arrive_in_increasing_tick_order() {
        let mut vault = SnapshotVault::new();
        assert!(vault.append(frame_at(5, 1)));
        assert!(!vault.append(frame_at(5, 1)), "same tick");
        assert!(!vault.append(frame_at(4, 1)), "older tick");
        assert!(vault.append(frame_at(9, 1)), "gaps are fine");
        assert_eq!(vault.oldest().unwrap().at_tick, Tick(5));
        assert_eq!(vault.newest().unwrap().at_tick, Tick(9));
        assert_eq!(vault.stats().accepted, 2);
    }

    #[test]
    fn records_and_bytes_follow_the_frames_held() {
        let mut vault = SnapshotVault::new();
        let (a, b) = (frame_at(1, 2), frame_at(2, 3));
        let expected_bytes = a.bytes() + b.bytes();
        vault.append(a);
        vault.append(b);
        assert_eq!(vault.row_total(), 5);
        assert_eq!(vault.bytes(), expected_bytes);
        vault.clear();
        assert!(vault.is_empty());
        assert_eq!((vault.row_total(), vault.bytes()), (0, 0));
    }

    #[test]
    fn past_the_frame_limit_the_oldest_go() {
        let mut vault = SnapshotVault::new();
        for tick in 0..HISTORY_FRAME_CAPACITY as u32 + 3 {
            vault.append(frame_at(tick, 1));
        }
        assert_eq!(vault.oldest().unwrap().at_tick, Tick(3));
        assert_eq!(vault.row_total(), HISTORY_FRAME_CAPACITY);
        let metrics = vault.stats();
        assert_eq!(metrics.dropped, 3);
        assert_eq!(metrics.high_water_steps, HISTORY_FRAME_CAPACITY);
        assert_eq!(metrics.high_water_entries, HISTORY_FRAME_CAPACITY);
    }

    #[test]
    fn the_newest_frame_stays_even_when_it_alone_is_over_budget() {
        let mut vault = SnapshotVault::new();
        vault.append(frame_at(1, 1));
        let mut huge = frame_at(2, 0);
        let entity_bytes = size_of::<entity_t6::EntitySnapshot>();
        huge.state.meta.entities.extend(std::iter::repeat_n(
            entity_t6::EntitySnapshot::default(),
            HISTORY_SIZE_LIMIT_BYTES / entity_bytes + 1,
        ));
        assert!(vault.append(huge));
        assert_eq!(vault.oldest().unwrap().at_tick, Tick(2));
        assert_eq!(vault.newest().unwrap().at_tick, Tick(2));
        assert!(vault.bytes() > HISTORY_SIZE_LIMIT_BYTES);
    }

    #[test]
    fn clearing_keeps_the_metrics() {
        let mut vault = SnapshotVault::new();
        vault.append(frame_at(1, 1));
        vault.clear();
        assert_eq!(vault.stats().accepted, 1);
        assert!(vault.append(frame_at(1, 1)), "ticks may restart after a clear");
    }

    #[test]
    fn frames_are_found_by_exact_tick() {
        let mut vault = SnapshotVault::new();
        vault.append(frame_at(2, 1));
        vault.append(frame_at(6, 2));
        assert_eq!(vault.held_at(Tick(6)).unwrap().row_total(), 2);
        assert!(vault.held_at(Tick(4)).is_none());
    }

    #[test]
    fn spectators_and_the_not_yet_playing_have_no_player_state() {
        let mut snapshot = snapshot_at(3, 0);
        let lifecycles = [
            (1, PlayerSessionStage::Alive),
            (2, PlayerSessionStage::Dead),
            (3, PlayerSessionStage::AwaitingNextLife),
            (4, PlayerSessionStage::Spectating),
            (5, PlayerSessionStage::Connecting),
            (6, PlayerSessionStage::Alive), // a spectator by seat, whatever the lifecycle says
        ];
        for (id, lifecycle) in lifecycles {
            snapshot.meta.clients.push((
                ClientId(id),
                ClientSnapshotMeta {
                    lifecycle,
                    ..Default::default()
                },
            ));
        }
        let mut vault = SnapshotVault::new();
        assert!(vault.append_snapshot(&snapshot, &[ClientId(6)]));
        let held = vault.newest().unwrap();
        assert_eq!(held.at_tick, Tick(3));
        assert_eq!(held.no_player_state, [ClientId(4), ClientId(5), ClientId(6)]);
    }
}
