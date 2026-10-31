//! The authority's snapshot arriving at the prediction: the world is replaced by the server's,
//! the commands it confirmed are dropped, the unconfirmed ones are run again on top, and the
//! distance the view has to travel is handed to the error tracker.

use playerstate_t6::game::{AvatarState, InputCommand, buttons, entity_flag_bits, secondary_state_bits, movement_state_bits};
use sim::{Snapshot, AdvanceCause, Tick};

use super::ground::{pmove_runs_for, world_state_of_self};
use super::history::{ReceiptLookup, InputCommandNumber, ForecastMoveEntry};
use super::metrics::CorrectionResult;
use super::state::LocalForecast;
use crate::snapshot_apply::CorrectionCutoff;
use crate::net_links::crossing_fields::{FieldDrift, avatar_drift};

/// After this many snapshots in a row that confirm nothing, the history is thrown away.
pub const UNCONFIRMED_STATE_CAP: u32 = 20;

/// True when the `BO2MP_PRED_LOG` variable asks for one log line per misprediction.
fn log_each_miss() -> bool {
    use std::sync::OnceLock;
    static SWITCH: OnceLock<bool> = OnceLock::new();
    *SWITCH.get_or_init(|| matches!(std::env::var_os("BO2MP_PRED_LOG"), Some(_)))
}

/// A verdict that only carries the given flag set by `fill`.
fn verdict_with(fill: impl FnOnce(&mut CorrectionResult)) -> CorrectionResult {
    let mut verdict = CorrectionResult::default();
    fill(&mut verdict);
    verdict
}

impl LocalForecast {
    /// Throws the unconfirmed history away so that the next snapshot is adopted as it is. This is
    /// the same recovery `UNCONFIRMED_STATE_CAP` triggers, available on request. The send window
    /// stays untouched: the authority eats a client's commands strictly in order, and a gap in
    /// that window would block it permanently.
    pub fn force_resync(&mut self) {
        self.quiet_snaps = 0;
        if self.replay_moves.is_empty() {
            return;
        }
        self.drop_history();
        self.stats.compelled_takeovers += 1;
    }

    /// Clears the history while keeping the newest command as the floor. Late receipts_batch for the
    /// cleared commands then count as retired; treated as broken, they would wipe every window
    /// that follows as well.
    fn drop_history(&mut self) {
        let last_sent = InputCommandNumber(self.seq_next.0.wrapping_sub(1));
        self.floor = self.newest_cmd.map(|cmd| (last_sent, cmd)).or(self.floor);
        self.replay_moves.clear();
    }

    /// The next predicted tick is the first one after the server's correction point.
    fn restart_ticks_at(&mut self, edge: CorrectionCutoff) {
        let Tick(first) = edge.playback_step;
        self.tick_next = first;
    }

    /// Handles an acknowledgement that arrives without a snapshot to adopt.
    pub fn drop_confirmed(&mut self, ack_seq: Option<InputCommandNumber>) {
        if self.forecast_on {
            self.stats.frames_seen += 1;
            self.apply_ack(ack_seq);
        }
    }

    /// An confirm_receipt older than the floor is read as the floor itself.
    fn lift_to_floor(&self, seq: InputCommandNumber) -> InputCommandNumber {
        self.floor.map_or(seq, |(floor, _)| seq.max(floor))
    }

    /// Checks `ack_seq` against the history. The flags describe what happened; the record of the
    /// confirmed command comes back too when the confirm_receipt matched one.
    fn apply_ack(
        &mut self,
        ack_seq: Option<InputCommandNumber>,
    ) -> (CorrectionResult, Option<ForecastMoveEntry>) {
        let Some(sent) = ack_seq else {
            return (self.snapshot_without_ack(), None);
        };
        let seq = self.lift_to_floor(sent);
        match self.replay_moves.drop_confirmed_moves(seq) {
            ReceiptLookup::Matched(record) => {
                self.acked_cmd = Some(record.cmd);
                self.stats.receipts_paired += 1;
                self.quiet_snaps = 0;
                (verdict_with(|v| v.receipt_confirmed = true), Some(record))
            }
            ReceiptLookup::Retired => {
                self.stats.discarded_receipts += 1;
                self.quiet_snaps = 0;
                (verdict_with(|v| v.discarded_receipt = true), None)
            }
            ReceiptLookup::Broken => {
                self.stats.compelled_takeovers += 1;
                self.drop_history();
                (verdict_with(|v| v.compelled_takeover = true), None)
            }
        }
    }

