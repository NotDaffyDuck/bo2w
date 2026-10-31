//! Every message that crosses the control stream, as plain data.

use core::fmt;

use crate::{
    RoomListing, ListingId, AssetCoverageBits, RoomOccupantId, LobbyOverview, OPAQUE_BODY_BYTE_CAP, ROOM_OCCUPANT_LIMIT,
    WIRE_REV,
};

/// Which kind of program opened the connection.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PeerKindTag {
    /// A command-line tool asking for status or the room list.
    Cli,
    Host,
    Join,
}

/// First frame on every control stream, in both directions.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ControlGreeting {
    pub wire_rev: u16,
    /// The game's own net protocol number; a host and its joiners must agree.
    pub match_wire_version: u32,
    pub role: PeerKindTag,
    pub build: String,
    pub display_name: String,
}

/// Why a member could not be let into a running match.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EntryRefusalReason {
    CutoffInstant,
    Cancelled,
    /// The two sides hold different map content (digests attached).
    LevelAssetHashes { host_digest: u64, peer_digest: u64 },
    WeaponLedger { host_digest: u64, peer_digest: u64 },
    LoadoutTableHashes { host_digest: u64, peer_digest: u64 },
}

impl core::fmt::Display for EntryRefusalReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let (what, host, peer, hint) = match *self {
            Self::CutoffInstant => return f.write_str("peer admission deadline"),
            Self::Cancelled => return f.write_str("peer admission cancelled"),
            Self::LevelAssetHashes { host_digest: host, peer_digest: peer } => ("map content", host, peer, ""),
            Self::WeaponLedger { host_digest: host, peer_digest: peer } => ("weapon table", host, peer, ""),
            Self::LoadoutTableHashes { host_digest: host, peer_digest: peer } => (
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
pub struct ControlQuery {
    pub call_id: u64,
    pub body: QueryPayload,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum QueryPayload {
    Status,
    OpenLobby {
        name: String,
        map: String,
        mode: String,
        seat_capacity: u8,
        requires: AssetCoverageBits,
    },
    JoinRoom {
        lobby_number: ListingId,
        have: AssetCoverageBits,
    },
    LeaveRoom,
    ConfigureLobby {
        map: String,
        mode: String,
        open_to_join: bool,
    },
    BeginGame {
        map: String,
        mode: String,
    },
    EndMatch {
        lobby_number: ListingId,
        epoch: u32,
    },
    CloseRoom,
    ListRooms,
    /// Host: the match world is built; these are its content digests.
    HostMapLoaded {
        lobby_number: ListingId,
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
    TransferInstalledMsg {
        epoch: u32,
        bootstrap_id: u32,
        frame_number_on_wire: u32,
        connection: u64,
    },
    /// Host: this member now has a client slot in the match.
    JoinGame {
        member_id: RoomOccupantId,
        epoch: u32,
        bootstrap_id: u32,
        connection_id: u64,
        player_slot_no: u32,
    },
    EntryRefused {
        member_id: RoomOccupantId,
        epoch: u32,
        reason: EntryRefusalReason,
    },
    BallotForSkip,
    /// The room's host removes a member (BO2's `UI_KickPlayer_f`).
    KickMember {
        member_id: RoomOccupantId,
    },
}

/// The service's answer to a request.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ControlReply {
    pub call_id: u64,
    pub body: ReplyPayload,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ReplyPayload {
    Status(HealthReply),
    LobbyOpened { member_id: RoomOccupantId, view: LobbyOverview },
    LobbyEntered { member_id: RoomOccupantId, view: LobbyOverview },
    RoomLeft,
    RoomUpdated { view: LobbyOverview },
    RoomList { generation: u64, adverts: Vec<RoomListing> },
    Ack,
    Error(DirectoryServiceFault),
}

/// What a running service reports about itself.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HealthReply {
    pub wire_rev: u16,
    pub opaque_body_ceiling: u16,
    pub member_count_ceiling: u8,
}

impl HealthReply {
    /// What this build of the protocol offers.
    pub const CURRENT: Self = {
        let (version, payload_cap, member_cap) =
            (WIRE_REV, OPAQUE_BODY_BYTE_CAP as u16, ROOM_OCCUPANT_LIMIT);
        Self {
            wire_rev: version,
            opaque_body_ceiling: payload_cap,
            member_count_ceiling: member_cap,
        }
    };
}

/// A request the service refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DirectoryServiceFault {
    GarbledRequest,
    ListingRejected,
    ListingsFull,
    HostSlotTaken,
    NoSuchListing,
    Full,
    Locked,
    NotRoomOwner,
    NotRoomOccupant,
    /// The joiner lacks content packs the room needs.
    AbsentResource {
        requires: AssetCoverageBits,
        have: AssetCoverageBits,
    },
}

impl core::fmt::Display for DirectoryServiceFault {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(self, f)
    }
}

impl core::error::Error for DirectoryServiceFault {}

/// Why the service closed a room.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RoomShutdownCause {
    HostLeft,
    TenancyLapsed,
    Overload,
    Kicked,
}

/// Admission traffic between the host and one member, forwarded by the service.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RoomMemberNotice {
    HostMapLoaded {
        epoch: u32,
        map: u64,
        weapon_digest: u64,
        class_digest: u64,
    },
    MapLoaded {
        member_id: RoomOccupantId,
        epoch: u32,
        map: u64,
        weapon_digest: u64,
        class_digest: u64,
    },
    TransferInstalledMsg {
        member_id: RoomOccupantId,
        epoch: u32,
        transfer_id: u32,
        frame_number_on_wire: u32,
        connection: u64,
    },
    JoinGame {
        member_id: RoomOccupantId,
        epoch: u32,
        transfer_id: u32,
        link_id: u64,
        player_slot_no: u32,
    },
    EntryRefused {
        member_id: RoomOccupantId,
        epoch: u32,
        reason: EntryRefusalReason,
    },
    BallotForSkip {
        member_id: RoomOccupantId,
    },
}

/// One message on the control stream.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ControlEnvelope {
    /// Game bytes tunnelled over the control stream when datagrams are not available.
    PassAlong(Vec<u8>),
    Hello(ControlGreeting),
    Request(ControlQuery),
    Response(ControlReply),
    /// Unprompted push of the room after a change.
    LobbyOverview(LobbyOverview),
    Closed {
        lobby_number: ListingId,
        epoch: u32,
        reason: RoomShutdownCause,
    },
    RoomMemberNotice(RoomMemberNotice),
}
