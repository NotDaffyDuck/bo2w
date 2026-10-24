//! What the match loader shares with the rest of the game: the system sets
//! others order themselves around, and the resources that carry a load from
//! request to result.

use std::path::PathBuf;

use asset_transport::progress::LoadProgress;
use bevy::prelude::*;

use crate::session_load::PreparedMatch;

/// Turns an approved map load into a `MatchLoadRequest`.
#[derive(SystemSet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct MapLoadApproval;

/// Starts, polls and settles the walk of a request.
#[derive(SystemSet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct MatchLoadDispatch;

/// Set while a walk is running or about to.
#[derive(Resource, Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct MatchLoadBusy(pub bool);

/// The request id whose load the player has given up on.
#[derive(Resource, Debug, Clone, Copy, PartialEq, Eq)]
pub struct MatchLoadAbort(pub u64);

impl MatchLoadAbort {
    /// Whether `request_id` is the load that was given up on.
    pub fn covers(abort: Option<&Self>, request_id: u64) -> bool {
        abort.is_some_and(|abort| abort.0 == request_id)
    }
}

/// A load waiting for its turn: where to find the zone, and the progress its
/// walk will report into.
#[derive(Resource)]
pub struct MatchLoadRequest {
    pub request_id: u64,
    pub load_key: frame::LocalLoadKey,
    pub zone: String,
    pub zone_ff: Result<PathBuf, String>,
    pub common_mp: Result<PathBuf, String>,
    pub progress: LoadProgress,
}

/// A request whose walk has been started.
#[derive(Resource, Clone, Debug)]
pub struct MatchLoadAccepted {
    pub request_id: u64,
    pub load_key: frame::LocalLoadKey,
    pub zone: String,
}

/// A finished walk, waiting to be applied to the session.
#[derive(Resource)]
pub struct PreparedMatchReady {
    pub request_id: u64,
    pub load_key: frame::LocalLoadKey,
    pub zone: String,
    pub prepared: PreparedMatch,
}

/// The sound half of a finished walk, handed on for the audio side.
#[derive(Resource)]
pub struct PreparedMatchSound {
    pub load_key: frame::LocalLoadKey,
    pub zone: String,
    pub common_profile_id: u64,
    pub products_id: u64,
    pub sound: Result<asset_audio::SoundCatalog, String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_abandoned_request_is_covered() {
        let abort = MatchLoadAbort(7);
        assert!(MatchLoadAbort::covers(Some(&abort), 7));
        assert!(!MatchLoadAbort::covers(Some(&abort), 8));
        assert!(!MatchLoadAbort::covers(None, 7));
    }
}
