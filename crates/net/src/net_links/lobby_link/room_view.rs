//! What the game reads back from the lobby link: who we are in the room, the
//! room as last seen, the server browser rows, and the menu's requests.

use std::collections::HashMap;

use bevy::prelude::Resource;
use master_protocol::{EntryRefusalReason, ListingId, AssetCoverageBits, RoomOccupantId, RoomShutdownCause};

use super::fault::LinkFailureRecord;
use ::frame::GameSessionStamp;

/// The ids that name this attempt, room, seat and match.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LobbySessionLabel {
    /// Unique per process; tells one connection attempt's logs from the next.
    pub join_try_number: u64,
    pub lobby_number: ListingId,
    pub member_id: RoomOccupantId,
    /// The room's match number; 0 before the first match starts.
    pub epoch: u32,
}

impl LobbySessionLabel {
    pub const fn unassigned(join_try_number: u64) -> LobbySessionLabel {
        let blank = [0u8; 16];
        LobbySessionLabel {
            epoch: 0,
            member_id: RoomOccupantId(blank),
            lobby_number: ListingId(blank),
            join_try_number,
        }
    }

    /// The match this identity is in, or `NONE` while it has no room or no match yet.
    pub fn game_session_key(&self) -> ::frame::GameSessionStamp {
        let has_room = self.lobby_number.0.iter().any(|&byte| byte != 0);
        if !(has_room && self.epoch != 0) {
            return GameSessionStamp::NONE;
        }
        GameSessionStamp::new(self.lobby_number.0, self.epoch)
    }
}

/// Things that happened to other seats or to the room, for the game to act on once.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DirectoryLifecycleNote {
    ParticipantDeparted {
        member_id: RoomOccupantId,
    },
    SessionEnded {
        reason: RoomShutdownCause,
    },
    EntryRefused {
        member_id: RoomOccupantId,
        reason: EntryRefusalReason,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DirectoryLinkStatus {
    Connecting {
        identity: LobbySessionLabel,
    },
    Hosting {
        identity: LobbySessionLabel,
        name: String,
        map: String,
        mode: String,
        members: Vec<RoomOccupantId>,
        member_names: HashMap<RoomOccupantId, String>,
        seat_capacity: u8,
        pass_vote_count: u8,
        in_match: bool,
    },
    Joining {
        identity: LobbySessionLabel,
    },
    Joined {
        identity: LobbySessionLabel,
        name: String,
        map: String,
        mode: String,
        members: Vec<RoomOccupantId>,
        member_names: HashMap<RoomOccupantId, String>,
        seat_capacity: u8,
        pass_vote_count: u8,
        playing: bool,
    },
    Closed {
        identity: LobbySessionLabel,
        reason: RoomShutdownCause,
    },
    Left {
        identity: LobbySessionLabel,
    },
    Failed {
        identity: LobbySessionLabel,
        error: LinkFailureRecord,
    },
}

/// The room facts shared by `Hosting` and `Joined`.
struct Seated<'a> {
    map: &'a str,
    mode: &'a str,
    members: &'a [RoomOccupantId],
    votes: u8,
    playing: bool,
}

