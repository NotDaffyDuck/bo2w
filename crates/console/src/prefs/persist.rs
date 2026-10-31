//! Where the settings file lives, reading it at startup and writing it when
//! the settings or the binds change.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::{fs, io};

use bevy::{ecs::change_detection::DetectChanges, ecs::resource::Resource, ecs::system::Res, ecs::system::ResMut, log::info, log::warn};
use frame::PlayerPrefsBundle;

use super::file;
use super::layout;
use crate::BindingTable;

/// The settings file, and the text it held when last read or written. A run
/// with no path keeps its settings for the session only.
#[derive(Resource, Default)]
pub(crate) struct PrefsStorage {
    path: Option<PathBuf>,
    on_disk: Option<String>,
}

/// The file's text; None for a first run (no file yet) and for a file that
/// could not be read, which is warned about.
fn read_existing(file_path: &Path) -> Option<String> {
    fs::read_to_string(file_path)
        .inspect_err(|why| {
            if why.kind() != io::ErrorKind::NotFound {
                warn!("could not read {}: {why}", file_path.display());
            }
        })
        .ok()
}

/// Replaces `target` with `text` whole, through a temporary beside it, so a
/// crash mid-write leaves the old file.
fn replace_whole(target: &Path, text: &str) -> io::Result<()> {
    if let Some(folder) = target.parent() {
        fs::create_dir_all(folder)?;
    }
    let staging = target.with_extension("cfg.tmp");
    fs::write(&staging, text)?;
    fs::rename(&staging, target)
}

impl PrefsStorage {
    /// Reads the file at `at` into `prefs` and `keymap`; a missing file
    /// is a first run. What they read as afterwards counts as already saved.
    fn open(&mut self, at: PathBuf, prefs: &mut PlayerPrefsBundle, binds: &mut BindingTable) {
        if let Some(saved) = read_existing(&at) {
            let report = file::read(&saved, prefs, binds);
            report.warnings.iter().for_each(|line| warn!("{line}"));
            if let Some(old_layout) = report.moved_from {
                let now = layout::CURRENT;
                info!("saved binds moved to the current keys (layout {old_layout} -> {now})");
            }
        }
        prefs.pull_into_limits();
        self.on_disk = Some(file::render(prefs, binds));
        self.path = Some(at);
    }

    /// Writes `text` unless the file already holds it. The file is replaced
    /// whole, so a crash mid-write leaves the old one.
    fn store(&mut self, text: String) -> Result<(), String> {
        let unchanged = self.on_disk.as_ref() == Some(&text);
        match self.path.as_deref() {
            Some(target) if !unchanged => {
                replace_whole(target, &text).map_err(|why| format!("{}: {why}", target.display()))?;
                self.on_disk = Some(text);
                Ok(())
            }
            _ => Ok(()),
        }
    }
}

/// `BO2MP_SETTINGS_PATH` wins. On Windows the file sits with the other
/// artifacts; elsewhere it is under the config home.
fn settings_path_in(
    override_path: Option<OsString>,
    artifact_dir: &Path,
    windows: bool,
    xdg_config_home: Option<OsString>,
    home: Option<OsString>,
) -> Option<PathBuf> {
    if override_path.is_some() {
        return override_path.map(Into::into);
    }
    if windows {
        return Some(artifact_dir.join("settings.cfg"));
    }
    let config: PathBuf = match (xdg_config_home, home) {
        (Some(xdg), _) => xdg.into(),
        (None, Some(home_dir)) => Path::new(&home_dir).join(".config"),
        (None, None) => return None,
    };
    Some(config.join("bo2mp/settings.cfg"))
}

fn prefs_file_location(artifacts: &Path) -> Option<PathBuf> {
    let env = |name: &str| std::env::var_os(name);
    settings_path_in(env("BO2MP_SETTINGS_PATH"), artifacts, cfg!(windows), env("XDG_CONFIG_HOME"), env("HOME"))
}

pub(crate) fn read_player_prefs(
    identity: Res<ui::StartupPersona>,
    mut prefs: ResMut<PlayerPrefsBundle>,
    mut table: ResMut<BindingTable>,
    mut persistence: ResMut<PrefsStorage>,
) {
    match prefs_file_location(&identity.artifacts) {
        None => warn!("no HOME or XDG_CONFIG_HOME; settings last only for this session"),
        Some(file_path) => {
            persistence.open(file_path, &mut prefs, &mut table);
            asset_core::t6_set_fast(prefs.bo2_fast);
        }
    }
}

pub(crate) fn write_player_prefs(
    settings: Res<PlayerPrefsBundle>,
    binds: Res<BindingTable>,
    mut persistence: ResMut<PrefsStorage>,
) {
    let anything_new = settings.is_changed() || binds.is_changed();
    if !anything_new {
        return;
    }
    let text = file::render(&settings, &binds);
    persistence
        .store(text)
        .unwrap_or_else(|why| warn!("could not save the user settings: {why}"));
}
#[cfg(test)]
mod tests {
    use bevy::prelude::*;

    use crate::prefs::persist::*;
    use crate::ActionBinding;

