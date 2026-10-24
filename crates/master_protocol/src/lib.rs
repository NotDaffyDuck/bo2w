#![forbid(unsafe_code)] // plain safe Rust; the wire code needs nothing else
//! Wire protocol between the game and the lobby service (`bo2mp-master`).
//!
//! Two kinds of traffic share one QUIC connection:
//! - control envelopes on a bidirectional stream ([`ControlFrame`], [`encode_stream_frame`]),
//! - relayed game packets in datagrams or one-way streams ([`RelayDatagram`]).
//!
//! Every multi-byte number is big-endian. Both ends must run the same [`PROTOCOL_VERSION`].
mod body;
mod bytes;
mod error;
mod frame;
mod ids;
mod limits;
mod messages;
mod relay;
mod room;
pub use error::ProtocolError;
pub use frame::{
    decode_control_frame, decode_stream_payload, encode_control_frame, encode_stream_frame,
    stream_frame_len,
};
pub use ids::{AdvertId, ContentFlags, MemberId};
pub use limits::{
    Channel, UnknownChannel, ALPN, MAX_ADVERT_NAME_BYTES, MAX_BOOTSTRAP_PAYLOAD,
    MAX_BOOTSTRAP_STREAM_BYTES, MAX_BUILD_BYTES, MAX_CONCURRENT_BOOTSTRAP, MAX_CONTROL_BYTES,
    MAX_LIST_ADVERTS, MAX_MAP_BYTES, MAX_MODE_BYTES, MAX_OPAQUE_PAYLOAD, MAX_PLAYER_NAME_BYTES,
    MAX_RELAY_UNI_STREAMS, MAX_SESSION_MEMBERS, PROTOCOL_VERSION, SESSION_IDLE,
    SESSION_KEEP_ALIVE,
};
pub use messages::{
    AdmissionFailure, ControlFrame, ControlHello, ControlRequest, ControlResponse, EndpointRole,
    PeerEvent, RequestBody, ResponseBody, ServiceError, SessionCloseReason, StatusResponse,
};
pub use relay::{decode_relay, decode_relay_stream, encode_relay, encode_relay_stream, RelayDatagram};
pub use room::{request_join_allowed, Advert, RoomPhase, RoomView};
