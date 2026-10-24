//! What the service knows about one room, and the short form shown in the room list.

use std::collections::HashMap;

use crate::{AdvertId, ContentFlags, MemberId, MAX_SESSION_MEMBERS};

/// Where a room is in its life: picking settings, loading the map, or playing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RoomPhase {
    Gathering,
    Loading,
    Running,
}

impl RoomPhase {
    /// True once the host has started a match (loading counts).
    pub const fn in_match(self) -> bool {
        !matches!(self, Self::Gathering)
    }
}

/// The full room state the service pushes to every member after each change.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RoomView {
    pub room_id: AdvertId,
    pub name: String,
    pub host: MemberId,
    /// In join order; the host is one of them.
    pub members: Vec<MemberId>,
    pub member_names: HashMap<MemberId, String>,
    pub map: String,
    pub mode: String,
    pub joinable: bool,
    /// Goes up by one on every change; the list uses it as the advert generation.
    pub revision: u64,
    /// Goes up by one per match started in this room.
    pub epoch: u32,
    pub phase: RoomPhase,
    pub max_players: u8,
    pub requires: ContentFlags,
}

impl RoomView {
    /// The room-list entry for this room.
    pub fn advert(&self) -> Advert {
        Advert {
            id: self.room_id,
            name: self.name.clone(),
            map: self.map.clone(),
            mode: self.mode.clone(),
            players: u8::try_from(self.members.len()).unwrap_or(MAX_SESSION_MEMBERS),
            max_players: self.max_players,
            locked: !self.joinable,
            in_match: self.phase.in_match(),
            requires: self.requires,
            generation: self.revision,
        }
    }

    pub fn contains(&self, member_id: MemberId) -> bool {
        self.members.contains(&member_id)
    }
}

/// One row of the room list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Advert {
    pub id: AdvertId,
    pub name: String,
    pub map: String,
    pub mode: String,
    pub players: u8,
    pub max_players: u8,
    pub locked: bool,
    pub in_match: bool,
    pub requires: ContentFlags,
    pub generation: u64,
}

impl Advert {
    /// Whether the menu should offer a join button for this row.
    pub const fn allows_join_request(&self) -> bool {
        request_join_allowed(self.locked)
    }
}

/// The one rule for offering a join: a locked room takes no requests. A room that is
/// full or mid-match still does; the service answers those itself.
pub const fn request_join_allowed(locked: bool) -> bool {
    !locked
}
