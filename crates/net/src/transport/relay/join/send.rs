//! What a client sends to the authority: snapshot acknowledgements, commands and actions.

use playerstate_t6::game::UserCmd;
use sim::{ClientAction, action_request_id};

use super::UdpClientLink;
use crate::client::predict::CmdSeq;
use crate::transport::protocol::{ClientPacket, ConnectionId, PacketHeader};
use crate::transport::relay::mailbox::send_refused;
use crate::transport::udp_socket::UdpSendError;

/// The most commands one packet carries; the peer's own limit can lower it further.
const CMDS_PER_PACKET: usize = 16;

type CmdWithSample = (CmdSeq, UserCmd, sim::ShotSampleProvenance);

impl UdpClientLink {
    fn next_header(&mut self, connection: ConnectionId) -> PacketHeader {
        self.out_seq = self.out_seq.wrapping_add(1);
        PacketHeader {
            connection,
            sequence: self.out_seq,
            ack: self.in_ack,
            epoch: self.match_epoch(),
        }
    }

    pub(super) fn send_snapshot_ack(&mut self, snapshot_seq: u32) -> Result<(), UdpSendError> {
        let Some(connection) = self.connection else {
            return Ok(());
        };
        let packet = ClientPacket::SnapshotAck {
            header: self.next_header(connection),
            snapshot_seq,
        };
        self.send_to_authority(packet.to_bytes())
    }

    /// Whether any of `actions` has not been sent yet.
    pub fn has_unsent_actions(&self, actions: &[ClientAction]) -> bool {
        actions
            .iter()
            .any(|action| !self.sent_actions.contains(&action_request_id(action)))
    }

    /// Sends the commands in packets of at most `CMDS_PER_PACKET`, then the actions not sent
    /// before. When there are no commands, the action packet goes out even if it carries no
    /// actions, so the authority keeps hearing from this client.
    pub fn send_commands(
        &mut self,
        cmds: &[CmdWithSample],
        actions: &[ClientAction],
        reliable_ack: u16,
    ) -> Result<(), UdpSendError> {
        let Some(connection) = self.connection else {
            return Ok(());
        };
        if let Some(newest) = self.ledger.newest() {
            let _ = self.send_snapshot_ack(newest);
        }

        let per_packet = CMDS_PER_PACKET.min(usize::from(self.limits.max_cmds_per_tick));
        if per_packet == 0 {
            return Err(send_refused("peer permits no commands per packet"));
        }
        for chunk in cmds.chunks(per_packet) {
            self.send_command_packet(connection, chunk, &[], reliable_ack)?;
        }

        // Actions the caller dropped are forgotten, so a later request with the same id
        // counts as new.
        self.sent_actions
            .retain(|id| actions.iter().any(|action| action_request_id(action) == *id));
        let fresh: Vec<ClientAction> = actions
            .iter()
            .filter(|action| !self.sent_actions.contains(&action_request_id(action)))
            .copied()
            .collect();
        if !fresh.is_empty() || cmds.is_empty() {
            self.send_command_packet(connection, &[], &fresh, reliable_ack)?;
            self.sent_actions
                .extend(fresh.iter().map(action_request_id));
        }
        Ok(())
    }

    fn send_command_packet(
        &mut self,
        connection: ConnectionId,
        cmds: &[CmdWithSample],
        actions: &[ClientAction],
        reliable_ack: u16,
    ) -> Result<(), UdpSendError> {
        let packet = ClientPacket::Commands {
            header: self.next_header(connection),
            claimed_client: self.assigned_client.map_or(0, |client| client.0),
            cmds: cmds.iter().map(|&(seq, cmd, _)| (seq, cmd)).collect(),
            samples: cmds.iter().map(|&(seq, _, sample)| (seq, sample)).collect(),
            actions: actions.to_vec(),
            reliable_ack,
        };
        let bytes = packet.to_bytes();
        if cmds.is_empty() {
            self.mailbox
                .push_control_outbound(master_protocol::MemberId([0; 16]), bytes)
                .map_err(send_refused)
        } else {
            self.send_to_authority(bytes)
        }
    }
}

#[cfg(test)]
mod tests {
    use master_protocol::MemberId;
    use sim::ClientId;

    use super::*;
    use crate::transport::protocol::{HandshakeHello, decode_client_packet};
    use crate::transport::relay::mailbox::RelayMailbox;

    fn joined_link() -> UdpClientLink {
        let mut link = UdpClientLink::relay(
            HandshakeHello::current(Default::default()),
            RelayMailbox::with_default_cap(),
        );
        link.note_accept(ConnectionId(3), ClientId(5));
        link
    }

