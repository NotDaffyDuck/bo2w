//! Putting fragments back into packets, with limits so a peer cannot make us hold unbounded
//! partial data.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use super::header::{Header, Refusal, VERSION};
use super::{FRAGMENT_HEADER_BYTES, SplitJoinFailure, SPLIT_PIECE_LIMIT, err, payload_room};

/// Packets that may be partly received at once.
const DATAGRAM_BACKLOG_CAP: usize = 32;

/// Bytes, summed over the partly received packets' full sizes.
const REASSEMBLY_BUFFER_LIMIT: usize = 2 * 1024 * 1024;

/// A partly received packet is dropped this long after its first fragment arrived.
const PIECE_LIFETIME: Duration = Duration::from_secs(1);

struct Partial {
    full_len: usize,
    first_seen: Instant,
    parts: Vec<Option<Vec<u8>>>,
    filled: usize,
}

impl core::fmt::Debug for Partial {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let fragments = self.parts.len();
        f.debug_struct("PartialPacket").field("total_len", &self.full_len).field("received", &self.filled).field("fragments", &fragments).finish()
    }
}

#[derive(Debug)]
pub struct FragmentStitcher {
    room: usize,
    packet_cap: usize,
    open: HashMap<u32, Partial>,
}

impl FragmentStitcher {
    pub fn new(mtu: usize, packet_cap: usize) -> Self {
        Self { room: payload_room(mtu), packet_cap, open: HashMap::new() }
    }

    /// Whether no packet is partly received.
    pub fn is_empty(&self) -> bool {
        self.open.is_empty()
    }

    /// Take in one datagram. Returns the whole packet when this was its last missing fragment;
    /// a repeat of a fragment already held is ignored. A fragment that contradicts the packet it
    /// belongs to drops that packet.
    pub fn ingest(&mut self, datagram: &[u8]) -> Result<Option<Vec<u8>>, SplitJoinFailure> {
        let header = match Header::read(datagram) {
            Ok(parsed) => parsed,
            Err(Refusal::GarbledRequest) => return Err(err("malformed fragment header")),
            Err(Refusal::OtherBuild { version, reserved }) => {
                return Err(err(format!(
                    "fragment header version {version} (reserved {reserved}), ours is {VERSION} — \
                     the sender runs a different build"
                )));
            }
        };
        let index = usize::from(header.index);
        let count = usize::from(header.count);
        let whole_len = header.total_len as usize;
        if whole_len == 0 || whole_len > self.packet_cap {
            return Err(err(format!(
                "declared packet size {whole_len} exceeds the cap of {}",
                self.packet_cap
            )));
        }
        if count == 0
            || count > SPLIT_PIECE_LIMIT
            || count != whole_len.div_ceil(self.room)
            || index >= count
        {
            return Err(err("fragment range is inconsistent"));
        }
        // Every fragment is full except the last, which holds what is left.
        let expected = if index + 1 == count {
            whole_len - self.room * index
        } else {
            self.room
        };
        let payload = &datagram[FRAGMENT_HEADER_BYTES..];
        if payload.len() != expected {
            return Err(err("fragment length is inconsistent"));
        }

        let id = header.packet_id;
        self.expire();
        if !self.open.contains_key(&id) {
            self.make_room_for(id, whole_len, count)?;
        }
        let partial = self.open.entry(id).or_insert_with(|| Partial { full_len: whole_len, first_seen: Instant::now(), parts: vec![None; count], filled: 0 });
        if partial.full_len != whole_len || partial.parts.len() != count {
            self.open.remove(&id);
            return Err(err("fragment disagrees with earlier ones on packet shape"));
        }
        match &partial.parts[index] {
            Some(held) if held.as_slice() != payload => {
                self.open.remove(&id);
                return Err(err("fragment repeats an index with different bytes"));
            }
            Some(_) => return Ok(None),
            None => {
                partial.parts[index] = Some(payload.to_vec());
                partial.filled += 1;
            }
        }
        if partial.filled != count {
            return Ok(None);
        }
        let done = self.open.remove(&id).expect("completed packet exists");
        let mut packet = Vec::with_capacity(done.full_len);
        for part in done.parts.into_iter().flatten() {
            packet.extend_from_slice(&part);
        }
        Ok(Some(packet))
    }