impl DirectoryLinkStatus {
    fn seated(&self) -> Option<Seated<'_>> {
        let (Self::Joined {
            map,
            mode,
            members,
            pass_vote_count,
            playing: in_match,
            ..
        }
        | Self::Hosting {
            map,
            mode,
            members,
            pass_vote_count,
            in_match,
            ..
        }) = self
        else {
            return None;
        };
        let (votes, playing) = (*pass_vote_count, *in_match);
        Some(Seated {
            map,
            mode,
            members,
            votes,
            playing,
        })
    }

    pub fn identity(&self) -> LobbySessionLabel {
        match self {
            Self::Failed { identity: id, .. }
            | Self::Closed { identity: id, .. }
            | Self::Joined { identity: id, .. }
            | Self::Hosting { identity: id, .. }
            | Self::Left { identity: id }
            | Self::Joining { identity: id }
            | Self::Connecting { identity: id } => *id,
        }
    }

    pub fn map(&self) -> Option<&'_ str> {
        self.seated().map(|seat| seat.map)
    }

    pub fn mode(&self) -> Option<&'_ str> {
        self.seated().map(|seat| seat.mode)
    }

    pub fn members(&self) -> &'_ [RoomOccupantId] {
        self.seated().map_or(&[], |seat| seat.members)
    }

    /// The lobby's other players, in the room's order (the same order as `other_names`).
    pub fn other_ids(&self) -> Vec<RoomOccupantId> {
        let me = self.identity().member_id;
        self.members().iter().copied().filter(|id| *id != me).collect()
    }

    /// The lobby's other players' names, in the room's order.
    pub fn other_names(&self) -> Vec<String> {
        let me = self.identity().member_id;
        let names = match self {
            Self::Hosting { member_names, .. } | Self::Joined { member_names, .. } => member_names,
            _ => return Vec::new(),
        };
        self.members()
            .iter()
            .filter(|id| **id != me)
            .map(|id| names.get(id).cloned().unwrap_or_else(|| "Player".into()))
            .collect()
    }

    pub fn pass_vote_count(&self) -> u8 {
        self.seated().map_or(0, |seat| seat.votes)
    }

    /// The room's host removed this player (BO2's PartyClient_HandleKicked).
    pub fn kicked(&self) -> bool {
        matches!(self, Self::Closed { reason: RoomShutdownCause::Kicked, .. })
    }

    /// The room closed under this player for any other reason (its host left or stopped
    /// answering: BO2's PartyClient_HostTimedOut).
    pub fn host_gone(&self) -> bool {
        matches!(self, Self::Closed { reason, .. } if *reason != RoomShutdownCause::Kicked)
    }

    pub fn underway(&self) -> bool {
        self.seated().is_some_and(|seat| seat.playing)
    }

    /// Failed, closed by the room, or left on purpose: nothing more will happen.
    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            Self::Left { .. } | Self::Closed { .. } | Self::Failed { .. }
        )
    }

    /// The word written to the status file.
    pub(super) fn status_word(&self) -> &'static str {
        match self {
            Self::Left { .. } => "left",
            Self::Closed { .. } => "closed",
            Self::Failed { .. } => "failed",
            Self::Joined { .. } => "joined",
            Self::Hosting { .. } => "hosting",
            Self::Joining { .. } | Self::Connecting { .. } => "connecting",
        }
    }
}

/// One server browser row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DirectoryListing {
    pub id: ListingId,
    pub name: String,
    pub map: String,
    pub mode: String,
    pub players: u8,
    pub seat_capacity: u8,
    pub locked: bool,
    pub underway: bool,
    pub requires: AssetCoverageBits,
    /// Required packs this PC lacks; non-empty means it cannot join.
    pub missing: AssetCoverageBits,
}

