//! `cargo xtask release <prod|dev|bundles>`.
//!
//! A channel release is built and staged locally under
//! `dist/releases/<channel>/<id>`: the zstd'd game, the launcher, the CA, the
//! client manifest, the Linux master, and `deployment.json` saying where each
//! file goes. `publish` uploads exactly that. `bundles` makes the two
//! portable Windows zips instead; nothing is uploaded.

use std::fmt::Display;
use std::io::BufRead;
use std::path::{Path, PathBuf};
use std::process::Command;

use master_protocol::{Channel, WIRE_REV};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};

use crate::dot_vars::DotVars;
use crate::host_setup::RELEASES_PORT;
use crate::cmd_run::{RemoteShellLink, Phase, grab, demand_host_programs, launch, set_mode};
use crate::windows;

const ZSTD_LEVEL: u32 = 3;
/// zstd's "one worker per core".
const ZSTD_THREADS: u32 = 0;
/// Not a secret: the zip password only stops scanners unpacking the exe.
const ARCHIVE_PASSWORD: &str = "contextrot";
const ZSTD: &str = "zstd";
const DEPLOYMENT: &str = "deployment.json";
/// The client manifest's place in the stage, and the name it is served as.
const MANIFEST_AT: &str = "client/manifest.json";
const MANIFEST_NAME: &str = "manifest.json";

/// Shipped beside the binaries under their own file names: the licence, the
/// notice, and the licences of the two fonts compiled into the game.
const LEGAL_FILES: [&str; 4] = [
    "LICENSE",
    "NOTICE",
    "crates/ui/assets/OFL-Oxanium.txt",
    "crates/console/assets/COPYING-FreeFont.txt",
];

/// Turns an I/O style error into `"<doing> <what>: <error>"`.
fn fail<'a, E: Display>(doing: &'a str, what: impl Display + 'a) -> impl Fn(E) -> String + 'a {
    move |error| format!("{doing} {what}: {error}")
}

/// A JSON object with its keys in the order given.
fn object<const N: usize>(entries: [(&str, Value); N]) -> Value {
    Value::Object(
        entries
            .into_iter()
            .map(|(knob, val)| (knob.to_owned(), val))
            .collect(),
    )
}

/// `object` for entries whose values are all strings.
fn text_object<const N: usize>(entries: [(&str, &str); N]) -> Value {
    object(entries.map(|(knob, text)| (knob, Value::from(text))))
}

/// A file's hash and length, as the manifest and the descriptor both want them.
struct Hashed {
    hash_hex: String,
    size: u64,
}

impl Hashed {
    fn of(file_at: &Path) -> Result<Self, String> {
        Ok(Self {
            hash_hex: digest_file_hex(file_at)?,
            size: byte_len(file_at)?,
        })
    }
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    hex(Sha256::digest(bytes).as_slice())
}

fn hex(digest: &[u8]) -> String {
    digest
        .iter()
        .flat_map(|byte| [byte >> 4, byte & 0x0f])
        .filter_map(|nibble| char::from_digit(u32::from(nibble), 16))
        .collect()
}

pub fn digest_file_hex(file_at: &Path) -> Result<String, String> {
    let file = std::fs::File::open(file_at).map_err(fail("opening", file_at.display()))?;
    let mut reader = std::io::BufReader::new(file);
    let mut hasher = Sha256::default();
    loop {
        let window = reader.fill_buf().map_err(fail("reading", file_at.display()))?;
        if window.is_empty() {
            break;
        }
        hasher.update(window);
        let used = window.len();
        reader.consume(used);
    }
    Ok(hex(hasher.finalize().as_slice()))
}

pub fn byte_len(file_at: &Path) -> Result<u64, String> {
    let meta = std::fs::metadata(file_at).map_err(fail("stat", file_at.display()))?;
    Ok(meta.len())
}

pub fn write_json(file_at: &Path, val: &Value) -> Result<(), String> {
    let text = serde_json::to_string_pretty(val)
        .map(|body| body + "\n")
        .map_err(fail("encoding", file_at.display()))?;
    std::fs::write(file_at, text).map_err(fail("writing", file_at.display()))
}

fn clone_file(from: &Path, to: &Path) -> Result<(), String> {
    let (src_shown, dst_shown) = (from.display(), to.display());
    let route = format!("{src_shown} to {dst_shown}");
    std::fs::copy(from, to).map_err(fail("copying", route))?;
    Ok(())
}

fn make_dir(file_at: &Path) -> Result<(), String> {
    std::fs::create_dir_all(file_at).map_err(fail("creating", file_at.display()))
}

/// The number in a `v1.5.6,` word, if the word looks like a version.
fn version_number(word: &str) -> Option<&str> {
    word.trim_end_matches(',')
        .strip_prefix('v')
        .filter(|number| number.starts_with(|c: char| c.is_ascii_digit()))
}