    fn cmd(seq: u32) -> CmdWithSample {
        (
            CmdSeq(seq),
            UserCmd::default(),
            sim::ShotSampleProvenance::default(),
        )
    }

    fn decoded(link: &UdpClientLink, bytes: &[u8]) -> ClientPacket {
        decode_client_packet(bytes, &link.limits).expect("our own packet decodes")
    }

    fn command_counts(link: &UdpClientLink) -> Vec<usize> {
        link.mailbox()
            .take_outbound()
            .iter()
            .filter_map(|(_, bytes)| match decoded(link, bytes) {
                ClientPacket::Commands { cmds, .. } => Some(cmds.len()),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn a_link_that_has_not_joined_sends_nothing() {
        let mut link = UdpClientLink::relay(
            HandshakeHello::current(Default::default()),
            RelayMailbox::with_default_cap(),
        );
        link.send_commands(&[cmd(1)], &[], 0).unwrap();
        link.send_snapshot_ack(4).unwrap();
        assert!(link.mailbox().take_outbound().is_empty());
        assert!(link.mailbox().take_control_outbound().is_empty());
    }

    #[test]
    fn commands_go_out_in_packets_of_at_most_sixteen() {
        let mut link = joined_link();
        let cmds: Vec<_> = (1..=40).map(cmd).collect();
        link.send_commands(&cmds, &[], 0).unwrap();
        assert_eq!(command_counts(&link), vec![16, 16, 8]);
        assert!(
            link.mailbox().take_control_outbound().is_empty(),
            "commands were sent, so no empty packet follows"
        );
    }

    #[test]
    fn the_peers_limit_can_shrink_the_packets() {
        let mut link = joined_link();
        link.limits.max_cmds_per_tick = 5;
        let cmds: Vec<_> = (1..=12).map(cmd).collect();
        link.send_commands(&cmds, &[], 0).unwrap();
        assert_eq!(command_counts(&link), vec![5, 5, 2]);
    }

    #[test]
    fn a_peer_that_permits_no_commands_is_an_error() {
        let mut link = joined_link();
        link.limits.max_cmds_per_tick = 0;
        assert!(link.send_commands(&[cmd(1)], &[], 0).is_err());
        assert!(link.mailbox().take_outbound().is_empty());
    }

    #[test]
    fn with_no_commands_an_empty_packet_goes_on_the_control_lane() {
        let mut link = joined_link();
        link.send_commands(&[], &[], 7).unwrap();
        assert!(link.mailbox().take_outbound().is_empty());
        let control = link.mailbox().take_control_outbound();
        assert_eq!(control.len(), 1);
        assert_eq!(control[0].0, MemberId([0; 16]));
        match decoded(&link, &control[0].1) {
            ClientPacket::Commands {
                cmds,
                actions,
                reliable_ack,
                claimed_client,
                ..
            } => {
                assert!(cmds.is_empty() && actions.is_empty());
                assert_eq!(reliable_ack, 7);
                assert_eq!(claimed_client, 5);
            }
            other => panic!("expected commands, got {other:?}"),
        }
    }

    #[test]
    fn the_newest_applied_snapshot_is_acknowledged_before_the_commands() {
        let mut link = joined_link();
        link.note_applied_snapshot(9);
        link.note_applied_snapshot(7);
        link.send_commands(&[cmd(1)], &[], 0).unwrap();
        let sent = link.mailbox().take_outbound();
        assert_eq!(sent.len(), 2);
        match decoded(&link, &sent[0].1) {
            ClientPacket::SnapshotAck { snapshot_seq, .. } => assert_eq!(snapshot_seq, 9),
            other => panic!("expected an ack first, got {other:?}"),
        }
        assert!(matches!(
            decoded(&link, &sent[1].1),
            ClientPacket::Commands { .. }
        ));
    }

    #[test]
    fn packet_sequence_numbers_count_up_and_carry_the_last_ack_heard() {
        let mut link = joined_link();
        link.in_ack = 31;
        link.note_applied_snapshot(2);
        link.send_commands(&[cmd(1), cmd(2)], &[], 0).unwrap();
        let sequences: Vec<(u32, u32)> = link
            .mailbox()
            .take_outbound()
            .iter()
            .map(|(_, bytes)| match decoded(&link, bytes) {
                ClientPacket::SnapshotAck { header, .. }
                | ClientPacket::Commands { header, .. } => (header.sequence, header.ack),
                other => panic!("unexpected {other:?}"),
            })
            .collect();
        assert_eq!(sequences, vec![(1, 31), (2, 31)]);
    }

    #[test]
    fn nothing_is_unsent_when_there_are_no_actions() {
        assert!(!joined_link().has_unsent_actions(&[]));
    }
}
