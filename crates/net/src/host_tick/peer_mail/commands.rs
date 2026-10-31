//! Movement commands waiting for the authority, one queue per peer.
//!
//! A peer sends sequenced commands (numbered, repeated redundantly, to be played in order and
//! acknowledged) and may also leave unsequenced ones (only the newest matters). Each authority
//! frame takes whatever is runnable from every peer's queue, and reports what it acknowledged,
//! which peers it had to speak for, and which peers have fallen too far behind.

use bevy::prelude::Resource;
use playerstate_t6::game::InputCommand;
use sim::{ClientId, ShotRecordOrigin};
use std::collections::{HashMap, VecDeque};

use super::backlog::{
    COMMAND_TIME_STEP_MS, MoveQueueOverflow, MAX_COMMAND_STALL_MS, PEER_INPUT_CAP_PER_STEP,
    MOVE_BACKLOG_SPAN_LIMIT, PEER_MOVE_BACKLOG_LIMIT, command_waits,
};
use crate::client::run_ahead::InputCommandNumber;

/// What one authority frame took from the inbox.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CollectedMoves {
    /// The commands to simulate, in the order they were consumed.
    pub to_simulate: Vec<(ClientId, InputCommand)>,

    /// The highest sequence number consumed (or, for an idle peer, already consumed) per peer.
    pub receipts_batch: Vec<(ClientId, InputCommandNumber)>,

    /// Peers whose last command was replayed because nothing new arrived.
    pub replayed: Vec<ClientId>,

    /// The shot-sample claim that came with each consumed command, with its server time.
    pub shot_claims: Vec<(ClientId, i32, ShotRecordOrigin)>,

    pub overflow_incidents: Vec<(ClientId, MoveQueueOverflow)>,
}

#[derive(Debug)]
struct Queued {
    seq: Option<InputCommandNumber>,
    cmd: InputCommand,
    sample: ShotRecordOrigin,
}

/// Everything remembered about one peer's input.
#[derive(Debug, Default)]
struct PeerInput {
    queue: VecDeque<Queued>,
    last_consumed: Option<InputCommand>,
    last_acked: Option<u32>,
    /// The sequence number at the head of the queue, and since when it has been there.
    stuck_head: Option<(InputCommandNumber, i32)>,
}

impl PeerInput {
    fn sequenced(&self) -> impl Iterator<Item = &Queued> {
        self.queue.iter().filter(|row| row.seq.is_some())
    }

    /// The next sequence number the authority is waiting for.
    fn next_expected(&self) -> u32 {
        self.last_acked.unwrap_or(0).wrapping_add(1)
    }

    fn already_seen(&self, seq: InputCommandNumber) -> bool {
        self.last_acked.is_some_and(|receipt_confirmed| seq.0 <= receipt_confirmed)
            || self.queue.iter().any(|row| row.seq == Some(seq))
    }

    fn accept(&mut self, row: Queued) {
        // An overfull queue drops what arrives; the backlog check will fault the peer.
        if self.queue.len() > PEER_MOVE_BACKLOG_LIMIT {
            return;
        }
        match row.seq {
            Some(number) => {
                let later = self.queue.iter().position(|other| other.seq.is_some_and(|other| other > number));
                let at = later.unwrap_or(self.queue.len());
                self.queue.insert(at, row);
            }
            None => self.queue.push_back(row),
        }
    }