/// The `v1.5.6` in `*** Zstandard CLI (64-bit) v1.5.6, by Yann Collet ***`.
pub fn parse_zstd_version(text: &str) -> Option<String> {
    text.split_whitespace()
        .find_map(version_number)
        .map(str::to_owned)
}

/// Where players reach the release host: the host half of
/// `BO2MP_DEPLOY_HOST`.
pub fn advertised_hostname(dot_conf: &DotVars) -> Result<String, String> {
    let dest = dot_conf.must_get("BO2MP_DEPLOY_HOST")?;
    Ok(RemoteShellLink::new(&dest)?.host_name().to_owned())
}

pub fn updates_base(host_name: &str, svc: Channel) -> String {
    format!("https://{host_name}:{RELEASES_PORT}/{svc}")
}

/// The three lines that point a game at its channel's master.
fn master_settings(host_name: &str, svc: Channel) -> [(&'static str, String); 3] {
    [
        ("BO2MP_MASTER_ADDR", format!("{host_name}:{}", svc.port())),
        ("BO2MP_MASTER_SERVER_NAME", svc.tls_name().to_string()),
        ("BO2MP_MASTER_CA_CERT", "bo2mp-ca.pem".to_owned()),
    ]
}

/// What `git` says about the checkout: the short revision, and whatever
/// `status --porcelain` printed.
struct Git {
    rev: String,
    status: String,
}

impl Git {
    fn dirty(&self) -> bool {
        !self.status.is_empty()
    }
}

fn read_git(repo_root: &Path) -> Result<Git, String> {
    let ask = |cli_words: &[&str]| {
        grab(Command::new("git").arg("-C").arg(repo_root).args(cli_words))
            .map(|printed| printed.trim().to_owned())
    };
    Ok(Git {
        rev: ask(&["rev-parse", "--short=12", "HEAD"])?,
        status: ask(&["status", "--porcelain"])?,
    })
}

/// Empties `path` by deleting it and making it again.
fn reset_dir(file_at: &Path) -> Result<(), String> {
    drop(std::fs::remove_dir_all(file_at));
    make_dir(file_at)
}

/// A directory next to the releases that vanishes unless it is kept, so a
/// failed build never leaves something that looks like a release.
struct StageDir {
    base_dir: Option<PathBuf>,
}

impl StageDir {
    fn open_in(parent: &Path, label: &str) -> Result<Self, String> {
        let base_dir = parent.join(format!(".stage-{label}-{}", std::process::id()));
        reset_dir(&base_dir)?;
        Ok(Self { base_dir: Some(base_dir) })
    }

    fn file_at(&self) -> &Path {
        self.base_dir.as_deref().expect("a kept scratch has no path")
    }

    /// Lets the directory outlive this guard.
    fn keep(&mut self) {
        self.base_dir = None;
    }
}

impl Drop for StageDir {
    fn drop(&mut self) {
        if let Some(base_dir) = self.base_dir.take() {
            let _ = std::fs::remove_dir_all(base_dir);
        }
    }
}

/// The master is static, so one binary runs on any Linux host.
pub fn compile_lobby_server(repo_root: &Path, flavour: &str) -> Result<PathBuf, String> {
    let inherited = std::env::var("RUSTFLAGS").unwrap_or_default();
    let flags = [inherited.as_str(), "-C target-feature=+crt-static"].join(" ");
    let mut cargo = Command::new("cargo");
    cargo
        .args(["build", "--locked", "--profile", flavour])
        .args(["--target", windows::LINUX_TARGET, "-p", "bo2mp-master"])
        .arg("--message-format=json-render-diagnostics")
        .env("RUSTFLAGS", flags)
        .current_dir(repo_root);
    grab(&mut cargo).and_then(|messages| windows::built_bin(&messages, "bo2mp-master"))
}

/// The packaged game: its stage file name, and the hashes of the exe and of
/// the compressed blob the launcher downloads.
struct PackedGame {
    stage_leaf: String,
    exe: Hashed,
    zst_blob: Hashed,
}

fn installed_zstd_version() -> Result<String, String> {
    let banner = grab(Command::new(ZSTD).arg("--version"))?;
    parse_zstd_version(&banner).ok_or_else(|| "cannot read the zstd version".to_owned())
}

fn compress(exe: &Path, dest_file: &Path) -> Result<(), String> {
    let mut zstd = Command::new(ZSTD);
    zstd.args(["-q", "-f"])
        .args([format!("-{ZSTD_LEVEL}"), format!("-T{ZSTD_THREADS}")])
        .arg("-o")
        .arg(dest_file)
        .arg(exe);
    launch(&mut zstd)
}

/// Where the compressed `exe` sits in `dist/cache/zstd`; a new file is only
/// made when the exe, the zstd build, the level or the thread count change.
fn cached_zstd(repo_root: &Path, exe: &Path, exe_sha: &str) -> Result<PathBuf, String> {
    let version = installed_zstd_version()?;
    let cache = repo_root.join("dist/cache/zstd");
    make_dir(&cache)?;
    let reused = cache.join(format!(
        "{exe_sha}.zstd-{version}.{ZSTD_LEVEL}.T{ZSTD_THREADS}.zst"
    ));
    if !reused.is_file() {
        let step = Phase::start("package.game", &format!("zstd {version} exe={exe_sha}"));
        let partial = reused.with_extension("part");
        compress(exe, &partial)?;
        std::fs::rename(&partial, &reused).map_err(fail("renaming", partial.display()))?;
        step.wrap("");
    }
    Ok(reused)
}

/// Puts the cached blob into `into` under its content name, checking an
/// earlier copy rather than overwriting it.
fn place_blob(reused: &Path, blob_sha: &str, into: &Path) -> Result<PathBuf, String> {
    let write_target = into.join(format!("bo2mp-{blob_sha}.exe.zst"));
    if !write_target.is_file() {
        clone_file(reused, &write_target)?;
        return Ok(write_target);
    }
    if digest_file_hex(&write_target)? == blob_sha {
        Ok(write_target)
    } else {
        Err(format!("{} differs from the cached blob", write_target.display()))
    }
}

/// Packs the game for the stage, reusing the cached compressed copy, so a
/// release of an unchanged game never calls zstd.
fn pack_game(repo_root: &Path, exe: &Path, into: &Path) -> Result<PackedGame, String> {
    demand_host_programs(&[ZSTD])?;
    let exe_hash = Hashed::of(exe)?;
    let reused = cached_zstd(repo_root, exe, &exe_hash.hash_hex)?;
    let blob_sha = digest_file_hex(&reused)?;
    let write_target = place_blob(&reused, &blob_sha, into)?;
    Ok(PackedGame {
        stage_leaf: match write_target.file_name() {
            Some(leaf) => leaf.to_string_lossy().into_owned(),
            None => String::new(),
        },
        zst_blob: Hashed {
            size: byte_len(&write_target)?,
            hash_hex: blob_sha,
        },
        exe: exe_hash,
    })
}

/// One `files` row of the client manifest; `download` adds the compressed
/// copy's name and hashes.
fn manifest_entry(file_at: &str, content: &Hashed, download: Option<(&str, &Hashed)>) -> Value {
    let mut row_by_kind = Map::new();
    let mut put = |knob: &str, val: Value| {
        row_by_kind.insert(knob.to_owned(), val);
    };
    put("path", file_at.into());
    put("sha256", content.hash_hex.as_str().into());
    put("size", content.size.into());
    if let Some((title, packed)) = download {
        put("download", title.into());
        put("download_sha256", packed.hash_hex.as_str().into());
        put("download_size", packed.size.into());
    }
    Value::Object(row_by_kind)
}

/// What the launcher fetches as `manifest.json`: the files that make a
/// working install, and the `.env` lines that point the game at the master.
fn client_manifest(meta: &ReleaseFacts, zst_blob: &PackedGame, ca: &Hashed) -> Value {
    let env_lines = master_settings(meta.advertised_hostname, meta.svc)
        .into_iter()
        .map(|(knob, line)| (knob.to_owned(), Value::from(line)))
        .collect();
    let files = vec![
        manifest_entry("bo2mp.exe", &zst_blob.exe, Some((&zst_blob.stage_leaf, &zst_blob.zst_blob))),
        manifest_entry("bo2mp-ca.pem", ca, None),
    ];
    object([
        ("id", meta.release.into()),
        ("channel", meta.svc.as_str().into()),
        ("protocol", WIRE_REV.into()),
        ("git", meta.source.rev.as_str().into()),
        ("env", Value::Object(env_lines)),
        ("files", files.into()),
    ])
}

/// The first 16 hex digits of a hash over everything that makes one release
/// differ from another, so an identical rebuild lands on the same id.
fn release_id(identity: &Value) -> String {
    sha256_hex(identity.to_string().as_bytes())[..16].to_owned()
}

/// Whether `publish` may overwrite the file already at its remote path.
#[derive(Clone, Copy, PartialEq)]
enum Keep {
    Replaceable,
    Immutable,
}

/// One staged file and where `publish` puts it. `remote` is under
/// `releases/<channel>/` except for the master, which is under the deploy root.
struct Shipped {
    slot_kind: &'static str,
    /// Path inside the stage.
    staged_at: String,
    /// Path under the remote root.
    remote_at: String,
    keep: Keep,
    live: Option<&'static str>,
}

impl Shipped {
    fn new(
        slot_kind: &'static str,
        staged_at: impl Into<String>,
        remote_at: impl Into<String>,
        keep: Keep,
    ) -> Self {
        Self {
            slot_kind,
            staged_at: staged_at.into(),
            remote_at: remote_at.into(),
            keep,
            live: None,
        }
    }

    /// Also published under this name at the live location.
    fn live_as(self, live_name: &'static str) -> Self {
        Self {
            live: Some(live_name),
            ..self
        }
    }

    fn to_json(&self, stage_dir: &Path) -> Result<Value, String> {
        let Hashed { hash_hex, size } = Hashed::of(&stage_dir.join(&self.staged_at))?;
        let mut row_by_kind = object([
            ("role", self.slot_kind.into()),
            ("local", self.staged_at.as_str().into()),
            ("remote", self.remote_at.as_str().into()),
            ("sha256", hash_hex.into()),
            ("size", size.into()),
            ("immutable", (self.keep == Keep::Immutable).into()),
        ]);
        if let (Some(title), Some(fields)) = (self.live, row_by_kind.as_object_mut()) {
            fields.insert("live".to_owned(), title.into());
        }
        Ok(row_by_kind)
    }
}

fn shipped(id: &str, blob_name: &str, master_sha: &str) -> Vec<Shipped> {
    use Keep::{Immutable, Replaceable};
    vec![
        Shipped::new("game-blob", format!("client/{blob_name}"), blob_name, Immutable),
        Shipped::new(
            "manifest",
            MANIFEST_AT,
            format!("manifests/{id}.json"),
            Replaceable,
        )
        .live_as(MANIFEST_NAME),
        Shipped::new(
            "launcher",
            "client/bo2mp-launcher.exe",
            "bo2mp-launcher.exe",
            Replaceable,
        ),
        Shipped::new("ca", "client/bo2mp-ca.pem", "bo2mp-ca.pem", Replaceable),
        Shipped::new(
            "master",
            "master/bo2mp-master",
            format!("masters/{master_sha}/bo2mp-master"),
            Immutable,
        ),
    ]
}

struct ReleaseFacts<'a> {
    release: &'a str,
    svc: Channel,
    assemble: &'a str,
    source: &'a Git,
    advertised_hostname: &'a str,
}

