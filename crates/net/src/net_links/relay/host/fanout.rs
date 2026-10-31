//! Building and sending the per-member snapshot stream.

use std::sync::Arc;

use sim::{ClientId, Snapshot, Tick, StepInputs};

use super::HostUdpSwitchboard;
use crate::client::run_ahead::InputCommandNumber;
use crate::net_links::match_handoff::{TransferChannelState, TransferExchange};
use crate::net_links::tick_frames::{Frame, frame_for_confirmed_tick};
use crate::net_links::datagrams::{LinkNumber, DatagramPreamble, HostToPlayerDatagram};
use crate::net_links::relay::entry_gate::{Admission, OfferKey};
use crate::net_links::relay::peer::{Feed, Peer};
use crate::net_links::send_fault::UdpTransmitFailure;

/// A peer that is sent no baseline-less snapshot more often than this many ticks, unless its
/// baseline went missing.
const COMPLETE_STATE_REPEAT_TICKS: u32 = 10;

/// What one fanout round shares between all peers.
struct Round<'a, F> {
    input: &'a StepInputs,
    world: &'a Snapshot,
    receipts_batch: &'a [(ClientId, InputCommandNumber)],
    view_for: F,
    host_sound_triggers: Option<&'a mut crate::StagedHostSounds>,
    player_cards: Option<&'a mut crate::StagedCardUpdate>,
    game_notifies: Option<&'a mut crate::StagedAnnouncements>,
    reliable: Option<&'a crate::GuaranteedEventRouter>,
    scoreboard_refresh_pending: bool,
    era: u32,
    chan: Option<Arc<TransferChannelState>>,
}