    fn backlog_fault(&mut self, clock_millis: i32) -> Option<MoveQueueOverflow> {
        let ordered = self.sequenced().count();
        if ordered == 0 {
            return None;
        }
        if ordered > PEER_MOVE_BACKLOG_LIMIT {
            return Some(MoveQueueOverflow::BacklogLength { queued: ordered });
        }

        // Command time already queued: each command is counted from the one before it, starting
        // from the last one played (or the oldest queued one if nothing has played yet).
        let mut before = self.last_consumed.map(|cmd| cmd.authority_clock_ms).or_else(|| {
            self.sequenced()
                .next()
                .map(|row| clock_millis.min(row.cmd.authority_clock_ms))
        });
        let mut buffered_span_millis: i32 = 0;
        for row in self.sequenced() {
            if let Some(before) = before {
                let gap = row.cmd.authority_clock_ms.saturating_sub(before);
                buffered_span_millis = buffered_span_millis.saturating_add(gap.clamp(0, COMMAND_TIME_STEP_MS));
            }
            before = Some(row.cmd.authority_clock_ms);
        }
        if buffered_span_millis > MOVE_BACKLOG_SPAN_LIMIT {
            return Some(MoveQueueOverflow::QueueDuration { buffered_span_millis });
        }

        let head = self.sequenced().next()?.seq?;
        let since = match self.stuck_head {
            Some((stuck, since)) if stuck == head => since,
            _ => {
                self.stuck_head = Some((head, clock_millis));
                clock_millis
            }
        };
        let stalled_ms = clock_millis.saturating_sub(since);
        if stalled_ms <= MAX_COMMAND_STALL_MS {
            return None;
        }
        let head_time = self.sequenced().next().map(|row| row.cmd.authority_clock_ms);
        diag::warn!(
            WireLink,
            "input backlog: head seq {} (expected {}), head time {head_time:?} vs authority {clock_millis}",
            head.0,
            self.next_expected()
        );
        Some(MoveQueueOverflow::NoProgress { stalled_ms })
    }

    /// Remove and return the commands that may run in a frame at `clock_millis`: sequenced ones that
    /// continue the acknowledged run and are not from the future, and any unsequenced ones
    /// between them, up to the per-frame budget.
    fn take_runnable(&mut self, clock_millis: i32) -> Vec<Queued> {
        let mut want = self.next_expected();
        let mut runnable = 0;
        for row in &self.queue {
            if runnable == PEER_INPUT_CAP_PER_STEP {
                break;
            }
            if let Some(number) = row.seq {
                if number.0 != want || command_waits(row.cmd.authority_clock_ms, clock_millis) {
                    break;
                }
                want = want.wrapping_add(1);
            }
            runnable += 1;
        }
        let mut taken: Vec<Queued> = self.queue.drain(..runnable).collect();
        // Of a burst of unsequenced commands only the newest is worth playing.
        if taken.len() > 1 && taken.iter().all(|row| row.seq.is_none()) {
            taken.drain(..taken.len() - 1);
        }
        taken.sort_by_key(|row| row.seq);
        taken
    }

    /// Serve this peer for one frame at `clock_millis`, adding to `out`.
    fn serve(&mut self, who: ClientId, clock_millis: i32, out: &mut CollectedMoves) {
        // A burst of datagrams can hold more than one frame of runnable work, so a queue that is
        // merely long in time is still served (and measured again afterwards). The other faults
        // stop the peer before anything is simulated.
        if let Some(fault) = self.backlog_fault(clock_millis)
            && !matches!(fault, MoveQueueOverflow::QueueDuration { .. })
        {
            out.overflow_incidents.push((who, fault));
            return;
        }

        let taken = self.take_runnable(clock_millis);
        if taken.is_empty() {
            self.serve_idle(who, clock_millis, out);
            return;
        }

        self.stuck_head = None;
        let mut newest_seq = None;
        for Queued { seq, mut cmd, sample } in taken {
            cmd.authority_clock_ms = if seq.is_some() {
                // bo2zm: a command stamped past the window runs at the host's time.
                cmd.authority_clock_ms.min(clock_millis)
            } else {
                clock_millis
            };
            self.last_consumed = Some(cmd);
            if let Some(number) = seq {
                self.last_acked = Some(number.0);
                newest_seq = Some(number);
            }
            out.shot_claims.push((who, cmd.authority_clock_ms, sample));
            out.to_simulate.push((who, cmd));
        }
        if let Some(seq) = newest_seq {
            out.receipts_batch.push((who, seq));
        }
        if let Some(fault) = self.backlog_fault(clock_millis) {
            out.overflow_incidents.push((who, fault));
        }
    }

