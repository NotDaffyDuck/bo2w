//! `bo2mp.exe` and `bo2mp-launcher.exe`, cross-built with `cargo xwin`, and
//! the one property of the game exe a refactor could silently lose.

use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::Value;

use crate::dotenv::Env;
use crate::shell::{Res, Step, capture, require_tools, run, tool_on_path};

pub const TARGET: &str = "x86_64-pc-windows-msvc";
/// The master runs on a Linux host.
pub const LINUX_TARGET: &str = "x86_64-unknown-linux-gnu";

/// `crates/launcher/build.rs` links the game with an 8 MiB main-thread stack;
/// the 1 MiB Windows default overflows on deep zone loads.
pub const MIN_STACK_RESERVE: u64 = 8 << 20;

const SHIP_PROFILES: [&str; 3] = ["play", "release", "perf"];

pub struct WindowsBins {
    pub game: PathBuf,
    pub launcher: PathBuf,
}

pub fn require_profile(profile: &str) -> Res<()> {
    if SHIP_PROFILES.contains(&profile) {
        Ok(())
    } else {
        Err(format!(
            "PROFILE must be one of {SHIP_PROFILES:?}, not {profile:?}"
        ))
    }
}

/// `PROFILE` from `.env` or the environment, `play` when unset.
pub fn profile(env: &Env) -> Res<String> {
    let profile = env.get("PROFILE").unwrap_or_else(|| "play".to_string());
    require_profile(&profile)?;
    Ok(profile)
}

/// The CA pem compiled into the launcher and shipped beside it. It must
/// already exist: minting one here would strand every installed launcher.
pub fn public_ca(env: &Env) -> Res<PathBuf> {
    let ca = match env.get("BO2MP_UPDATER_CA_CERT") {
        Some(path) => PathBuf::from(path),
        None => PathBuf::from(
            env.get("BO2MP_RELEASE_KEY")
                .ok_or("set BO2MP_UPDATER_CA_CERT (or BO2MP_RELEASE_KEY) in .env")?,
        )
        .join("bo2mp-ca.pem"),
    };
    if ca.is_file() {
        Ok(ca)
    } else {
        Err(format!(
            "no CA at {}; make it once with `cargo xtask certs <host>`, never a fresh one per build",
            ca.display()
        ))
    }
}

/// `cargo xtask windows setup`.
pub fn setup() -> Res<()> {
    require_tools(&["cargo", "rustup", "llvm-lib"])?;
    run(Command::new("rustup").args(["target", "add", TARGET]))?;
    if !tool_on_path("cargo-xwin") {
        run(Command::new("cargo").args(["install", "cargo-xwin", "--locked"]))?;
    }
    println!("windows setup: {TARGET} and cargo-xwin installed");
    Ok(())
}

fn check_toolchain() -> Res<()> {
    let hint = "; run `cargo xtask windows setup`";
    require_tools(&["cargo", "rustup", "llvm-lib", "cargo-xwin"])
        .map_err(|missing| format!("{missing}{hint}"))?;
    let installed = capture(Command::new("rustup").args(["target", "list", "--installed"]))?;
    if installed.lines().any(|line| line.trim() == TARGET) {
        Ok(())
    } else {
        Err(format!("rust target {TARGET} is not installed{hint}"))
    }
}

