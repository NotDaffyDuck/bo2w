//! What the client sends to its host each frame, and how often.

use bevy::prelude::*;
use playerstate_t6::game::InputCommand;
use sim::ShotRecordOrigin;

use super::outbox::QueuedOutgoingPlayerMessages;
use super::stores::{ReceiverStageLog, PlayerGuaranteedReceipt, NewestTakenSnapshot, note_phase};
use crate::host_tick::peer_mail::{PlayerActionIntake, PlayerMoveIntake};
use crate::client::run_ahead::InputCommandNumber;
use crate::client::presentation::shown_frame::HostViewClient;
use crate::client::realtime::ClientRealtime;
use crate::net_links::relay::PlayerUdpSession;

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
    newest_sent: Option<InputCommandNumber>,
    /// The last old command repeated, so the next packet picks up after it.
    repeat_cursor: Option<InputCommandNumber>,
    ack_sent: u16,
}

impl UplinkPacer {
    pub(super) fn packet_due(&self, clock_millis: i32, newest: Option<InputCommandNumber>) -> bool {
        newest.is_some()
            && self
                .sent_at_ms
                .is_none_or(|at| clock_millis.saturating_sub(at) >= PACKET_GAP_MS)
    }

    pub(super) fn note_sent(&mut self, clock_millis: i32, newest: Option<InputCommandNumber>, confirm_receipt: u16) {
        self.sent_at_ms = Some(clock_millis);
        self.newest_sent = newest;
        self.ack_sent = confirm_receipt;
    }