    /// A snapshot that confirms nothing: harmless while the history is empty, otherwise it counts
    /// towards the limit after which the history is dropped.
    fn snapshot_without_ack(&mut self) -> CorrectionResult {
        if self.replay_moves.is_empty() {
            self.quiet_snaps = 0;
            return CorrectionResult::default();
        }
        self.quiet_snaps = self.quiet_snaps.saturating_add(1);
        if self.quiet_snaps > UNCONFIRMED_STATE_CAP {
            self.stats.compelled_takeovers += 1;
            self.drop_history();
            self.quiet_snaps = 0;
            verdict_with(|v| v.compelled_takeover = true)
        } else {
            CorrectionResult::default()
        }
    }

    /// Takes in `snapshot`: adopts its world, retires what `ack_seq` confirms, runs the commands
    /// still waiting again and follows how the view error moved.
    pub fn on_authority(
        &mut self,
        snapshot: &Snapshot,
        ack_seq: Option<InputCommandNumber>,
    ) -> CorrectionResult {
        if !self.forecast_on {
            return CorrectionResult::default();
        }
        self.stats.frames_seen += 1;

        let prior = self.predicted_ps;
        let edge = CorrectionCutoff::after(snapshot.tick);
        let report = self.sandbox.adopt_prediction_snapshot(snapshot, self.me);
        let server_ps = world_state_of_self(snapshot, self.me);

        let idle_on_server = server_ps.is_some_and(|ps| !pmove_runs_for(&ps));
        let mut verdict = if idle_on_server {
            self.follow_authority_alone(server_ps, edge);
            CorrectionResult::default()
        } else {
            self.reconcile(ack_seq, server_ps, edge)
        };
        verdict.took_host_state = true;
        verdict.report = report;

        self.track_view_error(prior, server_ps);
        self.saw_local = server_ps.is_some();
        verdict
    }

    /// The server has this player on the kill camera. Nothing is predicted then: the history goes
    /// and the player is whatever the server says.
    fn follow_authority_alone(
        &mut self,
        server_ps: Option<AvatarState>,
        edge: CorrectionCutoff,
    ) {
        let last_sent = InputCommandNumber(self.seq_next.0.saturating_sub(1));
        self.floor = self.newest_cmd.map(|cmd| (last_sent, cmd)).or(self.floor);
        self.replay_moves.clear();
        self.predicted_ps = server_ps;
        self.restart_ticks_at(edge);
    }

    /// The normal path: retire confirmed commands, measure the confirmed one against what the
    /// server did, then run everything newer again.
    fn reconcile(
        &mut self,
        ack_seq: Option<InputCommandNumber>,
        server_ps: Option<AvatarState>,
        edge: CorrectionCutoff,
    ) -> CorrectionResult {
        let (mut verdict, anchor) = self.apply_ack(ack_seq);

        if let Some((record, theirs)) = anchor.as_ref().zip(server_ps.as_ref()) {
            verdict.deviation = avatar_drift(&record.output, theirs);
            if let Some(miss) = verdict.deviation {
                self.note_miss(miss, &record.output, theirs);
            }
        }

        self.seed_old_cmd(anchor.as_ref());
        self.restart_ticks_at(edge);
        let replayed = self.rerun_pending();
        verdict.rerun_move_total = replayed;
        self.stats.rerun_move_total += replayed as u64;
        self.predicted_ps = self.local_state().map_or(server_ps, Some);
        verdict
    }

    /// Counts a misprediction and, when asked to by the environment, logs it.
    fn note_miss(&mut self, miss: FieldDrift, ours: &AvatarState, theirs: &AvatarState) {
        self.stats.drift_samples += 1;
        let FieldDrift { name, gap, .. } = miss;
        self.stats.latest_drift = Some((name, gap));
        if !log_each_miss() {
            return;
        }
        let [ox, oy, oz] = ours.origin;
        let [sx, sy, sz] = theirs.origin;
        diag::info!(
            WireLink,
            "prediction miss: {} by {:.1}, predicted ({:.0} {:.0} {:.0}) server ({:.0} {:.0} {:.0})",
            miss.name,
            miss.gap,
            ox,
            oy,
            oz,
            sx,
            sy,
            sz
        );
    }

