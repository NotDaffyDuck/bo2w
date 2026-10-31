//! The bootstrap exchange between host and joiner: the host offers a world snapshot, the joiner
//! answers that it applied it. This module holds the two wire messages and the lane that carries
//! them between the netcode and the lobby worker thread.

use std::collections::{HashSet, VecDeque};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

use crate::net_links::datagrams::{LinkNumber, DATAGRAM_BYTE_CAP};
use crate::net_links::byte_codec::{PacketDecodeFault, ByteCursorReader, ByteSinkWriter};

/// Every bootstrap message opens with these two bytes, then the version, then one tag byte.
const MAGIC_PREFIX: [u8; 2] = [b'B', b'S'];
const WIRE_VERSION: u8 = 1;
const TAG_OFFER: u8 = 1;
const TAG_APPLIED: u8 = 2;

/// How many messages may wait in either direction between the netcode and the worker.
pub const SETUP_BACKLOG_CAP: usize = 8;

/// One acknowledgement can wait per member, so the queue is as long as a session is full.
pub const MAX_PENDING_ADMISSION_ACKS: usize = master_protocol::ROOM_OCCUPANT_LIMIT as usize;

const ACKS_FULL: &str = "bootstrap ack queue is full";

/// A host's offer, already encoded for the wire, with the numbers that identify it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TransferExchange {
    pub era: u32,
    pub handoff_id: u32,
    pub step_marker_b: u32,
    pub frame_number_on_wire: u32,
    pub link: LinkNumber,
    pub proposal_blob: Vec<u8>,
}

impl TransferExchange {
    /// Wraps `snapshot` as an offer and keeps its encoded form.
    pub fn from_world_state(
        era: u32,
        handoff_id: u32,
        step_marker_b: u32,
        frame_number_on_wire: u32,
        link: LinkNumber,
        snapshot: Vec<u8>,
    ) -> Result<Self, PacketDecodeFault> {
        pack_bootstrap(&InitialStateTransferMsg::Offer {
            era,
            handoff_id,
            step_marker_b,
            frame_number_on_wire,
            link,
            chunk: snapshot,
        })
        .map(|proposal_blob| Self {
            era,
            handoff_id,
            step_marker_b,
            frame_number_on_wire,
            link,
            proposal_blob,
        })
    }
}

/// What travels on the bootstrap channel.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InitialStateTransferMsg {
    /// Host to joiner: the snapshot packet to load.
    Offer {
        era: u32,
        handoff_id: u32,
        step_marker_b: u32,
        frame_number_on_wire: u32,
        link: LinkNumber,
        chunk: Vec<u8>,
    },
    /// Joiner to host: the offer with these numbers has been applied.
    TakenIntoEffect {
        era: u32,
        handoff_id: u32,
        step_marker_b: u32,
        frame_number_on_wire: u32,
        link: LinkNumber,
    },
}

/// A message belongs to the live session only when it carries the live, nonzero epoch.
pub fn same_generation(carried: u32, now_gen: u32) -> bool {
    matches!(carried, 1..) && carried == now_gen
}

/// Bytes ahead of the four ids: magic, version, tag.
const LEAD_LEN: usize = MAGIC_PREFIX.len() + 2;

const BAD_VERSION: &str = "unknown bootstrap version";
const BAD_TAG: &str = "unknown bootstrap tag";
const LEFTOVER: &str = "trailing bootstrap bytes";
const OVERSIZED: &str = "bootstrap packet exceeds max_packet_bytes";

/// The numbers every message carries, in wire order, plus the connection. A connection goes
/// out as two u32 words, high word first.
struct Ids {
    numbers: [u32; 4],
    link: LinkNumber,
}

impl Ids {
    fn of(message: &InitialStateTransferMsg) -> Self {
        let (InitialStateTransferMsg::Offer { era, handoff_id, step_marker_b, frame_number_on_wire, link, .. }
        | InitialStateTransferMsg::TakenIntoEffect { era, handoff_id, step_marker_b, frame_number_on_wire, link }) =
            message;
        Self {
            numbers: [*era, *handoff_id, *step_marker_b, *frame_number_on_wire],
            link: *link,
        }
    }

    fn put(&self, out: &mut ByteSinkWriter) {
        let id = self.link.0;
        let high = (id >> 32) as u32;
        let low = id as u32;
        for word in self.numbers.iter().copied().chain([high, low]) {
            out.put_u32(word);
        }
    }

