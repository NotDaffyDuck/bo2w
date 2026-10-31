//! `bo2mp.exe` and `bo2mp-launcher.exe`, cross-built with `cargo xwin`, and
//! the one property of the game exe a refactor could silently lose.

use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::Value;

use crate::dot_vars::DotVars;
use crate::cmd_run::{Phase, grab, demand_host_programs, launch, on_path};

pub const TARGET: &str = concat!("x86_64", "-pc-windows-msvc");
/// The master runs on a Linux host.
pub const LINUX_TARGET: &str = concat!("x86_64", "-unknown-linux-gnu");

/// `crates/launcher/build.rs` links the game with an 8 MiB main-thread stack;
/// the 1 MiB Windows default overflows on deep zone loads.
pub const MIN_STACK_RESERVE: u64 = 8 << 20;

const SHIP_PROFILES: [&str; 3] = ["play", "release", "perf"];

pub struct WinReleaseBinaries {
    pub game_exe: PathBuf,
    pub launcher_exe: PathBuf,
}

pub fn demand_build_profile(flavour: &str) -> Result<(), String> {
    if !SHIP_PROFILES.contains(&flavour) {
        return Err(format!(
            "PROFILE must be one of {SHIP_PROFILES:?}, not {flavour:?}"
        ));
    }
    Ok(())
}

/// `PROFILE` from `.env` or the environment, `play` when unset.
pub fn flavour(dot_conf: &DotVars) -> Result<String, String> {
    let chosen = dot_conf.value_of("PROFILE").unwrap_or_else(|| String::from("play"));
    demand_build_profile(&chosen).map(|()| chosen)
}

/// The CA pem compiled into the launcher and shipped beside it. It must
/// already exist: minting one here would strand every installed launcher.
pub fn issuer_cert_path(dot_conf: &DotVars) -> Result<PathBuf, String> {
    let configured = dot_conf.value_of("BO2MP_UPDATER_CA_CERT").map(PathBuf::from);
    let ca = configured.map_or_else(|| ca_in_release_key_dir(dot_conf), Ok)?;
    require_file(&ca, || {
        let shown = ca.display();
        format!(
            "no CA at {shown}; make it once with `cargo xtask certs <host>`, never a fresh one per build"
        )
    })?;
    Ok(ca)
}

fn ca_in_release_key_dir(dot_conf: &DotVars) -> Result<PathBuf, String> {
    let missing = "set BO2MP_UPDATER_CA_CERT (or BO2MP_RELEASE_KEY) in .env";
    let key_dir = dot_conf.value_of("BO2MP_RELEASE_KEY").ok_or(missing)?;
    Ok(Path::new(&key_dir).join("bo2mp-ca.pem"))
}

/// `Ok` when `path` is a regular file, else the complaint.
fn require_file(file_at: &Path, complaint: impl FnOnce() -> String) -> Result<(), String> {
    match file_at.is_file() {
        true => Ok(()),
        false => Err(complaint()),
    }
}

/// `cargo xtask windows setup`.
pub fn install_tools() -> Result<(), String> {
    const NEEDED: [&str; 3] = ["cargo", "rustup", "llvm-lib"];
    const XWIN_INSTALL: [&str; 3] = ["install", "cargo-xwin", "--locked"];
    demand_host_programs(&NEEDED)?;
    let mut add_target = Command::new("rustup");
    add_target.args(["target", "add", TARGET]);
    launch(&mut add_target)?;
    if !on_path("cargo-xwin") {
        let mut install = Command::new("cargo");
        launch(install.args(XWIN_INSTALL))?;
    }
    println!("windows setup: {TARGET} and cargo-xwin installed");
    Ok(())
}

