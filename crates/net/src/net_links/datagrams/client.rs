//! Datagrams a client sends to the authority.
//!
//! Layout: one tag byte, then the body. Lists go out as a u16 count followed by the items;
//! command and shot-sample items each lead with their u32 command sequence.

use playerstate_t6::game::InputCommand;
use sim::PlayerIntent;

use super::fields::{read_header, read_hello, read_shot_sample, write_header, write_hello, write_shot_sample};
use super::handshake::{JoinGreeting, NegotiatedCaps};
use super::identity::DatagramPreamble;
use crate::client::run_ahead::InputCommandNumber;
use crate::net_links::snapshot_diff::{unpack_input_record, pack_input_record};
use crate::net_links::snapshot_wire::{read_intent, pack_command};
use crate::net_links::byte_codec::{PacketDecodeFault, ByteCursorReader, ByteSinkWriter};

const TAG_CONNECT: u8 = 1;
const TAG_COMMANDS: u8 = 2;
const TAG_SNAPSHOT_ACK: u8 = 3;

/// A count in front of a list is trusted for no more than this many slots up front.
const PREALLOC_CAP: usize = 64;

const BAD_TAG: &str = "unknown ClientPacket tag";
const TOO_BIG: &str = "datagram exceeds max_packet_bytes";
const LEFTOVER: &str = "trailing bytes after ClientPacket";
const TOO_MANY_CMDS: &str = "cmd_count exceeds limit";
const TOO_MANY_ACTIONS: &str = "action_count exceeds limit";

#[derive(Clone, Debug, PartialEq)]
pub enum PlayerToHostDatagram {
    Connect(JoinGreeting),

    Commands {
        header: DatagramPreamble,
        asserted_player_slot: u32,
        moves: Vec<(InputCommandNumber, InputCommand)>,
        aim_origins: Vec<(InputCommandNumber, sim::ShotRecordOrigin)>,
        actions: Vec<PlayerIntent>,
        guaranteed_receipt: u16,
    },

    FrameReceipt {
        header: DatagramPreamble,
        frame_number_on_wire: u32,
    },
}