    /// Nothing was runnable: report a fault, else repeat the acknowledgement, else speak for a
    /// peer that only ever sent unsequenced commands by replaying its last one.
    fn serve_idle(&mut self, who: ClientId, clock_millis: i32, out: &mut CollectedMoves) {
        if let Some(fault) = self.backlog_fault(clock_millis) {
            out.overflow_incidents.push((who, fault));
        } else if let Some(receipt_confirmed) = self.last_acked {
            out.receipts_batch.push((who, InputCommandNumber(receipt_confirmed)));
        } else if let Some(cmd) = self.last_consumed.as_mut() {
            cmd.authority_clock_ms = clock_millis;
            out.replayed.push(who);
            out.to_simulate.push((who, *cmd));
        }
    }
}

#[derive(Resource, Debug, Default)]
pub struct PlayerMoveIntake {
    peers: HashMap<ClientId, PeerInput>,
}

impl PlayerMoveIntake {
    /// Queue a command from `pid`. A sequenced command that was already played or is already
    /// queued is dropped; a queue that is full drops everything.
    pub fn push(
        &mut self,
        pid: ClientId,
        seq: Option<InputCommandNumber>,
        cmd: InputCommand,
        sample: Option<ShotRecordOrigin>,
    ) {
        if let Some(numbered) = seq
            && self.peers.get(&pid).is_some_and(|peer| peer.already_seen(numbered))
        {
            return;
        }
        self.peers.entry(pid).or_default().accept(Queued {
            seq,
            cmd,
            sample: sample.unwrap_or(ShotRecordOrigin::UNCLAIMED),
        });
    }

    pub fn clear(&mut self) {
        self.peers = HashMap::new();
    }

    pub fn drop_player_record(&mut self, pid: ClientId) {
        self.peers.remove(&pid);
    }

    pub fn queued_count(&self, pid: ClientId) -> usize {
        self.peers.get(&pid).map_or(0, |peer| peer.queue.len())
    }

    pub fn queued_depth(&self, pid: ClientId) -> usize {
        self.queued_count(pid)
    }

    /// Has this peer queued too much, or been stuck behind a missing command for too long?
    pub fn backlog_fault(&mut self, pid: ClientId, clock_stamp_ms: i32) -> Option<MoveQueueOverflow> {
        self.peers.get_mut(&pid)?.backlog_fault(clock_stamp_ms)
    }

    pub fn acked_through(&self, pid: ClientId) -> Option<InputCommandNumber> {
        self.peers.get(&pid)?.last_acked.map(InputCommandNumber)
    }

    pub fn latest_played(&self, pid: ClientId) -> Option<InputCommand> {
        self.peers.get(&pid)?.last_consumed
    }