fn check_toolchain() -> Result<(), String> {
    let hint = "; run `cargo xtask windows setup`";
    if let Err(missing) = demand_host_programs(&["cargo", "rustup", "llvm-lib", "cargo-xwin"]) {
        return Err(missing + hint);
    }
    let listing = grab(Command::new("rustup").args(["target", "list", "--installed"]))?;
    let present = listing.lines().map(str::trim).any(|title| title == TARGET);
    if present {
        return Ok(());
    }
    Err(format!("rust target {TARGET} is not installed{hint}"))
}

/// Where cargo-xwin keeps the downloaded Windows SDK, created if needed.
fn xwin_cache() -> Result<PathBuf, String> {
    let base_dir = std::env::var_os("XWIN_CACHE_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            let home = std::env::var_os("HOME").unwrap_or_else(|| ".".into());
            Path::new(&home).join(".cache/cargo-xwin")
        });
    std::fs::create_dir_all(&base_dir).map_err(|error| format!("creating {}: {error}", base_dir.display()))?;
    Ok(base_dir)
}

/// cargo's JSON messages from cross-building the launcher and updater.
fn cross_build(flavour: &str, ca: &Path, cache: &Path) -> Result<String, String> {
    let mut cargo = Command::new("cargo");
    cargo.args(["xwin", "build", "--locked"]);
    cargo.args(["--profile", flavour, "--target", TARGET]);
    cargo.args(["-p", "launcher", "-p", "updater"]);
    cargo.arg("--message-format=json-render-diagnostics");
    cargo.env("XWIN_CACHE_DIR", cache);
    cargo.env("BO2MP_UPDATER_CA_CERT", ca);
    grab(&mut cargo)
}

/// The game's main-thread stack reserve, refusing anything below the minimum.
fn checked_stack(game: &Path) -> Result<u64, String> {
    let image = std::fs::read(game).map_err(|error| format!("reading {}: {error}", game.display()))?;
    let reserve = stack_reserve(&image).map_err(|why| format!("{}: {why}", game.display()))?;
    if reserve >= MIN_STACK_RESERVE {
        return Ok(reserve);
    }
    Err(format!(
        "{}: main-thread stack reserve is {reserve} bytes, want at least {MIN_STACK_RESERVE}; check the /STACK link arg in crates/launcher/build.rs",
        game.display()
    ))
}

pub fn assemble(flavour: &str, ca: &Path) -> Result<WinReleaseBinaries, String> {
    demand_build_profile(flavour).and_then(|()| check_toolchain())?;
    require_file(ca, || format!("no CA at {}", ca.display()))?;
    let cache = xwin_cache()?;

    let step = Phase::start("windows.build", &format!("profile={flavour}"));
    let messages = cross_build(flavour, ca, &cache)?;
    let (game, launcher) = (
        built_bin(&messages, "bo2mp")?,
        built_bin(&messages, "bo2mp-launcher")?,
    );
    let reserve = checked_stack(&game)?;
    let [shown_game, shown_launcher] = [&game, &launcher].map(|bin| bin.display());
    let summary = format!(
        "stack={mib}MiB game={shown_game} launcher={shown_launcher}",
        mib = reserve >> 20
    );
    step.wrap(&summary);
    Ok(WinReleaseBinaries { game_exe: game, launcher_exe: launcher })
}

/// `message`'s executable when it is cargo's artifact report for bin `name`.
fn bin_artifact(message: &Value, title: &str) -> Option<PathBuf> {
    let kinds = message.pointer("/target/kind")?.as_array()?;
    let is_artifact = message["reason"] == "compiler-artifact";
    let named = message.pointer("/target/name").is_some_and(|found| *found == title);
    let is_bin = kinds.iter().any(|kind| kind == "bin");
    if !(is_artifact && named && is_bin) {
        return None;
    }
    message["executable"].as_str().map(PathBuf::from)
}

/// The executable cargo reported for binary `name`, from its JSON messages.
/// When a target is reported more than once the last report is current.
pub fn last_bin(messages: &str, title: &str) -> Option<PathBuf> {
    messages
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line.trim()).ok())
        .filter_map(|message| bin_artifact(&message, title))
        .last()
}