/// Which body reader follows each tag byte.
const BODIES: [(u8, fn(&mut ByteCursorReader<'_>) -> Result<PlayerToHostDatagram, PacketDecodeFault>); 3] = [
    (TAG_CONNECT, body_connect),
    (TAG_COMMANDS, body_commands),
    (TAG_SNAPSHOT_ACK, body_snapshot_ack),
];

fn body_connect(cursor: &mut ByteCursorReader<'_>) -> Result<PlayerToHostDatagram, PacketDecodeFault> {
    read_hello(cursor).map(PlayerToHostDatagram::Connect)
}

fn body_commands(cursor: &mut ByteCursorReader<'_>) -> Result<PlayerToHostDatagram, PacketDecodeFault> {
    let header = read_header(cursor)?;
    let asserted_player_slot = cursor.read_word32()?;
    let guaranteed_receipt = cursor.get_u16()?;
    let moves = list_in(cursor, sequenced(unpack_input_record))?;
    let aim_origins = list_in(cursor, sequenced(read_shot_sample))?;
    let actions = list_in(cursor, read_intent)?;
    Ok(PlayerToHostDatagram::Commands { header, asserted_player_slot, moves, aim_origins, actions, guaranteed_receipt })
}

fn body_snapshot_ack(cursor: &mut ByteCursorReader<'_>) -> Result<PlayerToHostDatagram, PacketDecodeFault> {
    let header = read_header(cursor)?;
    cursor
        .read_word32()
        .map(|frame_number_on_wire| PlayerToHostDatagram::FrameReceipt { header, frame_number_on_wire })
}

/// A list behind a u16 count. Room is reserved for at most `PREALLOC_CAP` items whatever the
/// count claims, so a lying count costs nothing until the bytes run out.
fn list_in<T>(
    cursor: &mut ByteCursorReader<'_>,
    item: impl Fn(&mut ByteCursorReader<'_>) -> Result<T, PacketDecodeFault>,
) -> Result<Vec<T>, PacketDecodeFault> {
    let count = usize::from(cursor.get_u16()?);
    let mut list = Vec::with_capacity(PREALLOC_CAP.min(count));
    while list.len() < count {
        list.push(item(cursor)?);
    }
    Ok(list)
}

fn list_out<T>(sink: &mut ByteSinkWriter, list: &[T], item: impl Fn(&mut ByteSinkWriter, &T)) {
    sink.put_u16(list.len() as u16);
    list.iter().for_each(|one| item(sink, one));
}

/// Wraps an item reader so it first takes the item's command sequence.
fn sequenced<T>(
    item: fn(&mut ByteCursorReader<'_>) -> Result<T, PacketDecodeFault>,
) -> impl Fn(&mut ByteCursorReader<'_>) -> Result<(InputCommandNumber, T), PacketDecodeFault> {
    move |cursor| {
        let seq = cursor.read_word32().map(InputCommandNumber)?;
        item(cursor).map(|value| (seq, value))
    }
}

/// Wraps an item writer so it first puts the item's command sequence.
fn with_seq<T>(item: fn(&mut ByteSinkWriter, &T)) -> impl Fn(&mut ByteSinkWriter, &(InputCommandNumber, T)) {
    move |sink, (seq, value)| {
        sink.put_u32(seq.0);
        item(sink, value);
    }
}

impl PlayerToHostDatagram {
    fn tag(&self) -> u8 {
        match self {
            Self::Connect(_) => TAG_CONNECT,
            Self::Commands { .. } => TAG_COMMANDS,
            Self::FrameReceipt { .. } => TAG_SNAPSHOT_ACK,
        }
    }

    pub fn pack(&self, sink: &mut ByteSinkWriter) {
        sink.put_u8(self.tag());
        match self {
            Self::Connect(hello) => write_hello(sink, hello),
            Self::FrameReceipt { header, frame_number_on_wire } => {
                write_header(sink, header);
                sink.put_u32(*frame_number_on_wire);
            }
            Self::Commands { header, asserted_player_slot, moves, aim_origins, actions, guaranteed_receipt } => {
                write_header(sink, header);
                sink.put_u32(*asserted_player_slot);
                sink.put_u16(*guaranteed_receipt);
                list_out(sink, moves, with_seq(pack_input_record));
                list_out(sink, aim_origins, with_seq(write_shot_sample));
                list_out(sink, actions, pack_command);
            }
        }
    }

    pub fn unpack(cursor: &mut ByteCursorReader<'_>) -> Result<Self, PacketDecodeFault> {
        let kind_byte = cursor.get_u8()?;
        let (_, body) = BODIES
            .iter()
            .find(|(code, _)| *code == kind_byte)
            .ok_or(PacketDecodeFault::GarbledRequest(BAD_TAG))?;
        body(cursor)
    }

    pub fn wire_bytes(&self) -> Vec<u8> {
        let mut sink = ByteSinkWriter::new();
        self.pack(&mut sink);
        sink.into_bytes()
    }

    /// Breaks the per-tick limits? Commands only; the counts compare at wire width.
    fn over_limit(&self, limits: &NegotiatedCaps) -> Option<&'static str> {
        let Self::Commands { moves, actions, .. } = self else {
            return None;
        };
        let checks = [
            (moves.len() as u16, limits.moves_budget_per_step, TOO_MANY_CMDS),
            (actions.len() as u16, limits.action_budget_per_step, TOO_MANY_ACTIONS),
        ];
        checks
            .into_iter()
            .find(|&(count, cap, _)| count > cap)
            .map(|(_, _, why)| why)
    }
}

/// Decodes one whole datagram, refusing it if it is too big, has bytes left over, or carries
/// more commands or actions than `limits` allow.
pub fn parse_receiver_datagram(
    bytes: &[u8],
    limits: &NegotiatedCaps,
) -> Result<PlayerToHostDatagram, PacketDecodeFault> {
    let refuse = |why| Err(PacketDecodeFault::GarbledRequest(why));
    if u32::try_from(bytes.len()).map_or(true, |len| len > limits.datagram_size_ceiling) {
        return refuse(TOO_BIG);
    }
    let mut cursor = ByteCursorReader::new(bytes);
    let datagram = PlayerToHostDatagram::unpack(&mut cursor)?;
    match (cursor.unread_len(), datagram.over_limit(limits)) {
        (0, None) => Ok(datagram),
        (0, Some(why)) => refuse(why),
        _ => refuse(LEFTOVER),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::net_links::datagrams::LinkNumber;

    const HEADER: DatagramPreamble = DatagramPreamble {
        connection: LinkNumber(2),
        packet_no: 3,
        confirm_receipt: 4,
        round_era: 5,
    };

    fn commands(n: u32) -> PlayerToHostDatagram {
        let seqs = 1..=n;
        PlayerToHostDatagram::Commands {
            header: HEADER,
            asserted_player_slot: 6,
            moves: seqs.clone().map(|s| (InputCommandNumber(s), InputCommand::default())).collect(),
            aim_origins: seqs.map(|s| (InputCommandNumber(s), Default::default())).collect(),
            actions: vec![],
            guaranteed_receipt: 7,
        }
    }

    fn decode(bytes: &[u8]) -> Result<PlayerToHostDatagram, PacketDecodeFault> {
        parse_receiver_datagram(bytes, &NegotiatedCaps::default())
    }

    fn malformed(why: &'static str) -> Result<PlayerToHostDatagram, PacketDecodeFault> {
        Err(PacketDecodeFault::GarbledRequest(why))
    }

    #[test]
    fn each_kind_survives_encode_then_decode() {
        for sent in [
            PlayerToHostDatagram::Connect(JoinGreeting::current(Default::default())),
            commands(0),
            commands(3),
            PlayerToHostDatagram::FrameReceipt { header: HEADER, frame_number_on_wire: 99 },
        ] {
            assert_eq!(decode(&sent.wire_bytes()), Ok(sent));
        }
    }

    #[test]
    fn an_extra_byte_is_refused() {
        let mut bytes = commands(1).wire_bytes();
        bytes.extend([0]);
        assert_eq!(decode(&bytes), malformed("trailing bytes after ClientPacket"));
    }

    #[test]
    fn an_unknown_tag_is_refused() {
        assert_eq!(decode(&[200]), malformed("unknown ClientPacket tag"));
    }

    #[test]
    fn every_prefix_of_a_packet_is_refused() {
        let bytes = commands(2).wire_bytes();
        assert!((0..bytes.len()).all(|cut| decode(&bytes[..cut]).is_err()));
    }

    #[test]
    fn a_datagram_over_the_byte_limit_is_refused() {
        let small = NegotiatedCaps { datagram_size_ceiling: 10, ..NegotiatedCaps::default() };
        assert_eq!(
            parse_receiver_datagram(&commands(1).wire_bytes(), &small),
            malformed("datagram exceeds max_packet_bytes")
        );
    }

    #[test]
    fn the_command_limit_is_inclusive() {
        let tight = NegotiatedCaps { moves_budget_per_step: 2, ..NegotiatedCaps::default() };
        assert!(parse_receiver_datagram(&commands(2).wire_bytes(), &tight).is_ok());
        assert_eq!(
            parse_receiver_datagram(&commands(3).wire_bytes(), &tight),
            malformed("cmd_count exceeds limit")
        );
    }

    #[test]
    fn a_huge_count_over_too_few_bytes_fails_without_allocating_it() {
        let mut bytes = commands(0).wire_bytes();
        // tag 1 + header 20 + claimed client 4 + reliable confirm_receipt 2 = offset 27 for the command count.
        bytes[27..29].copy_from_slice(&[0xff, 0xff]);
        assert!(decode(&bytes).is_err());
    }
}
