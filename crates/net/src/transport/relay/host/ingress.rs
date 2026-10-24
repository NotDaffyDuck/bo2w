//! What the authority does with the packets members send it.

use master_protocol::MemberId;
use sim::ClientId;

use super::UdpAuthorityHub;
use crate::authority::runtime::ClientShotSamples;
use crate::client::predict::CmdSeq;
use crate::transport::bootstrap::{BootstrapAck, epoch_applies};
use crate::transport::protocol::{ClientPacket, PacketHeader, decode_client_packet};
use crate::transport::relay::admission::{Admission, CommittedAdmission, OfferKey, Verdict};

/// The commands of one `ClientPacket::Commands`, apart from its header.
struct CommandBatch {
    claimed_client: u32,
    cmds: Vec<(CmdSeq, playerstate_t6::game::UserCmd)>,
    samples: Vec<(CmdSeq, sim::ShotSampleProvenance)>,
    actions: Vec<sim::ClientAction>,
    reliable_ack: u16,
}

impl UdpAuthorityHub {
    /// Reads everything the members sent since the last call: first the confirmations of
    /// bootstrap offers, then the datagram lane, then the control lane.
    pub fn ingress(
        &mut self,
        cmd_inbox: &mut crate::ClientCommandInbox,
        action_inbox: &mut crate::ClientActionInbox,
        _samples: Option<&mut ClientShotSamples>,
        mut reliable: Option<&mut crate::ReliableEventHub>,
    ) -> Result<(), String> {
        self.apply_admission_acks();

        let datagrams = self.mailbox.take_inbound().into_iter().map(|p| (p, false));
        let controls = self
            .mailbox
            .take_control_inbound()
            .into_iter()
            .map(|p| (p, true));
        for ((from, bytes), via_control) in datagrams.chain(controls) {
            let Ok(packet) = decode_client_packet(&bytes, &self.limits) else {
                continue;
            };
            match packet {
                ClientPacket::Connect(_) => {}
                ClientPacket::Commands {
                    header,
                    claimed_client,
                    cmds,
                    samples,
                    actions,
                    reliable_ack,
                } => {
                    let batch = CommandBatch {
                        claimed_client,
                        cmds,
                        samples,
                        actions,
                        reliable_ack,
                    };
                    self.take_commands(
                        from,
                        header,
                        batch,
                        via_control,
                        cmd_inbox,
                        action_inbox,
                        reliable.as_deref_mut(),
                    );
                }
                ClientPacket::SnapshotAck {
                    header,
                    snapshot_seq,
                } => self.take_snapshot_ack(from, header, snapshot_seq),
            }
        }
        Ok(())
    }

    /// Whether a packet from `from` under `header` is for this match and from the member that
    /// owns the connection. Logs the stale-epoch case, which is worth seeing.
    fn packet_is_current(&self, from: MemberId, header: &PacketHeader, what: &str) -> bool {
        if self.member_on(header.connection) != Some(from) {
            return false;
        }
        let live = self.live_epoch();
        if !header.applies_to_epoch(live) {
            diag::warn!(
                Net,
                "authority ignored {what} for epoch {} (live {live})",
                header.epoch
            );
            return false;
        }
        true
    }

    #[allow(clippy::too_many_arguments)]
    fn take_commands(
        &mut self,
        from: MemberId,
        header: PacketHeader,
        batch: CommandBatch,
        via_control: bool,
        cmd_inbox: &mut crate::ClientCommandInbox,
        action_inbox: &mut crate::ClientActionInbox,
        reliable: Option<&mut crate::ReliableEventHub>,
    ) {
        if !self.packet_is_current(from, &header, "commands") {
            return;
        }
        let bootstrap_lane_present = self.bootstrap.is_some();
        let Some(peer) = self.peers.get_mut(&header.connection) else {
            return;
        };
        if !peer.feed.admits_gameplay(bootstrap_lane_present) {
            return;
        }
        let Ok(client) = self
            .connections
            .resolve(header.connection, batch.claimed_client)
            .map(ClientId)
        else {
            return;
        };
        peer.feed.in_ack = header.sequence;
        if let Some(reliable) = reliable {
            reliable.ack(client, batch.reliable_ack);
        }
        for (seq, cmd) in batch.cmds {
            let mut sample = batch
                .samples
                .iter()
                .find(|(sample_seq, _)| *sample_seq == seq)
                .map(|(_, sample)| *sample);
            if let Some(claim) = sample.as_mut()
                && claim.quality.claims_history()
                && !peer.feed.has_sent_both(claim.left, claim.right)
            {
                // The shooter names two snapshots it never received; its history claim is
                // worthless, so it is marked as unusable.
                claim.alpha = f32::NAN;
            }
            cmd_inbox.push(client, Some(seq), cmd, sample);
        }
        if via_control {
            for action in batch.actions {
                action_inbox.push_from_peer(client, action);
            }
        }
    }

