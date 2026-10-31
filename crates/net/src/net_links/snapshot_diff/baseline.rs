//! The state both ends of a delta stream agree on.

use playerstate_t6::game::AvatarState;
use sim::{ClientId, ProjectileId, FlyingProjectile, Snapshot};
use std::collections::HashMap;

use super::projectile::ProjectileChangeRecord;

/// Players and projectiles as of the last snapshot sent (encoder) or applied (decoder), plus what
/// the last projectile delta contained.
#[derive(Debug, Default)]
pub(super) struct Baseline {
    pub(super) players: Vec<(ClientId, AvatarState)>,
    pub(super) projectiles: HashMap<ProjectileId, FlyingProjectile>,
    pub(super) last_projectile_delta: ProjectileChangeRecord,
}

impl Baseline {
    /// The consensus_reached state of one player; a player not seen before starts from all zeroes.
    pub(super) fn player(&self, peer: ClientId) -> AvatarState {
        self.players
            .iter()
            .find(|(id, _)| *id == peer)
            .map_or(AvatarState::ZERO, |(_, pose)| *pose)
    }

    /// Start from `snapshot` as though it had just been sent and applied.
    pub(super) fn adopt(&mut self, frame: &Snapshot) {
        self.players = frame.players.clone();
        self.projectiles = frame
            .projectiles
            .iter()
            .map(|shot| (shot.id, *shot))
            .collect();
    }

    pub(super) fn clear(&mut self) {
        self.players.clear();
        self.projectiles.clear();
        self.last_projectile_delta = ProjectileChangeRecord::default();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sim::Tick;

    #[test]
    fn an_unseen_player_starts_from_zero() {
        assert_eq!(Baseline::default().player(ClientId(4)), AvatarState::ZERO);
    }

    #[test]
    fn adopting_a_snapshot_remembers_its_players() {
        let mut ps = AvatarState::ZERO;
        ps.last_cmd_ms = 77;
        let frame = Snapshot {
            tick: Tick(1),
            players: vec![(ClientId(2), ps)],
            projectiles: Vec::new(),
            meta: sim::SnapshotMeta::default(),
        };
        let mut baseline = Baseline::default();
        baseline.adopt(&frame);
        assert_eq!(baseline.player(ClientId(2)).last_cmd_ms, 77);
        assert_eq!(baseline.player(ClientId(3)), AvatarState::ZERO);
        baseline.clear();
        assert_eq!(baseline.player(ClientId(2)), AvatarState::ZERO);
    }
}
