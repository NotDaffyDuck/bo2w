//! What the service knows about one room, and the short form shown in the room list.

use std::collections::HashMap;

use crate::{ListingId, AssetCoverageBits, RoomOccupantId, ROOM_OCCUPANT_LIMIT};

/// Where a room is in its life: picking settings, loading the map, or playing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LobbyStage {
    Gathering,
    Loading,
    Running,
}

impl LobbyStage {
    /// True once the host has started a match (loading counts).
    pub const fn is_playing(self) -> bool {
        match self {
            Self::Loading | Self::Running => true,
            Self::Gathering => false,
        }
    }
}

/// The full room state the service pushes to every member after each change.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LobbyOverview {
    pub lobby_number: ListingId,
    pub name: String,
    pub host: RoomOccupantId,
    /// In join order; the host is one of them.
    pub members: Vec<RoomOccupantId>,
    pub member_names: HashMap<RoomOccupantId, String>,
    pub map: String,
    pub mode: String,
    pub open_to_join: bool,
    /// Goes up by one on every change; the list uses it as the advert generation.
    pub revision: u64,
    /// Goes up by one per match started in this room.
    pub epoch: u32,
    pub phase: LobbyStage,
    pub seat_capacity: u8,
    pub requires: AssetCoverageBits,
}

impl LobbyOverview {
    /// The room-list entry for this room.
    pub fn advert(&self) -> RoomListing {
        let Self { lobby_number, name, map, mode, members, .. } = self;
        let Self { open_to_join, revision, phase, seat_capacity, requires, .. } = self;
        // A head count past what a session can hold shows as a full room.
        let head_count = members.len().try_into().unwrap_or(ROOM_OCCUPANT_LIMIT);
        RoomListing {
            id: *lobby_number,
            name: String::from(name),
            map: String::from(map),
            mode: String::from(mode),
            players: head_count,
            seat_capacity: *seat_capacity,
            locked: !*open_to_join,
            playing: phase.is_playing(),
            requires: *requires,
            generation: *revision,
        }
    }
    pub fn contains(&self, who: RoomOccupantId) -> bool {
        self.members.iter().any(|member| *member == who)
    }
}

/// One row of the room list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RoomListing {
    pub id: ListingId,
    pub name: String,
    pub map: String,
    pub mode: String,
    pub players: u8,
    pub seat_capacity: u8,
    pub locked: bool,
    pub playing: bool,
    pub requires: AssetCoverageBits,
    pub generation: u64,
}

impl RoomListing {
    /// Whether the menu should offer a join button for this row.
    pub const fn permits_join_ask(&self) -> bool {
        let Self { locked, .. } = self;
        may_ask_to_join(*locked)
    }
}

/// The one rule for offering a join: a locked room takes no requests. A room that is
/// full or mid-match still does; the service answers those itself.
pub const fn may_ask_to_join(locked: bool) -> bool {
    !locked
}
