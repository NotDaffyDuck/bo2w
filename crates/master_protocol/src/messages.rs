//! Every message that crosses the control stream, as plain data.

use core::fmt;

use crate::{
    Advert, AdvertId, ContentFlags, MemberId, RoomView, MAX_OPAQUE_PAYLOAD, MAX_SESSION_MEMBERS,
    PROTOCOL_VERSION,
};

/// Which kind of program opened the connection.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EndpointRole {
    /// A command-line tool asking for status or the room list.
    Cli,
    Host,
    Join,
}

/// First frame on every control stream, in both directions.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ControlHello {
    pub protocol_version: u16,
    /// The game's own net protocol number; a host and its joiners must agree.
    pub game_protocol: u32,
    pub role: EndpointRole,
    pub build: String,
    pub player_name: String,
}

/// Why a member could not be let into a running match.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AdmissionFailure {
    Deadline,
    Cancelled,
    /// The two sides hold different map content (digests attached).
    MapContent { host: u64, peer: u64 },
    WeaponRegistry { host: u64, peer: u64 },
    ClassRegistry { host: u64, peer: u64 },
}

impl fmt::Display for AdmissionFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let (what, host, peer, hint) = match *self {
            Self::Deadline => return f.write_str("peer admission deadline"),
            Self::Cancelled => return f.write_str("peer admission cancelled"),
            Self::MapContent { host, peer } => ("map content", host, peer, ""),
            Self::WeaponRegistry { host, peer } => ("weapon table", host, peer, ""),
            Self::ClassRegistry { host, peer } => (
                "class catalog",
                host,
                peer,
                "; use the same game build as the host",
            ),
        };
        write!(f, "{what} mismatch: host={host:016x} peer={peer:016x}{hint}")
    }
}

/// A client's request; the service answers with the same `request_id`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ControlRequest {
    pub request_id: u64,
    pub body: RequestBody,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RequestBody {
    Status,
    CreateRoom {
        name: String,
        map: String,
        mode: String,
        max_players: u8,
        requires: ContentFlags,
    },
    JoinRoom {
        room_id: AdvertId,
        have: ContentFlags,
    },
    LeaveRoom,
    SetOptions {
        map: String,
        mode: String,
        joinable: bool,
    },
    StartMatch {
        map: String,
        mode: String,
    },
    EndMatch {
        room_id: AdvertId,
        epoch: u32,
    },
    CloseRoom,
    ListRooms,
    /// Host: the match world is built; these are its content digests.
    HostWorldReady {
        room_id: AdvertId,
        epoch: u32,
        map: u64,
        weapons: u64,
        classes: u64,
    },
    /// Joiner: the map is loaded; these are the joiner's digests.
    MapLoaded {
        epoch: u32,
        map: u64,
        weapons: u64,
        classes: u64,
    },
    /// Joiner: the host's join snapshot has been applied.
    BootstrapApplied {
        epoch: u32,
        bootstrap_id: u32,
        snapshot_seq: u32,
        connection: u64,
    },
    /// Host: this member now has a client slot in the match.
    EnterMatch {
        member_id: MemberId,
        epoch: u32,
        bootstrap_id: u32,
        connection_id: u64,
        client_id: u32,
    },
    AdmissionFailed {
        member_id: MemberId,
        epoch: u32,
        reason: AdmissionFailure,
    },
    VoteToSkip,
}

/// The service's answer to a request.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ControlResponse {
    pub request_id: u64,
    pub body: ResponseBody,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ResponseBody {
    Status(StatusResponse),
    RoomCreated { member_id: MemberId, view: RoomView },
    RoomJoined { member_id: MemberId, view: RoomView },
    RoomLeft,
    RoomUpdated { view: RoomView },
    RoomList { generation: u64, adverts: Vec<Advert> },
    Ack,
    Error(ServiceError),
}

/// What a running service reports about itself.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StatusResponse {
    pub protocol_version: u16,
    pub max_opaque_payload: u16,
    pub max_session_members: u8,
}

impl StatusResponse {
    /// The values this build of the protocol serves.
    pub const CURRENT: Self = Self {
        protocol_version: PROTOCOL_VERSION,
        max_opaque_payload: MAX_OPAQUE_PAYLOAD as u16,
        max_session_members: MAX_SESSION_MEMBERS,
    };
}

/// A request the service refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ServiceError {
    Malformed,
    InvalidAdvert,
    AdvertCapacity,
    AlreadyHosting,
    UnknownAdvert,
    Full,
    Locked,
    NotHost,
    NotMember,
    /// The joiner lacks content packs the room needs.
    MissingContent {
        requires: ContentFlags,
        have: ContentFlags,
    },
}

impl fmt::Display for ServiceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(self, f)
    }
}

impl std::error::Error for ServiceError {}

/// Why the service closed a room.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SessionCloseReason {
    HostLeft,
    LeaseExpired,
    Overload,
}

/// Admission traffic between the host and one member, forwarded by the service.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PeerEvent {
    HostWorldReady {
        epoch: u32,
        map: u64,
        weapons: u64,
        classes: u64,
    },
    MapLoaded {
        member_id: MemberId,
        epoch: u32,
        map: u64,
        weapons: u64,
        classes: u64,
    },
    BootstrapApplied {
        member_id: MemberId,
        epoch: u32,
        bootstrap_id: u32,
        snapshot_seq: u32,
        connection: u64,
    },
    EnterMatch {
        member_id: MemberId,
        epoch: u32,
        bootstrap_id: u32,
        connection_id: u64,
        client_id: u32,
    },
    AdmissionFailed {
        member_id: MemberId,
        epoch: u32,
        reason: AdmissionFailure,
    },
    VoteToSkip {
        member_id: MemberId,
    },
}

/// One message on the control stream.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ControlFrame {
    /// Game bytes tunnelled over the control stream when datagrams are not available.
    Relay(Vec<u8>),
    Hello(ControlHello),
    Request(ControlRequest),
    Response(ControlResponse),
    /// Unprompted push of the room after a change.
    RoomView(RoomView),
    Closed {
        room_id: AdvertId,
        epoch: u32,
        reason: SessionCloseReason,
    },
    PeerEvent(PeerEvent),
}
