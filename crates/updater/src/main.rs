//! `bo2mp-launcher`: starts the game that sits next to it.
//!
//! A release archive already holds `bo2mp.exe`, so by default nothing touches
//! the network and nothing replaces the game behind the player's back. The
//! update is asked for: `bo2mp-launcher update`, or `BO2MP_UPDATE=1` for a
//! shortcut that cannot pass an argument. It then fetches the manifest of the
//! channel `BO2MP_UPDATE_URL` names, installs what the manifest lists (every
//! file checked against its hash first) and starts the game with the
//! manifest's environment defaults.

#![forbid(unsafe_code)]

mod channel;
mod http;
mod install;
mod manifest;
mod sidecar;
#[cfg(test)]
mod testdir;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use install::Outcome;
use manifest::{GAME_FILE, Manifest};
use std::process::Command;
use std::process::ExitCode;

const TAG: &str = "[bo2mp-launcher]";
const UPDATE_URL_ENV: &str = "BO2MP_UPDATE_URL";
const UPDATE_FLAG_ENV: &str = "BO2MP_UPDATE";
const GAMES_ENV: &str = "BO2MP_GAMES";

fn main() -> ExitCode {
    launch().map_or_else(report_failure, |()| ExitCode::SUCCESS)
}

/// Prints why the run stopped and hands back the failing exit status.
fn report_failure(error: String) -> ExitCode {
    eprintln!("{TAG} {error}");
    // Double-clicked, the console closes with the message unless we wait.
    #[cfg(windows)]
    {
        eprintln!("{TAG} press Enter to exit");
        let mut ignored = String::new();
        std::io::stdin().read_line(&mut ignored).ok();
    }
    ExitCode::from(1)
}

fn launch() -> Result<(), String> {
    let dir = app_home_folder()?;
    sidecar::load(&dir)?;
    println!("{TAG} install {shown}", shown = dir.display());

    let flag = std::env::var(UPDATE_FLAG_ENV).ok();
    let shipping_variables = if update_asked(std::env::args().skip(1), flag.as_deref()) {
        let channel = std::env::var(UPDATE_URL_ENV).ok();
        update(&dir, channel.as_deref(), http::get)?
    } else {
        if !dir.join(GAME_FILE).is_file() {
            return Err(game_missing(&dir));
        }
        println!("{TAG} starting the {GAME_FILE} already here (pass `update` to fetch a new one)");
        BTreeMap::new()
    };
    start_game(&dir, &shipping_variables)
}

/// The folder the launcher lives in, which is the install.
fn app_home_folder() -> Result<PathBuf, String> {
    let beside_exe = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(Path::to_path_buf));
    match beside_exe {
        Some(dir) => Ok(dir),
        None => std::env::current_dir()
            .map_err(|error| format!("cannot find the install folder: {error}")),
    }
}

/// Whether the player asked for an update: an `update` / `--update` argument,
/// or `BO2MP_UPDATE` set to anything but nothing, `0` or `false`.
fn update_asked(args: impl IntoIterator<Item = String>, flag: Option<&str>) -> bool {
    if args
        .into_iter()
        .any(|word| matches!(word.as_str(), "update" | "--update"))
    {
        return true;
    }
    flag.map(str::trim)
        .is_some_and(|flag| !(flag.is_empty() || flag == "0" || flag.eq_ignore_ascii_case("false")))
}

fn game_missing(dir: &Path) -> String {
    let shown = dir.display();
    format!(
        "{GAME_FILE} is not in {shown}. A release archive carries it; to fetch one instead, \
         run `bo2mp-launcher update` with {UPDATE_URL_ENV} set in the portable .env"
    )
}

/// Installs the release the channel at `channel` offers into `dir` and gives
/// back the environment defaults it ships. `get` fetches an `https://` address.
fn update(
    dir: &Path,
    channel: Option<&str>,
    get: impl Fn(&str) -> Result<Vec<u8>, String>,
) -> Result<BTreeMap<String, String>, String> {
    let base = channel::base_from(channel)?;
    println!("{TAG} channel {base}");

    let Manifest { channel: lane, protocol, git, env, files } =
        Manifest::parse(&get(&channel::address_of(&base, "manifest.json"))?)?;
    for shipped in &files {
        let fetch = |name: &str| get(&channel::address_of(&base, name));
        let shown = &shipped.path;
        match install::bring_in(dir, shipped, fetch)? {
            Outcome::Current => println!("{TAG} {shown} up to date"),
            Outcome::Installed => println!("{TAG} installed {shown}"),
        }
    }
    sidecar::write_first(dir, &env, channel)?;
    println!("{TAG} updated to channel={lane} protocol={protocol} git={git}");
    Ok(env)
}

fn start_game(dir: &Path, shipping_variables: &BTreeMap<String, String>) -> Result<(), String> {
    println!("{TAG} starting {GAME_FILE}");
    game_command(dir, shipping_variables, |name| std::env::var_os(name).is_some())
        .spawn()
        .map(drop)
        .map_err(|error| format!("spawn {GAME_FILE}: {error}"))
}

