//! Datagrams the authority sends to clients.

use super::fields::{
    read_connection, read_header, read_hello, read_reject, write_connection, write_header,
    write_hello, write_reject,
};
use super::handshake::{HandshakeHello, HandshakeReject, MAX_PACKET_BYTES, ProtocolLimits};
use super::identity::{ConnectionId, PacketHeader};
use crate::transport::reliable::{decode_reliable_payload, encode_reliable_payload};
use crate::transport::wire::{WireError, WireReader, WireWriter};

const TAG_ACCEPT: u8 = 10;
const TAG_REJECT: u8 = 11;
const TAG_SNAPSHOT: u8 = 12;
const TAG_CONTROL: u8 = 13;
const TAG_COMPRESSED_SNAPSHOT: u8 = 14;

/// zstd level used for snapshots; speed matters more than size at this rate.
const COMPRESSION_LEVEL: i32 = 1;

#[derive(Clone, Debug, PartialEq)]
pub enum ServerPacket {
    Control {
        header: PacketHeader,
        payload: crate::ReliablePayload,
    },
    Accept {
        connection: ConnectionId,
        assigned_client: u32,
        hello: HandshakeHello,
    },
    Reject(HandshakeReject),
    Snapshot {
        header: PacketHeader,
        baseline_seq: u32,
        snapshot_seq: u32,
        payload: Vec<u8>,
    },
}

/// The compressed form of `payload`, when it is worth sending: at least five bytes smaller.
fn compress_if_smaller(payload: &[u8]) -> Option<Vec<u8>> {
    match zstd::bulk::compress(payload, COMPRESSION_LEVEL) {
        Ok(bytes) if bytes.len() + 4 < payload.len() => Some(bytes),
        Ok(_) => None,
        Err(error) => {
            diag::warn!(Net, "snapshot compression failed: {error}");
            None
        }
    }
}

fn read_length(input: &mut WireReader<'_>, what: &'static str) -> Result<usize, WireError> {
    let length = input.get_u32()? as usize;
    if length > MAX_PACKET_BYTES as usize {
        return Err(WireError::Malformed(what));
    }
    Ok(length)
}

impl ServerPacket {
    pub fn encode(&self, out: &mut WireWriter) {
        match self {
            Self::Control { header, payload } => {
                out.put_u8(TAG_CONTROL);
                write_header(out, header);
                encode_reliable_payload(
                    out,
                    payload.ack_through,
                    &payload.rows,
                    payload.dropped_oldest,
                );
            }
            Self::Accept {
                connection,
                assigned_client,
                hello,
            } => {
                out.put_u8(TAG_ACCEPT);
                write_connection(out, *connection);
                out.put_u32(*assigned_client);
                write_hello(out, hello);
            }
            Self::Reject(reject) => {
                out.put_u8(TAG_REJECT);
                write_reject(out, *reject);
            }
            Self::Snapshot {
                header,
                baseline_seq,
                snapshot_seq,
                payload,
            } => {
                let compressed = compress_if_smaller(payload);
                out.put_u8(match compressed {
                    Some(_) => TAG_COMPRESSED_SNAPSHOT,
                    None => TAG_SNAPSHOT,
                });
                write_header(out, header);
                out.put_u32(*baseline_seq);
                out.put_u32(*snapshot_seq);
                // The decoded length always comes first; a compressed body adds its own.
                out.put_u32(payload.len() as u32);
                match &compressed {
                    Some(bytes) => {
                        out.put_u32(bytes.len() as u32);
                        out.put_bytes(bytes);
                    }
                    None => out.put_bytes(payload),
                }
            }
        }
    }

    pub fn decode(input: &mut WireReader<'_>) -> Result<Self, WireError> {
        match input.get_u8()? {
            TAG_CONTROL => Ok(Self::Control {
                header: read_header(input)?,
                payload: decode_reliable_payload(input)?,
            }),
            TAG_ACCEPT => Ok(Self::Accept {
                connection: read_connection(input)?,
                assigned_client: input.get_u32()?,
                hello: read_hello(input)?,
            }),
            TAG_REJECT => Ok(Self::Reject(read_reject(input)?)),
            tag @ (TAG_SNAPSHOT | TAG_COMPRESSED_SNAPSHOT) => {
                let header = read_header(input)?;
                let baseline_seq = input.get_u32()?;
                let snapshot_seq = input.get_u32()?;
                let compressed = tag == TAG_COMPRESSED_SNAPSHOT;
                let decoded_len = if compressed {
                    Some(read_length(input, "snapshot exceeds decoded size limit")?)
                } else {
                    None
                };
                let body_len = read_length(input, "invalid snapshot payload length")?;
                if body_len > input.remaining() {
                    return Err(WireError::Malformed("invalid snapshot payload length"));
                }
                let mut payload = vec![0u8; body_len];
                input.get_bytes(&mut payload)?;
                if let Some(decoded_len) = decoded_len {
                    payload = zstd::bulk::decompress(&payload, decoded_len)
                        .map_err(|_| WireError::Malformed("invalid compressed snapshot"))?;
                    if payload.len() != decoded_len {
                        return Err(WireError::Malformed("snapshot decoded length mismatch"));
                    }
                }
                Ok(Self::Snapshot {
                    header,
                    baseline_seq,
                    snapshot_seq,
                    payload,
                })
            }
            _ => Err(WireError::Malformed("unknown ServerPacket tag")),
        }
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = WireWriter::new();
        self.encode(&mut out);
        out.finish()
    }
}

