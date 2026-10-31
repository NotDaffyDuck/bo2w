#![forbid(unsafe_code)] // plain safe Rust; the wire code needs nothing else
//! Wire protocol between the game and the lobby service (`bo2mp-master`).
//!
//! Two kinds of traffic share one QUIC connection:
//! - control envelopes on a bidirectional stream ([`ControlEnvelope`], [`encode_stream_frame`]),
//! - relayed game packets in datagrams or one-way streams ([`ForwardedPacket`]).
//!
//! Every multi-byte number is big-endian. Both ends must run the same [`WIRE_REVISION_NUMBER`].
mod body;
mod bytes;
mod error;
mod framing;
mod ids;
mod wire_caps;
mod wire_msgs;
mod relay;
mod room;
pub use error::ControlCodecFault;
pub use framing::{decode_control_frame, read_stream_body};
pub use framing::{encode_control_frame, write_stream_frame};
pub use framing::stream_frame_len;
pub use ids::{ListingId, AssetCoverageBits, RoomOccupantId};
pub use wire_caps::{Channel, UnrecognizedLane};
pub use wire_caps::{ALPN, WIRE_REV};
pub use wire_caps::{CONTROL_MESSAGE_SIZE_LIMIT, OPAQUE_BODY_BYTE_CAP};
pub use wire_caps::{TRANSFER_STREAM_CAP, TRANSFER_SIZE_LIMIT};
pub use wire_caps::{FORWARD_ONEWAY_CHANNEL_CAP, PARALLEL_TRANSFER_LIMIT};
pub use wire_caps::{LISTING_PAGE_LIMIT, LISTING_TITLE_BYTE_CAP, ROOM_OCCUPANT_LIMIT};
pub use wire_caps::{LEVEL_NAME_BYTE_CAP, RULESET_NAME_BYTE_CAP};
pub use wire_caps::{MAX_PLAYER_NAME_BYTES, VERSION_TEXT_BYTE_CAP};
pub use wire_caps::{LINK_IDLE_LIMIT, LINK_PING_EVERY};
pub use wire_msgs::{ControlEnvelope, ControlGreeting, PeerKindTag};
pub use wire_msgs::{ControlQuery, QueryPayload};
pub use wire_msgs::{ControlReply, HealthReply, ReplyPayload};
pub use wire_msgs::{DirectoryServiceFault, EntryRefusalReason};
pub use wire_msgs::{RoomMemberNotice, RoomShutdownCause};
pub use relay::ForwardedPacket;
pub use relay::{read_relay, read_relay_stream};
pub use relay::{write_relay, write_relay_stream};
pub use room::{may_ask_to_join, RoomListing, LobbyStage, LobbyOverview};