    /// The commands for the next packet in sequence order, and where the rotating repeat has
    /// got to. Everything not yet sent goes; of what was sent before, the oldest always goes
    /// and a few more follow in turn.
    pub(super) fn next_packet(&self, unsent: &QueuedOutgoingPlayerMessages) -> (Vec<(InputCommandNumber, InputCommand, ShotRecordOrigin)>, Option<InputCommandNumber>) {
        let (mut packet, sent_before): (Vec<(InputCommandNumber, InputCommand, ShotRecordOrigin)>, Vec<(InputCommandNumber, InputCommand, ShotRecordOrigin)>) = unsent
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
pub fn settle_setup_applied(
    link: Option<ResMut<PlayerUdpSession>>,
    adopted: Res<NewestTakenSnapshot>,
    defer: Option<Res<crate::HoldTransferInstalled>>,
) {
    let held_back = defer.is_some_and(|defer| defer.0);
    if held_back || !adopted.taken_in_current_step {
        return;
    }
    if let Some(mut session) = link { session.emit_installed_notices(); }
}

pub fn transmit_queued_inputs(
    udp: Option<ResMut<PlayerUdpSession>>,
    mut inbox: ResMut<PlayerMoveIntake>,
    mut action_inbox: Option<ResMut<PlayerActionIntake>>,
    guaranteed_receipt: Res<PlayerGuaranteedReceipt>,
    mut pacer: ResMut<UplinkPacer>,
    cls: Res<ClientRealtime>,
    unsent: Res<QueuedOutgoingPlayerMessages>,
    local: Res<HostViewClient>,
    trace: Option<ResMut<ReceiverStageLog>>,
) {
    note_phase(trace, "Send");
    let (now, newest_seq, confirm_receipt) = (cls.now_ms(), unsent.newest_seq(), guaranteed_receipt.0);
    let me = local.0;

    let Some(mut udp) = udp else {
        // Without a network the commands go straight into the host's inbox.
        if pacer.packet_due(now, newest_seq) {
            pacer.note_sent(now, newest_seq, confirm_receipt);
            unsent.iter().for_each(|&(seq, command, sample)| inbox.push(me, Some(seq), command, Some(sample)));
        }
        return;
    };
    if udp.connection.is_none() || !udp.is_inside_match() {
        return;
    }

    let actions = action_inbox.as_deref().map(|intake| intake.awaiting_from(me).collect::<Vec<_>>()).unwrap_or_default();
    // Commands go when the packet gap is up; a packet also goes early for a new ack or new actions.
    let commands_due = pacer.packet_due(now, newest_seq);
    let control_due = confirm_receipt != pacer.ack_sent || udp.actions_still_owed(&actions);
    if !(commands_due || control_due) {
        return;
    }

    let (packet, cursor) = if commands_due { pacer.next_packet(&unsent) } else { Default::default() };
    if let Err(e) = udp.transmit_inputs(&packet, &actions, confirm_receipt) {
        diag::warn!(WireLink, "udp send cmds: {e}");
        return;
    }
    if let Some(intake) = action_inbox.as_deref_mut() {
        actions.iter().for_each(|action| intake.begin_clock(me, sim::intent_serial(action)));
    }
    if !commands_due {
        pacer.ack_sent = confirm_receipt;
        return;
    }
    pacer.note_sent(now, newest_seq, confirm_receipt);
    pacer.repeat_cursor = cursor;
}

#[cfg(test)]
mod tests {
    use sim::ClientId;

    use super::*;

    fn queue(seqs: impl IntoIterator<Item = u32>) -> QueuedOutgoingPlayerMessages {
        let mut sends = QueuedOutgoingPlayerMessages::default();
        for n in seqs {
            let cmd = InputCommand { authority_clock_ms: n as i32 * 10, ..InputCommand::default() };
            sends.enqueue(InputCommandNumber(n), cmd, Default::default());
        }
        sends
    }

    fn seqs(packet: &[(InputCommandNumber, InputCommand, ShotRecordOrigin)]) -> Vec<u32> {
        packet.iter().map(|row| row.0.0).collect()
    }

    fn pacer_after(newest: u32, cursor: Option<u32>) -> UplinkPacer {
        UplinkPacer {
            newest_sent: Some(InputCommandNumber(newest)),
            repeat_cursor: cursor.map(InputCommandNumber),
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
        assert!(pacer.packet_due(0, Some(InputCommandNumber(1))));
    }

    #[test]
    fn the_next_packet_waits_out_the_gap() {
        let mut pacer = UplinkPacer::default();
        pacer.note_sent(100, Some(InputCommandNumber(1)), 0);
        assert!(!pacer.packet_due(100, Some(InputCommandNumber(2))));
        assert!(!pacer.packet_due(116, Some(InputCommandNumber(2))));
        assert!(pacer.packet_due(117, Some(InputCommandNumber(2))));
    }

    #[test]
    fn a_clock_that_ran_backwards_does_not_overflow() {
        let mut pacer = UplinkPacer::default();
        pacer.note_sent(i32::MAX, Some(InputCommandNumber(1)), 0);
        assert!(!pacer.packet_due(i32::MIN, Some(InputCommandNumber(2))));
    }

    #[test]
    fn noting_a_send_records_the_newest_command_and_the_ack() {
        let mut pacer = UplinkPacer::default();
        pacer.note_sent(5, Some(InputCommandNumber(9)), 3);
        assert_eq!(pacer.newest_sent, Some(InputCommandNumber(9)));
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
        assert_eq!(cursor, Some(InputCommandNumber(3)));
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
        assert_eq!(cursor, Some(InputCommandNumber(2)));
    }

    #[test]
    fn only_three_old_commands_are_repeated_besides_the_oldest() {
        let unsent = queue(1..=10);
        let (packet, cursor) = pacer_after(9, None).next_packet(&unsent);
        assert_eq!(seqs(&packet), [1, 2, 3, 4, 10]);
        assert_eq!(cursor, Some(InputCommandNumber(4)));
    }

    #[test]
    fn the_repeats_carry_on_where_the_last_packet_stopped_and_wrap_around() {
        let unsent = queue(1..=10);
        let mut pacer = pacer_after(9, None);
        let mut cursor = None;
        let mut repeated = Vec::new();
        for _round in 1..=4 {
            pacer.repeat_cursor = cursor;
            let (packet, carried_on) = pacer.next_packet(&unsent);
            // Leave out the oldest (1) and newest (10), which every packet carries.
            repeated.push(seqs(&packet).into_iter().filter(|n| ![1, 10].contains(n)).collect::<Vec<_>>());
            cursor = carried_on;
        }
        assert_eq!(repeated, [[2, 3, 4], [5, 6, 7], [2, 8, 9], [3, 4, 5]]);
    }

    #[test]
    fn a_cursor_past_every_old_command_starts_again_from_the_front() {
        let unsent = queue(1..=6);
        let (packet, cursor) = pacer_after(5, Some(99)).next_packet(&unsent);
        assert_eq!(seqs(&packet), [1, 2, 3, 4, 6]);
        assert_eq!(cursor, Some(InputCommandNumber(4)));
    }

    fn offline_app() -> App {
        let mut offline = App::new();
        offline.init_resource::<PlayerMoveIntake>()
            .init_resource::<PlayerGuaranteedReceipt>()
            .init_resource::<UplinkPacer>()
            .init_resource::<ClientRealtime>()
            .init_resource::<QueuedOutgoingPlayerMessages>()
            .insert_resource(HostViewClient(ClientId(1)))
            .add_systems(Update, transmit_queued_inputs);
        offline
    }

    fn queued_for_host(app: &App) -> usize {
        app.world().resource::<PlayerMoveIntake>().queued_count(ClientId(1))
    }

    #[test]
    fn without_a_network_the_commands_go_straight_to_the_host_inbox() {
        let mut app = offline_app();
        app.update();
        assert_eq!(queued_for_host(&app), 0);

        let cmd = InputCommand::default();
        for n in 1..=2 {
            let mut queued = app.world_mut().resource_mut::<QueuedOutgoingPlayerMessages>();
            queued.enqueue(InputCommandNumber(n), cmd, Default::default());
        }
        app.update();
        assert_eq!(queued_for_host(&app), 2);
    }

    #[test]
    fn offline_commands_wait_for_the_packet_gap_like_online_ones() {
        let mut app = offline_app();
        let push = |app: &mut App, n| {
            let mut queued = app.world_mut().resource_mut::<QueuedOutgoingPlayerMessages>();
            queued.enqueue(InputCommandNumber(n), InputCommand::default(), Default::default());
        };
        push(&mut app, 1);
        app.update();
        push(&mut app, 2);
        // The clock has not moved, so the second command is not due yet.
        app.update();
        assert_eq!(queued_for_host(&app), 1);

        // The first advance only starts the clock.
        let mut clock = app.world_mut().resource_mut::<ClientRealtime>();
        clock.advance_host_clock(0.0);
        clock.advance_host_clock(0.05);
        app.update();
        assert_eq!(queued_for_host(&app), 2);
    }

    #[test]
    fn the_send_phase_is_traced() {
        let mut app = offline_app();
        app.insert_resource(ReceiverStageLog::default());
        app.update();
        assert_eq!(app.world().resource::<ReceiverStageLog>().0, ["Send"]);
    }
}