    /// Take what every peer can run in the authority frame at `clock_stamp_ms`, peers in pid order.
    pub fn take_for_step(&mut self, clock_stamp_ms: i32) -> CollectedMoves {
        let mut order: Vec<ClientId> = self.peers.keys().copied().collect();
        order.sort_by_key(|client| client.0);
        let mut out = CollectedMoves::default();
        for pid in order {
            if let Some(peer) = self.peers.get_mut(&pid) {
                peer.serve(pid, clock_stamp_ms, &mut out);
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const A: ClientId = ClientId(1);
    const B: ClientId = ClientId(2);

    fn cmd_at(authority_clock_ms: i32) -> InputCommand {
        InputCommand {
            authority_clock_ms,
            ..InputCommand::default()
        }
    }

    fn push_seq(queue: &mut PlayerMoveIntake, pid: ClientId, seq: u32, authority_clock_ms: i32) {
        queue.push(pid, Some(InputCommandNumber(seq)), cmd_at(authority_clock_ms), None);
    }

    fn times(gathered: &CollectedMoves) -> Vec<i32> {
        gathered.to_simulate.iter().map(|(_, cmd)| cmd.authority_clock_ms).collect()
    }

    #[test]
    fn sequenced_commands_run_in_order_and_are_acknowledged_once() {
        let mut queue = PlayerMoveIntake::default();
        push_seq(&mut queue, A, 1, 10);
        push_seq(&mut queue, A, 2, 20);
        push_seq(&mut queue, A, 3, 30);
        let out = queue.take_for_step(100);
        assert_eq!(times(&out), vec![10, 20, 30]);
        assert_eq!(out.receipts_batch, vec![(A, InputCommandNumber(3))]);
        assert!(out.replayed.is_empty() && out.overflow_incidents.is_empty());
        assert_eq!(queue.acked_through(A), Some(InputCommandNumber(3)));
        assert_eq!(queue.queued_count(A), 0);
    }

    #[test]
    fn commands_arriving_out_of_order_are_sorted_into_place() {
        let mut queue = PlayerMoveIntake::default();
        push_seq(&mut queue, A, 2, 20);
        push_seq(&mut queue, A, 1, 10);
        assert_eq!(times(&queue.take_for_step(100)), vec![10, 20]);
    }

    #[test]
    fn a_gap_in_the_sequence_holds_the_rest_back() {
        let mut queue = PlayerMoveIntake::default();
        push_seq(&mut queue, A, 1, 10);
        push_seq(&mut queue, A, 3, 30);
        let out = queue.take_for_step(100);
        assert_eq!(times(&out), vec![10]);
        assert_eq!(queue.queued_count(A), 1);
        push_seq(&mut queue, A, 2, 20);
        assert_eq!(times(&queue.take_for_step(100)), vec![20, 30]);
    }

    #[test]
    fn a_command_from_the_future_waits_for_its_frame() {
        let mut queue = PlayerMoveIntake::default();
        push_seq(&mut queue, A, 1, 150);
        assert!(queue.take_for_step(100).to_simulate.is_empty());
        assert_eq!(times(&queue.take_for_step(150)), vec![150]);
    }

    #[test]
    fn a_command_stamped_past_the_window_runs_now_at_the_hosts_time() {
        let mut queue = PlayerMoveIntake::default();
        push_seq(&mut queue, A, 1, 100 + super::super::backlog::MAX_COMMAND_AHEAD_MS + 1);
        assert_eq!(times(&queue.take_for_step(100)), vec![100]);
    }

    #[test]
    fn a_resent_command_is_dropped_whether_queued_or_already_played() {
        let mut queue = PlayerMoveIntake::default();
        push_seq(&mut queue, A, 1, 10);
        push_seq(&mut queue, A, 1, 10);
        assert_eq!(queue.queued_count(A), 1);
        queue.take_for_step(100);
        push_seq(&mut queue, A, 1, 10);
        assert_eq!(queue.queued_count(A), 0);
    }

    #[test]
    fn of_a_burst_of_unsequenced_commands_only_the_newest_runs_stamped_with_now() {
        let mut queue = PlayerMoveIntake::default();
        for t in [1, 2, 3] {
            queue.push(A, None, cmd_at(t), None);
        }
        let out = queue.take_for_step(500);
        assert_eq!(times(&out), vec![500]);
        assert!(out.receipts_batch.is_empty());
        assert_eq!(out.shot_claims.len(), 1);
    }

    #[test]
    fn an_idle_peer_gets_its_acknowledgement_repeated() {
        let mut queue = PlayerMoveIntake::default();
        push_seq(&mut queue, A, 1, 10);
        queue.take_for_step(100);
        let idle = queue.take_for_step(150);
        assert!(idle.to_simulate.is_empty());
        assert_eq!(idle.receipts_batch, vec![(A, InputCommandNumber(1))]);
    }

    #[test]
    fn a_peer_that_only_sent_unsequenced_commands_is_spoken_for() {
        let mut queue = PlayerMoveIntake::default();
        queue.push(A, None, cmd_at(1), None);
        queue.take_for_step(100);
        let idle = queue.take_for_step(150);
        assert_eq!(idle.replayed, vec![A]);
        assert_eq!(times(&idle), vec![150]);
        assert_eq!(queue.latest_played(A).map(|cmd| cmd.authority_clock_ms), Some(150));
    }

    #[test]
    fn peers_are_served_in_id_order() {
        let mut queue = PlayerMoveIntake::default();
        push_seq(&mut queue, B, 1, 10);
        push_seq(&mut queue, A, 1, 20);
        let ids: Vec<ClientId> = queue
            .take_for_step(100)
            .to_simulate
            .iter()
            .map(|(who, _)| *who)
            .collect();
        assert_eq!(ids, vec![A, B]);
    }

    #[test]
    fn the_per_frame_budget_leaves_the_rest_for_the_next_frame() {
        let mut queue = PlayerMoveIntake::default();
        let total = PEER_INPUT_CAP_PER_STEP as u32 + 3;
        for seq in 1..=total {
            push_seq(&mut queue, A, seq, 0);
        }
        let first = queue.take_for_step(0);
        assert_eq!(first.to_simulate.len(), PEER_INPUT_CAP_PER_STEP);
        assert_eq!(queue.queued_count(A), 3);
        assert_eq!(queue.take_for_step(0).to_simulate.len(), 3);
    }

    #[test]
    fn too_many_queued_commands_fault_the_peer_instead_of_running() {
        let mut queue = PlayerMoveIntake::default();
        for seq in 1..=(PEER_MOVE_BACKLOG_LIMIT as u32 + 1) {
            push_seq(&mut queue, A, seq, 0);
        }
        // The queue is now over its limit and takes nothing more.
        push_seq(&mut queue, A, 9_999, 0);
        assert_eq!(queue.queued_count(A), PEER_MOVE_BACKLOG_LIMIT + 1);
        let out = queue.take_for_step(0);
        assert!(out.to_simulate.is_empty());
        assert_eq!(
            out.overflow_incidents,
            vec![(
                A,
                MoveQueueOverflow::BacklogLength {
                    queued: PEER_MOVE_BACKLOG_LIMIT + 1
                }
            )]
        );
    }

    #[test]
    fn a_queue_stuck_behind_a_missing_command_faults_after_the_stall_limit() {
        let mut queue = PlayerMoveIntake::default();
        push_seq(&mut queue, A, 2, 0);
        assert_eq!(queue.backlog_fault(A, 0), None);
        assert_eq!(queue.backlog_fault(A, MAX_COMMAND_STALL_MS), None);
        assert_eq!(
            queue.backlog_fault(A, MAX_COMMAND_STALL_MS + 1),
            Some(MoveQueueOverflow::NoProgress {
                stalled_ms: MAX_COMMAND_STALL_MS + 1
            })
        );
    }

    #[test]
    fn too_much_queued_command_time_is_a_duration_fault() {
        let mut queue = PlayerMoveIntake::default();
        for seq in 1..=8u32 {
            push_seq(&mut queue, A, seq, (seq as i32 - 1) * COMMAND_TIME_STEP_MS);
        }
        assert_eq!(
            queue.backlog_fault(A, 0),
            Some(MoveQueueOverflow::QueueDuration {
                buffered_span_millis: 7 * COMMAND_TIME_STEP_MS
            })
        );
    }

    #[test]
    fn retiring_a_peer_forgets_everything_about_it() {
        let mut queue = PlayerMoveIntake::default();
        push_seq(&mut queue, A, 1, 0);
        queue.take_for_step(0);
        push_seq(&mut queue, A, 2, 0);
        queue.drop_player_record(A);
        assert_eq!(queue.queued_count(A), 0);
        assert_eq!(queue.acked_through(A), None);
        assert!(queue.latest_played(A).is_none());
        assert_eq!(queue.take_for_step(0), CollectedMoves::default());
    }

    #[test]
    fn clearing_forgets_every_peer() {
        let mut queue = PlayerMoveIntake::default();
        push_seq(&mut queue, A, 1, 0);
        push_seq(&mut queue, B, 1, 0);
        queue.clear();
        assert_eq!(queue.take_for_step(0), CollectedMoves::default());
    }
}
