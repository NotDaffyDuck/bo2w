use std::collections::VecDeque;

use master_protocol::RoomOccupantId;
use sim::Tick;

use super::entry_gate::Admission;
use super::mailbox::{ForwardingInbox, send_refused};
use crate::GuaranteedEnvelope;
use crate::net_links::peer_baseline::ConfirmedReferenceTable;
use crate::net_links::snapshot_diff::FrameDiffWriter;
use crate::net_links::datagrams::{LinkNumber, DatagramPreamble, HostToPlayerDatagram};
use crate::net_links::once_queue::is_newer_seq;
use crate::net_links::send_fault::UdpTransmitFailure;

/// How many receipt_confirmed snapshots a peer's baseline table keeps.
const BASELINES_KEPT: usize = 32;

/// How many of the most recent send ticks are remembered per peer.
const SENT_TICKS_KEPT: usize = 32;

/// Everything the authority tracks about one remote member.
#[derive(Debug)]
pub(super) struct Peer {
    pub(super) member: RoomOccupantId,

    /// The id the next bootstrap offer to this peer goes out under. Never zero.
    next_offer_id: u32,

    pub(super) feed: Feed,
}

/// The per-peer state of the snapshot stream and of the reliable control stream.
#[derive(Debug)]
pub(super) struct Feed {
    pub(super) baseline: ConfirmedReferenceTable,
    pub(super) encoder: FrameDiffWriter,
    pub(super) next_snapshot_seq: u32,
    pub(super) next_outgoing_number: u32,
    pub(super) incoming_receipt: u32,

    /// When the last full (baseline-less) snapshot went out.
    pub(super) latest_complete_state: Option<Tick>,

    recent_send_ticks: VecDeque<Tick>,
    pub(super) admission: Admission,

    /// The newest reliable row already queued on the control lane.
    control_cursor: Option<u16>,
    control_drops_told: u32,
}

impl Peer {
    pub(super) fn new(member: RoomOccupantId) -> Self {
        Self {
            member,
            next_offer_id: 1,
            feed: Feed::new(),
        }
    }

    /// Hands out the next bootstrap offer id, skipping zero when the counter wraps.
    pub(super) fn take_offer_id(&mut self) -> u32 {
        let id = self.next_offer_id;
        self.next_offer_id = id.wrapping_add(1).max(1);
        id
    }

    /// Back to the state of a peer that has just been enrolled.
    pub(super) fn restart(&mut self) {
        self.next_offer_id = 1;
        self.feed = Feed::new();
    }

    /// Queues the reliable rows this peer has not been given yet on the control lane.
    ///
    /// The control lane is ordered and reliable, so each row is queued once; the rows stay in
    /// the event hub until the peer acknowledges them at the application level.
    pub(super) fn queue_control(
        &mut self,
        mailbox: &ForwardingInbox,
        link: LinkNumber,
        epoch: u32,
        payload: GuaranteedEnvelope,
    ) -> Result<(), UdpTransmitFailure> {
        let Some(payload) = self.feed.unsent_control(payload) else {
            return Ok(());
        };
        let cursor = payload
            .entries
            .last()
            .map(|(row_no, _)| *row_no)
            .or(self.feed.control_cursor);
        let drops_told = payload.evicted_stale_count;
        let packet = HostToPlayerDatagram::OwedBatch {
            header: DatagramPreamble {
                connection: link,
                packet_no: 0,
                confirm_receipt: 0,
                round_era: epoch,
            },
            payload,
        };
        let wire_bytes = packet.wire_bytes();
        mailbox
            .enqueue_control_outgoing(self.member, wire_bytes)
            .map_err(send_refused)?;
        self.feed.control_cursor = cursor;
        self.feed.control_drops_told = drops_told;
        Ok(())
    }
}

impl Feed {
    fn new() -> Self {
        Self {
            baseline: ConfirmedReferenceTable::new(BASELINES_KEPT),
            encoder: FrameDiffWriter::new(),
            next_snapshot_seq: 1,
            next_outgoing_number: 0,
            incoming_receipt: 0,
            latest_complete_state: None,
            recent_send_ticks: VecDeque::new(),
            admission: Admission::NotYetPublished,
            control_cursor: None,
            control_drops_told: 0,
        }
    }

    /// Whether this peer's commands and receipts_batch are taken. Without a bootstrap lane every peer
    /// counts; with one, only peers whose admission was committed do.
    pub(super) fn admits_gameplay(&self, bootstrap_lane_present: bool) -> bool {
        !bootstrap_lane_present || self.admission.is_committed()
    }