    /// Runs every command left in the history again on the newly adopted world and rewrites its
    /// record. The result is how many commands were waiting.
    fn rerun_pending(&mut self) -> usize {
        let waiting = self.replay_moves.len();
        let mut next_tick = self.tick_next;
        for slot in 0..waiting {
            if self.replay_slot(slot, next_tick) {
                next_tick = next_tick.wrapping_add(1);
            }
        }
        self.tick_next = next_tick;
        waiting
    }

    /// Runs the command in `slot` once more at `tick`. False when the command is not newer than
    /// the world's own clock, in which case nothing changes.
    fn replay_slot(
        &mut self,
        slot_idx: usize,
        tick: u32,
    ) -> bool {
        let Some(cmd) = self.replay_moves.record_mut(slot_idx).map(|record| record.cmd) else {
            return false;
        };
        let was_ps = self.local_state().unwrap_or(AvatarState::ZERO);
        let span_ms = cmd.authority_clock_ms.wrapping_sub(was_ps.last_cmd_ms);
        if cmd.authority_clock_ms <= was_ps.last_cmd_ms {
            return false;
        }
        let now_ps = self
            .advance_forecast(Tick(tick), cmd, span_ms, AdvanceCause::Replay)
            .unwrap_or(was_ps);
        if let Some(record) = self.replay_moves.record_mut(slot_idx) {
            record.at_tick = Tick(tick);
            record.input = was_ps;
            record.output = now_ps;
        }
        true
    }

    /// Tells the world which command came right before the oldest one waiting, because the
    /// movement code looks at the previous command's buttons, angles and stick.
    fn seed_old_cmd(
        &mut self,
        anchor: Option<&ForecastMoveEntry>,
    ) {
        let chained = self
            .floor
            .filter(|(floor, _)| self.replay_moves.oldest_seq() == Some(floor.next()))
            .map(|(_, cmd)| cmd);
        let previous: Option<InputCommand> = anchor
            .map(|record| record.cmd)
            .or(chained)
            .or(self.acked_cmd);

        let (held, angles, stick) = previous.map_or_else(
            || {
                let sprinting = self
                    .sandbox
                    .player(self.me)
                    .is_some_and(|ps| ps.movement_state_bits & movement_state_bits::SPRINTING != 0);
                (if sprinting { buttons::SPRINT } else { 0 }, [0; 3], [0; 2])
            },
            |cmd| (cmd.buttons, cmd.angles, [cmd.thrust_ahead, cmd.thrust_lateral]),
        );
        self.sandbox.store_prior_input(self.me, held, angles);
        self.sandbox.set_old_cmd_move(self.me, stick);
    }

