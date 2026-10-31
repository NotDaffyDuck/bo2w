//! Datagrams the authority sends to clients.

use super::fields::{
    read_connection, read_header, read_hello, read_reject, write_connection, write_header,
    write_hello, write_reject,
};
use super::handshake::{
    JoinGreeting, JoinRefusal, DATAGRAM_BYTE_CAP, MAX_SNAPSHOT_DECODED_BYTES, NegotiatedCaps,
};
use super::identity::{LinkNumber, DatagramPreamble};
use crate::net_links::once_queue::{unpack_guaranteed_body, pack_guaranteed_body};
use crate::net_links::byte_codec::{PacketDecodeFault, ByteCursorReader, ByteSinkWriter};

const TAG_ACCEPT: u8 = 10;
const TAG_REJECT: u8 = 11;
const TAG_SNAPSHOT: u8 = 12;
const TAG_CONTROL: u8 = 13;
const TAG_COMPRESSED_SNAPSHOT: u8 = 14;

/// zstd level used for snapshots; speed matters more than size at this rate.
const COMPRESSION_LEVEL: i32 = 1;

#[derive(Clone, Debug, PartialEq)]
pub enum HostToPlayerDatagram {
    OwedBatch {
        header: DatagramPreamble,
        payload: crate::GuaranteedEnvelope,
    },
    Admitted {
        connection: LinkNumber,
        granted_player_slot: u32,
        hello: JoinGreeting,
    },
    Reject(JoinRefusal),
    Snapshot {
        header: DatagramPreamble,
        reference_frame_number: u32,
        frame_number_on_wire: u32,
        body: Vec<u8>,
    },
}

/// The compressed form of `payload`, when it is worth sending: at least five bytes smaller.
fn compress_if_smaller(payload: &[u8]) -> Option<Vec<u8>> {
    let packed = zstd::bulk::compress(payload, COMPRESSION_LEVEL)
        .inspect_err(|error| diag::warn!(WireLink, "snapshot compression failed: {error}"))
        .ok()?;
    // The compressed form carries one extra length word, so it must save more than that.
    (packed.len() + 4 < payload.len()).then_some(packed)
}

/// A length word, refused (as `what`) when it is larger than any datagram.
fn read_length(cursor: &mut ByteCursorReader<'_>, what: &'static str) -> Result<usize, PacketDecodeFault> {
    read_length_up_to(cursor, what, DATAGRAM_BYTE_CAP)
}

fn read_length_up_to(cursor: &mut ByteCursorReader<'_>, what: &'static str, max: u32) -> Result<usize, PacketDecodeFault> {
    let length = cursor.read_word32()? as usize;
    match length <= max as usize {
        true => Ok(length),
        false => Err(PacketDecodeFault::GarbledRequest(what)),
    }
}

/// Snapshot body: header, the two frame numbers, the plain length, then either the plain
/// bytes or (compressed) their own length and the packed bytes; the tag byte comes first.
fn write_snapshot(sink: &mut ByteSinkWriter, header: &DatagramPreamble, frames: [u32; 2], payload: &[u8]) {
    let packed = compress_if_smaller(payload);
    sink.put_u8(if packed.is_some() { TAG_COMPRESSED_SNAPSHOT } else { TAG_SNAPSHOT });
    write_header(sink, header);
    for word in frames.into_iter().chain([payload.len() as u32]) {
        sink.put_u32(word);
    }
    let Some(packed) = packed else {
        sink.append_raw_slice(payload);
        return;
    };
    sink.put_u32(packed.len() as u32);
    sink.append_raw_slice(&packed);
}

