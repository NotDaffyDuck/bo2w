//! Admission: an approved map load is checked for its zone files and, when
//! they are there, queued as a `MatchLoadRequest`.

use std::path::PathBuf;

use asset_transport::progress::LoadProgress;
use asset_transport::{GamesRoot, ZoneGame};
use bevy::prelude::*;
use frame::{LaunchIdentity, MapLoadApproved, MapLoadFailed};

use crate::match_load_state::{MatchLoadAbort, MatchLoadRequest};

/// Where the files of a requested zone were looked for, and what was found.
struct ZoneFiles {
    /// The zone's own name, as the file on disk spells it when it was found.
    name: String,
    zone_ff: Result<PathBuf, String>,
    common_mp: Result<PathBuf, String>,
    is_t6: bool,
}

impl ZoneFiles {
    fn find(games: &GamesRoot, requested: &str) -> Self {
        let found = asset_transport::find_zone_file(games, requested);
        let name = match &found {
            Ok(zone) => zone.zone_name.clone(),
            Err(_) => requested.to_owned(),
        };
        let zone_ff = found.map(|zone| zone.path);
        let common_mp = match &zone_ff {
            Ok(path) => asset_transport::find_runtime_common_mp(games, path).map(|zone| zone.path),
            Err(error) => Err(error.clone()),
        };
        let is_t6 = zone_ff
            .as_ref()
            .ok()
            .and_then(|path| asset_transport::zone_game_for_path(path))
            == Some(ZoneGame::T6);
        Self {
            name,
            zone_ff,
            common_mp,
            is_t6,
        }
    }

    /// Why the load cannot start, if it cannot. A T6 map has no common zone
    /// to wait for, so only its own file has to be there.
    fn problem(&self) -> Option<String> {
        let common = self.common_mp.as_ref().err().filter(|_| !self.is_t6);
        self.zone_ff.as_ref().err().or(common).cloned()
    }
}

pub(crate) fn approve_map_load(
    mut commands: Commands,
    mut approved: MessageReader<MapLoadApproved>,
    mut failed: MessageWriter<MapLoadFailed>,
    identity: Res<LaunchIdentity>,
    abort: Option<Res<MatchLoadAbort>>,
) {
    let games = GamesRoot(identity.games_root.clone());
    for approval in approved.read() {
        if MatchLoadAbort::covers(abort.as_deref(), approval.request_id) {
            diag::info!(
                World,
                "match load: request #{} (`{}`) was aborted after approval; no walk",
                approval.request_id,
                approval.zone
            );
            continue;
        }
        let files = ZoneFiles::find(&games, &approval.zone);
        if let Some(error) = files.problem() {
            failed.write(MapLoadFailed {
                request_id: approval.request_id,
                load_key: approval.load_key,
                zone: files.name,
                error,
            });
            continue;
        }
        // Progress belongs to the request. Walks of earlier requests may still
        // hold the live map's, and their writes must not land in this one.
        commands.insert_resource(MatchLoadRequest {
            request_id: approval.request_id,
            load_key: approval.load_key,
            zone: files.name,
            zone_ff: files.zone_ff,
            common_mp: files.common_mp,
            progress: LoadProgress::new(approval.request_id),
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn files(
        zone_ff: Result<&str, &str>,
        common_mp: Result<&str, &str>,
        is_t6: bool,
    ) -> ZoneFiles {
        let owned = |side: Result<&str, &str>| {
            side.map(PathBuf::from).map_err(str::to_owned)
        };
        ZoneFiles {
            name: "mp_zone".into(),
            zone_ff: owned(zone_ff),
            common_mp: owned(common_mp),
            is_t6,
        }
    }

    #[test]
    fn a_missing_zone_file_always_stops_the_load() {
        let missing = files(Err("no zone"), Err("no zone"), true);
        assert_eq!(missing.problem().as_deref(), Some("no zone"));
    }

    #[test]
    fn a_missing_common_zone_only_matters_off_t6() {
        let other = files(Ok("a.ff"), Err("no common"), false);
        assert_eq!(other.problem().as_deref(), Some("no common"));
        let t6 = files(Ok("a.ff"), Err("no common"), true);
        assert_eq!(t6.problem(), None);
    }

    #[test]
    fn a_complete_set_has_no_problem() {
        assert_eq!(files(Ok("a.ff"), Ok("b.ff"), false).problem(), None);
    }
}