    fn take_snapshot_ack(&mut self, from: MemberId, header: PacketHeader, snapshot_seq: u32) {
        if !self.packet_is_current(from, &header, "snapshot ack") {
            return;
        }
        if let Some(peer) = self.peers.get_mut(&header.connection) {
            peer.feed.in_ack = header.sequence;
            let _ = peer.feed.baseline.ack(snapshot_seq);
        }
    }

    fn apply_admission_acks(&mut self) {
        let Some(lane) = &self.bootstrap else {
            return;
        };
        let live = lane.epoch();
        for ack in lane.take_acks() {
            self.confirm_admission(ack, live);
        }
    }

    /// A member says it applied a bootstrap offer. If it names the offer that is open, the
    /// member is in.
    fn confirm_admission(&mut self, ack: BootstrapAck, live: u32) {
        if !epoch_applies(ack.epoch, live) {
            diag::warn!(
                Net,
                "authority ignored bootstrap applied for epoch {} (live {live})",
                ack.epoch
            );
            return;
        }
        if self.member_on(ack.connection) != Some(ack.member_id) {
            diag::warn!(
                Net,
                "authority ignored bootstrap applied: connection {:?} does not map to member {}",
                ack.connection,
                ack.member_id
            );
            return;
        }
        let Some(peer) = self.peers.get_mut(&ack.connection) else {
            diag::warn!(
                Net,
                "authority ignored bootstrap applied: no peer on {:?}",
                ack.connection
            );
            return;
        };
        let named = OfferKey {
            bootstrap_id: ack.bootstrap_id,
            snapshot_seq: ack.snapshot_seq,
            epoch: ack.epoch,
        };
        let first_commit = match peer.feed.admission.judge(named) {
            Verdict::Ignore => {
                diag::warn!(
                    Net,
                    "authority ignored bootstrap applied {} for member {}: no matching offer",
                    ack.bootstrap_id,
                    ack.member_id
                );
                return;
            }
            Verdict::RepeatEnter => false,
            Verdict::Commit => {
                peer.feed.admission = Admission::Committed {
                    bootstrap_id: ack.bootstrap_id,
                };
                let _ = peer.feed.baseline.ack(ack.snapshot_seq);
                true
            }
        };
        self.committed.push(CommittedAdmission {
            member_id: ack.member_id,
            epoch: ack.epoch,
            bootstrap_id: ack.bootstrap_id,
            connection_id: ack.connection.0,
            first_commit,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transport::protocol::{ConnectionId, HandshakeHello};
    use crate::transport::relay::{RelayMailbox, UdpAuthorityHub};

    fn member(byte: u8) -> MemberId {
        MemberId([byte; 16])
    }

    fn hub_with(peer: MemberId) -> (UdpAuthorityHub, ConnectionId) {
        let hello = HandshakeHello::current(Default::default());
        let mut hub = UdpAuthorityHub::relay(hello, 1, RelayMailbox::with_default_cap());
        hub.reconcile_relay_membership(&[peer], member(0));
        let conn = hub.connection_of(peer).unwrap();
        (hub, conn)
    }

    fn header(conn: ConnectionId, sequence: u32) -> PacketHeader {
        PacketHeader {
            connection: conn,
            sequence,
            ack: 0,
            epoch: 0,
        }
    }

    fn commands(conn: ConnectionId, sequence: u32, cmd_seq: u32) -> Vec<u8> {
        ClientPacket::Commands {
            header: header(conn, sequence),
            claimed_client: 0,
            cmds: vec![(CmdSeq(cmd_seq), playerstate_t6::game::UserCmd::default())],
            samples: Vec::new(),
            actions: Vec::new(),
            reliable_ack: 0,
        }
        .to_bytes()
    }

    fn run_ingress(hub: &mut UdpAuthorityHub) -> crate::ClientCommandInbox {
        let mut cmds = crate::ClientCommandInbox::default();
        let mut actions = crate::ClientActionInbox::default();
        hub.ingress(&mut cmds, &mut actions, None, None).unwrap();
        cmds
    }

    fn queued_for(hub: &UdpAuthorityHub, cmds: &crate::ClientCommandInbox, who: MemberId) -> usize {
        cmds.pending(hub.client_of_member(who).unwrap())
    }

    #[test]
    fn commands_from_the_owning_member_reach_the_inbox() {
        let (mut hub, conn) = hub_with(member(2));
        hub.mailbox()
            .push_inbound(member(2), commands(conn, 5, 1))
            .unwrap();
        let cmds = run_ingress(&mut hub);
        assert_eq!(queued_for(&hub, &cmds, member(2)), 1);
        assert_eq!(hub.peers[&conn].feed.in_ack, 5);
    }

    #[test]
    fn commands_under_someone_elses_connection_are_dropped() {
        let (mut hub, conn) = hub_with(member(2));
        hub.mailbox()
            .push_inbound(member(7), commands(conn, 5, 1))
            .unwrap();
        let cmds = run_ingress(&mut hub);
        assert_eq!(queued_for(&hub, &cmds, member(2)), 0);
        assert_eq!(hub.peers[&conn].feed.in_ack, 0);
    }

    #[test]
    fn garbage_is_skipped_without_stopping_the_rest() {
        let (mut hub, conn) = hub_with(member(2));
        let mail = hub.mailbox();
        mail.push_inbound(member(2), vec![0xFF, 0xFF, 0xFF]).unwrap();
        mail.push_inbound(member(2), commands(conn, 1, 1)).unwrap();
        let cmds = run_ingress(&mut hub);
        assert_eq!(queued_for(&hub, &cmds, member(2)), 1);
    }

    #[test]
    fn a_snapshot_ack_moves_the_peers_ack_counter() {
        let (mut hub, conn) = hub_with(member(2));
        let bytes = ClientPacket::SnapshotAck {
            header: header(conn, 9),
            snapshot_seq: 3,
        }
        .to_bytes();
        hub.mailbox().push_inbound(member(2), bytes).unwrap();
        run_ingress(&mut hub);
        assert_eq!(hub.peers[&conn].feed.in_ack, 9);
    }

    #[test]
    fn a_stale_epoch_packet_is_ignored() {
        let (mut hub, conn) = hub_with(member(2));
        let mut stale = header(conn, 4);
        stale.epoch = 99;
        let bytes = ClientPacket::SnapshotAck {
            header: stale,
            snapshot_seq: 1,
        }
        .to_bytes();
        hub.mailbox().push_inbound(member(2), bytes).unwrap();
        run_ingress(&mut hub);
        assert_eq!(hub.peers[&conn].feed.in_ack, 0);
    }

    #[test]
    fn confirming_the_open_offer_commits_the_member_once() {
        let (mut hub, conn) = hub_with(member(2));
        let offer = OfferKey {
            bootstrap_id: 1,
            snapshot_seq: 4,
            epoch: 7,
        };
        hub.peers.get_mut(&conn).unwrap().feed.admission = Admission::Pending(offer);
        let ack = |id, seq| BootstrapAck {
            member_id: member(2),
            connection: conn,
            bootstrap_id: id,
            snapshot_seq: seq,
            epoch: 7,
        };

        hub.confirm_admission(ack(1, 4), 7);
        hub.confirm_admission(ack(1, 4), 7);
        hub.confirm_admission(ack(2, 4), 7);

        let committed = hub.take_committed_admissions();
        assert_eq!(committed.len(), 2);
        assert!(committed[0].first_commit);
        assert!(!committed[1].first_commit);
        assert!(hub.peers[&conn].feed.admission.is_committed());
    }

    #[test]
    fn a_confirmation_from_the_wrong_member_changes_nothing() {
        let (mut hub, conn) = hub_with(member(2));
        let offer = OfferKey {
            bootstrap_id: 1,
            snapshot_seq: 4,
            epoch: 7,
        };
        hub.peers.get_mut(&conn).unwrap().feed.admission = Admission::Pending(offer);
        hub.confirm_admission(
            BootstrapAck {
                member_id: member(8),
                connection: conn,
                bootstrap_id: 1,
                snapshot_seq: 4,
                epoch: 7,
            },
            7,
        );
        assert!(hub.take_committed_admissions().is_empty());
        assert!(!hub.peers[&conn].feed.admission.is_committed());
    }
}
