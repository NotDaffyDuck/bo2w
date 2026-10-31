//! What a client sends to the authority: snapshot acknowledgements, commands and actions.

use playerstate_t6::game::InputCommand;
use sim::{PlayerIntent, intent_serial};

use super::PlayerUdpSession;
use crate::client::run_ahead::InputCommandNumber;
use crate::net_links::datagrams::{PlayerToHostDatagram, LinkNumber, DatagramPreamble};
use crate::net_links::relay::mailbox::send_refused;
use crate::net_links::send_fault::UdpTransmitFailure;

/// The most commands one packet carries; the peer's own limit can lower it further.
const INPUTS_PER_DATAGRAM: usize = 16;

impl PlayerUdpSession {
    fn next_header(&mut self, link_id: LinkNumber) -> DatagramPreamble {
        self.round.last_sent = self.round.last_sent.wrapping_add(1);
        DatagramPreamble {
            connection: link_id,
            packet_no: self.round.last_sent,
            confirm_receipt: self.round.last_heard,
            round_era: self.game_generation(),
        }
    }

    pub(super) fn transmit_state_receipt(&mut self, snap_no: u32) -> Result<(), UdpTransmitFailure> {
        let Some(link_id) = self.connection else {
            return Ok(());
        };
        let receipt = PlayerToHostDatagram::FrameReceipt {
            header: self.next_header(link_id),
            frame_number_on_wire: snap_no,
        };
        self.send_to_authority(receipt.wire_bytes())
    }

    /// Whether any of `actions` has not been sent yet.
    pub fn actions_still_owed(&self, actions: &[PlayerIntent]) -> bool {
        actions
            .iter()
            .any(|action| !self.round.actions_sent.contains(&intent_serial(action)))
    }

    /// Sends the commands in packets of at most `INPUTS_PER_DATAGRAM`, then the actions not sent
    /// before. When there are no commands, the action packet goes out even if it carries no
    /// actions, so the authority keeps hearing from this client.
    pub fn transmit_inputs(
        &mut self,
        cmds: &[(InputCommandNumber, InputCommand, sim::ShotRecordOrigin)],
        actions: &[PlayerIntent],
        guaranteed_receipt: u16,
    ) -> Result<(), UdpTransmitFailure> {
        let Some(link_id) = self.connection else {
            return Ok(());
        };
        if let Some(newest) = self.round.ledger.newest() {
            let _ = self.transmit_state_receipt(newest);
        }

        let per_packet = INPUTS_PER_DATAGRAM.min(usize::from(self.negotiated.moves_budget_per_step));
        if per_packet == 0 {
            return Err(send_refused("peer permits no commands per packet"));
        }
        for chunk in cmds.chunks(per_packet) {
            self.transmit_input_datagram(link_id, chunk, &[], guaranteed_receipt)?;
        }

        // Actions the caller dropped are forgotten, so a later request with the same id
        // counts as new.
        self.round.actions_sent
            .retain(|id| actions.iter().any(|action| intent_serial(action) == *id));
        let fresh: Vec<PlayerIntent> = actions
            .iter()
            .filter(|action| !self.round.actions_sent.contains(&intent_serial(action)))
            .copied()
            .collect();
        if cmds.is_empty() || !fresh.is_empty() {
            self.transmit_input_datagram(link_id, &[], &fresh, guaranteed_receipt)?;
            self.round.actions_sent
                .extend(fresh.iter().map(intent_serial));
        }
        Ok(())
    }

    fn transmit_input_datagram(
        &mut self,
        link_id: LinkNumber,
        cmds: &[(InputCommandNumber, InputCommand, sim::ShotRecordOrigin)],
        actions: &[PlayerIntent],
        guaranteed_receipt: u16,
    ) -> Result<(), UdpTransmitFailure> {
        let commands = PlayerToHostDatagram::Commands {
            header: self.next_header(link_id),
            asserted_player_slot: self.granted_player_slot.map_or(0, |slot| slot.0),
            moves: cmds.iter().map(|&(num, cmd, _)| (num, cmd)).collect(),
            aim_origins: cmds.iter().map(|&(num, _, sample)| (num, sample)).collect(),
            actions: actions.iter().copied().collect(),
            guaranteed_receipt,
        };
        let wire = commands.wire_bytes();
        if cmds.is_empty() {
            self.mailbox
                .enqueue_control_outgoing(master_protocol::RoomOccupantId([0; 16]), wire)
                .map_err(send_refused)
        } else {
            self.send_to_authority(wire)
        }
    }
}

#[cfg(test)]
mod tests {
    use master_protocol::RoomOccupantId;
    use sim::ClientId;

    use super::*;
    use crate::net_links::datagrams::{JoinGreeting, parse_receiver_datagram};
    use crate::net_links::relay::mailbox::ForwardingInbox;

    fn joined_link() -> PlayerUdpSession {
        let mut link = PlayerUdpSession::relay(
            JoinGreeting::current(Default::default()),
            ForwardingInbox::standard_capacity(),
        );
        link.note_admission(LinkNumber(3), ClientId(5));
        link
    }