/// The game's command line: the menu, run from the install folder, with the
/// games folder and the release's defaults for whatever `is_set` says the
/// player has not already set.
fn game_command(
    dir: &Path,
    shipping_variables: &BTreeMap<String, String>,
    is_set: impl Fn(&str) -> bool,
) -> Command {
    let mut game = Command::new(dir.join(GAME_FILE));
    game.current_dir(dir).arg("menu");
    if !is_set(GAMES_ENV) {
        game.env(GAMES_ENV, dir);
    }
    for (key, default) in shipping_variables {
        if !is_set(key) {
            game.env(key, default);
        }
    }
    game
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::collections::HashMap;
    use std::ffi::OsStr;
    use std::fs;

    use serde_json::json;

    use super::*;
    use crate::install::digest;
    use crate::testdir::TestDir;

    const CHANNEL: &str = "https://updates.example.net/prod";

    /// A channel held in memory: every address it answers, and the order the
    /// addresses were asked for.
    struct FakeChannel {
        answers: HashMap<String, Vec<u8>>,
        asked: RefCell<Vec<String>>,
    }

    impl FakeChannel {
        /// What the release tool publishes for a game and a CA file.
        fn release(game: &[u8], ca: &[u8]) -> Self {
            let blob = zstd::encode_all(game, 3).unwrap();
            let manifest = json!({
                "channel": "prod",
                "protocol": 3,
                "git": "abc1234",
                "env": {
                    "BO2MP_MASTER_ADDR": "master.example.net:4433",
                    "BO2MP_MASTER_SERVER_NAME": "bo2mp-prod",
                    "BO2MP_MASTER_CA_CERT": "bo2mp-ca.pem",
                },
                "files": [
                    {
                        "path": "bo2mp.exe",
                        "sha256": digest(game),
                        "size": game.len(),
                        "download": "bo2mp-abc1234.exe.zst",
                        "download_sha256": digest(&blob),
                        "download_size": blob.len(),
                    },
                    { "path": "bo2mp-ca.pem", "sha256": digest(ca), "size": ca.len() },
                ],
            });
            let mut answers = HashMap::new();
            answers.insert(
                format!("{CHANNEL}/manifest.json"),
                manifest.to_string().into_bytes(),
            );
            answers.insert(format!("{CHANNEL}/bo2mp-abc1234.exe.zst"), blob);
            answers.insert(format!("{CHANNEL}/bo2mp-ca.pem"), ca.to_vec());
            Self {
                answers,
                asked: RefCell::default(),
            }
        }

        fn answer(&self, address: &str) -> Result<Vec<u8>, String> {
            self.asked.borrow_mut().push(address.to_owned());
            self.answers
                .get(address)
                .cloned()
                .ok_or_else(|| format!("GET {address}: HTTP/1.1 404 Not Found"))
        }

        fn asked(&self) -> Vec<String> {
            self.asked.borrow().clone()
        }
    }

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|arg| (*arg).to_owned()).collect()
    }

    #[test]
    fn an_update_is_asked_for_by_argument() {
        assert!(update_asked(args(&["update"]), None));
        assert!(update_asked(args(&["--update"]), None));
        assert!(update_asked(args(&["menu", "update"]), None));
        assert!(!update_asked(args(&[]), None));
        assert!(!update_asked(args(&["menu", "updates", "--updated"]), None));
    }

    #[test]
    fn an_update_is_asked_for_by_environment() {
        for on in ["1", "yes", "true", " 1 ", "anything"] {
            assert!(update_asked(args(&[]), Some(on)), "{on:?}");
        }
        for off in ["", "  ", "0", "false", "FALSE", " False "] {
            assert!(!update_asked(args(&[]), Some(off)), "{off:?}");
        }
        assert!(update_asked(args(&["update"]), Some("0")));
    }

    #[test]
    fn a_missing_game_says_how_to_fetch_one() {
        let said = game_missing(Path::new("C:/games/bo2mp"));
        assert!(
            said.contains("bo2mp.exe is not in C:/games/bo2mp"),
            "{said}"
        );
        assert!(said.contains("`bo2mp-launcher update`"), "{said}");
        assert!(said.contains("BO2MP_UPDATE_URL"), "{said}");
    }

    #[test]
    fn the_game_starts_the_menu_from_its_own_folder() {
        let dir = Path::new("install");
        let game = game_command(dir, &BTreeMap::new(), |_| false);
        assert_eq!(game.get_program(), dir.join("bo2mp.exe").as_os_str());
        assert_eq!(game.get_args().collect::<Vec<_>>(), ["menu"]);
        assert_eq!(game.get_current_dir(), Some(dir));
    }

    #[test]
    fn release_defaults_fill_only_what_the_player_left_unset() {
        let dir = Path::new("install");
        let release: BTreeMap<String, String> = [
            ("BO2MP_MASTER_ADDR", "release:4433"),
            ("BO2MP_MASTER_SERVER_NAME", "release-name"),
        ]
        .map(|pair| (pair.0.to_owned(), pair.1.to_owned()))
        .into();
        let set = ["BO2MP_MASTER_ADDR"];
        let game = game_command(dir, &release, |name| set.contains(&name));
        let envs: BTreeMap<&OsStr, Option<&OsStr>> = game.get_envs().collect();
        assert_eq!(envs.len(), 2, "{envs:?}");
        assert_eq!(
            envs[OsStr::new("BO2MP_MASTER_SERVER_NAME")],
            Some(OsStr::new("release-name"))
        );
        assert_eq!(envs[OsStr::new("BO2MP_GAMES")], Some(dir.as_os_str()));
    }

    #[test]
    fn a_games_folder_the_player_set_is_kept() {
        let game = game_command(Path::new("install"), &BTreeMap::new(), |name| {
            name == "BO2MP_GAMES"
        });
        assert_eq!(game.get_envs().count(), 0);
    }

    #[test]
    fn a_first_update_installs_everything_and_writes_the_env() {
        let dir = TestDir::new("first-update");
        let channel = FakeChannel::release(&b"game bytes ".repeat(300), b"the ca");
        let env = update(
            dir.path(),
            Some(" https://updates.example.net/prod/ "),
            |url| channel.answer(url),
        )
        .unwrap();

        assert_eq!(env["BO2MP_MASTER_SERVER_NAME"], "bo2mp-prod");
        assert_eq!(
            fs::read(dir.join("bo2mp.exe")).unwrap(),
            b"game bytes ".repeat(300)
        );
        assert_eq!(fs::read(dir.join("bo2mp-ca.pem")).unwrap(), b"the ca");
        let sidecar = fs::read_to_string(dir.join(".env")).unwrap();
        assert!(
            sidecar.contains("BO2MP_MASTER_ADDR=master.example.net:4433\n"),
            "{sidecar}"
        );
        assert!(
            sidecar.ends_with("BO2MP_UPDATE_URL=https://updates.example.net/prod/\n"),
            "{sidecar}"
        );
        assert_eq!(dir.entry_names(), [".env", "bo2mp-ca.pem", "bo2mp.exe"]);
        assert_eq!(
            channel.asked(),
            [
                format!("{CHANNEL}/manifest.json"),
                format!("{CHANNEL}/bo2mp-abc1234.exe.zst"),
                format!("{CHANNEL}/bo2mp-ca.pem"),
            ]
        );
    }

    #[test]
    fn a_second_update_downloads_nothing_it_already_has() {
        let dir = TestDir::new("second-update");
        let channel = FakeChannel::release(b"game", b"ca");
        update(dir.path(), Some(CHANNEL), |url| channel.answer(url)).unwrap();
        fs::write(dir.join(".env"), "BO2MP_MASTER_ADDR=edited\n").unwrap();
        let before = channel.asked().len();

        update(dir.path(), Some(CHANNEL), |url| channel.answer(url)).unwrap();
        assert_eq!(
            channel.asked()[before..],
            [format!("{CHANNEL}/manifest.json")]
        );
        assert_eq!(
            fs::read_to_string(dir.join(".env")).unwrap(),
            "BO2MP_MASTER_ADDR=edited\n"
        );
    }

    #[test]
    fn an_update_that_loses_a_file_writes_no_env() {
        let dir = TestDir::new("lost-file");
        let mut channel = FakeChannel::release(b"game", b"ca");
        channel.answers.remove(&format!("{CHANNEL}/bo2mp-ca.pem"));
        let error = update(dir.path(), Some(CHANNEL), |url| channel.answer(url)).unwrap_err();
        assert!(error.contains("404"), "{error}");
        assert!(!dir.join(".env").exists());
    }

    #[test]
    fn a_manifest_that_fails_its_checks_installs_nothing() {
        let dir = TestDir::new("bad-manifest");
        let mut channel = FakeChannel::release(b"game", b"ca");
        channel
            .answers
            .insert(format!("{CHANNEL}/manifest.json"), b"{ not json".to_vec());
        let error = update(dir.path(), Some(CHANNEL), |url| channel.answer(url)).unwrap_err();
        assert!(error.starts_with("manifest parse:"), "{error}");
        assert_eq!(channel.asked().len(), 1);
        assert!(dir.entry_names().is_empty());
    }

    #[test]
    fn an_update_needs_a_channel_before_it_asks_anyone() {
        let dir = TestDir::new("no-channel");
        let channel = FakeChannel::release(b"game", b"ca");
        for setting in [None, Some(""), Some("http://updates.example.net/prod")] {
            assert!(
                update(dir.path(), setting, |url| channel.answer(url)).is_err(),
                "{setting:?}"
            );
        }
        assert!(channel.asked().is_empty());
    }
}