    /// Feeds the view-error tracker with how far the player moved between the prediction before
    /// this snapshot and the one after it.
    fn track_view_error(&mut self, prior: Option<AvatarState>, server_ps: Option<AvatarState>) {
        let pair = prior.zip(self.predicted_ps);
        let spectating_someone = server_ps.is_some_and(|ps| {
            ps.secondary_state_bits & (secondary_state_bits::PLAYER | secondary_state_bits::DEAD_KILLCAM_TPV) != 0
        });

        if spectating_someone && !self.saw_local {
            if let Some((was, now)) = pair {
                self.view_error.record_jump(was.origin, now.origin);
            }
            self.view_error.initialize_fresh_object();
            return;
        }

        let Some((was, now)) = pair else {
            return;
        };
        if (was.e_flags ^ now.e_flags) & entity_flag_bits::TELEPORT != 0 {
            self.view_error.record_jump(was.origin, now.origin);
            self.view_error.start_over();
        } else if was.last_cmd_ms == now.last_cmd_ms {
            self.view_error.start_blend(was.origin, now.origin);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::HostInstant;
    use sim::{ClientId, SimWorld};

    fn armed(cap: usize) -> LocalForecast {
        let mut prediction = LocalForecast::with_log_limit(ClientId(1), cap);
        prediction.prime_from_assets(&SimWorld::new());
        prediction
    }

    fn run(prediction: &mut LocalForecast, commands: u32) {
        for n in 1..=commands {
            prediction
                .run_ahead(InputCommand::default(), HostInstant::from_millis_value(n as i32 * 16))
                .expect("the history has room");
        }
    }

    #[test]
    fn a_disarmed_prediction_ignores_snapshots_and_acks() {
        let mut prediction = LocalForecast::new(ClientId(1));
        let outcome = prediction.on_authority(&Snapshot::unpublished(Tick(1)), Some(InputCommandNumber(1)));
        assert_eq!(outcome, CorrectionResult::default());
        prediction.drop_confirmed(Some(InputCommandNumber(1)));
        assert_eq!(prediction.metrics().frames_seen, 0);
    }

    #[test]
    fn an_ack_for_a_held_command_retires_everything_up_to_it() {
        let mut prediction = armed(16);
        run(&mut prediction, 4);
        prediction.drop_confirmed(Some(InputCommandNumber(2)));
        assert_eq!(prediction.history().oldest_seq(), Some(InputCommandNumber(3)));
        let metrics = prediction.metrics();
        assert_eq!((metrics.frames_seen, metrics.receipts_paired), (1, 1));
    }

    #[test]
    fn an_ack_from_the_future_drops_the_history_and_counts_a_forced_adopt() {
        let mut prediction = armed(16);
        run(&mut prediction, 3);
        prediction.drop_confirmed(Some(InputCommandNumber(40)));
        assert!(prediction.history().is_empty());
        assert_eq!(prediction.metrics().compelled_takeovers, 1);
    }

    #[test]
    fn late_acks_for_dropped_commands_retire_instead_of_breaking() {
        let mut prediction = armed(16);
        run(&mut prediction, 3);
        prediction.force_resync();
        run(&mut prediction, 2);
        // Seq 1 is below the floor (3), so it reads as the floor itself, not as a broken confirm_receipt.
        prediction.drop_confirmed(Some(InputCommandNumber(1)));
        assert_eq!(prediction.metrics().compelled_takeovers, 1);
        assert_eq!(prediction.history().len(), 2);
    }

    #[test]
    fn forcing_a_resync_with_nothing_pending_changes_nothing() {
        let mut prediction = armed(16);
        prediction.force_resync();
        assert_eq!(prediction.metrics().compelled_takeovers, 0);
    }

    #[test]
    fn forcing_a_resync_empties_the_history_once() {
        let mut prediction = armed(16);
        run(&mut prediction, 2);
        prediction.force_resync();
        assert!(prediction.history().is_empty());
        assert_eq!(prediction.metrics().compelled_takeovers, 1);
    }

    #[test]
    fn too_many_snapshots_without_an_ack_drops_the_history() {
        let mut prediction = armed(16);
        run(&mut prediction, 2);
        for _ in 0..UNCONFIRMED_STATE_CAP {
            prediction.drop_confirmed(None);
            assert_eq!(prediction.history().len(), 2);
        }
        prediction.drop_confirmed(None);
        assert!(prediction.history().is_empty());
        assert_eq!(prediction.metrics().compelled_takeovers, 1);
    }

    #[test]
    fn snapshots_without_an_ack_are_free_while_nothing_is_pending() {
        let mut prediction = armed(16);
        for _ in 0..(UNCONFIRMED_STATE_CAP * 2) {
            prediction.drop_confirmed(None);
        }
        assert_eq!(prediction.metrics().compelled_takeovers, 0);
    }

    #[test]
    fn adopting_a_snapshot_counts_it_and_replays_what_is_pending() {
        let mut prediction = armed(16);
        run(&mut prediction, 3);
        let outcome = prediction.on_authority(&Snapshot::unpublished(Tick(1)), Some(InputCommandNumber(1)));
        assert!(outcome.took_host_state && outcome.receipt_confirmed);
        assert_eq!(outcome.rerun_move_total, 2);
        assert_eq!(prediction.metrics().rerun_move_total, 2);
        assert_eq!(prediction.metrics().frames_seen, 1);
        assert!(!prediction.self_present_in_prior_state());
    }
}
