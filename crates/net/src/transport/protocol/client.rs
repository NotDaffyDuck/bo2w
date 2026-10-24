//! Datagrams a client sends to the authority.

use playerstate_t6::game::UserCmd;
use sim::ClientAction;

use super::fields::{read_header, read_hello, read_shot_sample, write_header, write_hello, write_shot_sample};
use super::handshake::{HandshakeHello, ProtocolLimits};
use super::identity::PacketHeader;
use crate::client::predict::CmdSeq;
use crate::transport::delta::{decode_usercmd, encode_usercmd};
use crate::transport::snapshot_wire::{decode_action, encode_action};
use crate::transport::wire::{WireError, WireReader, WireWriter};

const TAG_CONNECT: u8 = 1;
const TAG_COMMANDS: u8 = 2;
const TAG_SNAPSHOT_ACK: u8 = 3;

/// A count in front of a list is trusted for no more than this many slots up front.
const PREALLOC_CAP: usize = 64;

#[derive(Clone, Debug, PartialEq)]
pub enum ClientPacket {
    Connect(HandshakeHello),

    Commands {
        header: PacketHeader,
        claimed_client: u32,
        cmds: Vec<(CmdSeq, UserCmd)>,
        samples: Vec<(CmdSeq, sim::ShotSampleProvenance)>,
        actions: Vec<ClientAction>,
        reliable_ack: u16,
    },

    SnapshotAck {
        header: PacketHeader,
        snapshot_seq: u32,
    },
}