    fn cmd(num: u32) -> (InputCommandNumber, InputCommand, sim::ShotRecordOrigin) {
        (
            InputCommandNumber(num),
            InputCommand::default(),
            sim::ShotRecordOrigin::default(),
        )
    }

    fn decoded(link: &PlayerUdpSession, raw: &[u8]) -> PlayerToHostDatagram {
        parse_receiver_datagram(raw, &link.negotiated).expect("our own packet decodes")
    }

    fn command_counts(link: &PlayerUdpSession) -> Vec<usize> {
        link.mailbox()
            .take_outbound()
            .iter()
            .filter_map(|(_, buf)| match decoded(link, buf) {
                PlayerToHostDatagram::Commands { moves, .. } => Some(moves.len()),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn a_link_that_has_not_joined_sends_nothing() {
        let mut link = PlayerUdpSession::relay(
            JoinGreeting::current(Default::default()),
            ForwardingInbox::standard_capacity(),
        );
        link.transmit_inputs(&[cmd(1)], &[], 0).unwrap();
        link.transmit_state_receipt(4).unwrap();
        assert!(link.mailbox().take_outbound().is_empty());
        assert!(link.mailbox().take_control_outbound().is_empty());
    }

    #[test]
    fn commands_go_out_in_packets_of_at_most_sixteen() {
        let mut link = joined_link();
        let cmds: Vec<_> = (1..=40).map(cmd).collect();
        link.transmit_inputs(&cmds, &[], 0).unwrap();
        assert_eq!(command_counts(&link), vec![16, 16, 8]);
        assert!(
            link.mailbox().take_control_outbound().is_empty(),
            "commands were sent, so no empty packet follows"
        );
    }

    #[test]
    fn the_peers_limit_can_shrink_the_packets() {
        let mut link = joined_link();
        link.negotiated.moves_budget_per_step = 5;
        let cmds: Vec<_> = (1..=12).map(cmd).collect();
        link.transmit_inputs(&cmds, &[], 0).unwrap();
        assert_eq!(command_counts(&link), vec![5, 5, 2]);
    }

    #[test]
    fn a_peer_that_permits_no_commands_is_an_error() {
        let mut link = joined_link();
        link.negotiated.moves_budget_per_step = 0;
        assert!(link.transmit_inputs(&[cmd(1)], &[], 0).is_err());
        assert!(link.mailbox().take_outbound().is_empty());
    }

    #[test]
    fn with_no_commands_an_empty_packet_goes_on_the_control_lane() {
        let mut link = joined_link();
        link.transmit_inputs(&[], &[], 7).unwrap();
        assert!(link.mailbox().take_outbound().is_empty());
        let control = link.mailbox().take_control_outbound();
        assert_eq!(control.len(), 1);
        assert_eq!(control[0].0, RoomOccupantId([0; 16]));
        match decoded(&link, &control[0].1) {
            PlayerToHostDatagram::Commands {
                moves,
                actions,
                guaranteed_receipt,
                asserted_player_slot,
                ..
            } => {
                assert!(moves.is_empty() && actions.is_empty());
                assert_eq!(guaranteed_receipt, 7);
                assert_eq!(asserted_player_slot, 5);
            }
            other => panic!("expected commands, got {other:?}"),
        }
    }

    #[test]
    fn the_newest_applied_snapshot_is_acknowledged_before_the_commands() {
        let mut link = joined_link();
        link.note_snapshot_applied(9);
        link.note_snapshot_applied(7);
        link.transmit_inputs(&[cmd(1)], &[], 0).unwrap();
        let sent = link.mailbox().take_outbound();
        assert_eq!(sent.len(), 2);
        match decoded(&link, &sent[0].1) {
            PlayerToHostDatagram::FrameReceipt { frame_number_on_wire, .. } => assert_eq!(frame_number_on_wire, 9),
            other => panic!("expected an ack first, got {other:?}"),
        }
        assert!(matches!(
            decoded(&link, &sent[1].1),
            PlayerToHostDatagram::Commands { .. }
        ));
    }

    #[test]
    fn packet_sequence_numbers_count_up_and_carry_the_last_ack_heard() {
        let mut link = joined_link();
        link.round.last_heard = 31;
        link.note_snapshot_applied(2);
        link.transmit_inputs(&[cmd(1), cmd(2)], &[], 0).unwrap();
        let sequences: Vec<(u32, u32)> = link
            .mailbox()
            .take_outbound()
            .iter()
            .map(|(_, raw)| match decoded(&link, raw) {
                PlayerToHostDatagram::FrameReceipt { header, .. }
                | PlayerToHostDatagram::Commands { header, .. } => (header.packet_no, header.confirm_receipt),
                other => panic!("unexpected {other:?}"),
            })
            .collect();
        assert_eq!(sequences, vec![(1, 31), (2, 31)]);
    }

    #[test]
    fn nothing_is_unsent_when_there_are_no_actions() {
        assert!(!joined_link().actions_still_owed(&[]));
    }
}
