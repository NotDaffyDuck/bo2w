//! What the client sends to its host each frame, and how often.

use bevy::prelude::*;
use playerstate_t6::game::UserCmd;
use sim::ShotSampleProvenance;

use super::outbox::PendingClientSends;
use super::stores::{ClientPhaseTrace, ClientReliableAck, LastAdoptedSnapshot, note_phase};
use crate::authority::inbox::{ClientActionInbox, ClientCommandInbox};
use crate::client::predict::CmdSeq;
use crate::client::presentation::presented::LocalPresentClient;
use crate::client::realtime::ClientRealtime;
use crate::transport::relay::UdpClientLink;

type Row = (CmdSeq, UserCmd, ShotSampleProvenance);

/// Command packets leave at most this many times a second.
const PACKETS_PER_SECOND: i32 = 60;
const PACKET_GAP_MS: i32 = (1000 + PACKETS_PER_SECOND - 1) / PACKETS_PER_SECOND;

/// Besides the oldest command the host has not acknowledged, each packet repeats this many more
/// of the old ones, taken in turn. Repeating all of them every time floods a slow link with
/// commands that are already on their way.
const ROTATING_REPEATS: usize = 3;

/// When the last command packet left, and what it carried.
#[derive(Resource, Debug, Default)]
pub struct UplinkPacer {
    sent_at_ms: Option<i32>,
    newest_sent: Option<CmdSeq>,
    /// The last old command repeated, so the next packet picks up after it.
    repeat_cursor: Option<CmdSeq>,
    ack_sent: u16,
}

impl UplinkPacer {
    pub(super) fn packet_due(&self, now_ms: i32, newest: Option<CmdSeq>) -> bool {
        newest.is_some()
            && self
                .sent_at_ms
                .is_none_or(|at| now_ms.saturating_sub(at) >= PACKET_GAP_MS)
    }

    pub(super) fn note_sent(&mut self, now_ms: i32, newest: Option<CmdSeq>, ack: u16) {
        self.sent_at_ms = Some(now_ms);
        self.newest_sent = newest;
        self.ack_sent = ack;
    }

    /// The commands for the next packet in sequence order, and where the rotating repeat has
    /// got to. Everything not yet sent goes; of what was sent before, the oldest always goes
    /// and a few more follow in turn.
    pub(super) fn next_packet(&self, pending: &PendingClientSends) -> (Vec<Row>, Option<CmdSeq>) {
        let (mut packet, sent_before): (Vec<Row>, Vec<Row>) = pending
            .iter()
            .copied()
            .partition(|row| self.newest_sent.is_none_or(|newest| row.0 > newest));

        let mut cursor = self.repeat_cursor;
        if let Some((oldest, rest)) = sent_before.split_first() {
            packet.push(*oldest);
            let begin = rest
                .iter()
                .position(|row| self.repeat_cursor.is_none_or(|last| row.0 > last))
                .unwrap_or(0);
            for row in rest
                .iter()
                .cycle()
                .skip(begin)
                .take(rest.len().min(ROTATING_REPEATS))
            {
                packet.push(*row);
                cursor = Some(row.0);
            }
        }
        packet.sort_by_key(|row| row.0);
        (packet, cursor)
    }
}

/// After a snapshot has been applied the host is told so, unless the match start is holding
/// that back.
pub fn flush_bootstrap_applied(
    link: Option<ResMut<UdpClientLink>>,
    adopted: Res<LastAdoptedSnapshot>,
    defer: Option<Res<crate::DeferBootstrapApplied>>,
) {
    let held_back = defer.is_some_and(|defer| defer.0);
    if held_back || !adopted.applied_this_frame {
        return;
    }
    if let Some(mut link) = link {
        link.flush_applied_after_adopt();
    }
}

