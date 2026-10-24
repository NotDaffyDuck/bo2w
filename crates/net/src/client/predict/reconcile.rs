//! Taking a snapshot from the authority back into the prediction: adopt its world, retire the
//! commands it acknowledged, replay the rest on top, and note how far the view had to move.

use playerstate_t6::game::{PlayerState, UserCmd, buttons, eflags, other_flags, pm_flags};
use sim::{Snapshot, StepReason, Tick};

use super::ground::{pmove_runs_for, snapshot_local};
use super::history::{AckMatch, CmdSeq, MoveRecord};
use super::metrics::ReconcileOutcome;
use super::state::ClientPrediction;
use crate::reconciliation::CorrectionBoundary;
use crate::transport::netfields::ps_deviation;

/// This many snapshots in a row without a usable acknowledgement and the history is dropped.
pub const MAX_UNACKED_SNAPSHOTS: u32 = 20;

/// Whether `BO2MP_PRED_LOG` asks for a log line per misprediction.
fn log_each_miss() -> bool {
    static ENABLED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ENABLED.get_or_init(|| std::env::var_os("BO2MP_PRED_LOG").is_some())
}

impl ClientPrediction {
    /// Drops the unacknowledged history so the next snapshot is adopted whole: the recovery
    /// `MAX_UNACKED_SNAPSHOTS` performs, on demand. The pending send window is left alone because
    /// the authority consumes a client's commands in sequence, and a hole in that window would
    /// wedge it for good.
    pub fn force_resync(&mut self) {
        self.snapshots_since_ack = 0;
        if self.history.is_empty() {
            return;
        }
        self.drop_history();
        self.metrics.forced_adopts += 1;
    }

    /// Empties the history but remembers its newest command as the floor, so stale acks for the
    /// dropped commands retire; read as broken they would drop every later window too.
    fn drop_history(&mut self) {
        if let Some(cmd) = self.last_cmd {
            self.replay_floor = Some((CmdSeq(self.next_seq.0.wrapping_sub(1)), cmd));
        }
        self.history.clear();
    }

    /// Applies an acknowledgement without a snapshot to adopt.
    pub fn retire_acks(&mut self, ack: Option<CmdSeq>) {
        if !self.armed {
            return;
        }
        self.metrics.snapshots += 1;
        let _ = self.apply_ack(ack);
    }

    /// Matches `ack` against the history. Returns the outcome flags and, on a match, the record
    /// of the acknowledged command.
    fn apply_ack(&mut self, ack: Option<CmdSeq>) -> (ReconcileOutcome, Option<MoveRecord>) {
        let mut outcome = ReconcileOutcome::default();
        let mut matched = None;

        let effective = ack.map(|seq| match self.replay_floor {
            Some((floor, _)) if seq.0 < floor.0 => floor,
            _ => seq,
        });

        match effective {
            Some(seq) => match self.history.clear_acknowledged(seq) {
                AckMatch::Matched(record) => {
                    self.acknowledged_cmd = Some(record.cmd);
                    outcome.acked = true;
                    self.metrics.acks_matched += 1;
                    self.snapshots_since_ack = 0;
                    matched = Some(record);
                }
                AckMatch::Retired => {
                    outcome.retired_ack = true;
                    self.metrics.retired_acks += 1;
                    self.snapshots_since_ack = 0;
                }
                AckMatch::Broken => {
                    outcome.forced_adopt = true;
                    self.metrics.forced_adopts += 1;
                    self.drop_history();
                }
            },
            None if self.history.is_empty() => self.snapshots_since_ack = 0,
            None => {
                self.snapshots_since_ack = self.snapshots_since_ack.saturating_add(1);
                if self.snapshots_since_ack > MAX_UNACKED_SNAPSHOTS {
                    outcome.forced_adopt = true;
                    self.metrics.forced_adopts += 1;
                    self.drop_history();
                    self.snapshots_since_ack = 0;
                }
            }
        }
        (outcome, matched)
    }

    /// Adopts `snapshot`, retires what `ack` covers, replays the commands still pending and
    /// updates the view-error tracking.
    pub fn on_authority(&mut self, snapshot: &Snapshot, ack: Option<CmdSeq>) -> ReconcileOutcome {
        if !self.armed {
            return ReconcileOutcome::default();
        }
        self.metrics.snapshots += 1;

        let before = self.predicted_local;
        let boundary = CorrectionBoundary::after(snapshot.tick);
        let report = self.world.adopt_prediction_snapshot(snapshot, self.local);
        let authoritative = snapshot_local(snapshot, self.local);

        let mut outcome = ReconcileOutcome {
            adopted: true,
            report,
            ..ReconcileOutcome::default()
        };

        if authoritative.is_some_and(|ps| !pmove_runs_for(&ps)) {
            self.follow_authority_alone(authoritative, boundary);
        } else {
            self.reconcile(&mut outcome, ack, authoritative, boundary);
        }

        self.track_view_error(before, authoritative);
        self.had_local_last_snap = authoritative.is_some();
        outcome
    }

