//! The `.env` that sits next to the game: settings that stay with the install.
//! It is written once, on the first update, and never overwritten after that.

use std::collections::BTreeMap;
use std::io::{self, Write as _};
use std::path::{Path, PathBuf};
use io::ErrorKind;
use std::fs::OpenOptions;

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

/// What sits at the `.env` path.
enum Found {
    Nothing,
    File,
    /// Something that is not a file, which is an error to report.
    Other(String),
}

fn look_at(path: &Path) -> Found {
    match (path.is_file(), path.exists()) {
        (true, _) => Found::File,
        (false, false) => Found::Nothing,
        (false, true) => Found::Other(format!("{shown} exists but is not a file", shown = path.display())),
    }
}

/// Puts the `.env` in `dir`, if there is one, into the environment. Variables
/// that are already set keep their value.
pub fn load(dir: &Path) -> Result<(), String> {
    let env_file = path_in(dir);
    match look_at(&env_file) {
        Found::Nothing => Ok(()),
        Found::Other(why) => Err(why),
        Found::File => dotenvy::from_path(&env_file).map_err(|why| format!("load {}: {why}", env_file.display())),
    }
}

/// Writes the first `.env` from the release's defaults, unless the folder has
/// one. `update_url` is the channel the update came from, kept so the next
/// `update` finds it.
pub fn write_first(
    dir: &Path,
    shipping_variables: &BTreeMap<String, String>,
    update_url: Option<&str>,
) -> Result<(), String> {
    let env_file = path_in(dir);
    match look_at(&env_file) {
        Found::File => return Ok(()),
        Found::Other(why) => return Err(why),
        Found::Nothing => {}
    }
    let text = first_contents(shipping_variables, update_url)?;
    let shown = env_file.display();
    let created = OpenOptions::new().write(true).create_new(true).open(&env_file);
    let mut out = match created {
        Ok(out) => out,
        // Someone else made it a moment ago; theirs stays.
        Err(why) if why.kind() == ErrorKind::AlreadyExists => return Ok(()),
        Err(why) => return Err(format!("create {shown}: {why}")),
    };
    out.write_all(text.as_bytes()).map_err(|why| format!("write {shown}: {why}"))
}

/// A channel address fit to keep: trimmed, not empty, one line, no NUL.
fn keepable_url(update_url: Option<&str>) -> Option<&str> {
    let url = update_url?.trim();
    let one_clean_line = !url.is_empty() && !url.contains(['\r', '\n', '\0']);
    one_clean_line.then_some(url)
}

fn first_contents(
    shipping_variables: &BTreeMap<String, String>,
    update_url: Option<&str>,
) -> Result<String, String> {
    let defaults: Vec<String> = MASTER_KEYS
        .iter()
        .map(|key| match shipping_variables.get(*key) {
            Some(value) => Ok(format!("{key}={value}\n")),
            None => Err(format!("manifest has no {key}")),
        })
        .collect::<Result<_, _>>()?;
    let channel = keepable_url(update_url).map(|url| format!("BO2MP_UPDATE_URL={url}\n"));
    Ok(std::iter::once(HEADER.to_owned()).chain(defaults).chain(channel).collect())
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::testdir::TestDir;
    use std::fs;

    fn shipping_variables() -> BTreeMap<String, String> {
        MASTER_KEYS
            .iter()
            .map(|key| ((*key).to_owned(), format!("<{key}>")))
            .collect()
    }

    #[test]
    fn the_first_env_carries_the_master_settings() {
        let text = first_contents(&shipping_variables(), None).unwrap();
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
            let mut env = shipping_variables();
            env.remove(missing);
            assert_eq!(
                first_contents(&env, None),
                Err(format!("manifest has no {missing}"))
            );
        }
    }

    #[test]
    fn the_channel_is_remembered_when_it_is_sane() {
        let text = first_contents(&shipping_variables(), Some("  https://e.net/prod  ")).unwrap();
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
            let text = first_contents(&shipping_variables(), skipped).unwrap();
            assert!(!text.contains("BO2MP_UPDATE_URL"), "{skipped:?}");
            assert!(!text.contains("INJECT"), "{skipped:?}");
        }
    }

    #[test]
    fn the_first_env_is_written_once() {
        let dir = TestDir::new("first");
        write_first(dir.path(), &shipping_variables(), None).unwrap();
        let written = fs::read_to_string(dir.join(".env")).unwrap();
        assert!(written.contains("BO2MP_MASTER_ADDR="));

        fs::write(dir.join(".env"), "BO2MP_MASTER_ADDR=mine\n").unwrap();
        write_first(dir.path(), &shipping_variables(), None).unwrap();
        assert_eq!(
            fs::read_to_string(dir.join(".env")).unwrap(),
            "BO2MP_MASTER_ADDR=mine\n"
        );
        assert_eq!(dir.entry_names(), [".env"]);
    }

    #[test]
    fn a_folder_called_env_is_in_the_way() {
        let dir = TestDir::new("folder");
        fs::create_dir(dir.join(".env")).unwrap();
        let written = write_first(dir.path(), &shipping_variables(), None).unwrap_err();
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