    fn take(input: &mut ByteCursorReader<'_>) -> Result<Self, PacketDecodeFault> {
        let mut read = [0u32; 6];
        for word in &mut read {
            *word = input.read_word32()?;
        }
        let [era, handoff_id, step_marker_b, frame_number_on_wire, high, low] = read;
        Ok(Self {
            numbers: [era, handoff_id, step_marker_b, frame_number_on_wire],
            link: LinkNumber((u64::from(high) << 32) | u64::from(low)),
        })
    }

    fn applied(self) -> InitialStateTransferMsg {
        let [era, handoff_id, step_marker_b, frame_number_on_wire] = self.numbers;
        InitialStateTransferMsg::TakenIntoEffect { era, handoff_id, step_marker_b, frame_number_on_wire, link: self.link }
    }

    fn offer(self, packet: Vec<u8>) -> InitialStateTransferMsg {
        let [era, handoff_id, step_marker_b, frame_number_on_wire] = self.numbers;
        InitialStateTransferMsg::Offer { era, handoff_id, step_marker_b, frame_number_on_wire, link: self.link, chunk: packet }
    }
}

/// An offered packet's length prefix, if the packet fits under the packet limit.
fn fitting_len(len: usize) -> Option<u32> {
    u32::try_from(len).ok().filter(|&n| n <= DATAGRAM_BYTE_CAP)
}

/// Writes `message` as `BS`, the version, a tag and its fields.
pub fn pack_bootstrap(message: &InitialStateTransferMsg) -> Result<Vec<u8>, PacketDecodeFault> {
    let (tag, packet) = match message {
        InitialStateTransferMsg::Offer { chunk: packet, .. } => {
            let len = fitting_len(packet.len()).ok_or(PacketDecodeFault::GarbledRequest(OVERSIZED))?;
            (TAG_OFFER, Some((len, packet)))
        }
        InitialStateTransferMsg::TakenIntoEffect { .. } => (TAG_APPLIED, None),
    };
    let mut out = ByteSinkWriter::with_capacity(LEAD_LEN + 24);
    out.append_raw_slice(&MAGIC_PREFIX);
    out.append_raw_slice(&[WIRE_VERSION, tag]);
    Ids::of(message).put(&mut out);
    if let Some((len, body)) = packet {
        out.put_u32(len);
        out.append_raw_slice(body);
    }
    Ok(out.into_bytes())
}

/// Reads one message. Bytes that do not start with the bootstrap magic are not ours: `Ok(None)`.
pub fn parse_setup_message(bytes: &[u8]) -> Result<Option<InitialStateTransferMsg>, PacketDecodeFault> {
    if !bytes.starts_with(&MAGIC_PREFIX) {
        return Ok(None);
    }
    let mut input = ByteCursorReader::new(&bytes[MAGIC_PREFIX.len()..]);
    if input.get_u8()? != WIRE_VERSION {
        return Err(PacketDecodeFault::GarbledRequest(BAD_VERSION));
    }
    let kind = input.get_u8()?;
    if kind != TAG_OFFER && kind != TAG_APPLIED {
        return Err(PacketDecodeFault::GarbledRequest(BAD_TAG));
    }
    let ids = Ids::take(&mut input)?;
    let message = if kind == TAG_APPLIED {
        ids.applied()
    } else {
        let len = input.read_word32()?;
        if len > DATAGRAM_BYTE_CAP {
            return Err(PacketDecodeFault::GarbledRequest(OVERSIZED));
        }
        let mut body = vec![0u8; len as usize];
        input.copy_out(&mut body)?;
        ids.offer(body)
    };
    match input.unread_len() {
        0 => Ok(Some(message)),
        _ => Err(PacketDecodeFault::GarbledRequest(LEFTOVER)),
    }
}

/// Bytes the worker received, with the member they came from if the worker knew.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TransferIntake {
    pub member_id: Option<master_protocol::RoomOccupantId>,
    pub coded_bytes: Vec<u8>,
}

/// A joiner's report that it applied the offer with these numbers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TransferReceiptMsg {
    pub member_id: master_protocol::RoomOccupantId,
    pub era: u32,
    pub handoff_id: u32,
    pub frame_number_on_wire: u32,
    pub link: LinkNumber,
}

fn guard<T>(queue: &Mutex<T>) -> MutexGuard<'_, T> {
    queue.lock().expect("bootstrap lane poisoned")
}