impl ReleaseFacts<'_> {
    /// The body of `deployment.json`, `files` being the `Shipped` rows.
    fn to_json(&self, files: Vec<Value>) -> Value {
        object([
            ("id", self.release.into()),
            ("channel", self.svc.as_str().into()),
            ("profile", self.assemble.into()),
            ("target", windows::TARGET.into()),
            ("git", self.source.rev.as_str().into()),
            ("dirty", self.source.dirty().into()),
            ("protocol", WIRE_REV.into()),
            ("master_port", self.svc.port().into()),
            ("update_url", updates_base(self.advertised_hostname, self.svc).into()),
            ("files", files.into()),
        ])
    }
}

/// What went into the stage, which is all the release id is made of.
struct Staged {
    zst_blob: PackedGame,
    master: Hashed,
    ca: Hashed,
    launcher: Hashed,
}

impl Staged {
    fn identity(&self, svc: Channel, flavour: &str, git: &Git) -> Value {
        let files = text_object([
            ("ca", self.ca.hash_hex.as_str()),
            ("game_blob", self.zst_blob.zst_blob.hash_hex.as_str()),
            ("game_exe", self.zst_blob.exe.hash_hex.as_str()),
            ("launcher", self.launcher.hash_hex.as_str()),
        ]);
        object([
            ("channel", svc.as_str().into()),
            ("dirty", git.dirty().into()),
            ("git", git.rev.as_str().into()),
            ("master_sha256", self.master.hash_hex.as_str().into()),
            ("profile", flavour.into()),
            ("protocol", WIRE_REV.into()),
            ("target", windows::TARGET.into()),
            ("files", files),
        ])
    }
}

