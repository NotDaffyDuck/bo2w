//! One archived snapshot and what it is estimated to cost to keep.

use playerstate_t6::game::AvatarState;
use sim::{ClientId, Snapshot, Tick};

/// A snapshot as the archive holds it.
#[derive(Clone, Debug, PartialEq)]
pub struct StoredHistoryFrame {
    pub at_tick: Tick,
    pub state: Snapshot,
    /// Clients in the snapshot that have no player state worth replaying: spectators, and anyone
    /// not yet playing.
    pub no_player_state: Vec<ClientId>,
}

impl StoredHistoryFrame {
    /// Player records in the frame.
    pub fn row_total(&self) -> usize {
        self.state.players.len()
    }

    /// Whether `client` has a replayable player state in this frame.
    pub fn player_state_exists(&self, seat: ClientId) -> bool {
        !self.no_player_state.contains(&seat)
            && self.state.players.iter().any(|(id, _)| *id == seat)
    }

    /// Estimated bytes the frame occupies: a fixed base, plus a cost for each player, shot,
    /// entity and HUD element.
    pub fn bytes(&self) -> usize {
        const BASE: usize = 64;
        const PER_PROJECTILE: usize = 64;
        let snapshot = &self.state;
        let hud_elements: usize = snapshot
            .meta
            .clients
            .iter()
            .map(|(_, per)| per.hud_archival.len() + per.hud_current.len())
            .sum();
        BASE + snapshot.players.len() * size_of::<AvatarState>()
            + snapshot.projectiles.len() * PER_PROJECTILE
            + snapshot.meta.entities.len() * size_of::<entity_t6::EntitySnapshot>()
            + hud_elements * size_of::<hud_t6::overlay_piece::OverlayPiece>()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame_of(players: &[u32], absent: &[u32]) -> StoredHistoryFrame {
        let mut snapshot = Snapshot::unpublished(Tick(1));
        for id in players {
            snapshot.players.push((ClientId(*id), AvatarState::ZERO));
        }
        StoredHistoryFrame {
            at_tick: Tick(1),
            state: snapshot,
            no_player_state: absent.iter().map(|id| ClientId(*id)).collect(),
        }
    }

    #[test]
    fn records_are_the_players() {
        assert_eq!(frame_of(&[], &[]).row_total(), 0);
        assert_eq!(frame_of(&[4, 9], &[]).row_total(), 2);
    }

    #[test]
    fn a_player_state_exists_for_a_player_who_is_not_listed_as_without() {
        let frame = frame_of(&[1, 2, 3], &[2]);
        assert!(frame.player_state_exists(ClientId(1)));
        assert!(!frame.player_state_exists(ClientId(2)), "listed as without");
        assert!(!frame.player_state_exists(ClientId(7)), "not in the snapshot");
    }

    #[test]
    fn an_empty_frame_costs_the_base_and_each_player_adds_its_size() {
        let empty = frame_of(&[], &[]).bytes();
        assert_eq!(empty, 64);
        assert_eq!(frame_of(&[1, 2], &[]).bytes(), empty + 2 * size_of::<AvatarState>());
    }

    #[test]
    fn each_entity_adds_its_size() {
        let mut frame = frame_of(&[], &[]);
        let base = frame.bytes();
        frame
            .state
            .meta
            .entities
            .extend(std::iter::repeat_n(entity_t6::EntitySnapshot::default(), 3));
        assert_eq!(frame.bytes(), base + 3 * size_of::<entity_t6::EntitySnapshot>());
    }
}
