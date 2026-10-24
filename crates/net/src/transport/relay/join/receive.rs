//! Reading what the authority sent: the datagram lane, the bootstrap offers and the control lane.

use sim::ClientId;

use super::UdpClientLink;
use crate::transport::frame::Frame;
use crate::transport::loopback_live::ReceivedTick;
use crate::transport::protocol::{ServerPacket, decode_server_packet};
use crate::transport::snapshot_wire::WorldObjectSyncDecoder;
use crate::transport::delta::SnapshotDecoder;
use crate::transport::wire::WireReader;

/// What one read of the mailboxes produced.
#[derive(Default)]
pub(super) struct Harvest {
    pub(super) ticks: Vec<ReceivedTick>,

    /// Snapshot sequences to acknowledge, in arrival order.
    pub(super) acks: Vec<u32>,
}

impl UdpClientLink {
    /// Applies everything waiting in the mailboxes and returns the snapshots that came out.
    ///
    /// Datagrams that do not decode are skipped; a control packet that does not decode, or a
    /// rejection, ends the read with an error.
    pub fn recv_ticks(&mut self) -> Result<Vec<ReceivedTick>, String> {
        let mut harvest = Harvest::default();

        for (_, bytes) in self.mailbox.take_inbound() {
            let Ok(packet) = decode_server_packet(&bytes, &self.limits) else {
                continue;
            };
            if !matches!(packet, ServerPacket::Control { .. }) {
                self.apply_packet(packet, &mut harvest)?;
            }
        }

        self.drain_bootstrap_offers(&mut harvest)?;

        for (_, bytes) in self.mailbox.take_control_inbound() {
            let packet = decode_server_packet(&bytes, &self.limits).map_err(|e| e.to_string())?;
            if matches!(packet, ServerPacket::Control { .. }) {
                self.apply_packet(packet, &mut harvest)?;
            }
        }

        self.adopt_entered_client();

        if self.bootstrap.is_none() || self.has_entered_match() {
            for seq in harvest.acks {
                let _ = self.send_snapshot_ack(seq);
            }
        }
        Ok(harvest.ticks)
    }

    pub(super) fn apply_bytes(&mut self, bytes: &[u8], harvest: &mut Harvest) -> Result<bool, String> {
        let packet = decode_server_packet(bytes, &self.limits).map_err(|e| e.to_string())?;
        self.apply_packet(packet, harvest)
    }

    /// Returns whether a new snapshot was applied.
    fn apply_packet(&mut self, packet: ServerPacket, harvest: &mut Harvest) -> Result<bool, String> {
        match packet {
            ServerPacket::Control { header, payload } => {
                let ours = self.failed.is_none()
                    && self.connection == Some(header.connection)
                    && header.applies_to_epoch(self.match_epoch());
                if ours {
                    self.controls.push(payload);
                }
                Ok(false)
            }
            ServerPacket::Accept {
                connection,
                assigned_client,
                ..
            } => {
                self.note_accept(connection, ClientId(assigned_client));
                Ok(false)
            }
            ServerPacket::Reject(reason) => {
                self.note_reject(reason);
                let ours = self.hello.content;
                Err(format!(
                    "handshake rejected: {reason}; our content gameplay={:016x} \
                     map={:016x} models={:016x} weapons={:016x} classes={:016x}",
                    ours.gameplay, ours.map, ours.models, ours.weapons, ours.classes
                ))
            }
            ServerPacket::Snapshot {
                header,
                baseline_seq,
                snapshot_seq,
                payload,
            } => {
                if self.failed.is_some() || self.connection != Some(header.connection) {
                    return Ok(false);
                }
                let live = self.match_epoch();
                if !header.applies_to_epoch(live) {
                    diag::warn!(
                        Net,
                        "client ignored snapshot for epoch {} (live {live})",
                        header.epoch
                    );
                    return Ok(false);
                }
                if self.ledger.holds(snapshot_seq) {
                    return Ok(false);
                }
                let Some(received) =
                    self.decode_snapshot(baseline_seq, header.sequence, &payload)?
                else {
                    return Ok(false);
                };
                self.ledger
                    .record(snapshot_seq, baseline_seq, received.snapshot.clone());
                harvest.acks.push(snapshot_seq);
                harvest.ticks.push(received);
                Ok(true)
            }
        }
    }

    /// Decodes a snapshot payload against the baseline it names. `None` when that baseline is
    /// not held, in which case the packet is dropped and the authority will pick another.
    fn decode_snapshot(
        &mut self,
        baseline_seq: u32,
        packet_sequence: u32,
        payload: &[u8],
    ) -> Result<Option<ReceivedTick>, String> {
        let mut snapshots = SnapshotDecoder::new();
        let mut world = WorldObjectSyncDecoder::default();
        if baseline_seq != 0 {
            let Some(baseline) = self.ledger.get(baseline_seq) else {
                return Ok(None);
            };
            snapshots.adopt_baseline(baseline);
            world.adopt_baseline(baseline.meta.world_objects.clone());
        }

        self.in_ack = packet_sequence;
        let mut input = WireReader::new(payload);
        let frame = Frame::decode(&mut input, &mut world).map_err(|e| e.to_string())?;
        let mut snapshot = snapshots
            .decode(&frame.snapshot_delta)
            .map_err(|e| e.to_string())?;
        snapshot.meta = frame.snapshot_meta.clone();
        Ok(Some(ReceivedTick { snapshot, frame }))
    }
}

#[cfg(test)]
mod tests {
    use master_protocol::MemberId;