/// Copies the launcher, CA, master and the packaged game into `stage`.
fn fill_stage(
    stage_dir: &Path,
    repo_root: &Path,
    ca_src: &Path,
    master_src: &Path,
    exes: &windows::WinReleaseBinaries,
) -> Result<Staged, String> {
    let client = stage_dir.join("client");
    let master_at = stage_dir.join("master/bo2mp-master");
    for base_dir in [client.clone(), stage_dir.join("master")] {
        make_dir(&base_dir)?;
    }
    let zst_blob = pack_game(repo_root, &exes.game_exe, &client)?;
    let place = |from: &Path, to: &Path| clone_file(from, to).and_then(|()| Hashed::of(to));
    let (launcher, ca) = (
        place(&exes.launcher_exe, &client.join("bo2mp-launcher.exe"))?,
        place(ca_src, &client.join("bo2mp-ca.pem"))?,
    );
    let master = place(master_src, &master_at)?;
    set_mode(&master_at, 0o755)?;
    Ok(Staged {
        zst_blob,
        master,
        ca,
        launcher,
    })
}

/// Writes `deployment.json` into the guarded stage, then swaps the stage in
/// as `write_target` and lets it outlive the guard.
fn seal_release(guard: &mut StageDir, write_target: &Path, meta: &ReleaseFacts, staged: &Staged) -> Result<(), String> {
    let stage_dir = guard.file_at().to_path_buf();
    let row_list = shipped(meta.release, &staged.zst_blob.stage_leaf, &staged.master.hash_hex);
    let files = row_list.iter().map(|row_by_kind| row_by_kind.to_json(&stage_dir)).collect::<Result<Vec<_>, String>>()?;
    write_json(&stage_dir.join(DEPLOYMENT), &meta.to_json(files))?;
    drop(std::fs::remove_dir_all(write_target));
    std::fs::rename(&stage_dir, write_target).map_err(fail("moving the stage to", write_target.display()))?;
    guard.keep();
    Ok(())
}

