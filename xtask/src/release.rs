//! `cargo xtask release <prod|dev|bundles>`.
//!
//! A channel release is built and staged locally under
//! `dist/releases/<channel>/<id>`: the zstd'd game, the launcher, the CA, the
//! client manifest, the Linux master, and `deployment.json` saying where each
//! file goes. `publish` uploads exactly that. `bundles` makes the two
//! portable Windows zips instead; nothing is uploaded.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Command;

use master_protocol::{Channel, PROTOCOL_VERSION};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::dotenv::Env;
use crate::provision::RELEASES_PORT;
use crate::shell::{Res, Ssh, Step, capture, require_tools, run, set_mode};
use crate::windows;

const ZSTD_LEVEL: u32 = 3;
/// zstd's "one worker per core".
const ZSTD_THREADS: u32 = 0;
/// Not a secret: the zip password only stops scanners unpacking the exe.
const ARCHIVE_PASSWORD: &[u8] = b"contextrot";

/// Shipped beside the binaries as `(repo path, archive name)`: the licence,
/// the notice, and the licences of the two fonts compiled into the game.
const LEGAL_FILES: [(&str, &str); 4] = [
    ("LICENSE", "LICENSE"),
    ("NOTICE", "NOTICE"),
    ("crates/ui/assets/OFL-Oxanium.txt", "OFL-Oxanium.txt"),
    (
        "crates/console/assets/COPYING-FreeFont.txt",
        "COPYING-FreeFont.txt",
    ),
];

pub fn sha256_hex(bytes: &[u8]) -> String {
    hex(&Sha256::digest(bytes))
}

fn hex(digest: &[u8]) -> String {
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

pub fn file_sha256(path: &Path) -> Res<String> {
    let mut file = std::fs::File::open(path)
        .map_err(|error| format!("opening {}: {error}", path.display()))?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0_u8; 1 << 20];
    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|error| format!("reading {}: {error}", path.display()))?;
        if read == 0 {
            return Ok(hex(&hasher.finalize()));
        }
        hasher.update(&buffer[..read]);
    }
}

pub fn file_size(path: &Path) -> Res<u64> {
    std::fs::metadata(path)
        .map(|meta| meta.len())
        .map_err(|error| format!("stat {}: {error}", path.display()))
}

pub fn write_json(path: &Path, value: &Value) -> Res<()> {
    let text = serde_json::to_string_pretty(value)
        .map_err(|error| format!("encoding {}: {error}", path.display()))?;
    std::fs::write(path, text + "\n")
        .map_err(|error| format!("writing {}: {error}", path.display()))
}

fn copy(from: &Path, to: &Path) -> Res<()> {
    std::fs::copy(from, to)
        .map(drop)
        .map_err(|error| format!("copying {} to {}: {error}", from.display(), to.display()))
}

fn make_dir(path: &Path) -> Res<()> {
    std::fs::create_dir_all(path).map_err(|error| format!("creating {}: {error}", path.display()))
}

/// The `v1.5.6` in `*** Zstandard CLI (64-bit) v1.5.6, by Yann Collet ***`.
pub fn parse_zstd_version(text: &str) -> Option<String> {
    text.split_whitespace()
        .map(|word| word.trim_end_matches(','))
        .find(|word| {
            word.strip_prefix('v')
                .is_some_and(|rest| rest.starts_with(|c: char| c.is_ascii_digit()))
        })
        .map(|word| word[1..].to_string())
}

/// Where players reach the release host: the host half of
/// `BO2MP_DEPLOY_HOST`.
pub fn public_host(env: &Env) -> Res<String> {
    Ok(Ssh::new(&env.require("BO2MP_DEPLOY_HOST")?)?
        .host()
        .to_string())
}

pub fn update_url(host: &str, channel: Channel) -> String {
    format!("https://{host}:{RELEASES_PORT}/{channel}")
}

struct Git {
    rev: String,
    dirty: bool,
}

fn git(root: &Path) -> Res<Git> {
    let git = |args: &[&str]| capture(Command::new("git").arg("-C").arg(root).args(args));
    Ok(Git {
        rev: git(&["rev-parse", "--short=12", "HEAD"])?
            .trim()
            .to_string(),
        dirty: !git(&["status", "--porcelain"])?.trim().is_empty(),
    })
}

