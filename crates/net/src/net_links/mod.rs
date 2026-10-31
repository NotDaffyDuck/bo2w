//! Snapshots on the wire, the packet protocol, the UDP links and the link to the master server.

pub mod peer_baseline; // what the host keeps per peer to send changes
pub mod snapshot_history; // the host's ring of past frames
pub mod match_handoff; // the match hand-off
pub mod snapshot_diff; // snapshot encoders and decoders
pub mod datagram_split; // cutting packets into datagram fragments
pub mod tick_frames; // frames and the in-process transport
pub mod lobby_link; // the master server link
pub mod listen_link; // the in-process link a listen host uses
pub mod crossing_fields; // the player-state field table
pub mod datagrams; // packets and the handshake
pub mod relay; // the host's and joiner's UDP links
pub mod once_queue; // reliable events and action verdicts
pub mod snapshot_wire; // snapshot sections and world-object sync
pub mod send_fault; // send failures
pub mod byte_codec; // the byte reader and writer
