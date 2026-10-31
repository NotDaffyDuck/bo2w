//! The relay transport: authority and client talk through `ForwardingInbox`es that the session
//! layer fills from and drains to the lobby relay, so no socket is owned here.
//!
//! - `host` is the authority side (`HostUdpSwitchboard`),
//! - `join` is the client side (`PlayerUdpSession`),
//! - `peer` and `admission` are the per-member state the host keeps,
//! - `mailbox` is the pair of queues between this crate and the session layer.

mod entry_gate;
mod host;
mod join;
mod mailbox;
mod peer;

pub use entry_gate::FinalizedEntry;
pub use host::HostUdpSwitchboard;
pub use join::PlayerUdpSession;
pub use mailbox::ForwardingInbox;
