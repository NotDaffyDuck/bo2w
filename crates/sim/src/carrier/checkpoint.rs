//! `SimWorld` operations that cross the line between the world and the outside: taking and
//! adopting snapshots, shutting a game down, and the collision-history queries.

use super::SimWorld;
use crate::AdoptReport;
use crate::frame::{
    collect_dropped_items, collect_players, collect_projectiles, collect_script_movers, player_ref,
};
use crate::snapshot::Snapshot;
use crate::world::{ClientId, HitvolDumpRow, SimState, Tick};

impl SimWorld {
    /// What a client would be told about the world at `tick`.
    pub fn snapshot(&self, tick: Tick) -> Snapshot {
        let state = self.ecs.get::<SimState>(self.state_entity).expect(super::STATE_MISSING);
        state.snapshot_with_dynamic_rows(
            tick,
            collect_players(&self.ecs),
            collect_projectiles(&self.ecs),
            collect_script_movers(&self.ecs),
            collect_dropped_items(&self.ecs),
        )
    }

    /// Replaces this world's rows with the ones in `snapshot`.
    ///
    /// A snapshot carries no script state, so this is for worlds that run no script program;
    /// a world with scripts must be restored from a full [`SimWorld`] copy instead.
    pub fn adopt_snapshot(&mut self, snapshot: &Snapshot) -> AdoptReport {
        assert!(
            self.gsc_program_fingerprint().is_none(),
            "snapshot lacks GSC state; restore a full SimWorld checkpoint"
        );
        self.frame().adopt_snapshot(snapshot)
    }

    /// As [`SimWorld::adopt_snapshot`], for a client that keeps its own predicted player (`local`).
    pub fn adopt_prediction_snapshot(
        &mut self,
        snapshot: &Snapshot,
        local: ClientId,
    ) -> AdoptReport {
        assert!(
            self.gsc_program_fingerprint().is_none(),
            "prediction snapshots cannot restore authority GSC state"
        );
        self.frame().adopt_prediction_snapshot(snapshot, local)
    }

    /// Ends the game: clears the match and drops everything the scripts were running.
    pub fn shutdown_game(&mut self) {
        self.frame().shutdown_game();
        crate::script::reset(&mut self.ecs);
    }

    /// The hit volumes of every player, for the `hitvol` dump.
    pub fn hitvol_dump(&self) -> Vec<HitvolDumpRow> {
        SimState::hitvol_dump(self, |id| player_ref(&self.ecs, id).copied())
    }

    /// The position history that lag compensation rewinds `attacker`'s targets to at `current`.
    pub fn lagcomp_query_for(
        &self,
        attacker: ClientId,
        current: Tick,
    ) -> crate::bullet_collision::LagcompQuery {
        SimState::lagcomp_query_for(self, attacker, current, |id| {
            player_ref(&self.ecs, id).copied()
        })
    }
}
