//! What the authority does with the packets members send it.

use master_protocol::RoomOccupantId;
use sim::ClientId;

use super::HostUdpSwitchboard;
use crate::host_tick::tick_systems::PlayerShotOrigins;
use crate::net_links::match_handoff::{TransferReceiptMsg, same_generation};
use crate::net_links::datagrams::{PlayerToHostDatagram, DatagramPreamble, parse_receiver_datagram};
use crate::net_links::relay::entry_gate::{Admission, FinalizedEntry, OfferKey, Verdict};

/// Who sent a packet and on which lane (the control lane is the only one that may carry
/// actions).
#[derive(Clone, Copy)]
struct Origin {
    member: RoomOccupantId,
    control: bool,
}

/// Where the game-facing results of one `ingress` call go.
struct Sinks<'a> {
    cmds: &'a mut crate::PlayerMoveIntake,
    actions: &'a mut crate::PlayerActionIntake,
    reliable: Option<&'a mut crate::GuaranteedEventRouter>,
}

/// Why a bootstrap confirmation was not taken; logged, then dropped.
enum AckRefusal {
    StaleEpoch { live: u32 },
    NotTheOwner,
    NoPeer,
    NoOpenOffer,
}

impl AckRefusal {
    fn log(&self, confirm_receipt: &TransferReceiptMsg) {
        let (seat_member, link, offer_no) = (confirm_receipt.member_id, confirm_receipt.link, confirm_receipt.handoff_id);
        match self {
            Self::StaleEpoch { live } => diag::warn!(
                WireLink,
                "bootstrap confirmation dropped: epoch {} is not the live epoch {live}",
                confirm_receipt.era
            ),
            Self::NotTheOwner => diag::warn!(
                WireLink,
                "bootstrap confirmation dropped: member {seat_member} does not own connection {link:?}"
            ),
            Self::NoPeer => {
                diag::warn!(WireLink, "bootstrap confirmation dropped: nobody on connection {link:?}")
            }
            Self::NoOpenOffer => diag::warn!(
                WireLink,
                "bootstrap confirmation dropped: member {seat_member} confirmed {offer_no}, which was never offered"
            ),
        }
    }
}

impl HostUdpSwitchboard {
    /// Reads everything the members sent since the last call: first the confirmations of
    /// bootstrap offers, then the datagram lane, then the control lane.
    pub fn take_datagram(
        &mut self,
        cmd_inbox: &mut crate::PlayerMoveIntake,
        action_inbox: &mut crate::PlayerActionIntake,
        _samples: Option<&mut PlayerShotOrigins>,
        reliable: Option<&mut crate::GuaranteedEventRouter>,
    ) -> Result<(), String> {
        if let Some(lane) = self.offer_lane.clone() {
            let current_era = lane.era();
            lane.take_acks()
                .into_iter()
                .for_each(|confirm_receipt| self.confirm_admission(confirm_receipt, current_era));
        }

        let mut sinks = Sinks { cmds: cmd_inbox, actions: action_inbox, reliable };
        let queues = [
            (false, self.mailbox.take_inbound()),
            (true, self.mailbox.take_control_inbound()),
        ];
        for (control, batch) in queues {
            for (seat_member, raw) in batch {
                // Garbled datagrams are skipped; the rest of the batch is still processed.
                if let Ok(packet) = parse_receiver_datagram(&raw, &self.negotiated) {
                    self.route(Origin { member: seat_member, control }, packet, &mut sinks);
                }
            }
        }
        Ok(())
    }

    fn route(&mut self, source: Origin, datagram: PlayerToHostDatagram, sinks: &mut Sinks<'_>) {
        let (header, what) = match &datagram {
            PlayerToHostDatagram::Connect(_) => return,
            PlayerToHostDatagram::Commands { header, .. } => (*header, "commands"),
            PlayerToHostDatagram::FrameReceipt { header, .. } => (*header, "snapshot ack"),
        };
        if !self.is_current(source.member, &header, what) {
            return;
        }
        match datagram {
            PlayerToHostDatagram::FrameReceipt { frame_number_on_wire, .. } => {
                if let Some(peer) = self.remotes.get_mut(&header.connection) {
                    peer.feed.incoming_receipt = header.packet_no;
                    let _ = peer.feed.baseline.confirm_receipt(frame_number_on_wire);
                }
            }
            commands @ PlayerToHostDatagram::Commands { .. } => self.take_commands(source, commands, sinks),
            PlayerToHostDatagram::Connect(_) => {}
        }
    }

    /// This datagram is from the member that owns its connection, and stamped with the live epoch.
    /// A stale epoch is worth a log line; a stranger is not.
    fn is_current(&self, from: RoomOccupantId, header: &DatagramPreamble, what: &str) -> bool {
        let live = self.running_generation();
        let owner_ok = self.member_on(header.connection) == Some(from);
        let epoch_ok = owner_ok && header.valid_for_generation(live);
        if owner_ok && !epoch_ok {
            diag::warn!(
                WireLink,
                "dropped {what} stamped with epoch {} while the live epoch is {live}",
                header.round_era
            );
        }
        epoch_ok
    }