/// What both release commands build before they stage anything: the host
/// players reach, the CA, and the Windows binaries.
struct Toolchain {
    reach: String,
    ca_pem: PathBuf,
    binaries: windows::WinReleaseBinaries,
}

impl Toolchain {
    fn load(dot_conf: &DotVars, flavour: &str) -> Result<Self, String> {
        let checked = windows::demand_build_profile(flavour).and_then(|()| advertised_hostname(dot_conf));
        let (reach, ca_pem) = (checked?, windows::issuer_cert_path(dot_conf)?);
        let binaries = windows::assemble(flavour, &ca_pem)?;
        Ok(Self { reach, ca_pem, binaries })
    }
}

pub fn stage_release(repo_root: &Path, dot_conf: &DotVars, svc: Channel, flavour: &str) -> Result<PathBuf, String> {
    let tools = Toolchain::load(dot_conf, flavour)?;
    let git = read_git(repo_root)?;
    let master = compile_lobby_server(repo_root, flavour)?;

    let release_root: PathBuf = [repo_root, Path::new("dist/releases"), Path::new(svc.as_str())]
        .iter()
        .collect();
    make_dir(&release_root)?;
    let mut guard = StageDir::open_in(&release_root, svc.as_str())?;
    let stage_dir = guard.file_at().to_path_buf();

    let staged = fill_stage(&stage_dir, repo_root, &tools.ca_pem, &master, &tools.binaries)?;
    let id = release_id(&staged.identity(svc, flavour, &git));
    let meta = ReleaseFacts {
        release: &id,
        svc,
        assemble: flavour,
        source: &git,
        advertised_hostname: &tools.reach,
    };
    write_json(
        &stage_dir.join(MANIFEST_AT),
        &client_manifest(&meta, &staged.zst_blob, &staged.ca),
    )?;

    let write_target = release_root.join(&id);
    match write_target.join(DEPLOYMENT).is_file() {
        true => println!("release: {} already staged", write_target.display()),
        false => seal_release(&mut guard, &write_target, &meta, &staged)?,
    }
    std::fs::write(release_root.join("LATEST"), format!("{id}\n"))
        .map_err(fail("writing", "LATEST"))?;
    println!("release.done path=dist/releases/{svc}/{id}");
    Ok(write_target)
}

/// The `.env` inside a portable zip: where to update from and which master
/// to trust.
pub fn bundle_env(host_name: &str, svc: Channel) -> String {
    let update = ("BO2MP_UPDATE_URL", updates_base(host_name, svc));
    std::iter::once(update)
        .chain(master_settings(host_name, svc))
        .fold(String::new(), |mut text, (knob, val)| {
            text.extend([knob, "=", val.as_str(), "\n"]);
            text
        })
}

/// Everything a portable zip is made from, whichever channel it is for.
struct BundleJob<'a> {
    repo_root: &'a Path,
    tools: &'a Toolchain,
}

impl BundleJob<'_> {
    /// `(source, name in the zip)` for every file but the `.env`.
    fn inputs(&self) -> Vec<(PathBuf, String)> {
        let built = [
            (&self.tools.binaries.launcher_exe, "bo2mp-launcher.exe"),
            (&self.tools.binaries.game_exe, "bo2mp.exe"),
            (&self.tools.ca_pem, "bo2mp-ca.pem"),
        ]
        .map(|(from, title)| (from.clone(), title.to_owned()));
        let legal = LEGAL_FILES.iter().map(|repo_path| {
            let title = repo_path.rsplit('/').next().unwrap_or(repo_path);
            (self.repo_root.join(repo_path), title.to_owned())
        });
        built.into_iter().chain(legal).collect()
    }

    /// One channel's folder, staged under `under` and zipped to `archive`.
    fn zip(&self, svc: Channel, under: &Path, zip_path: &Path) -> Result<(), String> {
        let stage_dir = under.join(svc.as_str());
        make_dir(&stage_dir)?;
        let mut files = Vec::new();
        for (from, title) in self.inputs() {
            let to = stage_dir.join(title);
            clone_file(&from, &to)?;
            files.push(to);
        }
        let env_file = stage_dir.join(".env");
        std::fs::write(&env_file, bundle_env(&self.tools.reach, svc))
            .map_err(fail("writing", format!("{}/.env", stage_dir.display())))?;
        files.push(env_file);
        let names: Vec<String> = files.iter().map(|file| file.display().to_string()).collect();
        crate::zip_pack::pack_zip(zip_path, &names, ARCHIVE_PASSWORD.as_bytes())
    }
}

