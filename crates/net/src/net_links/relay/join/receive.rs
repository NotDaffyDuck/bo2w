//! Reading what the authority sent: the datagram lane, the bootstrap offers and the control lane.

use sim::ClientId;

use super::PlayerUdpSession;
use crate::net_links::tick_frames::Frame;
use crate::net_links::listen_link::ArrivedHostStep;
use crate::net_links::datagrams::{HostToPlayerDatagram, parse_host_datagram};
use crate::net_links::snapshot_wire::WorldPropStreamDecoder;
use crate::net_links::snapshot_diff::FrameDiffReader;
use crate::net_links::byte_codec::ByteCursorReader;

/// What one read of the mailboxes produced.
#[derive(Default)]
pub(super) struct Harvest {
    pub(super) ticks: Vec<ArrivedHostStep>,

    /// Snapshot sequences to acknowledge, in arrival order.
    pub(super) receipts_batch: Vec<u32>,
}

/// Whether the datagram travels on the reliable control lane.
fn is_control_row(datagram: &HostToPlayerDatagram) -> bool {
    match datagram {
        HostToPlayerDatagram::OwedBatch { .. } => true,
        _ => false,
    }
}

impl PlayerUdpSession {
    /// Applies everything waiting in the mailboxes and returns the snapshots that came out.
    ///
    /// Datagrams that do not decode are skipped; a control packet that does not decode, or a
    /// rejection, ends the read with an error.
    pub fn arrived_steps(&mut self) -> Result<Vec<ArrivedHostStep>, String> {
        let mut harvest = Harvest::default();

        for (_, raw) in self.mailbox.take_inbound() {
            let Ok(datagram) = parse_host_datagram(&raw, &self.negotiated) else {
                continue;
            };
            if !is_control_row(&datagram) {
                self.apply_packet(datagram, &mut harvest)?;
            }
        }

        self.drain_bootstrap_offers(&mut harvest)?;

        for (_, raw) in self.mailbox.take_control_inbound() {
            let datagram = parse_host_datagram(&raw, &self.negotiated).map_err(|err| err.to_string())?;
            if is_control_row(&datagram) {
                self.apply_packet(datagram, &mut harvest)?;
            }
        }

        self.take_over_joined_receiver();

        if self.offer_lane.is_none() || self.is_inside_match() {
            for snap_no in harvest.receipts_batch {
                let _ = self.transmit_state_receipt(snap_no);
            }
        }
        Ok(harvest.ticks)
    }

    pub(super) fn apply_bytes(&mut self, raw: &[u8], harvest: &mut Harvest) -> Result<bool, String> {
        let datagram = parse_host_datagram(raw, &self.negotiated).map_err(|err| err.to_string())?;
        self.apply_packet(datagram, harvest)
    }

    /// Returns whether a new snapshot was applied.
    fn apply_packet(&mut self, datagram: HostToPlayerDatagram, harvest: &mut Harvest) -> Result<bool, String> {
        match datagram {
            HostToPlayerDatagram::OwedBatch { header: preamble, payload: envelope } => {
                let addressed_to_us = self.round.rejected.is_none()
                    && self.connection == Some(preamble.connection)
                    && preamble.valid_for_generation(self.game_generation());
                if addressed_to_us {
                    self.round.controls.push(envelope);
                }
                Ok(false)
            }
            HostToPlayerDatagram::Admitted {
                connection: link_id,
                granted_player_slot: slot,
                ..
            } => {
                self.note_admission(link_id, ClientId(slot));
                Ok(false)
            }
            HostToPlayerDatagram::Reject(reason) => {
                self.record_refusal(reason);
                let mine = self.hello.content;
                Err(format!(
                    "handshake rejected: {reason}; our content gameplay={:016x} \
                     map={:016x} models={:016x} weapons={:016x} classes={:016x}",
                    mine.rules_digest, mine.map, mine.model_digest, mine.weapon_digest, mine.class_digest
                ))
            }
            HostToPlayerDatagram::Snapshot {
                header: preamble,
                reference_frame_number: base_no,
                frame_number_on_wire: snap_no,
                body,
            } => {
                if self.round.rejected.is_some() || self.connection != Some(preamble.connection) {
                    return Ok(false);
                }
                let current_era = self.game_generation();
                if !preamble.valid_for_generation(current_era) {
                    diag::warn!(
                        WireLink,
                        "joiner dropped a snapshot stamped era {} (current {current_era})",
                        preamble.round_era
                    );
                    return Ok(false);
                }
                if self.round.ledger.has_snapshot(snap_no) {
                    return Ok(false);
                }
                let Some(arrived) =
                    self.decode_snapshot(base_no, preamble.packet_no, &body)?
                else {
                    return Ok(false);
                };
                self.round.ledger
                    .record(snap_no, base_no, arrived.snapshot.clone());
                harvest.receipts_batch.push(snap_no);
                harvest.ticks.push(arrived);
                Ok(true)
            }
        }
    }

