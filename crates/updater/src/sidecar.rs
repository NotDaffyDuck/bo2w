//! The `.env` that sits next to the game: settings that stay with the install.
//! It is written once, on the first update, and never overwritten after that.

use std::collections::BTreeMap;
use std::fs::OpenOptions;
use std::io::{ErrorKind, Write};
use std::path::{Path, PathBuf};

const FILE_NAME: &str = ".env";

/// The release defaults the first `.env` is made from.
const MASTER_KEYS: [&str; 3] = [
    "BO2MP_MASTER_ADDR",
    "BO2MP_MASTER_SERVER_NAME",
    "BO2MP_MASTER_CA_CERT",
];

const HEADER: &str = "# Settings for this bo2mp install. A value here beats the release default; \
                      a variable set in the shell beats both.\n";

fn path_in(dir: &Path) -> PathBuf {
    dir.join(FILE_NAME)
}

/// Puts the `.env` in `dir`, if there is one, into the environment. Variables
/// that are already set keep their value.
pub fn load(dir: &Path) -> Result<(), String> {
    let path = path_in(dir);
    if !path.exists() {
        return Ok(());
    }
    if !path.is_file() {
        return Err(format!("{} exists but is not a file", path.display()));
    }
    dotenvy::from_path(&path).map_err(|error| format!("load {}: {error}", path.display()))
}

/// Writes the first `.env` from the release's defaults, unless the folder has
/// one. `update_url` is the channel the update came from, kept so the next
/// `update` finds it.
pub fn write_first(
    dir: &Path,
    release_env: &BTreeMap<String, String>,
    update_url: Option<&str>,
) -> Result<(), String> {
    let path = path_in(dir);
    if path.is_file() {
        return Ok(());
    }
    if path.exists() {
        return Err(format!("{} exists but is not a file", path.display()));
    }
    let text = first_contents(release_env, update_url)?;
    match OpenOptions::new().write(true).create_new(true).open(&path) {
        Ok(mut file) => file
            .write_all(text.as_bytes())
            .map_err(|error| format!("write {}: {error}", path.display())),
        // Someone else made it a moment ago; theirs stays.
        Err(error) if error.kind() == ErrorKind::AlreadyExists => Ok(()),
        Err(error) => Err(format!("create {}: {error}", path.display())),
    }
}

fn first_contents(
    release_env: &BTreeMap<String, String>,
    update_url: Option<&str>,
) -> Result<String, String> {
    let mut text = HEADER.to_owned();
    for key in MASTER_KEYS {
        let value = release_env
            .get(key)
            .ok_or_else(|| format!("manifest has no {key}"))?;
        text.push_str(&format!("{key}={value}\n"));
    }
    if let Some(url) = update_url.map(str::trim)
        && !url.is_empty()
        && !url.contains(['\r', '\n', '\0'])
    {
        text.push_str(&format!("BO2MP_UPDATE_URL={url}\n"));
    }
    Ok(text)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testdir::TestDir;
    use std::fs;

    fn release_env() -> BTreeMap<String, String> {
        MASTER_KEYS
            .iter()
            .map(|key| ((*key).to_owned(), format!("<{key}>")))
            .collect()
    }

    #[test]
    fn the_first_env_carries_the_master_settings() {
        let text = first_contents(&release_env(), None).unwrap();
        let lines: Vec<&str> = text.lines().collect();
        assert!(lines[0].starts_with("# "), "{text}");
        assert_eq!(
            &lines[1..],
            [
                "BO2MP_MASTER_ADDR=<BO2MP_MASTER_ADDR>",
                "BO2MP_MASTER_SERVER_NAME=<BO2MP_MASTER_SERVER_NAME>",
                "BO2MP_MASTER_CA_CERT=<BO2MP_MASTER_CA_CERT>",
            ]
        );
    }

    #[test]
    fn a_release_without_master_settings_cannot_make_one() {
        for missing in MASTER_KEYS {
            let mut env = release_env();
            env.remove(missing);
            assert_eq!(
                first_contents(&env, None),
                Err(format!("manifest has no {missing}"))
            );
        }
    }

    #[test]
    fn the_channel_is_remembered_when_it_is_sane() {
        let text = first_contents(&release_env(), Some("  https://e.net/prod  ")).unwrap();
        assert_eq!(
            text.lines().last(),
            Some("BO2MP_UPDATE_URL=https://e.net/prod")
        );
        for skipped in [
            None,
            Some(""),
            Some("   "),
            Some("https://e.net/\nINJECT=1"),
        ] {
            let text = first_contents(&release_env(), skipped).unwrap();
            assert!(!text.contains("BO2MP_UPDATE_URL"), "{skipped:?}");
            assert!(!text.contains("INJECT"), "{skipped:?}");
        }
    }

    #[test]
    fn the_first_env_is_written_once() {
        let dir = TestDir::new("first");
        write_first(dir.path(), &release_env(), None).unwrap();
        let written = fs::read_to_string(dir.join(".env")).unwrap();
        assert!(written.contains("BO2MP_MASTER_ADDR="));

        fs::write(dir.join(".env"), "BO2MP_MASTER_ADDR=mine\n").unwrap();
        write_first(dir.path(), &release_env(), None).unwrap();
        assert_eq!(
            fs::read_to_string(dir.join(".env")).unwrap(),
            "BO2MP_MASTER_ADDR=mine\n"
        );
        assert_eq!(dir.names(), [".env"]);
    }

    #[test]
    fn a_folder_called_env_is_in_the_way() {
        let dir = TestDir::new("folder");
        fs::create_dir(dir.join(".env")).unwrap();
        let written = write_first(dir.path(), &release_env(), None).unwrap_err();
        assert!(written.ends_with("exists but is not a file"), "{written}");
        let loaded = load(dir.path()).unwrap_err();
        assert!(loaded.ends_with("exists but is not a file"), "{loaded}");
    }

    #[test]
    fn no_env_file_is_fine_to_load() {
        let dir = TestDir::new("none");
        assert_eq!(load(dir.path()), Ok(()));
    }

    #[test]
    fn loading_puts_the_file_into_the_environment() {
        let dir = TestDir::new("load");
        let name = format!("BO2MP_UPDATER_TEST_LOADED_{}", std::process::id());
        fs::write(dir.join(".env"), format!("{name}=from-file\n")).unwrap();
        assert!(std::env::var_os(&name).is_none());
        load(dir.path()).unwrap();
        assert_eq!(std::env::var(&name).as_deref(), Ok("from-file"));
    }
}