/// `dist/windows/bo2mp-windows-<channel>.zip` for both channels: a folder
/// that runs offline and updates itself only when asked.
pub fn pack_bundles(repo_root: &Path, dot_conf: &DotVars, flavour: &str) -> Result<(), String> {
    let tools = Toolchain::load(dot_conf, flavour)?;
    let win_dist = repo_root.join("dist/windows");
    reset_dir(&win_dist)?;
    let guard = StageDir::open_in(&win_dist, "zips")?;
    let job = BundleJob {
        repo_root,
        tools: &tools,
    };
    for (svc, zip_path) in Channel::ALL.map(|svc| {
        (svc, win_dist.join(format!("bo2mp-windows-{svc}.zip")))
    }) {
        job.zip(svc, guard.file_at(), &zip_path)?;
        println!("release.bundle {svc}: {}", zip_path.display());
    }
    Ok(())
}

/// What `cargo xtask release` was asked for.
enum Target {
    Bundles,
    Release(Channel),
}

fn parse_target(word: &str) -> Option<Target> {
    if word == "bundles" {
        return Some(Target::Bundles);
    }
    word.parse::<Channel>().ok().map(Target::Release)
}

pub fn dispatch_subcommand(repo_root: &Path, dot_conf: &DotVars, cli_words: &[String]) -> Result<(), String> {
    let usage = "usage: cargo xtask release <prod|dev|bundles>";
    let [what] = cli_words else {
        return Err(String::from(usage));
    };
    let flavour = windows::flavour(dot_conf)?;
    match parse_target(what) {
        Some(Target::Bundles) => pack_bundles(repo_root, dot_conf, &flavour),
        Some(Target::Release(svc)) => stage_release(repo_root, dot_conf, svc, &flavour).map(drop),
        None => Err(format!("{usage}, not {what:?}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn hashed(hash_hex: &str, size: u64) -> Hashed {
        Hashed {
            hash_hex: hash_hex.to_owned(),
            size,
        }
    }

    fn git_at(rev: &str, status: &str) -> Git {
        Git {
            rev: rev.to_owned(),
            status: status.to_owned(),
        }
    }

    fn zst_blob() -> PackedGame {
        PackedGame {
            stage_leaf: "bo2mp-bb.exe.zst".to_owned(),
            exe: hashed("aa", 100),
            zst_blob: hashed("bb", 40),
        }
    }

    /// A fresh empty directory named for the test.
    fn sandbox(git_label: &str) -> PathBuf {
        let base_dir = std::env::temp_dir().join(format!("xtask-release-{git_label}-{}", std::process::id()));
        std::fs::remove_dir_all(&base_dir).ok();
        std::fs::create_dir_all(&base_dir).unwrap();
        base_dir
    }

    /// Every `(json pointer, expected)` pair must hold in `value`.
    fn assert_at(val: &Value, expected: &[(&str, Value)]) {
        for (pointer, hoped) in expected {
            assert_eq!(val.pointer(pointer), Some(hoped), "at {pointer}");
        }
    }

    #[test]
    fn hashes_are_lowercase_sha256() {
        let abc = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";
        let base_dir = sandbox("sha");
        let file_at = base_dir.join("abc");
        std::fs::write(&file_at, b"abc").unwrap();
        assert_eq!(sha256_hex(b"abc"), abc);
        assert_eq!(digest_file_hex(&file_at).unwrap(), abc);
        assert_eq!(byte_len(&file_at).unwrap(), 3);
        let whole = Hashed::of(&file_at).unwrap();
        assert_eq!((whole.hash_hex.as_str(), whole.size), (abc, 3));
        std::fs::remove_dir_all(base_dir).unwrap();
    }

    #[test]
    fn hex_pads_each_nibble() {
        assert_eq!(hex(&[]), "");
        assert_eq!(hex(&[0x00, 0x0a, 0xf0, 0xff]), "000af0ff");
    }

    #[test]
    fn file_helpers_name_the_path_when_they_fail() {
        let missing = sandbox("missing").join("nope");
        for error in [
            digest_file_hex(&missing).unwrap_err(),
            byte_len(&missing).unwrap_err(),
            clone_file(&missing, &missing).unwrap_err(),
        ] {
            assert!(error.contains("nope"), "{error}");
        }
        assert!(digest_file_hex(&missing).unwrap_err().starts_with("opening "));
        assert!(byte_len(&missing).unwrap_err().starts_with("stat "));
    }

    #[test]
    fn zstd_version_comes_from_the_banner() {
        let banner = "*** Zstandard CLI (64-bit) v1.5.6, by Yann Collet ***";
        for (text, hoped) in [
            (banner, Some("1.5.6")),
            ("zstd command line interface\n", None),
            ("v-next v2", Some("2")),
        ] {
            assert_eq!(parse_zstd_version(text).as_deref(), hoped, "{text}");
        }
    }

    #[test]
    fn manifest_points_the_game_at_the_channel_master() {
        let git = git_at("abc123", "");
        let meta = ReleaseFacts {
            release: "id16",
            svc: Channel::PreReleaseBuild,
            assemble: "play",
            source: &git,
            advertised_hostname: "203.0.113.7",
        };
        let manifest = client_manifest(&meta, &zst_blob(), &hashed("cc", 9));
        assert_at(
            &manifest,
            &[
                ("/channel", json!("dev")),
                ("/protocol", json!(WIRE_REV)),
                ("/env/BO2MP_MASTER_ADDR", json!("203.0.113.7:4434")),
                ("/env/BO2MP_MASTER_SERVER_NAME", json!("bo2mp-dev")),
                ("/env/BO2MP_MASTER_CA_CERT", json!("bo2mp-ca.pem")),
                ("/files/0/path", json!("bo2mp.exe")),
                ("/files/0/sha256", json!("aa")),
                ("/files/0/size", json!(100)),
                ("/files/0/download", json!("bo2mp-bb.exe.zst")),
                ("/files/0/download_sha256", json!("bb")),
                ("/files/0/download_size", json!(40)),
                (
                    "/files/1",
                    json!({"path": "bo2mp-ca.pem", "sha256": "cc", "size": 9}),
                ),
            ],
        );
        assert_eq!(manifest["files"].as_array().map(Vec::len), Some(2));
    }

    #[test]
    fn manifest_entry_has_download_fields_only_when_given() {
        let key_count = |entry: Value| entry.as_object().map_or(0, Map::len);
        let plain = manifest_entry("x", &hashed("s", 1), None);
        assert_eq!(key_count(plain), 3);
        let packed = manifest_entry("x", &hashed("s", 1), Some(("d", &hashed("t", 2))));
        assert_eq!(key_count(packed), 6);
    }

    #[test]
    fn release_id_is_sixteen_hex_and_follows_the_inputs() {
        let with_ca = |ca: &str| release_id(&json!({"git": "a", "files": {"ca": ca}}));
        let one = with_ca("1");
        assert_eq!(one.len(), 16);
        assert!(one.chars().all(|c| c.is_ascii_hexdigit()));
        assert_eq!(one, with_ca("1"));
        assert_ne!(one, with_ca("2"));
    }

    #[test]
    fn identity_changes_with_any_staged_hash_and_the_git_state() {
        let staged = |ca: &str| Staged {
            zst_blob: zst_blob(),
            master: hashed("mm", 1),
            ca: hashed(ca, 2),
            launcher: hashed("ll", 3),
        };
        let (clean, dirty) = (git_at("abc123", ""), git_at("abc123", " M file"));
        let id = |staged: &Staged, git: &Git| release_id(&staged.identity(Channel::Prod, "play", git));
        let base = id(&staged("c1"), &clean);
        assert_eq!(base, id(&staged("c1"), &clean));
        assert_ne!(base, id(&staged("c2"), &clean));
        assert_ne!(base, id(&staged("c1"), &dirty));
        assert!(dirty.dirty() && !clean.dirty());
    }

    #[test]
    fn every_shipped_file_has_a_place() {
        let row_list = shipped("id16", "bo2mp-bb.exe.zst", "mm");
        let find = |slot_kind| row_list.iter().find(|row_by_kind| row_by_kind.slot_kind == slot_kind).unwrap();
        let table = [
            ("game-blob", "bo2mp-bb.exe.zst", Keep::Immutable),
            ("manifest", "manifests/id16.json", Keep::Replaceable),
            ("master", "masters/mm/bo2mp-master", Keep::Immutable),
            ("launcher", "bo2mp-launcher.exe", Keep::Replaceable),
            ("ca", "bo2mp-ca.pem", Keep::Replaceable),
        ];
        for (slot_kind, far_end, keep) in table {
            assert!(find(slot_kind).remote_at == far_end && find(slot_kind).keep == keep, "{slot_kind}");
        }
        assert_eq!(find("launcher").staged_at, "client/bo2mp-launcher.exe");
        assert_eq!(find("ca").staged_at, "client/bo2mp-ca.pem");
        assert_eq!(row_list.iter().filter(|row_by_kind| row_by_kind.live.is_some()).count(), 1);
    }

    #[test]
    fn shipped_rows_hash_the_staged_file_and_only_the_manifest_is_live() {
        let stage_dir = sandbox("rows");
        std::fs::create_dir_all(stage_dir.join("client")).unwrap();
        std::fs::write(stage_dir.join(MANIFEST_AT), b"abc").unwrap();
        std::fs::write(stage_dir.join("client/bo2mp-ca.pem"), b"abc").unwrap();
        let row_list = shipped("id16", "blob.zst", "mm");
        let row_for = |slot_kind| row_list.iter().find(|row_by_kind| row_by_kind.slot_kind == slot_kind).unwrap();
        let manifest = row_for("manifest").to_json(&stage_dir).unwrap();
        assert_at(
            &manifest,
            &[
                ("/role", json!("manifest")),
                ("/size", json!(3)),
                ("/immutable", json!(false)),
                ("/live", json!(MANIFEST_NAME)),
                (
                    "/sha256",
                    json!("ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"),
                ),
            ],
        );
        let ca = row_for("ca").to_json(&stage_dir).unwrap();
        assert!(ca.get("live").is_none());
        assert!(row_for("master").to_json(&stage_dir).is_err());
        std::fs::remove_dir_all(stage_dir).unwrap();
    }

    #[test]
    fn descriptor_carries_the_channel_and_update_url() {
        let git = git_at("abc123", "?? new");
        let meta = ReleaseFacts {
            release: "id16",
            svc: Channel::Prod,
            assemble: "play",
            source: &git,
            advertised_hostname: "203.0.113.7",
        };
        let val = meta.to_json(vec![json!({"role": "ca"})]);
        assert_at(
            &val,
            &[
                ("/master_port", json!(4433)),
                ("/update_url", json!("https://203.0.113.7:8443/prod")),
                ("/target", json!("x86_64-pc-windows-msvc")),
                ("/dirty", json!(true)),
                ("/files/0/role", json!("ca")),
            ],
        );
    }

    #[test]
    fn bundle_env_and_host_come_from_the_deploy_host() {
        let dot_conf = DotVars::parse("BO2MP_DEPLOY_HOST=root@203.0.113.7\n");
        let host_name = advertised_hostname(&dot_conf).unwrap();
        assert_eq!(host_name, "203.0.113.7");
        assert_eq!(
            bundle_env(&host_name, Channel::Prod),
            "BO2MP_UPDATE_URL=https://203.0.113.7:8443/prod\n\
             BO2MP_MASTER_ADDR=203.0.113.7:4433\n\
             BO2MP_MASTER_SERVER_NAME=bo2mp-prod\n\
             BO2MP_MASTER_CA_CERT=bo2mp-ca.pem\n"
        );
    }

    #[test]
    fn manifest_env_and_bundle_env_share_the_master_lines() {
        let git = git_at("g", "");
        let meta = ReleaseFacts {
            release: "i",
            svc: Channel::PreReleaseBuild,
            assemble: "play",
            source: &git,
            advertised_hostname: "h",
        };
        let manifest = client_manifest(&meta, &zst_blob(), &hashed("c", 1));
        let text = bundle_env("h", Channel::PreReleaseBuild);
        for (knob, line) in master_settings("h", Channel::PreReleaseBuild) {
            assert_eq!(manifest["env"][knob], line.as_str());
            assert!(text.contains(&format!("{knob}={line}\n")), "{knob}");
        }
    }

    #[test]
    fn a_placed_blob_is_reused_when_it_matches_and_refused_when_not() {
        let base_dir = sandbox("blob");
        let reused = base_dir.join("cached.zst");
        std::fs::write(&reused, b"abc").unwrap();
        let hash_hex = digest_file_hex(&reused).unwrap();
        let into = base_dir.join("client");
        std::fs::create_dir_all(&into).unwrap();

        let placed = place_blob(&reused, &hash_hex, &into).unwrap();
        assert_eq!(placed, into.join(format!("bo2mp-{hash_hex}.exe.zst")));
        assert_eq!(std::fs::read(&placed).unwrap(), b"abc");
        assert_eq!(place_blob(&reused, &hash_hex, &into).unwrap(), placed);

        std::fs::write(&placed, b"tampered").unwrap();
        let error = place_blob(&reused, &hash_hex, &into).unwrap_err();
        assert!(error.ends_with("differs from the cached blob"), "{error}");
        std::fs::remove_dir_all(base_dir).unwrap();
    }

    #[test]
    fn scratch_vanishes_unless_kept() {
        let parent = sandbox("scratch");
        let dropped = {
            let scratch = StageDir::open_in(&parent, "a").unwrap();
            std::fs::write(scratch.file_at().join("f"), b"x").unwrap();
            scratch.file_at().to_path_buf()
        };
        assert!(!dropped.exists());
        let kept = {
            let mut scratch = StageDir::open_in(&parent, "b").unwrap();
            scratch.keep();
            parent.join(format!(".stage-b-{}", std::process::id()))
        };
        assert!(kept.is_dir());
        std::fs::remove_dir_all(parent).unwrap();
    }

    #[test]
    fn the_command_line_takes_exactly_one_known_word() {
        assert!(matches!(parse_target("bundles"), Some(Target::Bundles)));
        assert!(matches!(
            parse_target("dev"),
            Some(Target::Release(Channel::PreReleaseBuild))
        ));
        assert!(parse_target("staging").is_none());
        let dot_conf = DotVars::parse("");
        for cli_words in [vec![], vec!["dev".to_owned(), "prod".to_owned()]] {
            let error = dispatch_subcommand(Path::new("."), &dot_conf, &cli_words).unwrap_err();
            assert_eq!(error, "usage: cargo xtask release <prod|dev|bundles>");
        }
    }
}