    /// Decodes a snapshot payload against the baseline it names. `None` when that baseline is
    /// not held, in which case the packet is dropped and the authority will pick another.
    fn decode_snapshot(
        &mut self,
        base_no: u32,
        packet_sequence: u32,
        body: &[u8],
    ) -> Result<Option<ArrivedHostStep>, String> {
        let mut diff_reader = FrameDiffReader::new();
        let mut world = WorldPropStreamDecoder::default();
        if base_no != 0 {
            let Some(baseline) = self.round.ledger.get(base_no) else {
                return Ok(None);
            };
            diff_reader.accept_reference_frame(baseline);
            world.accept_reference_frame(baseline.meta.world_objects.clone());
        }

        self.round.last_heard = packet_sequence;
        let mut cursor = ByteCursorReader::new(body);
        let frame = Frame::decode(&mut cursor, &mut world).map_err(|err| err.to_string())?;
        let mut rebuilt = diff_reader
            .decode(&frame.frame_diff_section)
            .map_err(|err| err.to_string())?;
        rebuilt.meta = frame.meta_rows.clone();
        Ok(Some(ArrivedHostStep { snapshot: rebuilt, frame }))
    }
}

#[cfg(test)]
mod tests {
    use master_protocol::RoomOccupantId;

    use super::*;
    use crate::net_links::datagrams::{
        LinkNumber, JoinGreeting, JoinRefusal, DatagramPreamble,
    };
    use crate::net_links::relay::mailbox::ForwardingInbox;

    fn link() -> PlayerUdpSession {
        PlayerUdpSession::relay(
            JoinGreeting::current(Default::default()),
            ForwardingInbox::standard_capacity(),
        )
    }

    fn from_authority(link: &PlayerUdpSession, datagram: &HostToPlayerDatagram) {
        link.mailbox()
            .take_arrival(RoomOccupantId([9; 16]), datagram.wire_bytes())
            .unwrap();
    }

    fn control_from_authority(link: &PlayerUdpSession, datagram: &HostToPlayerDatagram) {
        link.mailbox()
            .enqueue_control_incoming(RoomOccupantId([9; 16]), datagram.wire_bytes())
            .unwrap();
    }

    fn accept(link_no: u64, slot: u32, hello: JoinGreeting) -> HostToPlayerDatagram {
        HostToPlayerDatagram::Admitted {
            connection: LinkNumber(link_no),
            granted_player_slot: slot,
            hello,
        }
    }

    fn empty_control(link_no: u64) -> HostToPlayerDatagram {
        HostToPlayerDatagram::OwedBatch {
            header: DatagramPreamble {
                connection: LinkNumber(link_no),
                packet_no: 0,
                confirm_receipt: 0,
                round_era: 0,
            },
            payload: crate::GuaranteedEnvelope {
                confirmed_up_to: 0,
                entries: Vec::new(),
                evicted_stale_count: 0,
            },
        }
    }

    #[test]
    fn an_accept_names_the_connection_and_client() {
        let mut link = link();
        from_authority(&link, &accept(3, 5, link.hello));
        assert!(link.arrived_steps().unwrap().is_empty());
        assert_eq!(link.connection, Some(LinkNumber(3)));
        assert_eq!(link.granted_player_slot, Some(ClientId(5)));
    }