/// Reads a list that is announced by a 16-bit count, one element at a time.
fn read_counted<T>(
    input: &mut WireReader<'_>,
    mut element: impl FnMut(&mut WireReader<'_>) -> Result<T, WireError>,
) -> Result<Vec<T>, WireError> {
    let count = usize::from(input.get_u16()?);
    let mut items = Vec::with_capacity(count.min(PREALLOC_CAP));
    for _ in 0..count {
        items.push(element(input)?);
    }
    Ok(items)
}

impl ClientPacket {
    pub fn encode(&self, out: &mut WireWriter) {
        match self {
            Self::Connect(hello) => {
                out.put_u8(TAG_CONNECT);
                write_hello(out, hello);
            }
            Self::Commands {
                header,
                claimed_client,
                cmds,
                samples,
                actions,
                reliable_ack,
            } => {
                out.put_u8(TAG_COMMANDS);
                write_header(out, header);
                out.put_u32(*claimed_client);
                out.put_u16(*reliable_ack);
                out.put_u16(cmds.len() as u16);
                for (seq, cmd) in cmds {
                    out.put_u32(seq.0);
                    encode_usercmd(out, cmd);
                }
                out.put_u16(samples.len() as u16);
                for (seq, sample) in samples {
                    out.put_u32(seq.0);
                    write_shot_sample(out, sample);
                }
                out.put_u16(actions.len() as u16);
                for action in actions {
                    encode_action(out, action);
                }
            }
            Self::SnapshotAck {
                header,
                snapshot_seq,
            } => {
                out.put_u8(TAG_SNAPSHOT_ACK);
                write_header(out, header);
                out.put_u32(*snapshot_seq);
            }
        }
    }

    pub fn decode(input: &mut WireReader<'_>) -> Result<Self, WireError> {
        match input.get_u8()? {
            TAG_CONNECT => Ok(Self::Connect(read_hello(input)?)),
            TAG_COMMANDS => {
                let header = read_header(input)?;
                let claimed_client = input.get_u32()?;
                let reliable_ack = input.get_u16()?;
                let cmds = read_counted(input, |input| {
                    let seq = CmdSeq(input.get_u32()?);
                    Ok((seq, decode_usercmd(input)?))
                })?;
                let samples = read_counted(input, |input| {
                    let seq = CmdSeq(input.get_u32()?);
                    Ok((seq, read_shot_sample(input)?))
                })?;
                let actions = read_counted(input, decode_action)?;
                Ok(Self::Commands {
                    header,
                    claimed_client,
                    cmds,
                    samples,
                    actions,
                    reliable_ack,
                })
            }
            TAG_SNAPSHOT_ACK => Ok(Self::SnapshotAck {
                header: read_header(input)?,
                snapshot_seq: input.get_u32()?,
            }),
            _ => Err(WireError::Malformed("unknown ClientPacket tag")),
        }
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = WireWriter::new();
        self.encode(&mut out);
        out.finish()
    }
}

/// Decodes one whole datagram, refusing it if it is too big, has bytes left over, or carries
/// more commands or actions than `limits` allow.
pub fn decode_client_packet(
    bytes: &[u8],
    limits: &ProtocolLimits,
) -> Result<ClientPacket, WireError> {
    if bytes.len() as u32 > limits.max_packet_bytes {
        return Err(WireError::Malformed("datagram exceeds max_packet_bytes"));
    }
    let mut input = WireReader::new(bytes);
    let packet = ClientPacket::decode(&mut input)?;
    if !input.is_empty() {
        return Err(WireError::Malformed("trailing bytes after ClientPacket"));
    }
    if let ClientPacket::Commands { cmds, actions, .. } = &packet {
        if cmds.len() as u16 > limits.max_cmds_per_tick {
            return Err(WireError::Malformed("cmd_count exceeds limit"));
        }
        if actions.len() as u16 > limits.max_actions_per_tick {
            return Err(WireError::Malformed("action_count exceeds limit"));
        }
    }
    Ok(packet)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transport::protocol::ConnectionId;

    fn header() -> PacketHeader {
        PacketHeader {
            connection: ConnectionId(2),
            sequence: 3,
            ack: 4,
            epoch: 5,
        }
    }

    fn commands(count: usize) -> ClientPacket {
        ClientPacket::Commands {
            header: header(),
            claimed_client: 6,
            cmds: (1..=count as u32).map(|n| (CmdSeq(n), UserCmd::default())).collect(),
            samples: (1..=count as u32)
                .map(|n| (CmdSeq(n), sim::ShotSampleProvenance::default()))
                .collect(),
            actions: Vec::new(),
            reliable_ack: 7,
        }
    }

    fn limits() -> ProtocolLimits {
        ProtocolLimits::default()
    }

    #[test]
    fn every_client_packet_round_trips() {
        let packets = [
            ClientPacket::Connect(HandshakeHello::current(Default::default())),
            commands(0),
            commands(3),
            ClientPacket::SnapshotAck {
                header: header(),
                snapshot_seq: 99,
            },
        ];
        for packet in packets {
            let decoded = decode_client_packet(&packet.to_bytes(), &limits()).unwrap();
            assert_eq!(decoded, packet);
        }
    }

    #[test]
    fn bytes_left_over_are_refused() {
        let mut bytes = commands(1).to_bytes();
        bytes.push(0);
        assert_eq!(
            decode_client_packet(&bytes, &limits()),
            Err(WireError::Malformed("trailing bytes after ClientPacket"))
        );
    }

    #[test]
    fn an_unknown_tag_is_refused() {
        assert_eq!(
            decode_client_packet(&[200], &limits()),
            Err(WireError::Malformed("unknown ClientPacket tag"))
        );
    }

    #[test]
    fn a_truncated_packet_is_refused() {
        let bytes = commands(2).to_bytes();
        for cut in 0..bytes.len() {
            assert!(decode_client_packet(&bytes[..cut], &limits()).is_err(), "cut at {cut}");
        }
    }

    #[test]
    fn a_datagram_over_the_size_limit_is_refused() {
        let mut small = limits();
        small.max_packet_bytes = 10;
        assert_eq!(
            decode_client_packet(&commands(1).to_bytes(), &small),
            Err(WireError::Malformed("datagram exceeds max_packet_bytes"))
        );
    }

    #[test]
    fn more_commands_than_the_limit_are_refused() {
        let mut tight = limits();
        tight.max_cmds_per_tick = 2;
        assert!(decode_client_packet(&commands(2).to_bytes(), &tight).is_ok());
        assert_eq!(
            decode_client_packet(&commands(3).to_bytes(), &tight),
            Err(WireError::Malformed("cmd_count exceeds limit"))
        );
    }

    #[test]
    fn a_count_that_promises_more_than_the_bytes_hold_is_refused_without_a_huge_allocation() {
        let mut bytes = commands(0).to_bytes();
        // The command count sits after: tag(1) + header(20) + claimed(4) + ack(2).
        bytes[27] = 0xff;
        bytes[28] = 0xff;
        assert!(decode_client_packet(&bytes, &limits()).is_err());
    }
}