pub fn send_pending_commands(
    link: Option<ResMut<UdpClientLink>>,
    mut inbox: ResMut<ClientCommandInbox>,
    mut action_inbox: Option<ResMut<ClientActionInbox>>,
    reliable_ack: Res<ClientReliableAck>,
    mut pacer: ResMut<UplinkPacer>,
    cls: Res<ClientRealtime>,
    pending: Res<PendingClientSends>,
    local: Res<LocalPresentClient>,
    trace: Option<ResMut<ClientPhaseTrace>>,
) {
    note_phase(trace, "Send");
    let now = cls.realtime();
    let newest = pending.newest_seq();
    let ack = reliable_ack.0;

    let Some(mut link) = link else {
        // Without a network the commands go straight into the host's inbox.
        if pacer.packet_due(now, newest) {
            pacer.note_sent(now, newest, ack);
            for &(seq, cmd, sample) in pending.iter() {
                inbox.push(local.0, Some(seq), cmd, Some(sample));
            }
        }
        return;
    };
    if link.connection.is_none() || !link.has_entered_match() {
        return;
    }

    let actions: Vec<_> = action_inbox
        .as_deref()
        .map(|inbox| inbox.pending_for(local.0).collect())
        .unwrap_or_default();
    let commands_due = pacer.packet_due(now, newest);
    let control_due = ack != pacer.ack_sent || link.has_unsent_actions(&actions);
    if !commands_due && !control_due {
        return;
    }

    let (packet, cursor) = if commands_due {
        pacer.next_packet(&pending)
    } else {
        (Vec::new(), None)
    };
    if let Err(e) = link.send_commands(&packet, &actions, ack) {
        diag::warn!(Net, "udp send cmds: {e}");
        return;
    }
    if let Some(inbox) = action_inbox.as_deref_mut() {
        for action in &actions {
            inbox.mark_dispatched(local.0, sim::action_request_id(action));
        }
    }
    if commands_due {
        pacer.note_sent(now, newest, ack);
        pacer.repeat_cursor = cursor;
    } else {
        pacer.ack_sent = ack;
    }
}

#[cfg(test)]
mod tests {
    use sim::ClientId;

    use super::*;

    fn queue(seqs: impl IntoIterator<Item = u32>) -> PendingClientSends {
        let mut sends = PendingClientSends::default();
        for n in seqs {
            let cmd = UserCmd { server_time: n as i32 * 10, ..UserCmd::default() };
            sends.push(CmdSeq(n), cmd, Default::default());
        }
        sends
    }

    fn seqs(packet: &[Row]) -> Vec<u32> {
        packet.iter().map(|row| row.0.0).collect()
    }

    fn pacer_after(newest: u32, cursor: Option<u32>) -> UplinkPacer {
        UplinkPacer {
            newest_sent: Some(CmdSeq(newest)),
            repeat_cursor: cursor.map(CmdSeq),
            ..UplinkPacer::default()
        }
    }

    #[test]
    fn the_packet_gap_is_a_sixtieth_of_a_second_rounded_up() {
        assert_eq!(PACKET_GAP_MS, 17);
    }

    #[test]
    fn nothing_is_due_without_a_command_and_the_first_one_goes_at_once() {
        let pacer = UplinkPacer::default();
        assert!(!pacer.packet_due(1000, None));
        assert!(pacer.packet_due(0, Some(CmdSeq(1))));
    }

    #[test]
    fn the_next_packet_waits_out_the_gap() {
        let mut pacer = UplinkPacer::default();
        pacer.note_sent(100, Some(CmdSeq(1)), 0);
        assert!(!pacer.packet_due(100, Some(CmdSeq(2))));
        assert!(!pacer.packet_due(116, Some(CmdSeq(2))));
        assert!(pacer.packet_due(117, Some(CmdSeq(2))));
    }

    #[test]
    fn a_clock_that_ran_backwards_does_not_overflow() {
        let mut pacer = UplinkPacer::default();
        pacer.note_sent(i32::MAX, Some(CmdSeq(1)), 0);
        assert!(!pacer.packet_due(i32::MIN, Some(CmdSeq(2))));
    }

    #[test]
    fn noting_a_send_records_the_newest_command_and_the_ack() {
        let mut pacer = UplinkPacer::default();
        pacer.note_sent(5, Some(CmdSeq(9)), 3);
        assert_eq!(pacer.newest_sent, Some(CmdSeq(9)));
        assert_eq!(pacer.ack_sent, 3);
    }

    #[test]
    fn a_first_packet_carries_everything_queued() {
        let (packet, cursor) = UplinkPacer::default().next_packet(&queue(1..=3));
        assert_eq!(seqs(&packet), [1, 2, 3]);
        assert_eq!(cursor, None);
    }

