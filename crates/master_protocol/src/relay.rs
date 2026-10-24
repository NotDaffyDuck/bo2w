//! Game packets forwarded by the service between a host and its members. Each one is
//! the low byte of the protocol version, a route byte, the member id for the two routes
//! that name one, then the packet itself.

use crate::{MemberId, ProtocolError, MAX_BOOTSTRAP_PAYLOAD, MAX_OPAQUE_PAYLOAD, PROTOCOL_VERSION};

/// One forwarded packet and the way it travels.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RelayDatagram<'a> {
    /// A member's packet on its way to the host.
    ClientToHost(&'a [u8]),
    /// The host's packet for one member.
    HostToMember {
        member_id: MemberId,
        payload: &'a [u8],
    },
    /// A member's packet as the service hands it to the host, tagged with the sender.
    ServiceToHost {
        member_id: MemberId,
        payload: &'a [u8],
    },
    /// The host's packet as the service hands it to the member.
    ServiceToMember(&'a [u8]),
}

const VERSION_BYTE: u8 = PROTOCOL_VERSION as u8;

/// Route bytes.
mod route {
    pub const TO_HOST: u8 = 1;
    pub const TO_MEMBER: u8 = 2;
    pub const FROM_SERVICE_TO_HOST: u8 = 3;
    pub const FROM_SERVICE_TO_MEMBER: u8 = 4;
}

/// A datagram-sized packet (game traffic).
pub fn encode_relay(datagram: RelayDatagram<'_>) -> Result<Vec<u8>, ProtocolError> {
    pack(datagram, MAX_OPAQUE_PAYLOAD)
}

/// A join snapshot sent on its own one-way stream.
pub fn encode_relay_stream(datagram: RelayDatagram<'_>) -> Result<Vec<u8>, ProtocolError> {
    pack(datagram, MAX_BOOTSTRAP_PAYLOAD)
}

pub fn decode_relay(bytes: &[u8]) -> Result<RelayDatagram<'_>, ProtocolError> {
    unpack(bytes, MAX_OPAQUE_PAYLOAD)
}

pub fn decode_relay_stream(bytes: &[u8]) -> Result<RelayDatagram<'_>, ProtocolError> {
    unpack(bytes, MAX_BOOTSTRAP_PAYLOAD)
}

fn pack(datagram: RelayDatagram<'_>, cap: usize) -> Result<Vec<u8>, ProtocolError> {
    use RelayDatagram as D;
    let (code, addressee, packet) = match datagram {
        D::ClientToHost(packet) => (route::TO_HOST, None, packet),
        D::HostToMember { member_id, payload } => (route::TO_MEMBER, Some(member_id), payload),
        D::ServiceToHost { member_id, payload } => {
            (route::FROM_SERVICE_TO_HOST, Some(member_id), payload)
        }
        D::ServiceToMember(packet) => (route::FROM_SERVICE_TO_MEMBER, None, packet),
    };
    if packet.len() > cap {
        return Err(ProtocolError::LengthExceeded(packet.len()));
    }
    let id_bytes = addressee.as_ref().map_or(&[][..], |member| &member.0[..]);
    Ok([&[VERSION_BYTE, code][..], id_bytes, packet].concat())
}

fn unpack(bytes: &[u8], cap: usize) -> Result<RelayDatagram<'_>, ProtocolError> {
    use RelayDatagram as D;
    let [version, code, after @ ..] = bytes else {
        return Err(ProtocolError::Truncated);
    };
    if *version != VERSION_BYTE {
        return Err(ProtocolError::VersionMismatch {
            ours: PROTOCOL_VERSION,
            theirs: u16::from(*version),
        });
    }
    let names_member = matches!(*code, route::TO_MEMBER | route::FROM_SERVICE_TO_HOST);
    let (addressee, packet) = if names_member {
        let (id, packet) = after
            .split_first_chunk::<16>()
            .ok_or(ProtocolError::Truncated)?;
        (Some(MemberId(*id)), packet)
    } else {
        (None, after)
    };
    if packet.len() > cap {
        return Err(ProtocolError::LengthExceeded(packet.len()));
    }
    match (*code, addressee) {
        (route::TO_HOST, None) => Ok(D::ClientToHost(packet)),
        (route::TO_MEMBER, Some(member_id)) => Ok(D::HostToMember {
            member_id,
            payload: packet,
        }),
        (route::FROM_SERVICE_TO_HOST, Some(member_id)) => Ok(D::ServiceToHost {
            member_id,
            payload: packet,
        }),
        (route::FROM_SERVICE_TO_MEMBER, None) => Ok(D::ServiceToMember(packet)),
        (other, _) => Err(ProtocolError::UnknownTag(other)),
    }
}
