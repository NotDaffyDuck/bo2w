//! Game packets forwarded by the service between a host and its members. Each one is
//! the low byte of the protocol version, a route byte, the member id for the two routes
//! that name one, then the packet itself.

use crate::{RoomOccupantId, ControlCodecFault, TRANSFER_SIZE_LIMIT, OPAQUE_BODY_BYTE_CAP, WIRE_REV};

/// One forwarded packet and the way it travels.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ForwardedPacket<'a> {
    /// A member's packet on its way to the host.
    ReceiverToHost(&'a [u8]),
    /// The host's packet for one member.
    HostToParticipant {
        member_id: RoomOccupantId,
        payload: &'a [u8],
    },
    /// A member's packet as the service hands it to the host, tagged with the sender.
    DirectoryToHost {
        member_id: RoomOccupantId,
        payload: &'a [u8],
    },
    /// The host's packet as the service hands it to the member.
    DirectoryToParticipant(&'a [u8]),
}

const VERSION_BYTE: u8 = WIRE_REV as u8;

/// Route bytes.
mod route {
    pub const TO_HOST: u8 = 1;
    pub const TO_MEMBER: u8 = 2;
    pub const FROM_SERVICE_TO_HOST: u8 = 3;
    pub const FROM_SERVICE_TO_MEMBER: u8 = 4;
}

/// A datagram-sized packet (game traffic).
pub fn write_relay(datagram: ForwardedPacket<'_>) -> Result<Vec<u8>, ControlCodecFault> {
    pack(datagram, OPAQUE_BODY_BYTE_CAP)
}

/// A join snapshot sent on its own one-way stream.
pub fn write_relay_stream(datagram: ForwardedPacket<'_>) -> Result<Vec<u8>, ControlCodecFault> {
    pack(datagram, TRANSFER_SIZE_LIMIT)
}

pub fn read_relay(bytes: &[u8]) -> Result<ForwardedPacket<'_>, ControlCodecFault> {
    unpack(bytes, OPAQUE_BODY_BYTE_CAP)
}

pub fn read_relay_stream(bytes: &[u8]) -> Result<ForwardedPacket<'_>, ControlCodecFault> {
    unpack(bytes, TRANSFER_SIZE_LIMIT)
}

fn pack(datagram: ForwardedPacket<'_>, cap: usize) -> Result<Vec<u8>, ControlCodecFault> {
    let (code, addressee, packet) = match datagram {
        ForwardedPacket::ReceiverToHost(packet) => (route::TO_HOST, None, packet),
        ForwardedPacket::HostToParticipant { member_id: to, payload: body } => (route::TO_MEMBER, Some(to), body),
        ForwardedPacket::DirectoryToHost { member_id: from, payload: body } => {
            (route::FROM_SERVICE_TO_HOST, Some(from), body)
        }
        ForwardedPacket::DirectoryToParticipant(packet) => (route::FROM_SERVICE_TO_MEMBER, None, packet),
    };
    if packet.len() > cap {
        return Err(ControlCodecFault::SizeOverLimit(packet.len()));
    }
    let id_bytes = addressee.as_ref().map_or(&[][..], |member| &member.0[..]);
    Ok([&[VERSION_BYTE, code][..], id_bytes, packet].concat())
}

fn unpack(bytes: &[u8], cap: usize) -> Result<ForwardedPacket<'_>, ControlCodecFault> {
    let [version, code, after @ ..] = bytes else {
        return Err(ControlCodecFault::Truncated);
    };
    if *version != VERSION_BYTE {
        return Err(ControlCodecFault::WireRevisionSkew {
            local: WIRE_REV,
            remote: u16::from(*version),
        });
    }
    let names_member = matches!(*code, route::TO_MEMBER | route::FROM_SERVICE_TO_HOST);
    let (addressee, packet) = if names_member {
        let (id, packet) = after
            .split_first_chunk::<16>()
            .ok_or(ControlCodecFault::Truncated)?;
        (Some(RoomOccupantId(*id)), packet)
    } else {
        (None, after)
    };
    if packet.len() > cap {
        return Err(ControlCodecFault::SizeOverLimit(packet.len()));
    }
    match (*code, addressee) {
        (route::TO_HOST, None) => Ok(ForwardedPacket::ReceiverToHost(packet)),
        (route::TO_MEMBER, Some(member_id)) => Ok(ForwardedPacket::HostToParticipant {
            member_id,
            payload: packet,
        }),
        (route::FROM_SERVICE_TO_HOST, Some(member_id)) => Ok(ForwardedPacket::DirectoryToHost {
            member_id,
            payload: packet,
        }),
        (route::FROM_SERVICE_TO_MEMBER, None) => Ok(ForwardedPacket::DirectoryToParticipant(packet)),
        (other, _) => Err(ControlCodecFault::UnrecognizedKindByte(other)),
    }
}
