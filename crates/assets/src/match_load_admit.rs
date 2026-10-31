//! Admission: an approved map load is checked for its zone files and, when
//! they are there, queued as a `SessionLoadOrder`.

use std::path::PathBuf;

use asset_transport::progress::LoadCompletion;
use asset_transport::{GameInstallDir, ZoneTitleKind};
use bevy::prelude::*;
use frame::{StartupPersona, LevelOpenGranted, LevelOpenRejected};

use crate::match_load_state::{SessionLoadCancel, SessionLoadOrder};

/// Where the files of a requested zone were looked for, and what was found.
struct ZoneFiles {
    /// The zone's own name, as the file on disk spells it when it was found.
    name: String,
    zone_archive_file: Result<PathBuf, String>,
    common_zone: Result<PathBuf, String>,
    is_t6: bool,
}

impl ZoneFiles {
    fn find(games: &GameInstallDir, asked_map: &str) -> Self {
        let (name, zone_archive_file) = match asset_transport::locate_zone_file(games, asked_map) {
            Ok(located) => (located.zone_label, Ok(located.path)),
            Err(why) => (asked_map.to_owned(), Err(why)),
        };
        let common_zone = zone_archive_file.clone().and_then(|map_zone| {
            asset_transport::locate_shared_mp_zone(games, &map_zone).map(|located| located.path)
        });
        let game = zone_archive_file.as_deref().ok().and_then(asset_transport::title_of_zone_path);
        let is_t6 = matches!(game, Some(ZoneTitleKind::T6));
        Self { name, zone_archive_file, common_zone, is_t6 }
    }

    /// Why the load cannot start, if it cannot. A T6 map has no common zone
    /// to wait for, so only its own file has to be there.
    fn problem(&self) -> Option<String> {
        let common = self.common_zone.as_ref().err().filter(|_| !self.is_t6);
        self.zone_archive_file.as_ref().err().or(common).cloned()
    }

    /// The failure reported back for `approval` when the files are not there.
    fn into_failure(self, approval: &LevelOpenGranted, error: String) -> LevelOpenRejected {
        let LevelOpenGranted { ask_number: load_req_id, decrypt_secret, .. } = *approval;
        LevelOpenRejected { error, zone: self.name, decrypt_secret, ask_number: load_req_id }
    }

    /// The queued request for `approval` once its files are all there.
    fn into_request(self, approval: &LevelOpenGranted) -> SessionLoadOrder {
        let LevelOpenGranted { ask_number: ticket, decrypt_secret, .. } = *approval;
        // Progress belongs to the request. Walks of earlier requests may still
        // hold the live map's, and their writes must not land in this one.
        let progress = LoadCompletion::new(ticket);
        let Self { name: zone, zone_archive_file, common_zone: common_mp, .. } = self;
        SessionLoadOrder { progress, common_zone: common_mp, zone_archive_file, zone, decrypt_secret, ticket }
    }
}

pub(crate) fn approve_map_load(
    mut commands: Commands,
    mut approvals: MessageReader<LevelOpenGranted>,
    mut failures: MessageWriter<LevelOpenRejected>,
    identity: Res<StartupPersona>,
    abort: Option<Res<SessionLoadCancel>>,
) {
    let root = GameInstallDir(identity.titles_install_dir.clone());
    let still_wanted = |approval: &&LevelOpenGranted| {
        let aborted = SessionLoadCancel::covers(abort.as_deref(), approval.ask_number);
        if aborted {
            let (id, zone) = (approval.ask_number, &approval.zone);
            diag::info!(World, "match load: request #{id} (`{zone}`) was aborted after approval; no walk");
        }
        !aborted
    };
    for approval in approvals.read().filter(still_wanted) {
        let files = ZoneFiles::find(&root, &approval.zone);
        match files.problem() {
            Some(error) => {
                failures.write(files.into_failure(approval, error));
            }
            None => commands.insert_resource(files.into_request(approval)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn files(zone_archive_file: Result<&str, &str>, common_mp: Result<&str, &str>, is_t6: bool) -> ZoneFiles {
        let owned = |side: Result<&str, &str>| side.map(PathBuf::from).map_err(String::from);
        ZoneFiles { name: "mp_zone".into(), zone_archive_file: owned(zone_archive_file), common_zone: owned(common_mp), is_t6 }
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

    #[test]
    fn a_request_carries_the_approval_and_the_files() {
        let approval = LevelOpenGranted { ask_number: 7, decrypt_secret: Default::default(), zone: "asked".into() };
        let request = files(Ok("a.ff"), Ok("b.ff"), true).into_request(&approval);
        assert_eq!((request.ticket, request.zone.as_str()), (7, "mp_zone"));
        assert_eq!(request.zone_archive_file, Ok(PathBuf::from("a.ff")));
        assert_eq!(request.common_zone, Ok(PathBuf::from("b.ff")));
    }
}