    /// The authority has the player on the kill camera: nothing is predicted, so the history is
    /// dropped and the player simply is what the authority says.
    fn follow_authority_alone(
        &mut self,
        authoritative: Option<PlayerState>,
        boundary: CorrectionBoundary,
    ) {
        if let Some(cmd) = self.last_cmd {
            self.replay_floor = Some((CmdSeq(self.next_seq.0.saturating_sub(1)), cmd));
        }
        self.history.clear();
        self.predicted_local = authoritative;
        self.next_tick = boundary.replay_tick.0;
    }

    /// The usual path: retire acknowledged commands, compare the acknowledged one with what the
    /// authority did, and replay everything newer.
    fn reconcile(
        &mut self,
        outcome: &mut ReconcileOutcome,
        ack: Option<CmdSeq>,
        authoritative: Option<PlayerState>,
        boundary: CorrectionBoundary,
    ) {
        let (acked, source) = self.apply_ack(ack);
        outcome.acked = acked.acked;
        outcome.retired_ack = acked.retired_ack;
        outcome.forced_adopt = acked.forced_adopt;

        if let (Some(source), Some(authoritative)) = (&source, &authoritative) {
            outcome.deviation = ps_deviation(&source.output, authoritative);
            if let Some(deviation) = outcome.deviation {
                self.metrics.deviations += 1;
                self.metrics.last_deviation = Some((deviation.field, deviation.distance));
                if log_each_miss() {
                    let (ours, theirs) = (source.output.origin, authoritative.origin);
                    diag::info!(
                        Net,
                        "prediction miss: {} by {:.1}, predicted ({:.0} {:.0} {:.0}) server ({:.0} {:.0} {:.0})",
                        deviation.field,
                        deviation.distance,
                        ours[0],
                        ours[1],
                        ours[2],
                        theirs[0],
                        theirs[1],
                        theirs[2]
                    );
                }
            }
        }

        self.seed_old_cmd(source.as_ref());
        self.next_tick = boundary.replay_tick.0;
        outcome.replayed_moves = self.replay();
        self.metrics.replayed_moves += outcome.replayed_moves as u64;
        self.predicted_local = self.local_state().or(authoritative);
    }

    /// Runs every command still in the history again on the freshly adopted world, rewriting
    /// each record. Returns how many commands were pending.
    fn replay(&mut self) -> usize {
        let pending = self.history.len();
        let mut tick = self.next_tick;

        for index in 0..pending {
            let Some(cmd) = self.history.record_mut(index).map(|record| record.cmd) else {
                break;
            };
            let input = self.local_state().unwrap_or(PlayerState::ZERO);
            if cmd.server_time <= input.command_time {
                continue;
            }

            let msec = cmd.server_time.wrapping_sub(input.command_time);
            let output = self
                .step_predicted(Tick(tick), cmd, msec, StepReason::Replay)
                .unwrap_or(input);

            if let Some(record) = self.history.record_mut(index) {
                record.tick = Tick(tick);
                record.input = input;
                record.output = output;
            }
            tick = tick.wrapping_add(1);
        }

        self.next_tick = tick;
        pending
    }

    /// Tells the world what the command before the first pending one held, since the movement
    /// code reads the previous command's buttons, angles and stick.
    fn seed_old_cmd(&mut self, acked: Option<&MoveRecord>) {
        let before_history: Option<UserCmd> = self.replay_floor.and_then(|(floor, cmd)| {
            (self.history.oldest_seq() == Some(floor.next())).then_some(cmd)
        });
        let previous = acked
            .map(|record| record.cmd)
            .or(before_history)
            .or(self.acknowledged_cmd);

        let (held, angles, stick) = match previous {
            Some(cmd) => (cmd.buttons, cmd.angles, [cmd.forwardmove, cmd.rightmove]),
            None => {
                let sprinting = self
                    .world
                    .player(self.local)
                    .is_some_and(|ps| ps.pm_flags & pm_flags::SPRINTING != 0);
                let seeded = if sprinting { buttons::SPRINT } else { 0 };
                (seeded, [0; 3], [0; 2])
            }
        };
        self.world.set_old_cmd(self.local, held, angles);
        self.world.set_old_cmd_move(self.local, stick);
    }