    /// Forget packets whose first fragment arrived too long ago.
    fn expire(&mut self) {
        self.open.retain(|_, partial| partial.first_seen.elapsed() < PIECE_LIFETIME);
    }

    /// Refuse a new packet when holding it would pass either limit.
    fn make_room_for(&self, id: u32, whole_len: usize, count: usize) -> Result<(), SplitJoinFailure> {
        let held: usize = self.open.values().map(|partial| partial.full_len).sum();
        if self.open.len() >= DATAGRAM_BACKLOG_CAP || held + whole_len > REASSEMBLY_BUFFER_LIMIT {
            return Err(err(format!(
                "reassembly capacity exceeded: packet {id} of {whole_len} B \
                 ({count} fragments) with {} packet(s) / {held} B pending \
                 (max {DATAGRAM_BACKLOG_CAP} / {REASSEMBLY_BUFFER_LIMIT})",
                self.open.len()
            )));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::super::DatagramSplitter;
    use super::*;

    const DATAGRAM: usize = FRAGMENT_HEADER_BYTES + 10;

    fn cut(packet: &[u8]) -> Vec<Vec<u8>> {
        DatagramSplitter::new(DATAGRAM, 1 << 20).split_up(packet).unwrap()
    }

    fn catch() -> FragmentStitcher {
        FragmentStitcher::new(DATAGRAM, 1 << 20)
    }

    #[test]
    fn fragments_in_any_order_make_the_packet() {
        let packet: Vec<u8> = (0..35).collect();
        let parts = cut(&packet);
        assert_eq!(parts.len(), 4);
        let mut re = catch();
        for index in [2, 0, 3] {
            assert_eq!(re.ingest(&parts[index]), Ok(None));
            assert!(!re.is_empty());
        }
        assert_eq!(re.ingest(&parts[1]), Ok(Some(packet)));
        assert!(re.is_empty());
    }

    #[test]
    fn a_repeated_fragment_changes_nothing() {
        let parts = cut(&[7; 25]);
        let mut re = catch();
        assert_eq!(re.ingest(&parts[0]), Ok(None));
        assert_eq!(re.ingest(&parts[0]), Ok(None));
        assert_eq!(re.ingest(&parts[1]), Ok(None));
        assert_eq!(re.ingest(&parts[2]), Ok(Some(vec![7; 25])));
    }

    #[test]
    fn a_fragment_that_disagrees_with_the_one_before_drops_the_packet() {
        let parts = cut(&[1; 25]);
        let mut changed = parts[0].clone();
        *changed.last_mut().unwrap() = 99;
        let mut re = catch();
        re.ingest(&parts[0]).unwrap();
        assert_eq!(re.ingest(&changed).unwrap_err().0, "fragment repeats an index with different bytes");
        assert!(re.is_empty());
    }

    #[test]
    fn a_fragment_claiming_a_different_size_for_the_packet_drops_it() {
        // Same id (0), same index (0), but a total length that also needs three fragments.
        let long = cut(&[1; 25]);
        let other = cut(&[1; 21]);
        let mut re = catch();
        re.ingest(&long[0]).unwrap();
        assert_eq!(re.ingest(&other[0]).unwrap_err().0, "fragment disagrees with earlier ones on packet shape");
        assert!(re.is_empty());
    }

    #[test]
    fn bad_headers_say_why() {
        let parts = cut(&[1; 25]);
        let mut re = catch();
        assert_eq!(re.ingest(&parts[0][..10]).unwrap_err().0, "malformed fragment header");
        let mut other_build = parts[0].clone();
        other_build[2] = 9;
        assert_eq!(
            re.ingest(&other_build).unwrap_err().0,
            "fragment header version 9 (reserved 0), ours is 2 — the sender runs a different build"
        );
    }

    #[test]
    fn lengths_and_counts_must_add_up() {
        let parts = cut(&[1; 25]);
        let mut re = catch();
        let mut short = parts[1].clone();
        short.pop();
        assert_eq!(re.ingest(&short).unwrap_err().0, "fragment length is inconsistent");
        let mut last_too_long = parts[2].clone();
        last_too_long.push(0);
        assert_eq!(re.ingest(&last_too_long).unwrap_err().0, "fragment length is inconsistent");
        let mut wrong_count = parts[0].clone();
        wrong_count[6] = 4;
        assert_eq!(re.ingest(&wrong_count).unwrap_err().0, "fragment range is inconsistent");
        let mut past_the_end = parts[0].clone();
        past_the_end[4] = 3;
        assert_eq!(re.ingest(&past_the_end).unwrap_err().0, "fragment range is inconsistent");
        assert!(re.is_empty());
    }

    #[test]
    fn a_packet_longer_than_the_limit_is_refused() {
        let parts = cut(&[1; 25]);
        let mut small = FragmentStitcher::new(DATAGRAM, 24);
        assert_eq!(
            small.ingest(&parts[0]).unwrap_err().0,
            "declared packet size 25 exceeds the cap of 24"
        );
    }

    #[test]
    fn too_many_partial_packets_are_refused_until_one_finishes() {
        let mut splitter = DatagramSplitter::new(DATAGRAM, 1 << 20);
        let mut re = catch();
        for _ in 0..DATAGRAM_BACKLOG_CAP {
            let parts = splitter.split_up(&[2; 20]).unwrap();
            re.ingest(&parts[0]).unwrap();
        }
        let extra = splitter.split_up(&[2; 20]).unwrap();
        let error = re.ingest(&extra[0]).unwrap_err().0;
        assert!(error.starts_with("reassembly capacity exceeded: packet 32 of 20 B"), "{error}");
        // A packet already held still completes.
        let first = DatagramSplitter::new(DATAGRAM, 1 << 20).split_up(&[2; 20]).unwrap();
        assert_eq!(re.ingest(&first[1]), Ok(Some(vec![2; 20])));
    }

    #[test]
    fn the_byte_budget_counts_whole_packets() {
        // Wide datagrams keep the near-limit packet under the fragment-count cap.
        let wide = FRAGMENT_HEADER_BYTES + 1000;
        let mut re = FragmentStitcher::new(wide, REASSEMBLY_BUFFER_LIMIT);
        let mut splitter = DatagramSplitter::new(wide, REASSEMBLY_BUFFER_LIMIT);
        let big = splitter.split_up(&vec![0; REASSEMBLY_BUFFER_LIMIT - 5]).unwrap();
        re.ingest(&big[0]).unwrap();
        let next = splitter.split_up(&[0; 6]).unwrap();
        let error = re.ingest(&next[0]).unwrap_err().0;
        assert!(error.starts_with("reassembly capacity exceeded"), "{error}");
        // Five more bytes fit exactly.
        let fits = splitter.split_up(&[0; 5]).unwrap();
        assert_eq!(re.ingest(&fits[0]), Ok(Some(vec![0; 5])));
    }

    #[test]
    fn a_stale_partial_packet_is_forgotten_on_the_next_fragment() {
        let parts = cut(&[3; 25]);
        let mut re = catch();
        re.ingest(&parts[0]).unwrap();
        re.open.get_mut(&0).unwrap().first_seen =
            Instant::now().checked_sub(PIECE_LIFETIME * 2).unwrap();
        // The packet restarts from scratch: fragment 1 alone does not complete it.
        assert_eq!(re.ingest(&parts[1]), Ok(None));
        assert_eq!(re.open[&0].filled, 1);
    }
}
