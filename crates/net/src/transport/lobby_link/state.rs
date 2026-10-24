//! What the game reads back from the lobby link: who we are in the room, the
//! room as last seen, the server browser rows, and the menu's requests.

use std::collections::HashMap;

use bevy::prelude::Resource;
use master_protocol::{AdmissionFailure, AdvertId, ContentFlags, MemberId, SessionCloseReason};

use super::fault::TransportFault;
use ::frame::MatchKey;

/// The ids that name this attempt, room, seat and match.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SessionIdentity {
    /// Unique per process; tells one connection attempt's logs from the next.
    pub attempt_id: u64,
    pub room_id: AdvertId,
    pub member_id: MemberId,
    /// The room's match number; 0 before the first match starts.
    pub epoch: u32,
}

impl SessionIdentity {
    pub const fn unassigned(attempt_id: u64) -> Self {
        Self {
            attempt_id,
            room_id: AdvertId([0; 16]),
            member_id: MemberId([0; 16]),
            epoch: 0,
        }
    }

    /// The match this identity is in, or `NONE` while it has no room or no match yet.
    pub fn match_key(&self) -> MatchKey {
        let roomless = self.room_id.0.iter().all(|byte| *byte == 0);
        if roomless || self.epoch == 0 {
            return MatchKey::NONE;
        }
        MatchKey::new(self.room_id.0, self.epoch)
    }
}

/// Things that happened to other seats or to the room, for the game to act on once.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MasterLifecycleFact {
    MemberLeft {
        member_id: MemberId,
    },
    SessionClosed {
        reason: SessionCloseReason,
    },
    AdmissionFailed {
        member_id: MemberId,
        reason: AdmissionFailure,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MasterBridgeState {
    Connecting {
        identity: SessionIdentity,
    },
    Hosting {
        identity: SessionIdentity,
        name: String,
        map: String,
        mode: String,
        members: Vec<MemberId>,
        member_names: HashMap<MemberId, String>,
        max_players: u8,
        skip_votes: u8,
        in_match: bool,
    },
    Joining {
        identity: SessionIdentity,
    },
    Joined {
        identity: SessionIdentity,
        name: String,
        map: String,
        mode: String,
        members: Vec<MemberId>,
        member_names: HashMap<MemberId, String>,
        max_players: u8,
        skip_votes: u8,
        in_match: bool,
    },
    Closed {
        identity: SessionIdentity,
        reason: SessionCloseReason,
    },
    Left {
        identity: SessionIdentity,
    },
    Failed {
        identity: SessionIdentity,
        error: TransportFault,
    },
}

/// The room facts shared by `Hosting` and `Joined`.
struct Seated<'a> {
    map: &'a str,
    mode: &'a str,
    members: &'a [MemberId],
    skip_votes: u8,
    in_match: bool,
}

impl MasterBridgeState {
    fn seated(&self) -> Option<Seated<'_>> {
        match self {
            Self::Hosting {
                map,
                mode,
                members,
                skip_votes,
                in_match,
                ..
            }
            | Self::Joined {
                map,
                mode,
                members,
                skip_votes,
                in_match,
                ..
            } => Some(Seated {
                map,
                mode,
                members,
                skip_votes: *skip_votes,
                in_match: *in_match,
            }),
            _ => None,
        }
    }

    pub fn identity(&self) -> SessionIdentity {
        let (Self::Connecting { identity }
        | Self::Joining { identity }
        | Self::Left { identity }
        | Self::Hosting { identity, .. }
        | Self::Joined { identity, .. }
        | Self::Closed { identity, .. }
        | Self::Failed { identity, .. }) = self;
        *identity
    }

    pub fn map(&self) -> Option<&str> {
        self.seated().map(|seat| seat.map)
    }

    pub fn mode(&self) -> Option<&str> {
        self.seated().map(|seat| seat.mode)
    }

    pub fn members(&self) -> &[MemberId] {
        self.seated().map_or(&[], |seat| seat.members)
    }

    pub fn skip_votes(&self) -> u8 {
        self.seated().map_or(0, |seat| seat.skip_votes)
    }

    pub fn in_match(&self) -> bool {
        self.seated().is_some_and(|seat| seat.in_match)
    }

    /// Failed, closed by the room, or left on purpose: nothing more will happen.
    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            Self::Failed { .. } | Self::Closed { .. } | Self::Left { .. }
        )
    }

    /// The word written to the status file.
    pub(super) fn status_word(&self) -> &'static str {
        match self {
            Self::Connecting { .. } | Self::Joining { .. } => "connecting",
            Self::Hosting { .. } => "hosting",
            Self::Joined { .. } => "joined",
            Self::Failed { .. } => "failed",
            Self::Closed { .. } => "closed",
            Self::Left { .. } => "left",
        }
    }
}

/// One server browser row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MasterAdvert {
    pub id: AdvertId,
    pub name: String,
    pub map: String,
    pub mode: String,
    pub players: u8,
    pub max_players: u8,
    pub locked: bool,
    pub in_match: bool,
    pub requires: ContentFlags,
    /// Required packs this PC lacks; non-empty means it cannot join.
    pub missing: ContentFlags,
}