    /// Updates the view-error tracker with how far the player moved between the prediction
    /// before this snapshot and the one after it.
    fn track_view_error(&mut self, before: Option<PlayerState>, authoritative: Option<PlayerState>) {
        let after = self.predicted_local;
        let before_origin = before.map(|ps| ps.origin);
        let after_origin = after.map(|ps| ps.origin);

        let watching_a_player = authoritative.is_some_and(|ps| {
            ps.other_flags & (other_flags::PLAYER | other_flags::DEAD_KILLCAM_TPV) != 0
        });
        let first_sight = watching_a_player && !self.had_local_last_snap;

        if first_sight {
            if let (Some(pre), Some(post)) = (before_origin, after_origin) {
                self.predicted_error.note_teleport_delta(pre, post);
            }
            self.predicted_error.reset_new_entity();
            return;
        }

        let (Some(pre), Some(post)) = (before_origin, after_origin) else {
            return;
        };
        let teleported = match (before, after) {
            (Some(a), Some(b)) => (a.e_flags ^ b.e_flags) & eflags::TELEPORT != 0,
            _ => false,
        };
        if teleported {
            self.predicted_error.note_teleport_delta(pre, post);
            self.predicted_error.reset();
        } else if before.zip(after).is_some_and(|(a, b)| a.command_time == b.command_time) {
            self.predicted_error.begin(pre, post);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ServerTime;
    use sim::{ClientId, SimWorld};

    fn armed(cap: usize) -> ClientPrediction {
        let mut prediction = ClientPrediction::with_history_cap(ClientId(1), cap);
        prediction.arm_from_content(&SimWorld::new());
        prediction
    }

    fn run(prediction: &mut ClientPrediction, commands: u32) {
        for n in 1..=commands {
            prediction
                .predict(UserCmd::default(), ServerTime::from_ms(n as i32 * 16))
                .expect("the history has room");
        }
    }

    #[test]
    fn a_disarmed_prediction_ignores_snapshots_and_acks() {
        let mut prediction = ClientPrediction::new(ClientId(1));
        let outcome = prediction.on_authority(&Snapshot::unpublished(Tick(1)), Some(CmdSeq(1)));
        assert_eq!(outcome, ReconcileOutcome::default());
        prediction.retire_acks(Some(CmdSeq(1)));
        assert_eq!(prediction.metrics().snapshots, 0);
    }

    #[test]
    fn an_ack_for_a_held_command_retires_everything_up_to_it() {
        let mut prediction = armed(16);
        run(&mut prediction, 4);
        prediction.retire_acks(Some(CmdSeq(2)));
        assert_eq!(prediction.history().oldest_seq(), Some(CmdSeq(3)));
        let metrics = prediction.metrics();
        assert_eq!((metrics.snapshots, metrics.acks_matched), (1, 1));
    }

    #[test]
    fn an_ack_from_the_future_drops_the_history_and_counts_a_forced_adopt() {
        let mut prediction = armed(16);
        run(&mut prediction, 3);
        prediction.retire_acks(Some(CmdSeq(40)));
        assert!(prediction.history().is_empty());
        assert_eq!(prediction.metrics().forced_adopts, 1);
    }

    #[test]
    fn late_acks_for_dropped_commands_retire_instead_of_breaking() {
        let mut prediction = armed(16);
        run(&mut prediction, 3);
        prediction.force_resync();
        run(&mut prediction, 2);
        // Seq 1 is below the floor (3), so it reads as the floor itself, not as a broken ack.
        prediction.retire_acks(Some(CmdSeq(1)));
        assert_eq!(prediction.metrics().forced_adopts, 1);
        assert_eq!(prediction.history().len(), 2);
    }

    #[test]
    fn forcing_a_resync_with_nothing_pending_changes_nothing() {
        let mut prediction = armed(16);
        prediction.force_resync();
        assert_eq!(prediction.metrics().forced_adopts, 0);
    }

    #[test]
    fn forcing_a_resync_empties_the_history_once() {
        let mut prediction = armed(16);
        run(&mut prediction, 2);
        prediction.force_resync();
        assert!(prediction.history().is_empty());
        assert_eq!(prediction.metrics().forced_adopts, 1);
    }

    #[test]
    fn too_many_snapshots_without_an_ack_drops_the_history() {
        let mut prediction = armed(16);
        run(&mut prediction, 2);
        for _ in 0..MAX_UNACKED_SNAPSHOTS {
            prediction.retire_acks(None);
            assert_eq!(prediction.history().len(), 2);
        }
        prediction.retire_acks(None);
        assert!(prediction.history().is_empty());
        assert_eq!(prediction.metrics().forced_adopts, 1);
    }

    #[test]
    fn snapshots_without_an_ack_are_free_while_nothing_is_pending() {
        let mut prediction = armed(16);
        for _ in 0..(MAX_UNACKED_SNAPSHOTS * 2) {
            prediction.retire_acks(None);
        }
        assert_eq!(prediction.metrics().forced_adopts, 0);
    }

    #[test]
    fn adopting_a_snapshot_counts_it_and_replays_what_is_pending() {
        let mut prediction = armed(16);
        run(&mut prediction, 3);
        let outcome = prediction.on_authority(&Snapshot::unpublished(Tick(1)), Some(CmdSeq(1)));
        assert!(outcome.adopted && outcome.acked);
        assert_eq!(outcome.replayed_moves, 2);
        assert_eq!(prediction.metrics().replayed_moves, 2);
        assert_eq!(prediction.metrics().snapshots, 1);
        assert!(!prediction.had_local_last_snap());
    }
}