    #[test]
    fn new_commands_travel_with_the_old_ones_still_unacknowledged() {
        let (packet, cursor) = pacer_after(3, None).next_packet(&queue(1..=4));
        assert_eq!(seqs(&packet), [1, 2, 3, 4]);
        assert_eq!(cursor, Some(CmdSeq(3)));
    }

    #[test]
    fn with_nothing_new_the_oldest_still_goes() {
        let (packet, _) = pacer_after(1, None).next_packet(&queue(1..=1));
        assert_eq!(seqs(&packet), [1]);
    }

    #[test]
    fn an_empty_queue_makes_an_empty_packet() {
        let (packet, cursor) = pacer_after(4, Some(2)).next_packet(&queue([]));
        assert!(packet.is_empty());
        assert_eq!(cursor, Some(CmdSeq(2)));
    }

    #[test]
    fn only_three_old_commands_are_repeated_besides_the_oldest() {
        let pending = queue(1..=10);
        let (packet, cursor) = pacer_after(9, None).next_packet(&pending);
        assert_eq!(seqs(&packet), [1, 2, 3, 4, 10]);
        assert_eq!(cursor, Some(CmdSeq(4)));
    }

    #[test]
    fn the_repeats_carry_on_where_the_last_packet_stopped_and_wrap_around() {
        let pending = queue(1..=10);
        let mut pacer = pacer_after(9, None);
        let mut cursor = None;
        let mut repeated = Vec::new();
        for _ in 0..4 {
            pacer.repeat_cursor = cursor;
            let (packet, next) = pacer.next_packet(&pending);
            repeated.push(seqs(&packet).into_iter().filter(|n| ![1, 10].contains(n)).collect::<Vec<_>>());
            cursor = next;
        }
        assert_eq!(repeated, [[2, 3, 4], [5, 6, 7], [2, 8, 9], [3, 4, 5]]);
    }

    #[test]
    fn a_cursor_past_every_old_command_starts_again_from_the_front() {
        let pending = queue(1..=6);
        let (packet, cursor) = pacer_after(5, Some(99)).next_packet(&pending);
        assert_eq!(seqs(&packet), [1, 2, 3, 4, 6]);
        assert_eq!(cursor, Some(CmdSeq(4)));
    }

    fn offline_app() -> App {
        let mut app = App::new();
        app.init_resource::<ClientCommandInbox>()
            .init_resource::<ClientReliableAck>()
            .init_resource::<UplinkPacer>()
            .init_resource::<ClientRealtime>()
            .init_resource::<PendingClientSends>()
            .insert_resource(LocalPresentClient(ClientId(1)))
            .add_systems(Update, send_pending_commands);
        app
    }

    fn queued_for_host(app: &App) -> usize {
        app.world().resource::<ClientCommandInbox>().pending(ClientId(1))
    }

    #[test]
    fn without_a_network_the_commands_go_straight_to_the_host_inbox() {
        let mut app = offline_app();
        app.update();
        assert_eq!(queued_for_host(&app), 0);

        let cmd = UserCmd::default();
        for n in 1..=2 {
            app.world_mut()
                .resource_mut::<PendingClientSends>()
                .push(CmdSeq(n), cmd, Default::default());
        }
        app.update();
        assert_eq!(queued_for_host(&app), 2);
    }

    #[test]
    fn offline_commands_wait_for_the_packet_gap_like_online_ones() {
        let mut app = offline_app();
        let push = |app: &mut App, n| {
            app.world_mut()
                .resource_mut::<PendingClientSends>()
                .push(CmdSeq(n), UserCmd::default(), Default::default());
        };
        push(&mut app, 1);
        app.update();
        push(&mut app, 2);
        // The clock has not moved, so the second command is not due yet.
        app.update();
        assert_eq!(queued_for_host(&app), 1);

        // The first advance only starts the clock.
        let mut clock = app.world_mut().resource_mut::<ClientRealtime>();
        clock.advance_listen(0.0);
        clock.advance_listen(0.05);
        app.update();
        assert_eq!(queued_for_host(&app), 2);
    }

    #[test]
    fn the_send_phase_is_traced() {
        let mut app = offline_app();
        app.insert_resource(ClientPhaseTrace::default());
        app.update();
        assert_eq!(app.world().resource::<ClientPhaseTrace>().0, ["Send"]);
    }
}
