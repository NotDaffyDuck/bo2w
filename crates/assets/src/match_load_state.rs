//! What the match loader shares with the rest of the game: the system sets
//! others order themselves around, and the resources that carry a load from
//! request to result.

use std::path::PathBuf;

use asset_transport::progress::LoadCompletion;
use bevy::prelude::*;

use crate::load_stages::ReadiedSession;

/// Turns an approved map load into a `SessionLoadOrder`.
#[derive(SystemSet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct MapLoadApproval;

/// Starts, polls and settles the walk of a request.
#[derive(SystemSet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct MatchLoadDispatch;

/// Set while a walk is running or about to.
#[derive(Resource, Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SessionLoadOccupied(pub bool);

/// The request id whose load the player has given up on.
#[derive(Resource, Debug, Clone, Copy, PartialEq, Eq)]
pub struct SessionLoadCancel(pub u64);

impl SessionLoadCancel {
    /// Whether `request_id` is the load that was given up on.
    pub fn covers(abort: Option<&Self>, load_req_id: u64) -> bool {
        abort.is_some_and(|abort| abort.0 == load_req_id)
    }
}

/// A load waiting for its turn: where to find the zone, and the progress its
/// walk will report into.
#[derive(Resource)]
pub struct SessionLoadOrder {
    pub ticket: u64,
    pub decrypt_secret: frame::OwnLevelLoadTicket,
    pub zone: String,
    pub zone_archive_file: Result<PathBuf, String>,
    pub common_zone: Result<PathBuf, String>,
    pub progress: LoadCompletion,
}

/// A request whose walk has been started.
#[derive(Resource, Clone, Debug)]
pub struct SessionLoadApproved {
    pub ticket: u64,
    pub decrypt_secret: frame::OwnLevelLoadTicket,
    pub zone: String,
}

/// A finished walk, waiting to be applied to the session.
#[derive(Resource)]
pub struct ReadiedSessionSignal {
    pub ticket: u64,
    pub decrypt_secret: frame::OwnLevelLoadTicket,
    pub zone: String,
    pub prepared: ReadiedSession,
}

/// The sound half of a finished walk, handed on for the audio side.
#[derive(Resource)]
pub struct PreparedMatchSound {
    pub decrypt_secret: frame::OwnLevelLoadTicket,
    pub zone: String,
    pub common_profile_id: u64,
    pub products_id: u64,
    pub sound: Result<asset_audio::AudioRoster, String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_abandoned_request_is_covered() {
        let abort = SessionLoadCancel(7);
        assert!(SessionLoadCancel::covers(Some(&abort), 7));
        assert!(!SessionLoadCancel::covers(Some(&abort), 8));
        assert!(!SessionLoadCancel::covers(None, 7));
    }
}
