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

pub use client::{PlayerToHostDatagram, parse_receiver_datagram};
pub use handshake::{
    AssetDigestSet, JoinGreeting, JoinRefusal, DATAGRAM_BYTE_CAP, MAX_SNAPSHOT_DECODED_BYTES,
    GameSessionSummary,
    NegotiatedCaps, DATAGRAM_TRANSPORT_READY, judge_greeting,
};
pub use identity::{LinkNumber, LinkRegistry, LinkIdentityFault, DatagramPreamble};
pub use server::{HostToPlayerDatagram, parse_host_datagram};