    #[test]
    fn a_rejection_ends_the_read_with_our_content_in_the_message() {
        let mut link = link();
        from_authority(&link, &HostToPlayerDatagram::Reject(JoinRefusal::Limits));
        let error = link.arrived_steps().unwrap_err();
        assert!(error.starts_with("handshake rejected: "), "{error}");
        assert!(error.contains("our content gameplay=0000000000000000"), "{error}");
        assert_eq!(link.greeting_refusal(), Some(JoinRefusal::Limits));
    }

    #[test]
    fn garbage_on_the_datagram_lane_is_skipped_but_on_the_control_lane_it_is_an_error() {
        let mut link = link();
        link.mailbox()
            .take_arrival(RoomOccupantId([9; 16]), vec![0xff, 0xff, 0xff])
            .unwrap();
        assert!(link.arrived_steps().unwrap().is_empty());

        link.mailbox()
            .enqueue_control_incoming(RoomOccupantId([9; 16]), vec![0xff, 0xff, 0xff])
            .unwrap();
        assert!(link.arrived_steps().is_err());
    }

    #[test]
    fn control_rows_for_our_connection_are_kept_and_others_dropped() {
        let mut link = link();
        link.note_admission(LinkNumber(3), ClientId(5));
        control_from_authority(&link, &empty_control(3));
        control_from_authority(&link, &empty_control(4));
        link.arrived_steps().unwrap();
        assert_eq!(link.take_controls().len(), 1);
    }

    #[test]
    fn control_rows_before_the_accept_are_dropped() {
        let mut link = link();
        control_from_authority(&link, &empty_control(3));
        link.arrived_steps().unwrap();
        assert!(link.take_controls().is_empty());
    }

    #[test]
    fn a_control_packet_on_the_datagram_lane_is_not_applied() {
        let mut link = link();
        link.note_admission(LinkNumber(3), ClientId(5));
        from_authority(&link, &empty_control(3));
        link.arrived_steps().unwrap();
        assert!(link.take_controls().is_empty());
    }

    #[test]
    fn a_snapshot_for_another_connection_is_ignored() {
        let mut link = link();
        link.note_admission(LinkNumber(3), ClientId(5));
        let stray = HostToPlayerDatagram::Snapshot {
            header: DatagramPreamble {
                connection: LinkNumber(8),
                packet_no: 1,
                confirm_receipt: 0,
                round_era: 0,
            },
            reference_frame_number: 0,
            frame_number_on_wire: 1,
            body: vec![1, 2, 3],
        };
        from_authority(&link, &stray);
        assert!(link.arrived_steps().unwrap().is_empty());
        assert!(!link.took_in_world_state());
    }

    #[test]
    fn a_snapshot_against_a_baseline_we_lack_is_dropped_quietly() {
        let mut link = link();
        link.note_admission(LinkNumber(3), ClientId(5));
        let delta = HostToPlayerDatagram::Snapshot {
            header: DatagramPreamble {
                connection: LinkNumber(3),
                packet_no: 4,
                confirm_receipt: 0,
                round_era: 0,
            },
            reference_frame_number: 2,
            frame_number_on_wire: 3,
            body: vec![1, 2, 3],
        };
        from_authority(&link, &delta);
        assert!(link.arrived_steps().unwrap().is_empty());
        assert!(!link.took_in_world_state());
        assert_eq!(link.round.last_heard, 0, "a dropped packet does not move the ack");
    }

    #[test]
    fn a_snapshot_that_cannot_be_decoded_is_an_error() {
        let mut link = link();
        link.note_admission(LinkNumber(3), ClientId(5));
        let broken = HostToPlayerDatagram::Snapshot {
            header: DatagramPreamble {
                connection: LinkNumber(3),
                packet_no: 4,
                confirm_receipt: 0,
                round_era: 0,
            },
            reference_frame_number: 0,
            frame_number_on_wire: 3,
            body: vec![0xee; 3],
        };
        from_authority(&link, &broken);
        assert!(link.arrived_steps().is_err());
    }
}