    /// A folder that does not exist yet, named for this test.
    fn scratch(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("bo2mp-settings-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        dir
    }

    fn os(text: &str) -> Option<OsString> {
        Some(text.into())
    }

    fn fresh() -> (PlayerPrefsBundle, BindingTable) {
        let mut binds = BindingTable::default();
        binds.restore_stock_controls();
        (PlayerPrefsBundle::default(), binds)
    }

    fn opened(path: &Path) -> (PrefsStorage, PlayerPrefsBundle, BindingTable) {
        let (mut prefs, mut table) = fresh();
        let mut persistence = PrefsStorage::default();
        persistence.open(path.to_path_buf(), &mut prefs, &mut table);
        (persistence, prefs, table)
    }

    #[test]
    fn the_environment_override_wins() {
        let artifacts = Path::new("art");
        let path = settings_path_in(os("/x/mine.cfg"), artifacts, true, os("/cfg"), os("/home"));
        assert_eq!(path, Some(PathBuf::from("/x/mine.cfg")));
    }

    #[test]
    fn windows_keeps_it_with_the_artifacts() {
        let path = settings_path_in(None, Path::new("art"), true, os("/cfg"), os("/home"));
        assert_eq!(path, Some(Path::new("art").join("settings.cfg")));
    }

    #[test]
    fn elsewhere_it_is_under_the_config_home_or_failing_that_the_home() {
        let artifacts = Path::new("art");
        let xdg = settings_path_in(None, artifacts, false, os("/cfg"), os("/home"));
        assert_eq!(xdg, Some(Path::new("/cfg").join("bo2mp/settings.cfg")));
        let home = settings_path_in(None, artifacts, false, None, os("/home"));
        let expected = Some(
            Path::new("/home")
                .join(".config")
                .join("bo2mp/settings.cfg")
        );
        assert_eq!(home, expected);
        assert_eq!(settings_path_in(None, artifacts, false, None, None), None);
    }

    #[test]
    fn saving_makes_the_folder_and_the_file_and_leaves_no_temporary() {
        let dir = scratch("save");
        let path = dir.join("deeper").join("settings.cfg");
        let mut persistence = PrefsStorage {
            path: Some(path.clone()),
            on_disk: None,
        };
        persistence.store("one\n".into()).expect("saved");
        assert_eq!(fs::read_to_string(&path).unwrap(), "one\n");
        assert!(!path.with_extension("cfg.tmp").exists());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_same_text_is_not_written_twice() {
        let dir = scratch("twice");
        let path = dir.join("settings.cfg");
        let mut persistence = PrefsStorage {
            path: Some(path.clone()),
            on_disk: None,
        };
        persistence.store("one\n".into()).unwrap();
        fs::remove_file(&path).unwrap();
        persistence.store("one\n".into()).unwrap();
        assert!(!path.exists(), "unchanged text is left alone");
        persistence.store("two\n".into()).unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "two\n");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_session_with_no_path_saves_nothing_and_does_not_complain() {
        let mut persistence = PrefsStorage::default();
        assert_eq!(persistence.store("one\n".into()), Ok(()));
        assert!(persistence.on_disk.is_none());
    }

    #[test]
    fn a_path_that_cannot_be_written_says_which() {
        let dir = scratch("blocked");
        fs::create_dir_all(&dir).unwrap();
        // The "file" is a folder, so it cannot be replaced.
        let path = dir.join("settings.cfg");
        fs::create_dir_all(&path).unwrap();
        let mut persistence = PrefsStorage {
            path: Some(path.clone()),
            on_disk: None,
        };
        let error = persistence.store("one\n".into()).unwrap_err();
        assert!(error.starts_with(&path.display().to_string()), "{error}");
        assert!(persistence.on_disk.is_none(), "a failed save is retried");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_missing_file_is_a_first_run_that_counts_as_saved() {
        let dir = scratch("first");
        let path = dir.join("settings.cfg");
        let (mut persistence, settings, binds) = opened(&path);
        assert_eq!(settings, fresh().0);
        persistence.store(file::render(&settings, &binds)).unwrap();
        assert!(!path.exists(), "nothing changed, so nothing is written");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_file_is_applied_and_not_written_straight_back() {
        let dir = scratch("apply");
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("settings.cfg");
        fs::write(
            &path,
            "fov=100\nplayer_name=Sam\nbinds_layout=5\nunbindall\nbind J +activate\n",
        )
        .unwrap();
        let (mut persistence, settings, binds) = opened(&path);
        assert_eq!(settings.view_angle, 100.0);
        assert_eq!(settings.shown_name, "Sam");
        let expected = input_t6::action_number_for("+activate");
        assert_eq!(binds.bound_to(ActionBinding::Board(KeyCode::KeyJ)), expected);
        fs::remove_file(&path).unwrap();
        persistence.store(file::render(&settings, &binds)).unwrap();
        assert!(!path.exists());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_file_is_cleaned_up_as_it_is_read() {
        let dir = scratch("pull_into_limits");
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("settings.cfg");
        fs::write(&path, "fov=9000\nplayer_name=A name far too long to keep\n").unwrap();
        let (_, settings, _) = opened(&path);
        assert_eq!(settings.view_angle, PlayerPrefsBundle::FOV_MAX);
        assert_eq!(settings.shown_name.chars().count(), 16);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_changed_setting_is_saved_and_read_back() {
        let dir = scratch("roundtrip");
        let path = dir.join("settings.cfg");
        let (mut persistence, mut settings, binds) = opened(&path);
        settings.view_angle = 90.0;
        persistence.store(file::render(&settings, &binds)).unwrap();
        let (_, again, _) = opened(&path);
        assert_eq!(again.view_angle, 90.0);
        let _ = fs::remove_dir_all(&dir);
    }
}