pub fn build(profile: &str, ca: &Path) -> Res<WindowsBins> {
    require_profile(profile)?;
    check_toolchain()?;
    if !ca.is_file() {
        return Err(format!("no CA at {}", ca.display()));
    }
    let cache = match std::env::var("XWIN_CACHE_DIR") {
        Ok(dir) => PathBuf::from(dir),
        Err(_) => PathBuf::from(std::env::var("HOME").unwrap_or_else(|_| ".".to_string()))
            .join(".cache/cargo-xwin"),
    };
    std::fs::create_dir_all(&cache)
        .map_err(|error| format!("creating {}: {error}", cache.display()))?;

    let step = Step::start("windows.build", &format!("profile={profile}"));
    let messages = capture(
        Command::new("cargo")
            .args([
                "xwin",
                "build",
                "--locked",
                "--profile",
                profile,
                "--target",
                TARGET,
            ])
            .args(["-p", "launcher", "-p", "updater"])
            .arg("--message-format=json-render-diagnostics")
            .env("XWIN_CACHE_DIR", &cache)
            .env("BO2MP_UPDATER_CA_CERT", ca),
    )?;
    let bins = WindowsBins {
        game: built_bin(&messages, "bo2mp")?,
        launcher: built_bin(&messages, "bo2mp-launcher")?,
    };
    let image = std::fs::read(&bins.game)
        .map_err(|error| format!("reading {}: {error}", bins.game.display()))?;
    let reserve = stack_reserve(&image).map_err(|why| format!("{}: {why}", bins.game.display()))?;
    if reserve < MIN_STACK_RESERVE {
        return Err(format!(
            "{}: main-thread stack reserve is {reserve} bytes, want at least {MIN_STACK_RESERVE}; check the /STACK link arg in crates/launcher/build.rs",
            bins.game.display()
        ));
    }
    step.done(&format!(
        "stack={}MiB game={} launcher={}",
        reserve >> 20,
        bins.game.display(),
        bins.launcher.display()
    ));
    Ok(bins)
}

/// The executable cargo reported for binary `name`, from its JSON messages.
/// When a target is reported more than once the last report is current.
pub fn last_bin(messages: &str, name: &str) -> Option<PathBuf> {
    messages
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line.trim()).ok())
        .filter(|message| message["reason"] == "compiler-artifact")
        .filter(|message| message["target"]["name"] == name)
        .filter(|message| {
            message["target"]["kind"]
                .as_array()
                .is_some_and(|kinds| kinds.iter().any(|kind| kind == "bin"))
        })
        .filter_map(|message| message["executable"].as_str().map(PathBuf::from))
        .next_back()
}

pub fn built_bin(messages: &str, name: &str) -> Res<PathBuf> {
    let path = last_bin(messages, name)
        .ok_or_else(|| format!("cargo reported no executable for bin {name}"))?;
    if path.is_file() {
        Ok(path)
    } else {
        Err(format!(
            "cargo reported {} but it does not exist",
            path.display()
        ))
    }
}

fn bytes<const N: usize>(image: &[u8], at: usize) -> Res<[u8; N]> {
    image
        .get(at..at + N)
        .and_then(|slice| slice.try_into().ok())
        .ok_or_else(|| format!("image ends before offset {at:#x}"))
}

/// `SizeOfStackReserve` from a PE image's optional header: 4 bytes in PE32,
/// 8 in PE32+ (magic `0x20b`), 72 bytes past the optional header's start.
pub fn stack_reserve(image: &[u8]) -> Res<u64> {
    let pe = u32::from_le_bytes(bytes::<4>(image, 0x3c)?) as usize;
    if bytes::<4>(image, pe)? != *b"PE\0\0" {
        return Err("no PE signature".to_string());
    }
    let optional = pe + 24;
    let field = optional + 72;
    match u16::from_le_bytes(bytes::<2>(image, optional)?) {
        0x20b => Ok(u64::from_le_bytes(bytes::<8>(image, field)?)),
        0x10b => Ok(u64::from(u32::from_le_bytes(bytes::<4>(image, field)?))),
        other => Err(format!("unknown optional header magic {other:#x}")),
    }
}

/// `cargo xtask windows [build|setup]`, `build` by default.
pub fn run_cli(env: &Env, args: &[String]) -> Res<()> {
    match args.first().map_or("build", String::as_str) {
        "setup" => setup(),
        "build" => {
            let bins = build(&profile(env)?, &public_ca(env)?)?;
            println!("bo2mp.exe={}", bins.game.display());
            println!("bo2mp-launcher.exe={}", bins.launcher.display());
            Ok(())
        }
        other => Err(format!(
            "usage: cargo xtask windows [build|setup], not {other:?}"
        )),
    }
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
    fn only_ship_profiles_are_allowed() {
        for good in ["play", "release", "perf"] {
            assert!(require_profile(good).is_ok());
        }
        assert!(require_profile("dev").is_err());
    }
}