    fn take_commands(&mut self, source: Origin, datagram: PlayerToHostDatagram, sinks: &mut Sinks<'_>) {
        let PlayerToHostDatagram::Commands { header, asserted_player_slot, moves, aim_origins, actions, guaranteed_receipt } =
            datagram
        else {
            return;
        };
        let lane_present = self.offer_lane.is_some();
        let Some(peer) = self
            .remotes
            .get_mut(&header.connection)
            .filter(|peer| peer.feed.admits_gameplay(lane_present))
        else {
            return;
        };
        let Some(client) = self
            .links
            .trusted_slot(header.connection, asserted_player_slot)
            .ok()
            .map(ClientId)
        else {
            return;
        };

        peer.feed.incoming_receipt = header.packet_no;
        if let Some(hub) = sinks.reliable.as_deref_mut() {
            hub.confirm_receipt(client, guaranteed_receipt);
        }
        for (cmd_no, cmd) in moves {
            // A shot sample whose history claim names snapshots this peer was never sent is
            // worthless; its blend weight becomes NaN so nothing downstream trusts it.
            let sample = aim_origins
                .iter()
                .find_map(|&(at, shot)| (at == cmd_no).then_some(shot))
                .map(|mut shot| {
                    let unbacked = shot.quality.asserts_past_record()
                        && !peer.feed.has_sent_both(shot.left, shot.right);
                    if unbacked {
                        shot.opacity = f32::NAN;
                    }
                    shot
                });
            sinks.cmds.push(client, Some(cmd_no), cmd, sample);
        }
        if source.control {
            actions
                .into_iter()
                .for_each(|action| sinks.actions.accept_remote(client, action));
        }
    }

    /// A member says it applied a bootstrap offer. If it names the offer that is open, the
    /// member is in; a repeat of an accepted confirmation is reported again, not committed again.
    fn confirm_admission(&mut self, confirm_receipt: TransferReceiptMsg, live: u32) {
        match self.admit(&confirm_receipt, live) {
            Ok(initial_publish_stamp) => self.finalized.push(FinalizedEntry {
                member_id: confirm_receipt.member_id,
                round_era: confirm_receipt.era,
                transfer_ticket: confirm_receipt.handoff_id,
                link_no: confirm_receipt.link.0,
                initial_publish_stamp,
            }),
            Err(refusal) => refusal.log(&confirm_receipt),
        }
    }