fn read_snapshot(cursor: &mut ByteCursorReader<'_>, compressed: bool) -> Result<HostToPlayerDatagram, PacketDecodeFault> {
    let garbled = PacketDecodeFault::GarbledRequest;
    let header = read_header(cursor)?;
    let reference_frame_number = cursor.read_word32()?;
    let frame_number_on_wire = cursor.read_word32()?;
    let plain_len = match compressed {
        true => Some(read_length_up_to(cursor, "snapshot exceeds decoded size limit", MAX_SNAPSHOT_DECODED_BYTES)?),
        false => None,
    };
    let carried = read_length(cursor, "invalid snapshot payload length")?;
    if cursor.unread_len() < carried {
        return Err(garbled("invalid snapshot payload length"));
    }
    let mut body = vec![0u8; carried];
    cursor.copy_out(&mut body)?;
    let payload = match plain_len {
        None => body,
        Some(expected) => {
            let plain =
                zstd::bulk::decompress(&body, expected).map_err(|_| garbled("invalid compressed snapshot"))?;
            if plain.len() != expected {
                return Err(garbled("snapshot decoded length mismatch"));
            }
            plain
        }
    };
    Ok(HostToPlayerDatagram::Snapshot { header, reference_frame_number, frame_number_on_wire, body: payload })
}

impl HostToPlayerDatagram {
    /// Writes the tag byte and the body of this datagram.
    pub fn pack(&self, sink: &mut ByteSinkWriter) {
        match self {
            Self::OwedBatch { header, payload: envelope } => {
                sink.put_u8(TAG_CONTROL);
                write_header(sink, header);
                let (acked, dropped) = (envelope.confirmed_up_to, envelope.evicted_stale_count);
                pack_guaranteed_body(sink, acked, &envelope.entries, dropped);
            }
            Self::Admitted { connection, granted_player_slot, hello } => {
                sink.put_u8(TAG_ACCEPT);
                write_connection(sink, *connection);
                sink.put_u32(*granted_player_slot);
                write_hello(sink, hello);
            }
            Self::Reject(refusal) => {
                sink.put_u8(TAG_REJECT);
                write_reject(sink, *refusal);
            }
            Self::Snapshot { header, reference_frame_number, frame_number_on_wire, body } => {
                write_snapshot(sink, header, [*reference_frame_number, *frame_number_on_wire], body);
            }
        }
    }

    /// Reads one datagram (tag and body) from `cursor`.
    pub fn unpack(cursor: &mut ByteCursorReader<'_>) -> Result<Self, PacketDecodeFault> {
        let kind_byte = cursor.get_u8()?;
        if kind_byte == TAG_SNAPSHOT || kind_byte == TAG_COMPRESSED_SNAPSHOT {
            return read_snapshot(cursor, kind_byte == TAG_COMPRESSED_SNAPSHOT);
        }
        Ok(match kind_byte {
            TAG_CONTROL => {
                let header = read_header(cursor)?;
                Self::OwedBatch { header, payload: unpack_guaranteed_body(cursor)? }
            }
            TAG_ACCEPT => {
                let connection = read_connection(cursor)?;
                let granted_player_slot = cursor.read_word32()?;
                Self::Admitted { connection, granted_player_slot, hello: read_hello(cursor)? }
            }
            TAG_REJECT => Self::Reject(read_reject(cursor)?),
            _ => return Err(PacketDecodeFault::GarbledRequest("unknown ServerPacket tag")),
        })
    }

    /// The whole datagram as bytes.
    pub fn wire_bytes(&self) -> Vec<u8> {
        let mut sink = ByteSinkWriter::new();
        self.pack(&mut sink);
        sink.into_bytes()
    }
}