pub fn built_bin(messages: &str, title: &str) -> Result<PathBuf, String> {
    match last_bin(messages, title) {
        None => Err(format!("cargo reported no executable for bin {title}")),
        Some(file_at) if file_at.is_file() => Ok(file_at),
        Some(file_at) => Err(format!(
            "cargo reported {} but it does not exist",
            file_at.display()
        )),
    }
}

fn bytes<const N: usize>(image: &[u8], at: usize) -> Result<[u8; N], String> {
    let window = image.get(at..at.saturating_add(N));
    match window.and_then(|slice| <[u8; N]>::try_from(slice).ok()) {
        Some(array) => Ok(array),
        None => Err(format!("image ends before offset {at:#x}")),
    }
}

/// `SizeOfStackReserve` from a PE image's optional header: 4 bytes in PE32,
/// 8 in PE32+ (magic `0x20b`), 72 bytes past the optional header's start.
pub fn stack_reserve(image: &[u8]) -> Result<u64, String> {
    let pe_at = u32::from_le_bytes(bytes(image, 0x3c)?) as usize;
    let signature: [u8; 4] = bytes(image, pe_at)?;
    if &signature != b"PE\0\0" {
        return Err(String::from("no PE signature"));
    }
    let optional = pe_at + 24;
    let magic = u16::from_le_bytes(bytes(image, optional)?);
    let field = optional + 72;
    if magic == 0x20b {
        return Ok(u64::from_le_bytes(bytes(image, field)?));
    }
    if magic == 0x10b {
        return Ok(u32::from_le_bytes(bytes(image, field)?).into());
    }
    Err(format!("unknown optional header magic {magic:#x}"))
}