    /// Ok(true) on the first commit, Ok(false) on a repeat.
    fn admit(&mut self, confirm_receipt: &TransferReceiptMsg, live: u32) -> Result<bool, AckRefusal> {
        if !same_generation(confirm_receipt.era, live) {
            return Err(AckRefusal::StaleEpoch { live });
        }
        if self.member_on(confirm_receipt.link) != Some(confirm_receipt.member_id) {
            return Err(AckRefusal::NotTheOwner);
        }
        let feed = &mut self.remotes.get_mut(&confirm_receipt.link).ok_or(AckRefusal::NoPeer)?.feed;
        let named = OfferKey {
            era: confirm_receipt.era,
            snap_no: confirm_receipt.frame_number_on_wire,
            offer_no: confirm_receipt.handoff_id,
        };
        match feed.admission.judge(named) {
            Verdict::Ignore => Err(AckRefusal::NoOpenOffer),
            Verdict::ReEntry => Ok(false),
            Verdict::Commit => {
                feed.admission = Admission::Committed { offer_no: named.offer_no };
                let _ = feed.baseline.confirm_receipt(named.snap_no);
                Ok(true)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::run_ahead::InputCommandNumber;
    use crate::net_links::datagrams::{LinkNumber, JoinGreeting};
    use crate::net_links::relay::{ForwardingInbox, HostUdpSwitchboard};

    fn member(byte: u8) -> RoomOccupantId {
        RoomOccupantId([byte; 16])
    }

    /// A hub (member 0) with `peer` joined, and the connection the peer got.
    fn hub_with(peer: RoomOccupantId) -> (HostUdpSwitchboard, LinkNumber) {
        let mut hub = HostUdpSwitchboard::relay(
            JoinGreeting::current(Default::default()),
            1,
            ForwardingInbox::standard_capacity(),
        );
        hub.sync_room_members(&[peer], member(0));
        let conn = hub.connection_of(peer).expect("peer joined");
        (hub, conn)
    }

    fn header(link_no: LinkNumber, sequence: u32) -> DatagramPreamble {
        DatagramPreamble { connection: link_no, packet_no: sequence, confirm_receipt: 0, round_era: 0 }
    }

    fn one_command(conn: LinkNumber, sequence: u32, cmd_seq: u32) -> Vec<u8> {
        let cmds = vec![(InputCommandNumber(cmd_seq), Default::default())];
        let packet = PlayerToHostDatagram::Commands {
            header: header(conn, sequence),
            asserted_player_slot: 0,
            moves: cmds,
            aim_origins: vec![],
            actions: vec![],
            guaranteed_receipt: 0,
        };
        packet.wire_bytes()
    }

    fn snapshot_ack(header: DatagramPreamble, frame_number_on_wire: u32) -> Vec<u8> {
        PlayerToHostDatagram::FrameReceipt { header, frame_number_on_wire }.wire_bytes()
    }

    fn drain(hub: &mut HostUdpSwitchboard) -> crate::PlayerMoveIntake {
        let mut cmds = crate::PlayerMoveIntake::default();
        hub.take_datagram(&mut cmds, &mut Default::default(), None, None).expect("ingress");
        cmds
    }

    fn pending(hub: &HostUdpSwitchboard, cmds: &crate::PlayerMoveIntake, who: RoomOccupantId) -> usize {
        cmds.queued_count(hub.seat_for_occupant(who).expect("member has a client"))
    }

    fn with_open_offer(conn: LinkNumber, hub: &mut HostUdpSwitchboard) {
        let offer = OfferKey { offer_no: 1, snap_no: 4, era: 7 };
        hub.remotes.get_mut(&conn).unwrap().feed.admission = Admission::Pending(offer);
    }

    fn confirmation(from: RoomOccupantId, link_no: LinkNumber, offer_no: u32) -> TransferReceiptMsg {
        TransferReceiptMsg { member_id: from, link: link_no, handoff_id: offer_no, frame_number_on_wire: 4, era: 7 }
    }

    #[test]
    fn the_owners_commands_are_queued_and_acked() {
        let (mut hub, conn) = hub_with(member(2));
        hub.mailbox().take_arrival(member(2), one_command(conn, 5, 1)).unwrap();
        let cmds = drain(&mut hub);
        assert_eq!(pending(&hub, &cmds, member(2)), 1);
        assert_eq!(hub.remotes[&conn].feed.incoming_receipt, 5);
    }

    #[test]
    fn a_stranger_using_the_connection_is_ignored() {
        let (mut hub, conn) = hub_with(member(2));
        hub.mailbox().take_arrival(member(7), one_command(conn, 5, 1)).unwrap();
        let cmds = drain(&mut hub);
        assert_eq!(pending(&hub, &cmds, member(2)), 0);
        assert_eq!(hub.remotes[&conn].feed.incoming_receipt, 0);
    }

    #[test]
    fn junk_does_not_block_later_packets() {
        let (mut hub, conn) = hub_with(member(2));
        let mailbox = hub.mailbox();
        for bytes in [vec![0xFF; 3], one_command(conn, 1, 1)] {
            mailbox.take_arrival(member(2), bytes).unwrap();
        }
        let cmds = drain(&mut hub);
        assert_eq!(pending(&hub, &cmds, member(2)), 1);
    }

    #[test]
    fn a_snapshot_ack_updates_in_ack() {
        let (mut hub, conn) = hub_with(member(2));
        hub.mailbox().take_arrival(member(2), snapshot_ack(header(conn, 9), 3)).unwrap();
        drain(&mut hub);
        assert_eq!(hub.remotes[&conn].feed.incoming_receipt, 9);
    }

    #[test]
    fn the_wrong_epoch_is_dropped() {
        let (mut hub, conn) = hub_with(member(2));
        let old = DatagramPreamble { round_era: 99, ..header(conn, 4) };
        hub.mailbox().take_arrival(member(2), snapshot_ack(old, 1)).unwrap();
        drain(&mut hub);
        assert_eq!(hub.remotes[&conn].feed.incoming_receipt, 0);
    }

    #[test]
    fn the_open_offer_commits_once_then_repeats() {
        let (mut hub, conn) = hub_with(member(2));
        with_open_offer(conn, &mut hub);
        for id in [1, 1, 2] {
            hub.confirm_admission(confirmation(member(2), conn, id), 7);
        }
        let firsts: Vec<bool> = hub
            .take_committed_admissions()
            .iter()
            .map(|done| done.initial_publish_stamp)
            .collect();
        assert_eq!(firsts, [true, false]);
        assert!(hub.remotes[&conn].feed.admission.is_committed());
    }

    #[test]
    fn another_members_confirmation_is_refused() {
        let (mut hub, conn) = hub_with(member(2));
        with_open_offer(conn, &mut hub);
        hub.confirm_admission(confirmation(member(8), conn, 1), 7);
        assert_eq!(hub.take_committed_admissions().len(), 0);
        assert!(!hub.remotes[&conn].feed.admission.is_committed());
    }
}