/// Appends `item` unless `cap` entries already wait, in which case `refusal` comes back.
fn queue_capped<T>(queue: &Mutex<VecDeque<T>>, cap: usize, item: T, refusal: &'static str) -> Result<(), &'static str> {
    let mut waiting = guard(queue);
    (waiting.len() < cap)
        .then(|| waiting.push_back(item))
        .ok_or(refusal)
}

/// Empties a queue and hands back what it held, oldest first.
fn drain_all<T>(queue: &Mutex<VecDeque<T>>) -> Vec<T> {
    guard(queue).drain(..).collect()
}

/// The queues and counters shared by the netcode and the lobby worker. A new epoch empties
/// everything: whatever was in flight belonged to the old session.
#[derive(Default)]
pub struct TransferChannelState {
    pub(super) host_map_ready: Mutex<HashSet<master_protocol::RoomOccupantId>>,
    outgoing: Mutex<VecDeque<(Option<master_protocol::RoomOccupantId>, Vec<u8>)>>,
    inbound: Mutex<VecDeque<TransferIntake>>,
    pending_acks: Mutex<VecDeque<TransferReceiptMsg>>,
    running_generation: AtomicU32,
    entered_boot: AtomicU32,
    entered_cli: AtomicU32,
}

impl TransferChannelState {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    /// Moves to `epoch`; if that is a change, every queue and counter starts over.
    pub fn assign_generation(&self, era: u32) {
        if self.running_generation.swap(era, Ordering::AcqRel) == era {
            return;
        }
        self.host_map_ready
            .lock()
            .expect("bootstrap readiness poisoned")
            .clear();
        for counter in [&self.entered_boot, &self.entered_cli] {
            counter.store(0, Ordering::Release);
        }
        guard(&self.outgoing).clear();
        guard(&self.inbound).clear();
        guard(&self.pending_acks).clear();
    }

    pub fn era(&self) -> u32 {
        self.running_generation.load(Ordering::Acquire)
    }

    /// Records which bootstrap and client the local player has entered.
    pub fn record_entry(&self, handoff_id: u32, player_slot_no: u32) {
        // Client first, bootstrap last: a reader that sees the bootstrap also sees its client.
        self.entered_cli.store(player_slot_no, Ordering::Release);
        self.entered_boot.store(handoff_id, Ordering::Release);
    }

    pub fn joined_setup_stage(&self) -> u32 {
        self.entered_boot.load(Ordering::Acquire)
    }

    pub fn joined_receiver_role(&self) -> u32 {
        self.entered_cli.load(Ordering::Acquire)
    }

    pub fn hand_to_worker(&self, peer: Option<master_protocol::RoomOccupantId>, bytes: Vec<u8>) -> Result<(), &'static str> {
        queue_capped(
            &self.outgoing,
            SETUP_BACKLOG_CAP,
            (peer, bytes),
            "bootstrap to-worker queue is full",
        )
    }

    pub fn take_to_worker(&self) -> Vec<(Option<master_protocol::RoomOccupantId>, Vec<u8>)> {
        drain_all(&self.outgoing)
    }

    pub fn enqueue_from_helper(
        &self,
        member_id: Option<master_protocol::RoomOccupantId>,
        coded_bytes: Vec<u8>,
    ) -> Result<(), &'static str> {
        queue_capped(
            &self.inbound,
            SETUP_BACKLOG_CAP,
            TransferIntake { member_id, coded_bytes },
            "bootstrap from-worker queue is full",
        )
    }

    pub fn take_from_worker(&self) -> Vec<TransferIntake> {
        drain_all(&self.inbound)
    }

    /// Queues `confirm_receipt`; a member's newer confirm_receipt replaces its older one instead of taking a second slot.
    pub fn queue_receipt(&self, confirm_receipt: TransferReceiptMsg) -> Result<(), &'static str> {
        let mut waiting = guard(&self.pending_acks);
        match waiting.iter().position(|queued| queued.member_id == confirm_receipt.member_id) {
            Some(slot) => waiting[slot] = confirm_receipt,
            None if waiting.len() < MAX_PENDING_ADMISSION_ACKS => waiting.push_back(confirm_receipt),
            None => return Err(ACKS_FULL),
        }
        Ok(())
    }

    pub fn take_acks(&self) -> Vec<TransferReceiptMsg> {
        drain_all(&self.pending_acks)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// epoch 7, bootstrap 3, tick 900, snapshot 41, connection 2:5.
    fn sample_ids() -> Ids {
        Ids {
            numbers: [7, 3, 900, 41],
            link: LinkNumber(0x0000_0002_0000_0005),
        }
    }

    fn offer(packet: Vec<u8>) -> InitialStateTransferMsg {
        sample_ids().offer(packet)
    }

    fn applied() -> InitialStateTransferMsg {
        sample_ids().applied()
    }

    fn member(id: u8) -> master_protocol::RoomOccupantId {
        master_protocol::RoomOccupantId([id; 16])
    }

    fn confirm_receipt(id: u8, era: u32) -> TransferReceiptMsg {
        TransferReceiptMsg {
            member_id: member(id),
            era,
            handoff_id: 1,
            frame_number_on_wire: 2,
            link: LinkNumber(9),
        }
    }

    #[test]
    fn an_offer_has_a_fixed_layout() {
        let bytes = pack_bootstrap(&offer(vec![0xAA, 0xBB])).unwrap();
        let mut expected = vec![b'B', b'S', 1, 1];
        for word in [7_u32, 3, 900, 41, 2, 5, 2] {
            expected.extend_from_slice(&word.to_le_bytes());
        }
        expected.extend_from_slice(&[0xAA, 0xBB]);
        assert_eq!(bytes, expected);
    }

    #[test]
    fn both_messages_round_trip() {
        for message in [offer(vec![1, 2, 3, 4, 5]), offer(Vec::new()), applied()] {
            let bytes = pack_bootstrap(&message).unwrap();
            assert_eq!(parse_setup_message(&bytes).unwrap(), Some(message));
        }
    }

    #[test]
    fn a_connection_id_keeps_both_halves() {
        let message = InitialStateTransferMsg::TakenIntoEffect {
            era: 1,
            handoff_id: 1,
            step_marker_b: 1,
            frame_number_on_wire: 1,
            link: LinkNumber(0xDEAD_BEEF_0000_0001),
        };
        let bytes = pack_bootstrap(&message).unwrap();
        assert_eq!(parse_setup_message(&bytes).unwrap(), Some(message));
    }

    #[test]
    fn other_traffic_is_not_ours() {
        assert_eq!(parse_setup_message(b"XS\x01\x01").unwrap(), None);
        assert_eq!(parse_setup_message(b"").unwrap(), None);
        assert_eq!(parse_setup_message(b"B").unwrap(), None);
    }

    #[test]
    fn a_wrong_version_or_tag_is_refused() {
        let mut bytes = pack_bootstrap(&applied()).unwrap();
        bytes[2] = 2;
        assert!(matches!(
            parse_setup_message(&bytes),
            Err(PacketDecodeFault::GarbledRequest("unknown bootstrap version"))
        ));
        bytes[2] = 1;
        bytes[3] = 9;
        assert!(matches!(
            parse_setup_message(&bytes),
            Err(PacketDecodeFault::GarbledRequest("unknown bootstrap tag"))
        ));
    }

    #[test]
    fn extra_bytes_after_a_message_are_refused() {
        let mut bytes = pack_bootstrap(&applied()).unwrap();
        bytes.push(0);
        assert!(matches!(
            parse_setup_message(&bytes),
            Err(PacketDecodeFault::GarbledRequest("trailing bootstrap bytes"))
        ));
    }

    #[test]
    fn a_cut_off_message_is_an_error() {
        let bytes = pack_bootstrap(&offer(vec![1, 2, 3])).unwrap();
        assert!(parse_setup_message(&bytes[..bytes.len() - 1]).is_err());
        assert!(parse_setup_message(&bytes[..8]).is_err());
    }

    #[test]
    fn a_packet_over_the_limit_is_refused_both_ways() {
        let too_big = vec![0_u8; DATAGRAM_BYTE_CAP as usize + 1];
        assert!(matches!(
            pack_bootstrap(&offer(too_big)),
            Err(PacketDecodeFault::GarbledRequest("bootstrap packet exceeds max_packet_bytes"))
        ));

        let mut bytes = pack_bootstrap(&offer(Vec::new())).unwrap();
        let at = bytes.len() - 4;
        bytes[at..].copy_from_slice(&(DATAGRAM_BYTE_CAP + 1).to_le_bytes());
        assert!(matches!(
            parse_setup_message(&bytes),
            Err(PacketDecodeFault::GarbledRequest("bootstrap packet exceeds max_packet_bytes"))
        ));
    }

    #[test]
    fn a_transaction_keeps_the_encoded_offer() {
        let transaction =
            TransferExchange::from_world_state(7, 3, 900, 41, LinkNumber(0x0000_0002_0000_0005), vec![9, 9])
                .unwrap();
        assert_eq!(transaction.proposal_blob, pack_bootstrap(&offer(vec![9, 9])).unwrap());
        assert_eq!(transaction.frame_number_on_wire, 41);
    }

    #[test]
    fn epoch_zero_never_applies() {
        assert!(!same_generation(0, 0));
        assert!(!same_generation(3, 4));
        assert!(same_generation(4, 4));
    }

    #[test]
    fn the_to_worker_queue_stops_at_its_limit() {
        let lane = TransferChannelState::new();
        for n in 0..SETUP_BACKLOG_CAP {
            lane.hand_to_worker(None, vec![n as u8]).unwrap();
        }
        assert_eq!(lane.hand_to_worker(None, vec![0]), Err("bootstrap to-worker queue is full"));
        let taken = lane.take_to_worker();
        assert_eq!(taken.len(), SETUP_BACKLOG_CAP);
        assert_eq!(taken[0].1, vec![0]);
        assert!(lane.take_to_worker().is_empty());
    }

    #[test]
    fn the_from_worker_queue_stops_at_its_limit_and_keeps_the_sender() {
        let lane = TransferChannelState::new();
        lane.enqueue_from_helper(Some(member(4)), vec![1]).unwrap();
        for _ in 1..SETUP_BACKLOG_CAP {
            lane.enqueue_from_helper(None, vec![2]).unwrap();
        }
        assert_eq!(lane.enqueue_from_helper(None, vec![3]), Err("bootstrap from-worker queue is full"));
        let taken = lane.take_from_worker();
        assert_eq!(taken[0], TransferIntake { member_id: Some(member(4)), coded_bytes: vec![1] });
    }

    #[test]
    fn a_members_newer_ack_replaces_its_older_one() {
        let lane = TransferChannelState::new();
        lane.queue_receipt(confirm_receipt(1, 5)).unwrap();
        lane.queue_receipt(confirm_receipt(2, 5)).unwrap();
        lane.queue_receipt(confirm_receipt(1, 6)).unwrap();
        let taken = lane.take_acks();
        assert_eq!(taken.len(), 2);
        assert_eq!(taken[0].era, 6);
        assert_eq!(taken[1].member_id, member(2));
    }

    #[test]
    fn the_ack_queue_holds_one_per_member_slot() {
        let lane = TransferChannelState::new();
        for id in 0..MAX_PENDING_ADMISSION_ACKS {
            lane.queue_receipt(confirm_receipt(id as u8, 1)).unwrap();
        }
        assert_eq!(lane.queue_receipt(confirm_receipt(MAX_PENDING_ADMISSION_ACKS as u8, 1)), Err("bootstrap ack queue is full"));
        lane.queue_receipt(confirm_receipt(0, 2)).unwrap();
    }

    #[test]
    fn a_new_epoch_clears_everything_and_the_same_one_changes_nothing() {
        let lane = TransferChannelState::new();
        lane.assign_generation(1);
        lane.hand_to_worker(None, vec![1]).unwrap();
        lane.enqueue_from_helper(None, vec![2]).unwrap();
        lane.queue_receipt(confirm_receipt(1, 1)).unwrap();
        lane.record_entry(8, 9);
        lane.host_map_ready.lock().unwrap().insert(member(1));

        lane.assign_generation(1);
        assert_eq!((lane.joined_setup_stage(), lane.joined_receiver_role()), (8, 9));
        assert_eq!(lane.take_to_worker().len(), 1);

        lane.hand_to_worker(None, vec![1]).unwrap();
        lane.assign_generation(2);
        assert_eq!(lane.era(), 2);
        assert_eq!((lane.joined_setup_stage(), lane.joined_receiver_role()), (0, 0));
        assert!(lane.take_to_worker().is_empty());
        assert!(lane.take_from_worker().is_empty());
        assert!(lane.take_acks().is_empty());
        assert!(lane.host_map_ready.lock().unwrap().is_empty());
    }
}
