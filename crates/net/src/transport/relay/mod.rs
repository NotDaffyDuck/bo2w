//! The relay transport: authority and client talk through `RelayMailbox`es that the session
//! layer fills from and drains to the lobby relay, so no socket is owned here.
//!
//! - `host` is the authority side (`UdpAuthorityHub`),
//! - `join` is the client side (`UdpClientLink`),
//! - `peer` and `admission` are the per-member state the host keeps,
//! - `mailbox` is the pair of queues between this crate and the session layer.

mod admission;
mod host;
mod join;
mod mailbox;
mod peer;

pub use admission::CommittedAdmission;
pub use host::UdpAuthorityHub;
pub use join::UdpClientLink;
pub use mailbox::RelayMailbox;