    use super::*;
    use crate::transport::protocol::{
        ConnectionId, HandshakeHello, HandshakeReject, PacketHeader,
    };
    use crate::transport::relay::mailbox::RelayMailbox;

    fn link() -> UdpClientLink {
        UdpClientLink::relay(
            HandshakeHello::current(Default::default()),
            RelayMailbox::with_default_cap(),
        )
    }

    fn from_authority(link: &UdpClientLink, packet: &ServerPacket) {
        link.mailbox()
            .push_inbound(MemberId([9; 16]), packet.to_bytes())
            .unwrap();
    }

    fn control_from_authority(link: &UdpClientLink, packet: &ServerPacket) {
        link.mailbox()
            .push_control_inbound(MemberId([9; 16]), packet.to_bytes())
            .unwrap();
    }

    fn accept(connection: u64, client: u32, hello: HandshakeHello) -> ServerPacket {
        ServerPacket::Accept {
            connection: ConnectionId(connection),
            assigned_client: client,
            hello,
        }
    }

    fn empty_control(connection: u64) -> ServerPacket {
        ServerPacket::Control {
            header: PacketHeader {
                connection: ConnectionId(connection),
                sequence: 0,
                ack: 0,
                epoch: 0,
            },
            payload: crate::ReliablePayload {
                ack_through: 0,
                rows: Vec::new(),
                dropped_oldest: 0,
            },
        }
    }

    #[test]
    fn an_accept_names_the_connection_and_client() {
        let mut link = link();
        from_authority(&link, &accept(3, 5, link.hello));
        assert!(link.recv_ticks().unwrap().is_empty());
        assert_eq!(link.connection, Some(ConnectionId(3)));
        assert_eq!(link.assigned_client, Some(ClientId(5)));
    }

    #[test]
    fn a_rejection_ends_the_read_with_our_content_in_the_message() {
        let mut link = link();
        from_authority(&link, &ServerPacket::Reject(HandshakeReject::Limits));
        let error = link.recv_ticks().unwrap_err();
        assert!(error.starts_with("handshake rejected: "), "{error}");
        assert!(error.contains("our content gameplay=0000000000000000"), "{error}");
        assert_eq!(link.handshake_reject(), Some(HandshakeReject::Limits));
    }

    #[test]
    fn garbage_on_the_datagram_lane_is_skipped_but_on_the_control_lane_it_is_an_error() {
        let mut link = link();
        link.mailbox()
            .push_inbound(MemberId([9; 16]), vec![0xff, 0xff, 0xff])
            .unwrap();
        assert!(link.recv_ticks().unwrap().is_empty());

        link.mailbox()
            .push_control_inbound(MemberId([9; 16]), vec![0xff, 0xff, 0xff])
            .unwrap();
        assert!(link.recv_ticks().is_err());
    }

    #[test]
    fn control_rows_for_our_connection_are_kept_and_others_dropped() {
        let mut link = link();
        link.note_accept(ConnectionId(3), ClientId(5));
        control_from_authority(&link, &empty_control(3));
        control_from_authority(&link, &empty_control(4));
        link.recv_ticks().unwrap();
        assert_eq!(link.take_controls().len(), 1);
    }

    #[test]
    fn control_rows_before_the_accept_are_dropped() {
        let mut link = link();
        control_from_authority(&link, &empty_control(3));
        link.recv_ticks().unwrap();
        assert!(link.take_controls().is_empty());
    }

    #[test]
    fn a_control_packet_on_the_datagram_lane_is_not_applied() {
        let mut link = link();
        link.note_accept(ConnectionId(3), ClientId(5));
        from_authority(&link, &empty_control(3));
        link.recv_ticks().unwrap();
        assert!(link.take_controls().is_empty());
    }

    #[test]
    fn a_snapshot_for_another_connection_is_ignored() {
        let mut link = link();
        link.note_accept(ConnectionId(3), ClientId(5));
        let stray = ServerPacket::Snapshot {
            header: PacketHeader {
                connection: ConnectionId(8),
                sequence: 1,
                ack: 0,
                epoch: 0,
            },
            baseline_seq: 0,
            snapshot_seq: 1,
            payload: vec![1, 2, 3],
        };
        from_authority(&link, &stray);
        assert!(link.recv_ticks().unwrap().is_empty());
        assert!(!link.has_applied_snapshot());
    }

    #[test]
    fn a_snapshot_against_a_baseline_we_lack_is_dropped_quietly() {
        let mut link = link();
        link.note_accept(ConnectionId(3), ClientId(5));
        let delta = ServerPacket::Snapshot {
            header: PacketHeader {
                connection: ConnectionId(3),
                sequence: 4,
                ack: 0,
                epoch: 0,
            },
            baseline_seq: 2,
            snapshot_seq: 3,
            payload: vec![1, 2, 3],
        };
        from_authority(&link, &delta);
        assert!(link.recv_ticks().unwrap().is_empty());
        assert!(!link.has_applied_snapshot());
        assert_eq!(link.in_ack, 0, "a dropped packet does not move the ack");
    }

    #[test]
    fn a_snapshot_that_cannot_be_decoded_is_an_error() {
        let mut link = link();
        link.note_accept(ConnectionId(3), ClientId(5));
        let broken = ServerPacket::Snapshot {
            header: PacketHeader {
                connection: ConnectionId(3),
                sequence: 4,
                ack: 0,
                epoch: 0,
            },
            baseline_seq: 0,
            snapshot_seq: 3,
            payload: vec![0xee; 3],
        };
        from_authority(&link, &broken);
        assert!(link.recv_ticks().is_err());
    }
}
