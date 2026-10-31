//! `WholeSim` operations that cross the line between the world and the outside: taking and
//! adopting snapshots, shutting a game down, and the collision-history queries.

use crate::TakeoverReport;
use crate::shot_trace::RewindRequest;
use crate::step_view::avatar_of;
use crate::step_view::{gather_loot, gather_avatars};
use crate::step_view::{gather_shells, gather_movers};
use crate::tick_publish::TickSnapshot;
use crate::sim_parts::{SeatNo, HitboxReportRow, MatchSimulation, StepNo};

impl super::WholeSim {
    /// What a client would be told about the world at `tick`.
    pub fn snapshot(&self, tick: StepNo) -> TickSnapshot {
        let core = self.bevy.get::<MatchSimulation>(self.state_row).expect(super::STATE_MISSING);
        let players = gather_avatars(&self.bevy);
        let shells = gather_shells(&self.bevy);
        let movers = gather_movers(&self.bevy);
        let items = gather_loot(&self.bevy);
        core.snapshot_plus_live_rows(tick, players, shells, movers, items)
    }

    /// Replaces this world's rows with the ones in `snap`.
    ///
    /// A snapshot carries no script state, so this is for worlds that run no script program;
    /// a world with scripts must be restored from a full [`WholeSim`] copy instead.
    pub fn take_over_from(&mut self, snap: &TickSnapshot) -> TakeoverReport {
        if self.gsc_program_fingerprint().is_some() {
            panic!("snapshot lacks GSC state; restore a full SimWorld checkpoint");
        }
        self.frame().take_over_from(snap)
    }

    /// As [`WholeSim::take_over_from`], for a client that keeps its own predicted player (`own`).
    pub fn adopt_prediction_snapshot(&mut self, snap: &TickSnapshot, own: SeatNo) -> TakeoverReport {
        if self.gsc_program_fingerprint().is_some() {
            panic!("prediction snapshots cannot restore authority GSC state");
        }
        self.frame().adopt_prediction_snapshot(snap, own)
    }

    /// Ends the game: clears the match and drops everything the scripts were running.
    pub fn end_game_session(&mut self) {
        let mut frame = self.frame();
        frame.end_game_session();
        crate::gsc_host::reset(&mut self.bevy);
    }

    /// The hit volumes of every player, for the `hitvol` dump.
    pub fn hitvol_dump(&self) -> Vec<HitboxReportRow> {
        let lookup = |client| avatar_of(&self.bevy, client).copied();
        MatchSimulation::hitvol_dump(self, lookup)
    }

    /// The position history that lag compensation rewinds `attacker`'s targets to at `current`.
    pub fn rewind_request_for(
        &self,
        attacker: SeatNo,
        current: StepNo,
    ) -> RewindRequest {
        let lookup = |client| avatar_of(&self.bevy, client).copied();
        MatchSimulation::rewind_request_for(self, attacker, current, lookup)
    }
}
