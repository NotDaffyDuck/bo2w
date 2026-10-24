//! Building and sending the per-member snapshot stream.

use std::sync::Arc;

use sim::{ClientId, Snapshot, Tick, TickInput};

use super::UdpAuthorityHub;
use crate::client::predict::CmdSeq;
use crate::transport::bootstrap::{BootstrapLane, BootstrapTransaction};
use crate::transport::frame::{Frame, frame_from_acked_tick};
use crate::transport::protocol::{ConnectionId, PacketHeader, ServerPacket};
use crate::transport::relay::admission::{Admission, OfferKey};
use crate::transport::relay::peer::{Feed, Peer};
use crate::transport::udp_socket::UdpSendError;

/// A peer that is sent no baseline-less snapshot more often than this many ticks, unless its
/// baseline went missing.
const FULL_SNAPSHOT_RESEND_TICKS: u32 = 10;

/// What one fanout round shares between all peers.
struct Round<'a, F> {
    input: &'a TickInput,
    snapshot: &'a Snapshot,
    acks: &'a [(ClientId, CmdSeq)],
    view_for: F,
    svc_sounds: Option<&'a mut crate::PendingSvcSounds>,
    player_cards: Option<&'a mut crate::PendingPlayerCard>,
    game_notifies: Option<&'a mut crate::PendingGameNotify>,
    reliable: Option<&'a crate::ReliableEventHub>,
    scores_due: bool,
    epoch: u32,
    lane: Option<Arc<BootstrapLane>>,
}

impl UdpAuthorityHub {
    pub fn fanout(
        &mut self,
        input: &TickInput,
        snapshot: &Snapshot,
        acks: &[(ClientId, CmdSeq)],
    ) -> Result<(), UdpSendError> {
        self.fanout_with_seats(
            input,
            snapshot,
            acks,
            |_, live| live.clone(),
            None,
            None,
            None,
            None,
            false,
        )
    }

    /// Sends every enrolled peer what it is due this tick. `for_peer` lets the caller show
    /// each peer its own view of the world (for instance with seats applied). A failure to
    /// hand one peer's packet to the mailbox does not stop the others; the last error is
    /// returned.
    #[allow(clippy::too_many_arguments)]
    pub fn fanout_with_seats(
        &mut self,
        input: &TickInput,
        snapshot: &Snapshot,
        acks: &[(ClientId, CmdSeq)],
        for_peer: impl FnMut(ClientId, &Snapshot) -> Snapshot,
        pending_svc: Option<&mut crate::PendingSvcSounds>,
        pending_playercard: Option<&mut crate::PendingPlayerCard>,
        pending_gamenotify: Option<&mut crate::PendingGameNotify>,
        reliable: Option<&crate::ReliableEventHub>,
        scores_due: bool,
    ) -> Result<(), UdpSendError> {
        let mut round = Round {
            input,
            snapshot,
            acks,
            view_for: for_peer,
            svc_sounds: pending_svc,
            player_cards: pending_playercard,
            game_notifies: pending_gamenotify,
            reliable,
            scores_due,
            epoch: self.live_epoch(),
            lane: self.bootstrap.clone(),
        };
        let mut last_error = None;
        let connections: Vec<ConnectionId> = self.peers.keys().copied().collect();
        for conn in connections {
            self.serve(conn, &mut round, &mut last_error);
        }
        last_error.map_or(Ok(()), Err)
    }

