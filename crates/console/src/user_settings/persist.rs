//! Where the settings file lives, reading it at startup and writing it when
//! the settings or the binds change.

use std::{
    ffi::OsString,
    fs, io,
    path::{Path, PathBuf},
};

use bevy::prelude::*;
use frame::GameSettings;

use super::{file, layout};
use crate::KeyBinds;

/// The settings file, and the text it held when last read or written. A run
/// with no path keeps its settings for the session only.
#[derive(Resource, Default)]
pub(crate) struct UserSettingsPersistence {
    path: Option<PathBuf>,
    on_disk: Option<String>,
}

impl UserSettingsPersistence {
    /// Reads the file at `path` into `settings` and `binds`; a missing file
    /// is a first run. What they read as afterwards counts as already saved.
    fn open(&mut self, path: PathBuf, settings: &mut GameSettings, binds: &mut KeyBinds) {
        match fs::read_to_string(&path) {
            Ok(source) => {
                let notes = file::read(&source, settings, binds);
                for warning in &notes.warnings {
                    warn!("{warning}");
                }
                if let Some(from) = notes.moved_from {
                    info!(
                        "saved binds moved to the current keys (layout {from} -> {})",
                        layout::CURRENT
                    );
                }
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => warn!("could not read {}: {error}", path.display()),
        }
        settings.sanitize();
        self.on_disk = Some(file::render(settings, binds));
        self.path = Some(path);
    }

    /// Writes `text` unless the file already holds it. The file is replaced
    /// whole, so a crash mid-write leaves the old one.
    fn store(&mut self, text: String) -> Result<(), String> {
        let Some(path) = &self.path else {
            return Ok(());
        };
        if self.on_disk.as_deref() == Some(text.as_str()) {
            return Ok(());
        }
        let failed = |error: io::Error| format!("{}: {error}", path.display());
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(failed)?;
        }
        let temporary = path.with_extension("cfg.tmp");
        fs::write(&temporary, &text).map_err(failed)?;
        fs::rename(&temporary, path).map_err(failed)?;
        self.on_disk = Some(text);
        Ok(())
    }
}

/// `BO2MP_SETTINGS_PATH` wins. On Windows the file sits with the other
/// artifacts; elsewhere it is under the config home.
fn settings_path_in(
    override_path: Option<OsString>,
    artifacts: &Path,
    windows: bool,
    xdg_config_home: Option<OsString>,
    home: Option<OsString>,
) -> Option<PathBuf> {
    if let Some(path) = override_path {
        return Some(PathBuf::from(path));
    }
    if windows {
        return Some(artifacts.join("settings.cfg"));
    }
    xdg_config_home
        .map(PathBuf::from)
        .or_else(|| home.map(|home| PathBuf::from(home).join(".config")))
        .map(|config| config.join("bo2mp/settings.cfg"))
}

fn settings_path(artifacts: &Path) -> Option<PathBuf> {
    settings_path_in(
        std::env::var_os("BO2MP_SETTINGS_PATH"),
        artifacts,
        cfg!(windows),
        std::env::var_os("XDG_CONFIG_HOME"),
        std::env::var_os("HOME"),
    )
}

pub(crate) fn load_user_settings(
    identity: Res<ui::LaunchIdentity>,
    mut settings: ResMut<GameSettings>,
    mut binds: ResMut<KeyBinds>,
    mut persistence: ResMut<UserSettingsPersistence>,
) {
    let Some(path) = settings_path(&identity.artifacts) else {
        warn!("no HOME or XDG_CONFIG_HOME; user settings are session-only");
        return;
    };
    persistence.open(path, &mut settings, &mut binds);
    asset_core::t6_set_fast(settings.bo2_fast);
}

pub(crate) fn save_user_settings(
    settings: Res<GameSettings>,
    binds: Res<KeyBinds>,
    mut persistence: ResMut<UserSettingsPersistence>,
) {
    if !settings.is_changed() && !binds.is_changed() {
        return;
    }
    if let Err(error) = persistence.store(file::render(&settings, &binds)) {
        warn!("could not save the user settings: {error}");
    }
}

#[cfg(test)]
mod tests {
    use bevy::prelude::KeyCode;

    use super::*;
    use crate::BindButton;

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

    fn fresh() -> (GameSettings, KeyBinds) {
        let mut binds = KeyBinds::default();
        binds.apply_defaults();
        (GameSettings::default(), binds)
    }

    fn opened(path: &Path) -> (UserSettingsPersistence, GameSettings, KeyBinds) {
        let (mut settings, mut binds) = fresh();
        let mut persistence = UserSettingsPersistence::default();
        persistence.open(path.to_path_buf(), &mut settings, &mut binds);
        (persistence, settings, binds)
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
        assert_eq!(
            home,
            Some(
                Path::new("/home")
                    .join(".config")
                    .join("bo2mp/settings.cfg")
            )
        );
        assert_eq!(settings_path_in(None, artifacts, false, None, None), None);
    }

    #[test]
    fn saving_makes_the_folder_and_the_file_and_leaves_no_temporary() {
        let dir = scratch("save");
        let path = dir.join("deeper").join("settings.cfg");
        let mut persistence = UserSettingsPersistence {
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
        let mut persistence = UserSettingsPersistence {
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
        let mut persistence = UserSettingsPersistence::default();
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
        let mut persistence = UserSettingsPersistence {
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
        assert_eq!(settings.fov, 100.0);
        assert_eq!(settings.player_name, "Sam");
        assert_eq!(
            binds.get(BindButton::Key(KeyCode::KeyJ)),
            input_t6::command_id_lookup("+activate")
        );
        fs::remove_file(&path).unwrap();
        persistence.store(file::render(&settings, &binds)).unwrap();
        assert!(!path.exists());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_file_is_cleaned_up_as_it_is_read() {
        let dir = scratch("sanitize");
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("settings.cfg");
        fs::write(&path, "fov=9000\nplayer_name=A name far too long to keep\n").unwrap();
        let (_, settings, _) = opened(&path);
        assert_eq!(settings.fov, GameSettings::FOV_MAX);
        assert_eq!(settings.player_name.chars().count(), 16);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_changed_setting_is_saved_and_read_back() {
        let dir = scratch("roundtrip");
        let path = dir.join("settings.cfg");
        let (mut persistence, mut settings, binds) = opened(&path);
        settings.fov = 90.0;
        persistence.store(file::render(&settings, &binds)).unwrap();
        let (_, again, _) = opened(&path);
        assert_eq!(again.fov, 90.0);
        let _ = fs::remove_dir_all(&dir);
    }
}