/// Decodes one whole datagram, refusing it if it is too big or has bytes left over.
pub fn parse_host_datagram(
    bytes: &[u8],
    limits: &NegotiatedCaps,
) -> Result<HostToPlayerDatagram, PacketDecodeFault> {
    if bytes.len() as u32 > limits.datagram_size_ceiling {
        return Err(PacketDecodeFault::GarbledRequest("datagram exceeds max_packet_bytes"));
    }
    let mut cursor = ByteCursorReader::new(bytes);
    let datagram = HostToPlayerDatagram::unpack(&mut cursor)?;
    if !cursor.is_empty() {
        return Err(PacketDecodeFault::GarbledRequest("trailing bytes after ServerPacket"));
    }
    Ok(datagram)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn header() -> DatagramPreamble {
        DatagramPreamble {
            connection: LinkNumber(3),
            packet_no: 4,
            confirm_receipt: 5,
            round_era: 6,
        }
    }

    fn snapshot(payload: Vec<u8>) -> HostToPlayerDatagram {
        HostToPlayerDatagram::Snapshot {
            header: header(),
            reference_frame_number: 7,
            frame_number_on_wire: 8,
            body: payload,
        }
    }

    fn limits() -> NegotiatedCaps {
        NegotiatedCaps::default()
    }

    fn round_trip(packet: &HostToPlayerDatagram) -> HostToPlayerDatagram {
        parse_host_datagram(&packet.wire_bytes(), &limits()).unwrap()
    }

    #[test]
    fn accept_reject_and_control_round_trip() {
        let packets = [
            HostToPlayerDatagram::Admitted {
                connection: LinkNumber(0x1_0000_0002),
                granted_player_slot: 9,
                hello: JoinGreeting::current(Default::default()),
            },
            HostToPlayerDatagram::Reject(JoinRefusal::Limits),
            HostToPlayerDatagram::OwedBatch {
                header: header(),
                payload: crate::GuaranteedEnvelope {
                    confirmed_up_to: 3,
                    entries: Vec::new(),
                    evicted_stale_count: 0,
                },
            },
        ];
        for packet in &packets {
            assert_eq!(&round_trip(packet), packet);
        }
    }

    #[test]
    fn a_small_snapshot_goes_out_uncompressed() {
        let packet = snapshot(vec![1, 2, 3, 4, 5, 6]);
        let bytes = packet.wire_bytes();
        assert_eq!(bytes[0], TAG_SNAPSHOT);
        assert_eq!(round_trip(&packet), packet);
    }

    #[test]
    fn a_compressible_snapshot_goes_out_compressed_and_comes_back_whole() {
        let packet = snapshot(vec![42; 4000]);
        let bytes = packet.wire_bytes();
        assert_eq!(bytes[0], TAG_COMPRESSED_SNAPSHOT);
        assert!(bytes.len() < 400, "{} bytes", bytes.len());
        assert_eq!(round_trip(&packet), packet);
    }

    #[test]
    fn an_empty_snapshot_round_trips() {
        let packet = snapshot(Vec::new());
        assert_eq!(round_trip(&packet), packet);
    }

    #[test]
    fn a_body_length_longer_than_the_datagram_is_refused() {
        let mut bytes = snapshot(vec![1, 2, 3]).wire_bytes();
        let len_at = 1 + 20 + 4 + 4;
        bytes[len_at] = 50;
        assert_eq!(
            parse_host_datagram(&bytes, &limits()),
            Err(PacketDecodeFault::GarbledRequest("invalid snapshot payload length"))
        );
    }

    #[test]
    fn a_compressed_body_that_lies_about_its_size_is_refused() {
        let mut bytes = snapshot(vec![42; 4000]).wire_bytes();
        let decoded_at = 1 + 20 + 4 + 4;
        bytes[decoded_at] ^= 1;
        assert!(parse_host_datagram(&bytes, &limits()).is_err());
    }

    #[test]
    fn a_decoded_size_over_the_cap_is_refused() {
        let mut bytes = snapshot(vec![42; 4000]).wire_bytes();
        let decoded_at = 1 + 20 + 4 + 4;
        bytes[decoded_at..decoded_at + 4].copy_from_slice(&u32::MAX.to_le_bytes());
        assert_eq!(
            parse_host_datagram(&bytes, &limits()),
            Err(PacketDecodeFault::GarbledRequest("snapshot exceeds decoded size limit"))
        );
    }

    #[test]
    fn bytes_left_over_and_unknown_tags_are_refused() {
        let mut bytes = HostToPlayerDatagram::Reject(JoinRefusal::Limits).wire_bytes();
        bytes.push(0);
        assert_eq!(
            parse_host_datagram(&bytes, &limits()),
            Err(PacketDecodeFault::GarbledRequest("trailing bytes after ServerPacket"))
        );
        assert_eq!(
            parse_host_datagram(&[99], &limits()),
            Err(PacketDecodeFault::GarbledRequest("unknown ServerPacket tag"))
        );
    }

    #[test]
    fn a_datagram_over_the_size_limit_is_refused() {
        let mut small = limits();
        small.datagram_size_ceiling = 4;
        assert_eq!(
            parse_host_datagram(&snapshot(vec![1; 9]).wire_bytes(), &small),
            Err(PacketDecodeFault::GarbledRequest("datagram exceeds max_packet_bytes"))
        );
    }
}