/// `cargo xtask windows [build|setup]`, `build` by default.
pub fn dispatch_subcommand(dot_conf: &DotVars, cli_words: &[String]) -> Result<(), String> {
    let sub = cli_words.first().map(String::as_str).unwrap_or("build");
    if sub == "setup" {
        return install_tools();
    }
    if sub != "build" {
        return Err(format!(
            "usage: cargo xtask windows [build|setup], not {sub:?}"
        ));
    }
    let exes = assemble(&flavour(dot_conf)?, &issuer_cert_path(dot_conf)?)?;
    println!("bo2mp.exe={}", exes.game_exe.display());
    println!("bo2mp-launcher.exe={}", exes.launcher_exe.display());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn image(magic: u16, reserve: u64) -> Vec<u8> {
        let pe = 0x80;
        let mut image = vec![0_u8; 0x200];
        image[0x3c..0x40].copy_from_slice(&(pe as u32).to_le_bytes());
        image[pe..pe + 4].copy_from_slice(b"PE\0\0");
        image[pe + 24..pe + 26].copy_from_slice(&magic.to_le_bytes());
        let field = pe + 24 + 72;
        if magic == 0x20b {
            image[field..field + 8].copy_from_slice(&reserve.to_le_bytes());
        } else {
            image[field..field + 4].copy_from_slice(&(reserve as u32).to_le_bytes());
        }
        image
    }

    #[test]
    fn stack_reserve_reads_pe32_plus_and_pe32() {
        assert_eq!(stack_reserve(&image(0x20b, 8 << 20)).unwrap(), 8 << 20);
        assert_eq!(stack_reserve(&image(0x10b, 1 << 20)).unwrap(), 1 << 20);
    }

    #[test]
    fn stack_reserve_refuses_what_is_not_a_pe() {
        let mut broken = image(0x20b, 8 << 20);
        broken[0x80] = b'X';
        assert!(stack_reserve(&broken).is_err());
        assert!(stack_reserve(&image(0x20b, 0)[..0x90]).is_err());
        assert!(stack_reserve(&image(0x999, 0)).is_err());
        assert!(stack_reserve(&[0_u8; 8]).is_err());
    }

    #[test]
    fn last_reported_bin_wins_and_libs_are_ignored() {
        let messages = [
            r#"{"reason":"compiler-artifact","target":{"name":"bo2mp","kind":["bin"]},"executable":"/old/bo2mp.exe"}"#,
            r#"{"reason":"compiler-artifact","target":{"name":"bo2mp","kind":["lib"]},"executable":null}"#,
            r#"{"reason":"build-finished","success":true}"#,
            "not json",
            r#"{"reason":"compiler-artifact","target":{"name":"bo2mp","kind":["bin"]},"executable":"/new/bo2mp.exe"}"#,
            r#"{"reason":"compiler-artifact","target":{"name":"bo2mp-launcher","kind":["bin"]},"executable":"/new/bo2mp-launcher.exe"}"#,
        ]
        .join("\n");
        assert_eq!(
            last_bin(&messages, "bo2mp"),
            Some(PathBuf::from("/new/bo2mp.exe"))
        );
        assert_eq!(
            last_bin(&messages, "bo2mp-launcher"),
            Some(PathBuf::from("/new/bo2mp-launcher.exe"))
        );
        assert_eq!(last_bin(&messages, "updater"), None);
        assert!(
            built_bin(&messages, "bo2mp").is_err(),
            "a path that is not there is an error"
        );
    }

    #[test]
    fn an_artifact_report_needs_reason_name_and_bin_kind() {
        let report = |reason: &str, crate_name: &str, kind: &str| {
            let json = format!(
                r#"{{"reason":"{reason}","target":{{"name":"{crate_name}","kind":["{kind}"]}},"executable":"/x/{crate_name}"}}"#
            );
            bin_artifact(&serde_json::from_str(&json).unwrap(), "tool")
        };
        assert_eq!(
            report("compiler-artifact", "tool", "bin"),
            Some(PathBuf::from("/x/tool"))
        );
        assert_eq!(report("build-finished", "tool", "bin"), None);
        assert_eq!(report("compiler-artifact", "other", "bin"), None);
        assert_eq!(report("compiler-artifact", "tool", "lib"), None);
        let no_target: Value = serde_json::from_str(r#"{"reason":"compiler-artifact"}"#).unwrap();
        assert_eq!(bin_artifact(&no_target, "tool"), None);
    }

    #[test]
    fn a_missing_ca_is_refused_with_its_path() {
        let ca = std::env::temp_dir().join("xtask-no-such-ca.pem");
        let error = assemble("play", &ca).err().unwrap();
        // Either the toolchain is not set up here, or the CA is the complaint.
        assert!(error.contains("xtask windows setup") || error.contains("no CA at"), "{error}");
        assert!(require_file(&ca, || String::from("gone")).is_err());
        assert!(require_file(Path::new("Cargo.toml"), || String::new()).is_ok());
        assert!(assemble("dev", &ca).err().unwrap().contains("PROFILE must be one of"));
    }

    #[test]
    fn a_ca_comes_from_the_explicit_variable_or_the_key_dir() {
        let key_only = DotVars::parse("BO2MP_RELEASE_KEY=/keys\nBO2MP_UPDATER_CA_CERT=");
        if std::env::var_os("BO2MP_UPDATER_CA_CERT").is_none() {
            let error = issuer_cert_path(&key_only).unwrap_err();
            assert!(error.contains("bo2mp-ca.pem"), "{error}");
            assert!(error.starts_with("no CA at "), "{error}");
        }
        assert_eq!(
            ca_in_release_key_dir(&key_only).unwrap(),
            Path::new("/keys").join("bo2mp-ca.pem")
        );
    }

    #[test]
    fn only_ship_profiles_are_allowed() {
        for good in ["play", "release", "perf"] {
            assert!(demand_build_profile(good).is_ok());
        }
        assert!(demand_build_profile("dev").is_err());
    }
}