    /// Remembers that a snapshot for `sent_at` went to this peer.
    pub(super) fn note_sent(&mut self, sent_at: Tick) {
        self.recent_send_ticks.push_back(sent_at);
        while self.recent_send_ticks.len() > SENT_TICKS_KEPT {
            self.recent_send_ticks.pop_front();
        }
    }

    /// Whether snapshots for both ticks were sent to this peer, so a shot that interpolates
    /// between them can be believed.
    pub(super) fn has_sent_both(&self, left: Tick, right: Tick) -> bool {
        self.recent_send_ticks.contains(&left) && self.recent_send_ticks.contains(&right)
    }

    /// Strips the rows that were already queued. `None` when nothing new is left to say.
    fn unsent_control(&self, mut payload: GuaranteedEnvelope) -> Option<GuaranteedEnvelope> {
        payload.entries.retain(|(seq, _)| {
            self.control_cursor
                .is_none_or(|cursor| is_newer_seq(*seq, cursor))
        });
        let nothing_new =
            payload.entries.is_empty() && payload.evicted_stale_count == self.control_drops_told;
        (!nothing_new).then_some(payload)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::GuaranteedEntry;

    fn member(byte: u8) -> RoomOccupantId {
        RoomOccupantId([byte; 16])
    }

    fn row(seq: u16) -> (u16, GuaranteedEntry) {
        (seq, GuaranteedEntry::RefusalText(format!("row {seq}")))
    }

    fn payload(seqs: &[u16], evicted_stale_count: u32) -> GuaranteedEnvelope {
        GuaranteedEnvelope {
            confirmed_up_to: 0,
            entries: seqs.iter().map(|&seq| row(seq)).collect(),
            evicted_stale_count,
        }
    }

    fn queued_row_seqs(mail: &ForwardingInbox) -> Vec<Vec<u16>> {
        mail.take_control_outbound()
            .into_iter()
            .map(|(_, bytes)| {
                match crate::net_links::datagrams::parse_host_datagram(
                    &bytes,
                    &crate::net_links::datagrams::NegotiatedCaps::fallback_bind_address(),
                )
                .expect("a control packet decodes")
                {
                    HostToPlayerDatagram::OwedBatch { payload: body, .. } => {
                        body.entries.iter().map(|(row_no, _)| *row_no).collect()
                    }
                    other => panic!("expected a control packet, got {other:?}"),
                }
            })
            .collect()
    }

    #[test]
    fn offer_ids_count_up_from_one() {
        let mut peer = Peer::new(member(1));
        assert_eq!(peer.take_offer_id(), 1);
        assert_eq!(peer.take_offer_id(), 2);
        assert_eq!(peer.take_offer_id(), 3);
    }

    #[test]
    fn offer_ids_skip_zero_when_the_counter_wraps() {
        let mut peer = Peer::new(member(1));
        peer.next_offer_id = u32::MAX;
        assert_eq!(peer.take_offer_id(), u32::MAX);
        assert_eq!(peer.take_offer_id(), 1);
    }

    #[test]
    fn restarting_gives_back_a_fresh_peer() {
        let mut peer = Peer::new(member(1));
        peer.take_offer_id();
        peer.take_offer_id();
        peer.feed.next_outgoing_number = 40;
        peer.feed.incoming_receipt = 39;
        peer.feed.next_snapshot_seq = 12;
        peer.feed.latest_complete_state = Some(Tick(5));
        peer.feed.control_cursor = Some(8);
        peer.feed.note_sent(Tick(5));

        peer.restart();

        assert_eq!(peer.member, member(1));
        assert_eq!(peer.take_offer_id(), 1);
        assert_eq!(peer.feed.next_outgoing_number, 0);
        assert_eq!(peer.feed.incoming_receipt, 0);
        assert_eq!(peer.feed.next_snapshot_seq, 1);
        assert_eq!(peer.feed.latest_complete_state, None);
        assert_eq!(peer.feed.control_cursor, None);
        assert!(!peer.feed.has_sent_both(Tick(5), Tick(5)));
    }

    #[test]
    fn gameplay_is_open_to_everyone_without_a_bootstrap_lane() {
        let feed = Feed::new();
        assert!(feed.admits_gameplay(false));
        assert!(!feed.admits_gameplay(true));
    }

    #[test]
    fn with_a_lane_only_a_committed_peer_plays() {
        let mut feed = Feed::new();
        feed.admission = Admission::Committed { offer_no: 1 };
        assert!(feed.admits_gameplay(true));
    }

    #[test]
    fn only_the_latest_send_ticks_are_remembered() {
        let mut feed = Feed::new();
        for tick in 0..40 {
            feed.note_sent(Tick(tick));
        }
        assert!(!feed.has_sent_both(Tick(0), Tick(39)));
        assert!(!feed.has_sent_both(Tick(7), Tick(39)));
        assert!(feed.has_sent_both(Tick(8), Tick(39)));
    }

    #[test]
    fn an_interpolated_shot_needs_both_of_its_ticks() {
        let mut feed = Feed::new();
        feed.note_sent(Tick(10));
        assert!(!feed.has_sent_both(Tick(10), Tick(11)));
        feed.note_sent(Tick(11));
        assert!(feed.has_sent_both(Tick(10), Tick(11)));
    }

    #[test]
    fn a_first_payload_goes_through_whole() {
        let feed = Feed::new();
        let kept = feed.unsent_control(payload(&[1, 2, 3], 0)).unwrap();
        assert_eq!(kept.entries.len(), 3);
    }

    #[test]
    fn rows_already_queued_are_stripped() {
        let mut feed = Feed::new();
        feed.control_cursor = Some(2);
        let kept = feed.unsent_control(payload(&[1, 2, 3, 4], 0)).unwrap();
        let seqs: Vec<u16> = kept.entries.iter().map(|(seq, _)| *seq).collect();
        assert_eq!(seqs, vec![3, 4]);
    }

    #[test]
    fn nothing_new_and_nothing_dropped_means_no_packet() {
        let mut feed = Feed::new();
        feed.control_cursor = Some(3);
        assert!(feed.unsent_control(payload(&[2, 3], 0)).is_none());
        assert!(feed.unsent_control(payload(&[], 0)).is_none());
    }

    #[test]
    fn a_new_drop_count_is_worth_a_packet_with_no_rows() {
        let mut feed = Feed::new();
        feed.control_cursor = Some(3);
        feed.control_drops_told = 1;
        assert!(feed.unsent_control(payload(&[], 1)).is_none());
        let kept = feed.unsent_control(payload(&[], 2)).unwrap();
        assert!(kept.entries.is_empty());
        assert_eq!(kept.evicted_stale_count, 2);
    }

    #[test]
    fn row_numbers_that_wrap_around_still_count_as_newer() {
        let mut feed = Feed::new();
        feed.control_cursor = Some(u16::MAX);
        let kept = feed.unsent_control(payload(&[u16::MAX, 0, 1], 0)).unwrap();
        let seqs: Vec<u16> = kept.entries.iter().map(|(seq, _)| *seq).collect();
        assert_eq!(seqs, vec![0, 1]);
    }

    #[test]
    fn queued_control_rows_are_not_queued_twice() {
        let mail = ForwardingInbox::standard_capacity();
        let mut peer = Peer::new(member(3));
        let conn = LinkNumber(1);

        peer.queue_control(&mail, conn, 5, payload(&[1, 2], 0)).unwrap();
        peer.queue_control(&mail, conn, 5, payload(&[1, 2, 3], 0)).unwrap();
        peer.queue_control(&mail, conn, 5, payload(&[1, 2, 3], 0)).unwrap();

        assert_eq!(queued_row_seqs(&mail), vec![vec![1, 2], vec![3]]);
    }

    #[test]
    fn control_packets_name_the_peer_the_connection_and_the_epoch() {
        let mail = ForwardingInbox::standard_capacity();
        let mut peer = Peer::new(member(9));
        peer.queue_control(&mail, LinkNumber(77), 4, payload(&[1], 0))
            .unwrap();
        let parcels = mail.take_control_outbound();
        assert_eq!(parcels.len(), 1);
        assert_eq!(parcels[0].0, member(9));
        match crate::net_links::datagrams::parse_host_datagram(
            &parcels[0].1,
            &crate::net_links::datagrams::NegotiatedCaps::fallback_bind_address(),
        )
        .unwrap()
        {
            HostToPlayerDatagram::OwedBatch { header, .. } => {
                assert_eq!(header.connection, LinkNumber(77));
                assert_eq!(header.round_era, 4);
            }
            other => panic!("expected a control packet, got {other:?}"),
        }
    }

    #[test]
    fn a_refused_control_packet_is_retried_with_the_same_rows() {
        let mail = ForwardingInbox::new(1);
        mail.enqueue_control_outgoing(member(9), vec![0]).unwrap();
        let mut peer = Peer::new(member(9));
        let conn = LinkNumber(1);

        assert!(peer.queue_control(&mail, conn, 1, payload(&[1], 0)).is_err());
        assert_eq!(peer.feed.control_cursor, None);

        mail.take_control_outbound();
        peer.queue_control(&mail, conn, 1, payload(&[1], 0)).unwrap();
        assert_eq!(queued_row_seqs(&mail), vec![vec![1]]);
    }
}