impl DirectoryListing {
    pub(super) fn seen_with(advert: master_protocol::RoomListing, have: AssetCoverageBits) -> Self {
        let master_protocol::RoomListing {
            id,
            name,
            map,
            mode,
            players,
            seat_capacity,
            locked,
            playing: in_match,
            requires,
            ..
        } = advert;
        let missing = requires.lacking_in(have);
        Self {
            id,
            name,
            map,
            mode: mode,
            players: players,
            seat_capacity,
            locked: locked,
            underway: in_match,
            requires,
            missing,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DirectoryBrowserView {
    pub generation: u64,
    pub loading: bool,
    pub adverts: Vec<DirectoryListing>,
    pub error: Option<String>,
    pub have: AssetCoverageBits,
}

#[derive(Clone, Debug)]
pub enum DirectoryMenuChoice {
    Refresh,
    Host {
        map: String,
        mode: String,
    },
    Join {
        listing_token: ListingId,
        map: String,
        mode: String,
    },
    RefreshRoom {
        map: String,
        mode: String,
    },
    BeginGame {
        map: String,
        mode: String,
    },
    BallotForSkip,
    /// BO2's "kickplayer <xuid>" from the host's lobby.
    Kick {
        member_id: RoomOccupantId,
    },
    ExitRoom,
}

/// The menu leaves one action here; the link takes it on the next frame.
#[derive(Resource, Default)]
pub struct QueuedMenuChoice(pub Option<DirectoryMenuChoice>);

/// A match the room just started, offered once to the session layer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DirectoryGameOffer {
    pub map: String,
    pub mode: String,
    pub game_session_key: GameSessionStamp,
}

#[derive(Resource, Default)]
pub struct DirectoryMatchBegin(pub(super) Option<DirectoryGameOffer>);

impl DirectoryMatchBegin {
    pub fn take(&mut self) -> Option<DirectoryGameOffer> {
        std::mem::take(&mut self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seated_identity() -> LobbySessionLabel {
        let blank = LobbySessionLabel::unassigned(7);
        LobbySessionLabel {
            epoch: 2,
            lobby_number: ListingId([3; 16]),
            member_id: RoomOccupantId([4; 16]),
            ..blank
        }
    }

    fn joined(in_match: bool) -> DirectoryLinkStatus {
        DirectoryLinkStatus::Joined {
            identity: seated_identity(),
            name: "room".into(),
            map: "mp_x".into(),
            mode: "tdm".into(),
            members: vec![RoomOccupantId([1; 16]), RoomOccupantId([4; 16])],
            member_names: HashMap::new(),
            seat_capacity: 12,
            pass_vote_count: 1,
            playing: in_match,
        }
    }

    #[test]
    fn match_key_needs_both_room_and_epoch() {
        assert!(LobbySessionLabel::unassigned(1).game_session_key().is_none());
        let identity = seated_identity();
        assert_eq!(identity.game_session_key(), GameSessionStamp::new([3; 16], 2));
        let no_epoch = LobbySessionLabel {
            epoch: 0,
            ..identity
        };
        let no_room = LobbySessionLabel {
            lobby_number: ListingId([0; 16]),
            ..identity
        };
        for lacking in [no_epoch, no_room] {
            assert!(lacking.game_session_key().is_none());
        }
    }

    #[test]
    fn seated_states_expose_the_room() {
        let state = joined(true);
        assert_eq!(state.map(), Some("mp_x"));
        assert_eq!(state.mode(), Some("tdm"));
        assert_eq!(state.members().len(), 2);
        assert_eq!(state.pass_vote_count(), 1);
        assert!(state.underway());
        assert!(!state.is_terminal());
        assert_eq!(state.identity().join_try_number, 7);
        assert_eq!(state.status_word(), "joined");
    }

    #[test]
    fn unseated_states_have_no_room() {
        let identity = seated_identity();
        let waiting = DirectoryLinkStatus::Joining { identity };
        assert_eq!(waiting.map(), None);
        assert!(waiting.members().is_empty());
        assert!(!waiting.underway());
        assert_eq!(waiting.status_word(), "connecting");
        let left = DirectoryLinkStatus::Left { identity };
        assert!(left.is_terminal());
        let failed = DirectoryLinkStatus::Failed {
            identity,
            error: LinkFailureRecord::new("connect", "join", "refused"),
        };
        assert!(failed.is_terminal());
        assert_eq!(failed.identity(), identity);
        assert_eq!(failed.status_word(), "failed");
    }

    #[test]
    fn advert_marks_packs_this_pc_lacks() {
        let advert = master_protocol::RoomListing {
            id: ListingId([9; 16]),
            name: "n".into(),
            map: "m".into(),
            mode: "dm".into(),
            players: 1,
            seat_capacity: 18,
            playing: false,
            requires: AssetCoverageBits(0b1100),
            generation: 3,
            locked: Default::default(),
        };
        let row = DirectoryListing::seen_with(advert, AssetCoverageBits(0b0100));
        assert_eq!(row.missing, AssetCoverageBits(0b1000));
        assert_eq!(row.requires, AssetCoverageBits(0b1100));
    }

    #[test]
    fn a_match_offer_is_taken_once() {
        let offer = DirectoryGameOffer {
            mode: "dm".into(),
            game_session_key: GameSessionStamp::new([1; 16], 1),
            map: "m".into(),
        };
        let mut start = DirectoryMatchBegin(Some(offer));
        assert!(start.take().is_some());
        assert!(start.take().is_none());
    }
}