impl HostUdpSwitchboard {
    pub fn send_to_all(
        &mut self,
        input: &StepInputs,
        world: &Snapshot,
        receipts_batch: &[(ClientId, InputCommandNumber)],
    ) -> Result<(), UdpTransmitFailure> {
        self.broadcast_per_seat(
            input,
            world,
            receipts_batch,
            |_, current| current.clone(),
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
    pub fn broadcast_per_seat(
        &mut self,
        input: &StepInputs,
        world: &Snapshot,
        receipts_batch: &[(ClientId, InputCommandNumber)],
        for_peer: impl FnMut(ClientId, &Snapshot) -> Snapshot,
        pending_svc: Option<&mut crate::StagedHostSounds>,
        pending_playercard: Option<&mut crate::StagedCardUpdate>,
        pending_gamenotify: Option<&mut crate::StagedAnnouncements>,
        reliable: Option<&crate::GuaranteedEventRouter>,
        scoreboard_refresh_pending: bool,
    ) -> Result<(), UdpTransmitFailure> {
        let mut round = Round {
            input,
            world,
            receipts_batch,
            view_for: for_peer,
            host_sound_triggers: pending_svc,
            player_cards: pending_playercard,
            game_notifies: pending_gamenotify,
            reliable,
            scoreboard_refresh_pending,
            era: self.running_generation(),
            chan: self.offer_lane.clone(),
        };
        let mut final_err = None;
        let all_links: Vec<LinkNumber> = self.remotes.keys().copied().collect();
        for link in all_links {
            self.serve(link, &mut round, &mut final_err);
        }
        final_err.map_or(Ok(()), Err)
    }

    fn serve<F>(
        &mut self,
        link: LinkNumber,
        round: &mut Round<'_, F>,
        final_err: &mut Option<UdpTransmitFailure>,
    ) where
        F: FnMut(ClientId, &Snapshot) -> Snapshot,
    {
        let Some(peer) = self.remotes.get_mut(&link) else {
            return;
        };
        let dest = peer.member;
        let Ok(raw_slot) = self.links.trusted_slot(link, 0) else {
            return;
        };
        // A peer that is still loading content gets no snapshot: the bootstrap's clock must
        // start when its offer can actually go out.
        if let Some(chan) = &round.chan
            && !chan
                .host_map_ready
                .lock()
                .expect("bootstrap readiness poisoned")
                .contains(&dest)
        {
            return;
        }
        let seat = ClientId(raw_slot);

        if peer.feed.admits_gameplay(round.chan.is_some())
            && let Some(reliable) = round.reliable
            && let Err(error) =
                peer.queue_control(&self.mailbox, link, round.era, reliable.owed_envelope(seat))
        {
            *final_err = Some(error);
        }

        let Some(base_no) = choose_baseline(
            &mut peer.feed,
            round.world.tick,
            round.chan.as_deref(),
            dest,
        ) else {
            return;
        };

        let (frame, view) = build_frame(&mut peer.feed, seat, base_no, round);
        let snap_no = peer.feed.next_snapshot_seq;
        peer.feed.next_snapshot_seq = snap_no.wrapping_add(1);
        peer.feed.baseline.keep_frame(snap_no, view);
        peer.feed.next_outgoing_number = peer.feed.next_outgoing_number.wrapping_add(1);
        let datagram = HostToPlayerDatagram::Snapshot {
            header: DatagramPreamble {
                connection: link,
                packet_no: peer.feed.next_outgoing_number,
                confirm_receipt: peer.feed.incoming_receipt,
                round_era: round.era,
            },
            reference_frame_number: base_no,
            frame_number_on_wire: snap_no,
            body: frame.wire_bytes(),
        };

        let joiner_pending = base_no == 0 && !peer.feed.admission.is_committed();
        if let (true, Some(chan)) = (joiner_pending, &round.chan) {
            offer_bootstrap(peer, chan, link, round.world.tick, snap_no, &datagram);
            return;
        }
        match self.send_datagram(dest, datagram.wire_bytes()) {
            Ok(()) => {
                if let Some(peer) = self.remotes.get_mut(&link) {
                    peer.feed.note_sent(round.world.tick);
                }
            }
            Err(error) => *final_err = Some(error),
        }
    }
}

/// Decides what the next snapshot for this peer is encoded against. `None` means the peer is
/// skipped this tick.
fn choose_baseline(
    feed: &mut Feed,
    at_tick: Tick,
    chan: Option<&TransferChannelState>,
    dest: master_protocol::RoomOccupantId,
) -> Option<u32> {
    let must_resync = feed.baseline.forget_absent_receipt();
    if must_resync {
        diag::warn!(
            WireLink,
            "acked baseline missing for {dest:?}: sending a complete snapshot"
        );
    }
    let base_no = feed.baseline.reference_number_for_packing();
    if !feed.baseline.can_diff_from(base_no) {
        diag::warn!(
            WireLink,
            "delta encode refused for {dest:?} against baseline {base_no}"
        );
        return None;
    }
    if base_no == 0 {
        let recent_full = feed
            .latest_complete_state
            .is_some_and(|last| at_tick.0.saturating_sub(last.0) < COMPLETE_STATE_REPEAT_TICKS);
        if !must_resync && recent_full {
            return None;
        }
        // Until the match has an epoch there is nothing a joiner could be admitted into.
        if chan.is_some_and(|chan| chan.era() == 0) {
            return None;
        }
    }
    Some(base_no)
}

/// Encodes this tick for one peer. Returns the frame and the view of the world that was
/// encoded, which becomes the baseline for later snapshots.
fn build_frame<F>(
    feed: &mut Feed,
    seat: ClientId,
    base_no: u32,
    round: &mut Round<'_, F>,
) -> (Frame, Snapshot)
where
    F: FnMut(ClientId, &Snapshot) -> Snapshot,
{
    let mut writer = std::mem::take(&mut feed.encoder);
    if base_no == 0 {
        feed.latest_complete_state = Some(round.world.tick);
        writer.start_over();
    } else if let Some(baseline) = feed.baseline.reference_state_for_packing() {
        writer.accept_reference_frame(baseline);
    }
    let mine: Vec<(ClientId, InputCommandNumber)> = round
        .receipts_batch
        .iter()
        .copied()
        .filter(|(who, _)| *who == seat)
        .collect();
    let view = (round.view_for)(seat, round.world);
    let mut frame = frame_for_confirmed_tick(&mut writer, round.input, &view, mine);
    feed.encoder = writer;

    if let Some(staged) = round.host_sound_triggers.as_mut() {
        frame.host_sound_triggers = staged.hand_to(seat);
    }
    if let Some(staged) = round.player_cards.as_mut() {
        let (slots, menus, hud_notes) = staged.hand_to(seat);
        frame.host_nameplate_entries = slots;
        frame.host_menu_openings = menus;
        frame.flash_rows = hud_notes;
    }
    if let Some(staged) = round.game_notifies.as_mut() {
        frame.alert_rows = staged.hand_to(seat);
    }
    // Reliable events travel on the control lane; the journal must not repeat them.
    frame
        .meta_rows
        .journal
        .retain(|record| !sim::occurrence_needs_delivery(&record.event));
    if round.scoreboard_refresh_pending {
        frame.board_text = Some(crate::compose_standings_from_state(&view));
    }
    (frame, view)
}

/// Sends a joining peer its snapshot as a bootstrap offer, unless an offer for the current
/// epoch is already waiting for an answer.
fn offer_bootstrap(
    peer: &mut Peer,
    chan: &TransferChannelState,
    link: LinkNumber,
    at_tick: Tick,
    snap_no: u32,
    datagram: &HostToPlayerDatagram,
) {
    let dest = peer.member;
    let epoch = chan.era();
    if peer.feed.admission.pending_epoch() == Some(epoch) {
        return;
    }
    let offer_no = peer.take_offer_id();
    let txn = match TransferExchange::from_world_state(
        epoch,
        offer_no,
        at_tick.0,
        snap_no,
        link,
        datagram.wire_bytes(),
    ) {
        Ok(txn) => txn,
        Err(error) => {
            diag::warn!(WireLink, "bootstrap offer encode failed for {dest:?}: {error}");
            return;
        }
    };
    peer.feed.admission = Admission::Pending(OfferKey {
        offer_no,
        snap_no,
        era: epoch,
    });
    diag::info!(
        WireLink,
        "bootstrap-io session=host event=bootstrap offer encoded peer={dest:?} epoch={epoch} bootstrap_id={offer_no} snapshot_seq={snap_no} len={}",
        txn.proposal_blob.len()
    );
    match chan.hand_to_worker(Some(dest), txn.proposal_blob) {
        Ok(()) => peer.feed.note_sent(at_tick),
        Err(error) => diag::warn!(WireLink, "bootstrap offer dropped for {dest:?}: {error}"),
    }
}

#[cfg(test)]
mod tests {
    use master_protocol::RoomOccupantId;

    use super::*;
    use crate::net_links::datagrams::{PlayerToHostDatagram, JoinGreeting, parse_host_datagram};
    use crate::net_links::relay::{ForwardingInbox, HostUdpSwitchboard};

    fn member(byte: u8) -> RoomOccupantId {
        RoomOccupantId([byte; 16])
    }

    fn hub_with(others_in: &[RoomOccupantId]) -> HostUdpSwitchboard {
        let hello = JoinGreeting::current(Default::default());
        let mut hub = HostUdpSwitchboard::relay(hello, 1, ForwardingInbox::standard_capacity());
        hub.sync_room_members(others_in, member(0));
        hub
    }

    fn transmit_step(hub: &mut HostUdpSwitchboard, at_tick: u32) {
        let world = Snapshot::unpublished(Tick(at_tick));
        hub.send_to_all(&StepInputs::default(), &world, &[]).unwrap();
    }

    /// The (reference_frame_number, frame_number_on_wire) of every snapshot packet waiting for `who`.
    fn sent_to(hub: &HostUdpSwitchboard, who: RoomOccupantId) -> Vec<(u32, u32)> {
        let post = hub.mailbox();
        let (mine, others): (Vec<_>, Vec<_>) = post
            .take_outbound()
            .into_iter()
            .partition(|(member, _)| *member == who);
        for (who, parcel) in others {
            post.queue_departure(who, parcel).unwrap();
        }
        mine.into_iter()
            .map(|(_, raw)| match parse_host_datagram(&raw, &hub.negotiated).unwrap() {
                HostToPlayerDatagram::Snapshot {
                    reference_frame_number: base_no,
                    frame_number_on_wire: snap_no,
                    ..
                } => (base_no, snap_no),
                other => panic!("expected snapshot datagram, got {other:?}"),
            })
            .collect()
    }

    #[test]
    fn a_new_peer_gets_a_full_snapshot() {
        let mut hub = hub_with(&[member(2)]);
        transmit_step(&mut hub, 100);
        assert_eq!(sent_to(&hub, member(2)), vec![(0, 1)]);
    }

    #[test]
    fn every_peer_is_served() {
        let mut hub = hub_with(&[member(2), member(3)]);
        transmit_step(&mut hub, 100);
        assert_eq!(sent_to(&hub, member(2)).len(), 1);
        assert_eq!(sent_to(&hub, member(3)).len(), 1);
    }

    #[test]
    fn an_unacknowledged_peer_is_not_flooded_with_full_snapshots() {
        let mut hub = hub_with(&[member(2)]);
        transmit_step(&mut hub, 100);
        sent_to(&hub, member(2));
        for at_tick in 101..110 {
            transmit_step(&mut hub, at_tick);
        }
        assert!(sent_to(&hub, member(2)).is_empty());
        transmit_step(&mut hub, 110);
        assert_eq!(sent_to(&hub, member(2)).len(), 1);
    }

    #[test]
    fn after_an_ack_the_next_snapshot_is_a_delta_against_it() {
        let mut hub = hub_with(&[member(2)]);
        transmit_step(&mut hub, 100);
        sent_to(&hub, member(2));

        let link = hub.connection_of(member(2)).unwrap();
        let receipt_msg = PlayerToHostDatagram::FrameReceipt {
            header: DatagramPreamble {
                connection: link,
                packet_no: 1,
                confirm_receipt: 0,
                round_era: 0,
            },
            frame_number_on_wire: 1,
        };
        hub.mailbox().take_arrival(member(2), receipt_msg.wire_bytes()).unwrap();
        let mut cmds = crate::PlayerMoveIntake::default();
        let mut act_sink = crate::PlayerActionIntake::default();
        hub.take_datagram(&mut cmds, &mut act_sink, None, None).unwrap();

        transmit_step(&mut hub, 101);
        assert_eq!(sent_to(&hub, member(2)), vec![(1, 2)]);
    }

    #[test]
    fn packets_carry_the_connection_and_a_rising_sequence() {
        let mut hub = hub_with(&[member(2)]);
        let link = hub.connection_of(member(2)).unwrap();
        transmit_step(&mut hub, 100);
        transmit_step(&mut hub, 110);
        let sequences: Vec<u32> = hub
            .mailbox()
            .take_outbound()
            .into_iter()
            .map(|(_, raw)| match parse_host_datagram(&raw, &hub.negotiated).unwrap() {
                HostToPlayerDatagram::Snapshot { header, .. } => {
                    assert_eq!(header.connection, link);
                    header.packet_no
                }
                other => panic!("expected snapshot datagram, got {other:?}"),
            })
            .collect();
        assert_eq!(sequences, vec![1, 2]);
    }

    #[test]
    fn a_full_mailbox_is_reported_but_does_not_stop_the_other_peers() {
        let hello = JoinGreeting::current(Default::default());
        let mut hub = HostUdpSwitchboard::relay(hello, 1, ForwardingInbox::new(1));
        hub.sync_room_members(&[member(2), member(3)], member(0));
        let world = Snapshot::unpublished(Tick(5));
        let result = hub.send_to_all(&StepInputs::default(), &world, &[]);
        assert!(result.is_err(), "the second one does not fit");
        assert_eq!(hub.mailbox().take_outbound().len(), 1);
    }

    #[test]
    fn the_caller_chooses_each_peers_view() {
        let mut hub = hub_with(&[member(2), member(3)]);
        let world = Snapshot::unpublished(Tick(5));
        let mut asked = Vec::new();
        hub.broadcast_per_seat(
            &StepInputs::default(),
            &world,
            &[],
            |seat, live| {
                asked.push(seat.0);
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
        hub.release_occupant(member(2));
        transmit_step(&mut hub, 100);
        assert!(hub.mailbox().take_outbound().is_empty());
    }
}