    fn serve<F>(
        &mut self,
        conn: ConnectionId,
        round: &mut Round<'_, F>,
        last_error: &mut Option<UdpSendError>,
    ) where
        F: FnMut(ClientId, &Snapshot) -> Snapshot,
    {
        let Some(peer) = self.peers.get_mut(&conn) else {
            return;
        };
        let target = peer.member;
        let Ok(client_number) = self.connections.resolve(conn, 0) else {
            return;
        };
        // A peer that is still loading content gets no snapshot: the bootstrap's clock must
        // start when its offer can actually go out.
        if let Some(lane) = &round.lane
            && !lane
                .host_map_ready
                .lock()
                .expect("bootstrap readiness poisoned")
                .contains(&target)
        {
            return;
        }
        let client = ClientId(client_number);

        if peer.feed.admits_gameplay(round.lane.is_some())
            && let Some(reliable) = round.reliable
            && let Err(error) =
                peer.queue_control(&self.mailbox, conn, round.epoch, reliable.payload(client))
        {
            *last_error = Some(error);
        }

        let Some(baseline_seq) = choose_baseline(
            &mut peer.feed,
            round.snapshot.tick,
            round.lane.as_deref(),
            target,
        ) else {
            return;
        };

        let (frame, view) = build_frame(&mut peer.feed, client, baseline_seq, round);
        let snapshot_seq = peer.feed.next_snapshot_seq;
        peer.feed.next_snapshot_seq = snapshot_seq.wrapping_add(1);
        peer.feed.baseline.remember(snapshot_seq, view);
        peer.feed.out_seq = peer.feed.out_seq.wrapping_add(1);
        let packet = ServerPacket::Snapshot {
            header: PacketHeader {
                connection: conn,
                sequence: peer.feed.out_seq,
                ack: peer.feed.in_ack,
                epoch: round.epoch,
            },
            baseline_seq,
            snapshot_seq,
            payload: frame.to_bytes(),
        };

        let still_joining = baseline_seq == 0 && !peer.feed.admission.is_committed();
        if let (true, Some(lane)) = (still_joining, &round.lane) {
            offer_bootstrap(peer, lane, conn, round.snapshot.tick, snapshot_seq, &packet);
            return;
        }
        match self.send_datagram(target, packet.to_bytes()) {
            Ok(()) => {
                if let Some(peer) = self.peers.get_mut(&conn) {
                    peer.feed.note_sent(round.snapshot.tick);
                }
            }
            Err(error) => *last_error = Some(error),
        }
    }
}

/// Decides what the next snapshot for this peer is encoded against. `None` means the peer is
/// skipped this tick.
fn choose_baseline(
    feed: &mut Feed,
    tick: Tick,
    lane: Option<&BootstrapLane>,
    target: master_protocol::MemberId,
) -> Option<u32> {
    let resync = feed.baseline.clear_missing_ack();
    if resync {
        diag::warn!(
            Net,
            "acked baseline missing for {target:?}: sending a full snapshot"
        );
    }
    let baseline_seq = feed.baseline.baseline_seq_for_encode();
    if !feed.baseline.may_encode_against(baseline_seq) {
        diag::warn!(
            Net,
            "snapshot encode refused for {target:?} against baseline {baseline_seq}"
        );
        return None;
    }
    if baseline_seq == 0 {
        let sent_recently = feed
            .last_full
            .is_some_and(|last| tick.0.saturating_sub(last.0) < FULL_SNAPSHOT_RESEND_TICKS);
        if !resync && sent_recently {
            return None;
        }
        // Until the match has an epoch there is nothing a joiner could be admitted into.
        if lane.is_some_and(|lane| lane.epoch() == 0) {
            return None;
        }
    }
    Some(baseline_seq)
}

/// Encodes this tick for one peer. Returns the frame and the view of the world that was
/// encoded, which becomes the baseline for later snapshots.
fn build_frame<F>(
    feed: &mut Feed,
    client: ClientId,
    baseline_seq: u32,
    round: &mut Round<'_, F>,
) -> (Frame, Snapshot)
where
    F: FnMut(ClientId, &Snapshot) -> Snapshot,
{
    let mut encoder = std::mem::take(&mut feed.encoder);
    if baseline_seq == 0 {
        feed.last_full = Some(round.snapshot.tick);
        encoder.reset();
    } else if let Some(baseline) = feed.baseline.baseline_for_encode() {
        encoder.adopt_baseline(baseline);
    }
    let own_acks: Vec<(ClientId, CmdSeq)> = round
        .acks
        .iter()
        .copied()
        .filter(|(id, _)| *id == client)
        .collect();
    let view = (round.view_for)(client, round.snapshot);
    let mut frame = frame_from_acked_tick(&mut encoder, round.input, &view, own_acks);
    feed.encoder = encoder;

    if let Some(pending) = round.svc_sounds.as_mut() {
        frame.svc_sounds = pending.take_for(client);
    }
    if let Some(pending) = round.player_cards.as_mut() {
        let (slots, menus, splashes) = pending.take_for(client);
        frame.svc_card_slots = slots;
        frame.svc_open_menus = menus;
        frame.svc_hud_splashes = splashes;
    }
    if let Some(pending) = round.game_notifies.as_mut() {
        frame.svc_game_notifies = pending.take_for(client);
    }
    // Reliable events travel on the control lane; the journal must not repeat them.
    frame
        .snapshot_meta
        .journal
        .retain(|record| !sim::sim_event_is_reliable(&record.event));
    if round.scores_due {
        frame.svc_scores = Some(crate::format_scoreboard_from_snapshot(&view));
    }
    (frame, view)
}