impl MasterAdvert {
    pub(super) fn seen_with(advert: master_protocol::Advert, have: ContentFlags) -> Self {
        Self {
            missing: advert.requires.missing_from(have),
            id: advert.id,
            name: advert.name,
            map: advert.map,
            mode: advert.mode,
            players: advert.players,
            max_players: advert.max_players,
            locked: advert.locked,
            in_match: advert.in_match,
            requires: advert.requires,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MasterBrowserSnapshot {
    pub generation: u64,
    pub loading: bool,
    pub adverts: Vec<MasterAdvert>,
    pub error: Option<String>,
    pub have: ContentFlags,
}

#[derive(Clone, Debug)]
pub enum MasterMenuAction {
    Refresh,
    Host {
        map: String,
        mode: String,
    },
    Join {
        advert_id: AdvertId,
        map: String,
        mode: String,
    },
    UpdateLobby {
        map: String,
        mode: String,
    },
    StartMatch {
        map: String,
        mode: String,
    },
    VoteToSkip,
    LeaveLobby,
}

/// The menu leaves one action here; the link takes it on the next frame.
#[derive(Resource, Default)]
pub struct PendingMasterMenuAction(pub Option<MasterMenuAction>);

/// A match the room just started, offered once to the session layer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MasterMatchOffer {
    pub map: String,
    pub mode: String,
    pub match_key: MatchKey,
}

#[derive(Resource, Default)]
pub struct MasterMatchStart(pub(super) Option<MasterMatchOffer>);

impl MasterMatchStart {
    pub fn take(&mut self) -> Option<MasterMatchOffer> {
        self.0.take()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seated_identity() -> SessionIdentity {
        SessionIdentity {
            attempt_id: 7,
            room_id: AdvertId([3; 16]),
            member_id: MemberId([4; 16]),
            epoch: 2,
        }
    }

    fn joined(in_match: bool) -> MasterBridgeState {
        MasterBridgeState::Joined {
            identity: seated_identity(),
            name: "room".into(),
            map: "mp_x".into(),
            mode: "tdm".into(),
            members: vec![MemberId([1; 16]), MemberId([4; 16])],
            member_names: HashMap::new(),
            max_players: 12,
            skip_votes: 1,
            in_match,
        }
    }

    #[test]
    fn match_key_needs_both_room_and_epoch() {
        assert!(SessionIdentity::unassigned(1).match_key().is_none());
        let mut identity = seated_identity();
        assert_eq!(identity.match_key(), MatchKey::new([3; 16], 2));
        identity.epoch = 0;
        assert!(identity.match_key().is_none());
        identity.epoch = 2;
        identity.room_id = AdvertId([0; 16]);
        assert!(identity.match_key().is_none());
    }

    #[test]
    fn seated_states_expose_the_room() {
        let state = joined(true);
        assert_eq!(state.map(), Some("mp_x"));
        assert_eq!(state.mode(), Some("tdm"));
        assert_eq!(state.members().len(), 2);
        assert_eq!(state.skip_votes(), 1);
        assert!(state.in_match());
        assert!(!state.is_terminal());
        assert_eq!(state.identity().attempt_id, 7);
        assert_eq!(state.status_word(), "joined");
    }

    #[test]
    fn unseated_states_have_no_room() {
        let identity = seated_identity();
        let waiting = MasterBridgeState::Joining { identity };
        assert_eq!(waiting.map(), None);
        assert!(waiting.members().is_empty());
        assert!(!waiting.in_match());
        assert_eq!(waiting.status_word(), "connecting");
        let left = MasterBridgeState::Left { identity };
        assert!(left.is_terminal());
        let failed = MasterBridgeState::Failed {
            identity,
            error: TransportFault::new("connect", "join", "refused"),
        };
        assert!(failed.is_terminal());
        assert_eq!(failed.identity(), identity);
        assert_eq!(failed.status_word(), "failed");
    }

    #[test]
    fn advert_marks_packs_this_pc_lacks() {
        let advert = master_protocol::Advert {
            id: AdvertId([9; 16]),
            name: "n".into(),
            map: "m".into(),
            mode: "dm".into(),
            players: 1,
            max_players: 18,
            locked: false,
            in_match: false,
            requires: ContentFlags(0b1100),
            generation: 3,
        };
        let row = MasterAdvert::seen_with(advert, ContentFlags(0b0100));
        assert_eq!(row.missing, ContentFlags(0b1000));
        assert_eq!(row.requires, ContentFlags(0b1100));
    }

    #[test]
    fn a_match_offer_is_taken_once() {
        let mut start = MasterMatchStart::default();
        start.0 = Some(MasterMatchOffer {
            map: "m".into(),
            mode: "dm".into(),
            match_key: MatchKey::new([1; 16], 1),
        });
        assert!(start.take().is_some());
        assert!(start.take().is_none());
    }
}