/// Decodes one whole datagram, refusing it if it is too big or has bytes left over.
pub fn decode_server_packet(
    bytes: &[u8],
    limits: &ProtocolLimits,
) -> Result<ServerPacket, WireError> {
    if bytes.len() as u32 > limits.max_packet_bytes {
        return Err(WireError::Malformed("datagram exceeds max_packet_bytes"));
    }
    let mut input = WireReader::new(bytes);
    let packet = ServerPacket::decode(&mut input)?;
    if !input.is_empty() {
        return Err(WireError::Malformed("trailing bytes after ServerPacket"));
    }
    Ok(packet)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn header() -> PacketHeader {
        PacketHeader {
            connection: ConnectionId(3),
            sequence: 4,
            ack: 5,
            epoch: 6,
        }
    }

    fn snapshot(payload: Vec<u8>) -> ServerPacket {
        ServerPacket::Snapshot {
            header: header(),
            baseline_seq: 7,
            snapshot_seq: 8,
            payload,
        }
    }

    fn limits() -> ProtocolLimits {
        ProtocolLimits::default()
    }

    fn round_trip(packet: &ServerPacket) -> ServerPacket {
        decode_server_packet(&packet.to_bytes(), &limits()).unwrap()
    }

    #[test]
    fn accept_reject_and_control_round_trip() {
        let packets = [
            ServerPacket::Accept {
                connection: ConnectionId(0x1_0000_0002),
                assigned_client: 9,
                hello: HandshakeHello::current(Default::default()),
            },
            ServerPacket::Reject(HandshakeReject::Limits),
            ServerPacket::Control {
                header: header(),
                payload: crate::ReliablePayload {
                    ack_through: 3,
                    rows: Vec::new(),
                    dropped_oldest: 0,
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
        let bytes = packet.to_bytes();
        assert_eq!(bytes[0], TAG_SNAPSHOT);
        assert_eq!(round_trip(&packet), packet);
    }

    #[test]
    fn a_compressible_snapshot_goes_out_compressed_and_comes_back_whole() {
        let packet = snapshot(vec![42; 4000]);
        let bytes = packet.to_bytes();
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
        let mut bytes = snapshot(vec![1, 2, 3]).to_bytes();
        let len_at = 1 + 20 + 4 + 4;
        bytes[len_at] = 50;
        assert_eq!(
            decode_server_packet(&bytes, &limits()),
            Err(WireError::Malformed("invalid snapshot payload length"))
        );
    }

    #[test]
    fn a_compressed_body_that_lies_about_its_size_is_refused() {
        let mut bytes = snapshot(vec![42; 4000]).to_bytes();
        let decoded_at = 1 + 20 + 4 + 4;
        bytes[decoded_at] ^= 1;
        assert!(decode_server_packet(&bytes, &limits()).is_err());
    }

    #[test]
    fn a_decoded_size_over_the_cap_is_refused() {
        let mut bytes = snapshot(vec![42; 4000]).to_bytes();
        let decoded_at = 1 + 20 + 4 + 4;
        bytes[decoded_at..decoded_at + 4].copy_from_slice(&u32::MAX.to_le_bytes());
        assert_eq!(
            decode_server_packet(&bytes, &limits()),
            Err(WireError::Malformed("snapshot exceeds decoded size limit"))
        );
    }

    #[test]
    fn bytes_left_over_and_unknown_tags_are_refused() {
        let mut bytes = ServerPacket::Reject(HandshakeReject::Limits).to_bytes();
        bytes.push(0);
        assert_eq!(
            decode_server_packet(&bytes, &limits()),
            Err(WireError::Malformed("trailing bytes after ServerPacket"))
        );
        assert_eq!(
            decode_server_packet(&[99], &limits()),
            Err(WireError::Malformed("unknown ServerPacket tag"))
        );
    }

    #[test]
    fn a_datagram_over_the_size_limit_is_refused() {
        let mut small = limits();
        small.max_packet_bytes = 4;
        assert_eq!(
            decode_server_packet(&snapshot(vec![1; 9]).to_bytes(), &small),
            Err(WireError::Malformed("datagram exceeds max_packet_bytes"))
        );
    }
}