/// Sends a joining peer its snapshot as a bootstrap offer, unless an offer for the current
/// epoch is already waiting for an answer.
fn offer_bootstrap(
    peer: &mut Peer,
    lane: &BootstrapLane,
    conn: ConnectionId,
    tick: Tick,
    snapshot_seq: u32,
    packet: &ServerPacket,
) {
    let target = peer.member;
    let epoch = lane.epoch();
    if peer.feed.admission.pending_epoch() == Some(epoch) {
        return;
    }
    let bootstrap_id = peer.take_offer_id();
    let txn = match BootstrapTransaction::from_snapshot(
        epoch,
        bootstrap_id,
        tick.0,
        snapshot_seq,
        conn,
        packet.to_bytes(),
    ) {
        Ok(txn) => txn,
        Err(error) => {
            diag::warn!(Net, "bootstrap offer encode failed for {target:?}: {error}");
            return;
        }
    };
    peer.feed.admission = Admission::Pending(OfferKey {
        bootstrap_id,
        snapshot_seq,
        epoch,
    });
    diag::info!(
        Net,
        "bootstrap-io session=host event=bootstrap offer encoded peer={target:?} epoch={epoch} bootstrap_id={bootstrap_id} snapshot_seq={snapshot_seq} bytes={}",
        txn.offer_bytes.len()
    );
    match lane.push_to_worker(Some(target), txn.offer_bytes) {
        Ok(()) => peer.feed.note_sent(tick),
        Err(error) => diag::warn!(Net, "bootstrap offer dropped for {target:?}: {error}"),
    }
}

#[cfg(test)]
mod tests {
    use master_protocol::MemberId;

    use super::*;
    use crate::transport::protocol::{ClientPacket, HandshakeHello, decode_server_packet};
    use crate::transport::relay::{RelayMailbox, UdpAuthorityHub};

    fn member(byte: u8) -> MemberId {
        MemberId([byte; 16])
    }

    fn hub_with(peers: &[MemberId]) -> UdpAuthorityHub {
        let hello = HandshakeHello::current(Default::default());
        let mut hub = UdpAuthorityHub::relay(hello, 1, RelayMailbox::with_default_cap());
        hub.reconcile_relay_membership(peers, member(0));
        hub
    }

    fn send_tick(hub: &mut UdpAuthorityHub, tick: u32) {
        let snapshot = Snapshot::unpublished(Tick(tick));
        hub.fanout(&TickInput::default(), &snapshot, &[]).unwrap();
    }

    /// The (baseline_seq, snapshot_seq) of every snapshot packet waiting for `who`.
    fn sent_to(hub: &UdpAuthorityHub, who: MemberId) -> Vec<(u32, u32)> {
        let mail = hub.mailbox();
        let (mine, others): (Vec<_>, Vec<_>) = mail
            .take_outbound()
            .into_iter()
            .partition(|(member, _)| *member == who);
        for (member, bytes) in others {
            mail.push_outbound(member, bytes).unwrap();
        }
        mine.into_iter()
            .map(|(_, bytes)| match decode_server_packet(&bytes, &hub.limits).unwrap() {
                ServerPacket::Snapshot {
                    baseline_seq,
                    snapshot_seq,
                    ..
                } => (baseline_seq, snapshot_seq),
                other => panic!("expected a snapshot, got {other:?}"),
            })
            .collect()
    }

