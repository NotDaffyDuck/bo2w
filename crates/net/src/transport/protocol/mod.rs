//! The datagram protocol between an authority and its clients.
//!
//! - `handshake` is what peers compare before playing together,
//! - `identity` is connection ids, packet headers and the authority's connection table,
//! - `fields` is the byte layout of the small structures packets are made of,
//! - `client` and `server` are the two directions' packets.

mod client;
mod fields;
mod handshake;
mod identity;
mod server;

pub use client::{ClientPacket, decode_client_packet};
pub use handshake::{
    ContentFingerprint, HandshakeHello, HandshakeReject, MAX_PACKET_BYTES, MatchDescriptor,
    ProtocolLimits, UDP_IMPLEMENTED, evaluate_handshake,
};
pub use identity::{ConnectionId, ConnectionTable, IdentityError, PacketHeader};
pub use server::{ServerPacket, decode_server_packet};
