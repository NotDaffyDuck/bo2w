//! Cutting one packet into fragments.

use super::header::Header;
use super::{FRAGMENT_HEADER_BYTES, SplitJoinFailure, SPLIT_PIECE_LIMIT, err, payload_room};

/// Cuts packets for datagrams of a fixed size, numbering each packet so the far end can tell
/// fragments of different packets apart.
#[derive(Debug)]
pub struct DatagramSplitter {
    room: usize,
    packet_cap: usize,
    id_counter: u32,
}

impl DatagramSplitter {
    pub fn new(mtu: usize, packet_cap: usize) -> Self {
        Self {
            room: payload_room(mtu),
            packet_cap,
            id_counter: 0,
        }
    }

    /// One datagram per fragment, in order. A packet that is empty, longer than the limit, or
    /// needs more fragments than the count can hold is refused, and uses no packet id.
    pub fn split_up(&mut self, bytes: &[u8]) -> Result<Vec<Vec<u8>>, SplitJoinFailure> {
        if bytes.is_empty() || bytes.len() > self.packet_cap {
            return Err(err(format!(
                "packet size {} outside the allowed 1..={}",
                bytes.len(),
                self.packet_cap
            )));
        }
        let count = bytes.len().div_ceil(self.room);
        if count > SPLIT_PIECE_LIMIT {
            return Err(err(format!(
                "packet length {} needs {count} fragments (max {SPLIT_PIECE_LIMIT})",
                bytes.len()
            )));
        }
        let packet_id = self.id_counter;
        self.id_counter = packet_id.wrapping_add(1);

        let pieces = bytes
            .chunks(self.room)
            .enumerate()
            .map(|(nth, piece)| {
                let mut datagram = Vec::with_capacity(FRAGMENT_HEADER_BYTES + piece.len());
                Header {
                    index: nth as u16,
                    count: count as u16,
                    packet_id,
                    total_len: bytes.len() as u32,
                }
                .write(&mut datagram);
                datagram.extend_from_slice(piece);
                datagram
            })
            .collect();
        Ok(pieces)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_small_packet_is_one_fragment_with_its_header() {
        let mut splitter = DatagramSplitter::new(100, 1000);
        let parts = splitter.split_up(b"hello").unwrap();
        assert_eq!(parts.len(), 1);
        assert_eq!(parts[0].len(), FRAGMENT_HEADER_BYTES + 5);
        assert_eq!(&parts[0][FRAGMENT_HEADER_BYTES..], b"hello");
        assert_eq!(
            Header::read(&parts[0]),
            Ok(Header { index: 0, count: 1, packet_id: 0, total_len: 5 })
        );
    }

    #[test]
    fn a_big_packet_is_cut_into_full_fragments_and_a_remainder() {
        let mut splitter = DatagramSplitter::new(FRAGMENT_HEADER_BYTES + 10, 1000);
        let packet: Vec<u8> = (0..25).collect();
        let parts = splitter.split_up(&packet).unwrap();
        let sizes: Vec<usize> = parts.iter().map(|p| p.len() - FRAGMENT_HEADER_BYTES).collect();
        assert_eq!(sizes, [10, 10, 5]);
        for (index, part) in parts.iter().enumerate() {
            let header = Header::read(part).unwrap();
            assert_eq!((header.index as usize, header.count, header.total_len), (index, 3, 25));
        }
        let joined: Vec<u8> = parts.iter().flat_map(|p| p[FRAGMENT_HEADER_BYTES..].to_vec()).collect();
        assert_eq!(joined, packet);
    }

    #[test]
    fn each_packet_gets_the_id_counter_and_ids_wrap() {
        let mut splitter = DatagramSplitter::new(100, 1000);
        let id = |parts: Vec<Vec<u8>>| Header::read(&parts[0]).unwrap().packet_id;
        assert_eq!(id(splitter.split_up(b"a").unwrap()), 0);
        assert_eq!(id(splitter.split_up(b"b").unwrap()), 1);
        splitter.id_counter = u32::MAX;
        assert_eq!(id(splitter.split_up(b"c").unwrap()), u32::MAX);
        assert_eq!(id(splitter.split_up(b"d").unwrap()), 0);
    }

    #[test]
    fn empty_and_oversize_packets_are_refused_without_using_an_id() {
        let mut splitter = DatagramSplitter::new(100, 8);
        let empty = splitter.split_up(&[]).unwrap_err();
        assert_eq!(empty.0, "packet size 0 outside the allowed 1..=8");
        let big = splitter.split_up(&[0; 9]).unwrap_err();
        assert_eq!(big.0, "packet size 9 outside the allowed 1..=8");
        assert_eq!(splitter.id_counter, 0);
    }

    #[test]
    fn a_packet_needing_more_fragments_than_the_count_holds_is_refused() {
        // One byte of room per fragment, so a packet of 65536 bytes needs 65536 fragments.
        let mut splitter = DatagramSplitter::new(FRAGMENT_HEADER_BYTES, 1 << 20);
        let error = splitter.split_up(&vec![0; SPLIT_PIECE_LIMIT + 1]).unwrap_err();
        assert_eq!(error.0, "packet length 65536 needs 65536 fragments (max 65535)");
        assert_eq!(splitter.split_up(&vec![0; SPLIT_PIECE_LIMIT]).unwrap().len(), SPLIT_PIECE_LIMIT);
    }
}