    #[test]
    fn a_new_peer_gets_a_full_snapshot() {
        let mut hub = hub_with(&[member(2)]);
        send_tick(&mut hub, 100);
        assert_eq!(sent_to(&hub, member(2)), vec![(0, 1)]);
    }

    #[test]
    fn every_peer_is_served() {
        let mut hub = hub_with(&[member(2), member(3)]);
        send_tick(&mut hub, 100);
        assert_eq!(sent_to(&hub, member(2)).len(), 1);
        assert_eq!(sent_to(&hub, member(3)).len(), 1);
    }

    #[test]
    fn an_unacknowledged_peer_is_not_flooded_with_full_snapshots() {
        let mut hub = hub_with(&[member(2)]);
        send_tick(&mut hub, 100);
        sent_to(&hub, member(2));
        for tick in 101..110 {
            send_tick(&mut hub, tick);
        }
        assert!(sent_to(&hub, member(2)).is_empty());
        send_tick(&mut hub, 110);
        assert_eq!(sent_to(&hub, member(2)).len(), 1);
    }

    #[test]
    fn after_an_ack_the_next_snapshot_is_a_delta_against_it() {
        let mut hub = hub_with(&[member(2)]);
        send_tick(&mut hub, 100);
        sent_to(&hub, member(2));

        let conn = hub.connection_of(member(2)).unwrap();
        let ack = ClientPacket::SnapshotAck {
            header: PacketHeader {
                connection: conn,
                sequence: 1,
                ack: 0,
                epoch: 0,
            },
            snapshot_seq: 1,
        };
        hub.mailbox().push_inbound(member(2), ack.to_bytes()).unwrap();
        let mut cmds = crate::ClientCommandInbox::default();
        let mut actions = crate::ClientActionInbox::default();
        hub.ingress(&mut cmds, &mut actions, None, None).unwrap();

        send_tick(&mut hub, 101);
        assert_eq!(sent_to(&hub, member(2)), vec![(1, 2)]);
    }

    #[test]
    fn packets_carry_the_connection_and_a_rising_sequence() {
        let mut hub = hub_with(&[member(2)]);
        let conn = hub.connection_of(member(2)).unwrap();
        send_tick(&mut hub, 100);
        send_tick(&mut hub, 110);
        let sequences: Vec<u32> = hub
            .mailbox()
            .take_outbound()
            .into_iter()
            .map(|(_, bytes)| match decode_server_packet(&bytes, &hub.limits).unwrap() {
                ServerPacket::Snapshot { header, .. } => {
                    assert_eq!(header.connection, conn);
                    header.sequence
                }
                other => panic!("expected a snapshot, got {other:?}"),
            })
            .collect();
        assert_eq!(sequences, vec![1, 2]);
    }

    #[test]
    fn a_full_mailbox_is_reported_but_does_not_stop_the_other_peers() {
        let hello = HandshakeHello::current(Default::default());
        let mut hub = UdpAuthorityHub::relay(hello, 1, RelayMailbox::new(1));
        hub.reconcile_relay_membership(&[member(2), member(3)], member(0));
        let snapshot = Snapshot::unpublished(Tick(5));
        let result = hub.fanout(&TickInput::default(), &snapshot, &[]);
        assert!(result.is_err(), "the second packet does not fit");
        assert_eq!(hub.mailbox().take_outbound().len(), 1);
    }

    #[test]
    fn the_caller_chooses_each_peers_view() {
        let mut hub = hub_with(&[member(2), member(3)]);
        let snapshot = Snapshot::unpublished(Tick(5));
        let mut asked = Vec::new();
        hub.fanout_with_seats(
            &TickInput::default(),
            &snapshot,
            &[],
            |client, live| {
                asked.push(client.0);
                live.clone()
            },
            None,
            None,
            None,
            None,
            false,
        )
        .unwrap();
        asked.sort_unstable();
        asked.dedup();
        assert_eq!(asked.len(), 2);
    }

    #[test]
    fn a_retired_peer_gets_nothing() {
        let mut hub = hub_with(&[member(2)]);
        hub.retire_member(member(2));
        send_tick(&mut hub, 100);
        assert!(hub.mailbox().take_outbound().is_empty());
    }
}