/// A directory next to the releases that vanishes unless it is kept, so a
/// failed build never leaves something that looks like a release.
struct Scratch {
    path: PathBuf,
    keep: bool,
}

impl Scratch {
    fn new(parent: &Path, tag: &str) -> Res<Self> {
        let path = parent.join(format!(".stage-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        make_dir(&path)?;
        Ok(Self { path, keep: false })
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        if !self.keep {
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }
}

/// The master is static, so one binary runs on any Linux host.
pub fn build_master(root: &Path, profile: &str) -> Res<PathBuf> {
    let rustflags = std::env::var("RUSTFLAGS").unwrap_or_else(|_| String::new());
    let messages = capture(
        Command::new("cargo")
            .current_dir(root)
            .args([
                "build",
                "--locked",
                "--profile",
                profile,
                "--target",
                windows::LINUX_TARGET,
            ])
            .args([
                "-p",
                "bo2mp-master",
                "--message-format=json-render-diagnostics",
            ])
            .env(
                "RUSTFLAGS",
                format!("{rustflags} -C target-feature=+crt-static"),
            ),
    )?;
    windows::built_bin(&messages, "bo2mp-master")
}

struct GameBlob {
    name: String,
    exe_sha: String,
    exe_size: u64,
    blob_sha: String,
    blob_size: u64,
}

/// The game exe compressed once per (exe, zstd version, level, threads) and
/// cached, so re-releasing an unchanged game runs no zstd.
fn package_game(root: &Path, exe: &Path, into: &Path) -> Res<GameBlob> {
    require_tools(&["zstd"])?;
    let exe_sha = file_sha256(exe)?;
    let version = parse_zstd_version(&capture(Command::new("zstd").arg("--version"))?)
        .ok_or("cannot read the zstd version")?;
    let cache = root.join("dist/cache/zstd");
    make_dir(&cache)?;
    let cached = cache.join(format!(
        "{exe_sha}.zstd-{version}.{ZSTD_LEVEL}.T{ZSTD_THREADS}.zst"
    ));
    if !cached.is_file() {
        let step = Step::start("package.game", &format!("zstd {version} exe={exe_sha}"));
        let part = cached.with_extension("part");
        run(Command::new("zstd")
            .args([
                "-q",
                "-f",
                &format!("-{ZSTD_LEVEL}"),
                &format!("-T{ZSTD_THREADS}"),
                "-o",
            ])
            .arg(&part)
            .arg(exe))?;
        std::fs::rename(&part, &cached)
            .map_err(|error| format!("renaming {}: {error}", part.display()))?;
        step.done("");
    }
    let blob_sha = file_sha256(&cached)?;
    let name = format!("bo2mp-{blob_sha}.exe.zst");
    let dest = into.join(&name);
    if dest.is_file() {
        if file_sha256(&dest)? != blob_sha {
            return Err(format!("{} differs from the cached blob", dest.display()));
        }
    } else {
        copy(&cached, &dest)?;
    }
    Ok(GameBlob {
        name,
        exe_sha,
        exe_size: file_size(exe)?,
        blob_sha,
        blob_size: file_size(&dest)?,
    })
}

/// What the launcher fetches as `manifest.json`: the files that make a
/// working install, and the `.env` lines that point the game at the master.
fn client_manifest(
    channel: Channel,
    host: &str,
    git_rev: &str,
    id: &str,
    blob: &GameBlob,
    ca: (&str, u64),
) -> Value {
    json!({
        "id": id,
        "channel": channel.as_str(),
        "protocol": PROTOCOL_VERSION,
        "git": git_rev,
        "env": {
            "BO2MP_MASTER_ADDR": format!("{host}:{}", channel.port()),
            "BO2MP_MASTER_SERVER_NAME": channel.server_name(),
            "BO2MP_MASTER_CA_CERT": "bo2mp-ca.pem",
        },
        "files": [
            {
                "path": "bo2mp.exe",
                "sha256": blob.exe_sha,
                "size": blob.exe_size,
                "download": blob.name,
                "download_sha256": blob.blob_sha,
                "download_size": blob.blob_size,
            },
            { "path": "bo2mp-ca.pem", "sha256": ca.0, "size": ca.1 },
        ],
    })
}

/// The first 16 hex digits of a hash over everything that makes one release
/// differ from another, so an identical rebuild lands on the same id.
fn release_id(identity: &Value) -> String {
    sha256_hex(identity.to_string().as_bytes())[..16].to_string()
}

/// One staged file and where `publish` puts it. `remote` is under
/// `releases/<channel>/` except for the master, which is under the deploy root.
struct Shipped {
    role: &'static str,
    local: String,
    remote: String,
    immutable: bool,
}

impl Shipped {
    fn to_json(&self, stage: &Path) -> Res<Value> {
        let path = stage.join(&self.local);
        let mut row = json!({
            "role": self.role,
            "local": self.local,
            "remote": self.remote,
            "sha256": file_sha256(&path)?,
            "size": file_size(&path)?,
            "immutable": self.immutable,
        });
        if self.role == "manifest" {
            row["live"] = json!("manifest.json");
        }
        Ok(row)
    }
}

fn shipped(id: &str, blob_name: &str, master_sha: &str) -> [Shipped; 5] {
    let row = |role, local: &str, remote: &str, immutable| Shipped {
        role,
        local: local.to_string(),
        remote: remote.to_string(),
        immutable,
    };
    [
        row("game-blob", &format!("client/{blob_name}"), blob_name, true),
        row(
            "manifest",
            "client/manifest.json",
            &format!("manifests/{id}.json"),
            false,
        ),
        row(
            "launcher",
            "client/bo2mp-launcher.exe",
            "bo2mp-launcher.exe",
            false,
        ),
        row("ca", "client/bo2mp-ca.pem", "bo2mp-ca.pem", false),
        row(
            "master",
            "master/bo2mp-master",
            &format!("masters/{master_sha}/bo2mp-master"),
            true,
        ),
    ]
}

struct Meta<'a> {
    id: &'a str,
    channel: Channel,
    profile: &'a str,
    git: &'a Git,
    host: &'a str,
}

fn descriptor(meta: &Meta, files: Vec<Value>) -> Value {
    json!({
        "id": meta.id,
        "channel": meta.channel.as_str(),
        "profile": meta.profile,
        "target": windows::TARGET,
        "git": meta.git.rev,
        "dirty": meta.git.dirty,
        "protocol": PROTOCOL_VERSION,
        "master_port": meta.channel.port(),
        "update_url": update_url(meta.host, meta.channel),
        "files": files,
    })
}

pub fn prepare(root: &Path, env: &Env, channel: Channel, profile: &str) -> Res<PathBuf> {
    windows::require_profile(profile)?;
    let host = public_host(env)?;
    let git = git(root)?;
    let ca = windows::public_ca(env)?;
    let bins = windows::build(profile, &ca)?;
    let master = build_master(root, profile)?;

    let releases = root.join("dist/releases").join(channel.as_str());
    make_dir(&releases)?;
    let mut scratch = Scratch::new(&releases, channel.as_str())?;
    let stage = scratch.path.clone();
    let client = stage.join("client");
    make_dir(&client)?;
    make_dir(&stage.join("master"))?;

    let blob = package_game(root, &bins.game, &client)?;
    copy(&bins.launcher, &client.join("bo2mp-launcher.exe"))?;
    copy(&ca, &client.join("bo2mp-ca.pem"))?;
    let staged_master = stage.join("master/bo2mp-master");
    copy(&master, &staged_master)?;
    set_mode(&staged_master, 0o755)?;
    let master_sha = file_sha256(&staged_master)?;
    let ca_sha = file_sha256(&client.join("bo2mp-ca.pem"))?;

    let id = release_id(&json!({
        "channel": channel.as_str(),
        "dirty": git.dirty,
        "git": git.rev,
        "master_sha256": master_sha,
        "profile": profile,
        "protocol": PROTOCOL_VERSION,
        "target": windows::TARGET,
        "files": {
            "ca": ca_sha,
            "game_blob": blob.blob_sha,
            "game_exe": blob.exe_sha,
            "launcher": file_sha256(&client.join("bo2mp-launcher.exe"))?,
        },
    }));
    let manifest = client_manifest(
        channel,
        &host,
        &git.rev,
        &id,
        &blob,
        (&ca_sha, file_size(&client.join("bo2mp-ca.pem"))?),
    );
    write_json(&client.join("manifest.json"), &manifest)?;

    let dest = releases.join(&id);
    if dest.join("deployment.json").is_file() {
        println!("release: {} already staged", dest.display());
    } else {
        let files = shipped(&id, &blob.name, &master_sha)
            .iter()
            .map(|file| file.to_json(&stage))
            .collect::<Res<Vec<_>>>()?;
        let meta = Meta {
            id: &id,
            channel,
            profile,
            git: &git,
            host: &host,
        };
        write_json(&stage.join("deployment.json"), &descriptor(&meta, files))?;
        let _ = std::fs::remove_dir_all(&dest);
        std::fs::rename(&stage, &dest)
            .map_err(|error| format!("moving the stage to {}: {error}", dest.display()))?;
        scratch.keep = true;
    }
    std::fs::write(releases.join("LATEST"), format!("{id}\n"))
        .map_err(|error| format!("writing LATEST: {error}"))?;
    println!("release.done path=dist/releases/{channel}/{id}");
    Ok(dest)
}

/// The `.env` inside a portable zip: where to update from and which master
/// to trust.
pub fn bundle_env(host: &str, channel: Channel) -> String {
    format!(
        "BO2MP_UPDATE_URL={url}\n\
         BO2MP_MASTER_ADDR={host}:{port}\n\
         BO2MP_MASTER_SERVER_NAME={name}\n\
         BO2MP_MASTER_CA_CERT=bo2mp-ca.pem\n",
        url = update_url(host, channel),
        port = channel.port(),
        name = channel.server_name(),
    )
}

/// `dist/windows/bo2mp-windows-<channel>.zip` for both channels: a folder
/// that runs offline and updates itself only when asked.
pub fn bundles(root: &Path, env: &Env, profile: &str) -> Res<()> {
    windows::require_profile(profile)?;
    let host = public_host(env)?;
    let ca = windows::public_ca(env)?;
    let bins = windows::build(profile, &ca)?;

    let out = root.join("dist/windows");
    let _ = std::fs::remove_dir_all(&out);
    make_dir(&out)?;
    let scratch = Scratch::new(&out, "bundles")?;
    for channel in Channel::ALL {
        let stage = scratch.path.join(channel.as_str());
        make_dir(&stage)?;
        let mut inputs = vec![
            (bins.launcher.clone(), "bo2mp-launcher.exe"),
            (bins.game.clone(), "bo2mp.exe"),
            (ca.clone(), "bo2mp-ca.pem"),
        ];
        inputs.extend(LEGAL_FILES.iter().map(|(from, to)| (root.join(from), *to)));
        let mut files = Vec::new();
        for (from, name) in inputs {
            copy(&from, &stage.join(name))?;
            files.push(stage.join(name).display().to_string());
        }
        std::fs::write(stage.join(".env"), bundle_env(&host, channel))
            .map_err(|error| format!("writing {}/.env: {error}", stage.display()))?;
        files.push(stage.join(".env").display().to_string());
        let archive = out.join(format!("bo2mp-windows-{channel}.zip"));
        crate::bundle_zip::write_archive(&archive, &files, ARCHIVE_PASSWORD)?;
        println!("release.bundle {channel}: {}", archive.display());
    }
    Ok(())
}

pub fn run_cli(root: &Path, env: &Env, args: &[String]) -> Res<()> {
    let usage = "usage: cargo xtask release <prod|dev|bundles>";
    let [what] = args else {
        return Err(usage.to_string());
    };
    let profile = windows::profile(env)?;
    if what == "bundles" {
        return bundles(root, env, &profile);
    }
    let channel = what
        .parse::<Channel>()
        .map_err(|_| format!("{usage}, not {what:?}"))?;
    prepare(root, env, channel, &profile).map(drop)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn blob() -> GameBlob {
        GameBlob {
            name: "bo2mp-bb.exe.zst".to_string(),
            exe_sha: "aa".to_string(),
            exe_size: 100,
            blob_sha: "bb".to_string(),
            blob_size: 40,
        }
    }

    #[test]
    fn hashes_are_lowercase_sha256() {
        let abc = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";
        assert_eq!(sha256_hex(b"abc"), abc);
        let path = std::env::temp_dir().join(format!("xtask-sha-{}", std::process::id()));
        std::fs::write(&path, b"abc").unwrap();
        assert_eq!(file_sha256(&path).unwrap(), abc);
        assert_eq!(file_size(&path).unwrap(), 3);
        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn zstd_version_comes_from_the_banner() {
        let banner = "*** Zstandard CLI (64-bit) v1.5.6, by Yann Collet ***";
        assert_eq!(parse_zstd_version(banner).as_deref(), Some("1.5.6"));
        assert_eq!(parse_zstd_version("zstd command line interface\n"), None);
    }

    #[test]
    fn manifest_points_the_game_at_the_channel_master() {
        let manifest = client_manifest(
            Channel::Dev,
            "203.0.113.7",
            "abc123",
            "id16",
            &blob(),
            ("cc", 9),
        );
        assert_eq!(manifest["channel"], "dev");
        assert_eq!(manifest["protocol"], PROTOCOL_VERSION);
        assert_eq!(manifest["env"]["BO2MP_MASTER_ADDR"], "203.0.113.7:4434");
        assert_eq!(manifest["env"]["BO2MP_MASTER_SERVER_NAME"], "bo2mp-dev");
        assert_eq!(manifest["env"]["BO2MP_MASTER_CA_CERT"], "bo2mp-ca.pem");
        let game = &manifest["files"][0];
        assert_eq!(game["path"], "bo2mp.exe");
        assert_eq!(game["sha256"], "aa");
        assert_eq!(game["download"], "bo2mp-bb.exe.zst");
        assert_eq!(game["download_sha256"], "bb");
        assert_eq!(
            manifest["files"][1],
            json!({"path": "bo2mp-ca.pem", "sha256": "cc", "size": 9})
        );
    }

    #[test]
    fn release_id_is_sixteen_hex_and_follows_the_inputs() {
        let one = release_id(&json!({"git": "a", "files": {"ca": "1"}}));
        assert_eq!(one.len(), 16);
        assert!(one.chars().all(|c| c.is_ascii_hexdigit()));
        assert_eq!(one, release_id(&json!({"git": "a", "files": {"ca": "1"}})));
        assert_ne!(one, release_id(&json!({"git": "a", "files": {"ca": "2"}})));
    }

    #[test]
    fn every_shipped_file_has_a_place() {
        let rows = shipped("id16", "bo2mp-bb.exe.zst", "mm");
        let find = |role| rows.iter().find(|row| row.role == role).unwrap();
        assert_eq!(find("game-blob").remote, "bo2mp-bb.exe.zst");
        assert!(find("game-blob").immutable);
        assert_eq!(find("manifest").remote, "manifests/id16.json");
        assert!(!find("manifest").immutable);
        assert_eq!(find("master").remote, "masters/mm/bo2mp-master");
        assert!(find("master").immutable);
        assert_eq!(find("launcher").local, "client/bo2mp-launcher.exe");
        assert_eq!(find("ca").local, "client/bo2mp-ca.pem");
    }

    #[test]
    fn descriptor_carries_the_channel_and_update_url() {
        let git = Git {
            rev: "abc123".to_string(),
            dirty: true,
        };
        let meta = Meta {
            id: "id16",
            channel: Channel::Prod,
            profile: "play",
            git: &git,
            host: "203.0.113.7",
        };
        let value = descriptor(&meta, vec![json!({"role": "ca"})]);
        assert_eq!(value["master_port"], 4433);
        assert_eq!(value["update_url"], "https://203.0.113.7:8443/prod");
        assert_eq!(value["target"], "x86_64-pc-windows-msvc");
        assert_eq!(value["dirty"], true);
        assert_eq!(value["files"][0]["role"], "ca");
    }

    #[test]
    fn bundle_env_and_host_come_from_the_deploy_host() {
        let env = Env::parse("BO2MP_DEPLOY_HOST=root@203.0.113.7\n");
        let host = public_host(&env).unwrap();
        assert_eq!(host, "203.0.113.7");
        assert_eq!(
            bundle_env(&host, Channel::Prod),
            "BO2MP_UPDATE_URL=https://203.0.113.7:8443/prod\n\
             BO2MP_MASTER_ADDR=203.0.113.7:4433\n\
             BO2MP_MASTER_SERVER_NAME=bo2mp-prod\n\
             BO2MP_MASTER_CA_CERT=bo2mp-ca.pem\n"
        );
    }
}
