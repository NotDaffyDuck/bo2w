//! `cargo xtask publish <prod|dev>`: put a prepared release live.
//!
//! The order is the point. Files go to a staging directory and are checked,
//! then moved into place; the channel's master is switched and has to answer
//! with the release's protocol; only then does the client manifest change.
//! A master that does not come up is rolled back before any launcher can see
//! the new manifest.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use master_protocol::{Channel, WIRE_REV};
use serde_json::Value;

use crate::dot_vars::DotVars;
use crate::lobby_host::{DEFAULT_SINCE, validate_log_window, journal};
use crate::host_setup::{RELEASES_PORT, master_link};
use crate::release::{digest_file_hex, updates_base};
use crate::cmd_run::{RemoteShellLink, Phase, validate_install_dir, demand_host_programs, launch};

const HEALTH_SECS: u64 = 15;
const POLL_EVERY: Duration = Duration::from_secs(1);
/// What `HASH_EACH_SCRIPT` prints for a path that is not a file.
const ABSENT: &str = "MISSING";
const NEEDED_TOOLS: [&str; 3] = ["curl", "rsync", "ssh"];
const NO_DETAIL: &str = "";
const REMOTE_CA: &str = "/etc/bo2mp/bo2mp-ca.pem";
const PUBLISH_USAGE: &str = "usage: cargo xtask publish <prod|dev>";
const LOGS_USAGE: &str = "usage: cargo xtask logs <prod|dev> [--since 2h]";
/// Roles whose files are run on the host, not served.
const EXECUTABLE_ROLES: [&str; 1] = ["master"];
/// Interface name prefixes that mean the traffic goes through a tunnel.
const TUNNEL_PREFIXES: [&str; 4] = ["tun", "wg", "tap", "ppp"];

/// Reads absolute paths on stdin, prints `path<TAB>sha256` or
/// `path<TAB>MISSING` for each.
const HASH_EACH_SCRIPT: &str = r#"while IFS= read -r path; do
  sum=MISSING
  [ -f "$path" ] && sum="$(sha256sum < "$path" | cut -d' ' -f1)"
  printf '%s\t%s\n' "$path" "$sum"
done"#;

/// Reads `staged<TAB>final<TAB>mode` lines and moves each file into place.
const MOVE_INTO_PLACE_SCRIPT: &str = r#"tab="$(printf '\t')"
while IFS="$tab" read -r staged final mode; do
  mkdir -p "$(dirname "$final")" && mv -f "$staged" "$final" && chmod "$mode" "$final" || exit 1
done"#;

/// The one-line remote commands this tool sends, a builder each, so the
/// exact text of every command sits in one place.
mod sh {
    use super::REMOTE_CA;

    pub fn locks_parent(repo_root: &str) -> String {
        format!("install -d -m0755 '{repo_root}/locks'")
    }

    pub fn take_lock(base_dir: &str) -> String {
        format!("mkdir '{base_dir}'")
    }

    pub fn drop_lock(base_dir: &str) -> String {
        format!("rmdir '{base_dir}' 2>/dev/null || true")
    }

    pub fn prepare_dirs(staging: &str, release_root: &str, repo_root: &str) -> String {
        format!("install -d -m0755 '{staging}' '{release_root}/manifests' '{repo_root}/bin'")
    }

    pub fn status(link: &str, port: u16, server_name: &str) -> String {
        format!(
            "'{link}' status --connect 127.0.0.1:{port} --server-name '{server_name}' --ca-cert '{REMOTE_CA}'"
        )
    }

    pub fn main_pid(unit: &str) -> String {
        format!("systemctl show -p MainPID --value '{unit}'")
    }

    pub fn restart(unit: &str) -> String {
        format!("systemctl restart '{unit}'")
    }

    pub fn point_at(hash_hex: &str, link: &str, unit: &str) -> String {
        let swap = format!("ln -sfn '../masters/{hash_hex}/bo2mp-master' '{link}.new'");
        let rename = format!("mv -T '{link}.new' '{link}'");
        [swap, rename, restart(unit)].join(" && ")
    }

    pub fn link_sha(link: &str) -> String {
        format!("if [ -e '{link}' ]; then sha256sum < '{link}' | cut -d' ' -f1; fi")
    }

    pub fn keep_copy(link: &str, keep_dir: &str) -> String {
        format!(
            "[ -e '{keep_dir}/bo2mp-master' ] || {{ install -d '{keep_dir}' && cp -L '{link}' '{keep_dir}/bo2mp-master'; }}"
        )
    }

    pub fn executable(file_at: &str) -> String {
        format!("test -x '{file_at}'")
    }

    pub fn publish_manifest(staged: &str, versioned: &str, live: &str) -> String {
        let steps = [
            format!("cp '{staged}' '{versioned}'"),
            format!("cp '{staged}' '{live}.new'"),
            format!("mv -T '{live}.new' '{live}'"),
        ];
        steps.join(" && ")
    }

    pub fn remove(base_dir: &str) -> String {
        format!("rm -rf '{base_dir}'")
    }
}

/// One file of `deployment.json`. `remote` is relative to the deploy root.
#[derive(Debug, PartialEq)]
pub struct Row {
    pub slot_kind: String,
    pub near: String,
    pub far_end: String,
    pub sha256: String,
    pub size: u64,
    pub write_once: bool,
}

impl Row {
    fn has_role(&self, slot_kind: &str) -> bool {
        self.slot_kind == slot_kind
    }

    fn mode(&self) -> &'static str {
        match EXECUTABLE_ROLES.iter().any(|slot_kind| self.has_role(slot_kind)) {
            true => "755",
            false => "644",
        }
    }

    /// Where the file lands on the host.
    fn placed(&self, repo_root: &str) -> String {
        format!("{repo_root}/{}", self.far_end)
    }

    /// Where the file waits in staging, named after its role.
    fn staged(&self, staging: &str) -> String {
        format!("{staging}/{}", self.slot_kind)
    }
}

/// Remote paths go into single-quoted shell words, so they stay plain.
fn plain_relative(file_at: &str) -> bool {
    // An empty part covers a leading, doubled or trailing slash and the empty path.
    file_at.split('/').all(|partial| {
        let tidy = |c: char| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-');
        !partial.is_empty() && partial != ".." && partial.chars().all(tidy)
    })
}

fn text<'a>(val: &'a Value, knob: &str) -> Result<&'a str, String> {
    val
        .get(knob)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("deployment.json: {knob} is not text"))
}

/// Everything but the master lives under the channel's release tree.
fn remote_path(slot_kind: &str, listed: &str, svc: Channel) -> String {
    if slot_kind == "master" {
        return listed.to_owned();
    }
    format!("releases/{svc}/{listed}")
}

fn size_of(file: &Value) -> Result<u64, String> {
    file["size"]
        .as_u64()
        .ok_or_else(|| "deployment.json: size is not a number".to_owned())
}

/// One entry of the descriptor's `files`, its remote path put under the
/// channel's release tree unless it is the master.
fn row_of(file: &Value, svc: Channel) -> Result<Row, String> {
    let slot_kind = text(file, "role")?;
    let far_end = remote_path(slot_kind, text(file, "remote")?, svc);
    if !plain_relative(&far_end) {
        return Err(format!("deployment.json: odd remote path {far_end:?}"));
    }
    let [near, sha256] = ["local", "sha256"].map(|knob| text(file, knob));
    let (near, sha256) = (near?, sha256?);
    let size = size_of(file)?;
    let (slot_kind, near, sha256) = (slot_kind.to_owned(), near.to_owned(), sha256.to_owned());
    let frozen = file["immutable"].as_bool().unwrap_or(false);
    Ok(Row { slot_kind, near, far_end, sha256, size, write_once: frozen })
}

/// The descriptor's files, with every non-master path moved under the
/// channel's release tree.
pub fn rows_from(recipe: &Value, svc: Channel) -> Result<Vec<Row>, String> {
    recipe["files"]
        .as_array()
        .ok_or_else(|| "deployment.json has no files".to_owned())?
        .iter()
        .map(|file| row_of(file, svc))
        .collect()
}

/// The descriptor belongs to this channel and this build of the protocol.
pub fn check_descriptor(recipe: &Value, dir_name: &str, svc: Channel) -> Result<(), String> {
    let expectations = [
        ("id", Value::from(dir_name)),
        ("channel", Value::from(svc.as_str())),
        ("protocol", Value::from(WIRE_REV)),
        ("master_port", Value::from(svc.port())),
    ];
    let wrong = expectations
        .iter()
        .map(|(knob, expected)| (knob, expected, &recipe[*knob]))
        .find(|(_, expected, actual)| actual != expected);
    wrong.map_or(Ok(()), |(knob, _, actual)| {
        Err(format!(
            "deployment.json {knob} is {actual} but this is {dir_name} on {svc} with protocol {WIRE_REV}"
        ))
    })
}

pub fn parse_inventory(listing: &str) -> BTreeMap<String, String> {
    listing
        .lines()
        .flat_map(|line| line.split_once('\t'))
        .fold(BTreeMap::new(), |mut found, (file_at, hash_hex)| {
            found.insert(file_at.to_owned(), hash_hex.trim().to_owned());
            found
        })
}

fn inventory(shell_link: &RemoteShellLink, paths: &[String]) -> Result<BTreeMap<String, String>, String> {
    let request = format!("{}\n", paths.join("\n"));
    let listing = shell_link.pour(HASH_EACH_SCRIPT, &request)?;
    Ok(parse_inventory(&listing))
}

/// Whether `row` must be sent. Already in place means no; an immutable path
/// holding other bytes is an error.
fn needs_upload(row_by_kind: &Row, repo_root: &str, in_place: &BTreeMap<String, String>) -> Result<bool, String> {
    let file_at = row_by_kind.placed(repo_root);
    let there = in_place.get(&file_at).map_or(ABSENT, String::as_str);
    if there == row_by_kind.sha256 {
        return Ok(false);
    }
    let collides = row_by_kind.write_once && there != ABSENT;
    match collides {
        true => Err(format!(
            "immutable collision at {file_at}: {there} there, {} here",
            row_by_kind.sha256
        )),
        false => Ok(true),
    }
}

/// Which rows need uploading. A row already in place is skipped; an immutable
/// path holding other bytes means two releases disagree, and stops the run.
pub fn plan<'a>(
    row_list: &'a [Row],
    repo_root: &str,
    in_place: &BTreeMap<String, String>,
) -> Result<Vec<&'a Row>, String> {
    let verdicts = row_list
        .iter()
        .filter(|row_by_kind| !row_by_kind.has_role("manifest"))
        .map(|row_by_kind| needs_upload(row_by_kind, repo_root, in_place).map(|wanted| (row_by_kind, wanted)))
        .collect::<Result<Vec<_>, String>>()?;
    Ok(verdicts.into_iter().filter_map(|(row_by_kind, wanted)| wanted.then_some(row_by_kind)).collect())
}

pub fn promote_lines(row_list: &[&Row], repo_root: &str, staging: &str) -> String {
    row_list.iter()
        .map(|row_by_kind| format!("{}\t{}\t{}\n", row_by_kind.staged(staging), row_by_kind.placed(repo_root), row_by_kind.mode()))
        .collect()
}

/// The `protocol=N` a master's `status` prints.
pub fn parse_protocol(status: &str) -> Option<u16> {
    status
        .split(char::is_whitespace)
        .find_map(|word| word.strip_prefix("protocol="))
        .and_then(|val| val.trim_end_matches(',').parse().ok())
}

/// `RELEASE=` (absolute, or relative to the repo), else the channel's LATEST.
pub fn release_folder(repo_root: &Path, release: Option<String>, svc: Channel) -> Result<PathBuf, String> {
    // Joining an absolute path onto `root` yields that path itself.
    if let Some(given) = release {
        return Ok(repo_root.join(given));
    }
    let release_root = repo_root.join("dist/releases").join(svc.as_str());
    let marker = release_root.join("LATEST");
    let newest = std::fs::read_to_string(&marker).ok();
    newest
        .map(|id| release_root.join(id.trim()))
        .ok_or_else(|| no_release_given(&marker, svc))
}

fn no_release_given(marker: &Path, svc: Channel) -> String {
    let shown = marker.display();
    format!("no RELEASE= and no {shown}; run `cargo xtask release {svc}` first")
}

/// One publish per channel at a time: `mkdir` either makes the lock or fails.
struct ChannelLock<'a> {
    host_name: &'a RemoteShellLink,
    unlock: String,
}

impl<'a> ChannelLock<'a> {
    fn take(shell_link: &'a RemoteShellLink, repo_root: &str, svc: Channel) -> Result<Self, String> {
        shell_link.launch(&sh::locks_parent(repo_root))?;
        let base_dir = format!("{repo_root}/locks/{svc}");
        if !shell_link.try_launch(&sh::take_lock(&base_dir))? {
            return Err(format!(
                "publish: channel {svc} already being published (or {base_dir} was left behind)"
            ));
        }
        let unlock = sh::drop_lock(&base_dir);
        Ok(Self { host_name: shell_link, unlock })
    }
}

impl Drop for ChannelLock<'_> {
    fn drop(&mut self) {
        let _ = self.host_name.try_launch(&self.unlock);
    }
}

/// The channel's master runs whatever `bin/bo2mp-master-<channel>` points at.
struct MasterControl<'a> {
    far_end: &'a RemoteShellLink,
    repo_root: &'a str,
    svc: Channel,
    link: String,
    unit: String,
}

impl<'a> MasterControl<'a> {
    fn new(shell_link: &'a RemoteShellLink, repo_root: &'a str, svc: Channel) -> Self {
        let (link, unit) = (master_link(repo_root, svc), svc.systemd_unit().to_owned());
        Self { far_end: shell_link, repo_root, svc, link, unit }
    }

    /// The `status` call a player's client would make, from the host itself.
    fn status_script(&self) -> String {
        sh::status(&self.link, self.svc.port(), self.svc.tls_name())
    }

    fn probe(&self) -> Result<Option<u16>, String> {
        let said = self.far_end.grab(&format!("{} 2>&1 || true", self.status_script()))?;
        Ok(parse_protocol(&said))
    }

    fn main_pid(&self) -> Result<String, String> {
        let pid = self.far_end.grab(&sh::main_pid(&self.unit))?;
        Ok(pid.trim().to_owned())
    }

    fn restart(&self) -> Result<(), String> {
        self.far_end.launch(&sh::restart(&self.unit))
    }

    fn relink(&self, hash_hex: &str) -> Result<(), String> {
        self.far_end.launch(&sh::point_at(hash_hex, &self.link, &self.unit))
    }

    fn wait_until_up(&self) -> Result<Option<u16>, String> {
        let give_up = Instant::now() + Duration::from_secs(HEALTH_SECS);
        loop {
            let answered = self.probe()?;
            if answered.is_some() || Instant::now() >= give_up {
                return Ok(answered);
            }
            std::thread::sleep(POLL_EVERY);
        }
    }

    /// The sha256 of the binary the link points at, empty when there is none.
    fn running_sha(&self) -> Result<String, String> {
        let said = self.far_end.grab(&sh::link_sha(&self.link))?;
        Ok(said.trim().to_owned())
    }

    /// Keeps a copy of the binary being replaced, unless one is already kept.
    fn keep_previous(&self, current: &str) -> Result<(), String> {
        match current {
            "" => Ok(()),
            hash_hex => {
                let keep_dir = format!("{}/masters/{hash_hex}", self.repo_root);
                self.far_end.launch(&sh::keep_copy(&self.link, &keep_dir))
            }
        }
    }

    /// The master is already the wanted one: it only has to answer.
    fn confirm_unchanged(&self, pid: &str) -> Result<u16, String> {
        if let Some(protocol) = self.probe()? {
            println!("activate.master: unchanged pid={pid}");
            return Ok(protocol);
        }
        self.restart()?;
        self.wait_until_up()?
            .ok_or_else(|| "publish: the unchanged master does not answer".to_string())
    }

    /// Points the link back at the master that ran before, when there was one.
    fn restore(&self, previous: &str) -> Result<(), String> {
        if previous.is_empty() {
            return Ok(());
        }
        self.relink(previous)?;
        println!("activate.master: restored {previous}");
        Ok(())
    }

    /// The new master did not answer: put the old one back and fail.
    fn roll_back(&self, previous: &str) -> Result<u16, String> {
        println!(
            "activate.master: no answer in {}s",
            HEALTH_SECS
        );
        self.restore(previous)?;
        Err("publish: master activation failed; client manifest not switched".to_string())
    }

    /// The link moves to `new_sha`; the old binary is kept first.
    fn replace(&self, current: &str, new_sha: &str) -> Result<Option<u16>, String> {
        self.keep_previous(current)?;
        let wanted = format!("{}/masters/{new_sha}/bo2mp-master", self.repo_root);
        self.far_end.launch(&sh::executable(&wanted))?;
        self.relink(new_sha)?;
        self.wait_until_up()
    }

    /// Switch to `new_sha` and wait for it to answer, or go back to the
    /// master that ran before.
    fn activate(&self, new_sha: &str) -> Result<u16, String> {
        let old_pid = self.main_pid()?;
        let current = self.running_sha()?;
        if current == new_sha {
            return self.confirm_unchanged(&old_pid);
        }

        let step = Phase::start("activate.master", &format!("old={current} new={new_sha}"));
        match self.replace(&current, new_sha)? {
            None => self.roll_back(&current),
            Some(protocol) => {
                step.wrap(&format!("pid={} protocol={protocol}", self.main_pid()?));
                println!("activate.master: sessions on the old master dropped; clients reconnect");
                Ok(protocol)
            }
        }
    }
}

/// What it takes to ask the release master a question over UDP from here.
struct Reach<'a> {
    master: &'a Path,
    host_name: &'a str,
    svc: Channel,
    ca: &'a Path,
}

impl Reach<'_> {
    /// Whether `master status` over UDP to the release host works from here.
    fn udp_answers(&self) -> bool {
        let address = format!("{}:{}", self.host_name, self.svc.port());
        let mut probe = Command::new("timeout");
        let title = self.svc.tls_name();
        probe.arg("5").arg(self.master);
        probe.args(["status", "--connect", &address, "--server-name", title]);
        probe.arg("--ca-cert").arg(self.ca);
        probe.status().is_ok_and(|status| status.success())
    }

    fn cannot_reach(&self) -> String {
        let (port, host_name) = (self.svc.port(), self.host_name);
        format!("reachability: this machine cannot reach udp/{port} on {host_name}")
    }

    /// From this machine, as a player would: UDP to the master. A VPN often
    /// carries no UDP, so a failure says which interface the traffic took.
    fn check(&self, required: bool) -> Result<(), String> {
        if !cfg!(unix) {
            println!("reachability: skipped; the release master only runs on Linux");
            return Ok(());
        }
        if self.udp_answers() {
            return Ok(());
        }
        let why = self.cannot_reach();
        println!("{why}");
        explain_route(self.host_name);
        if required {
            return Err(
                "publish: BO2MP_DEPLOY_REQUIRE_LOCAL_UDP=1 and the master is unreachable from here"
                    .to_owned(),
            );
        }
        Ok(())
    }
}

/// The lines that say which interface `route` (output of `ip route get`)
/// sends traffic through.
fn route_notes(route: &str) -> Vec<String> {
    let Some(dev) = egress_dev(route) else {
        return Vec::new();
    };
    let mut notes = vec![format!("reachability: traffic leaves through {dev}")];
    notes.extend(is_tunnel(dev).then(|| {
        format!("reachability: {dev} looks like a VPN; players outside it may still connect")
    }));
    notes
}

/// Prints which interface traffic to `host` leaves through, when `ip` says.
fn explain_route(host_name: &str) {
    let lookup = format!("ip -o route get \"$(getent ahostsv4 '{host_name}' | head -n1 | cut -d' ' -f1)\"");
    if let Ok(probe) = Command::new("sh").args(["-c", &lookup]).output() {
        let route = String::from_utf8_lossy(&probe.stdout);
        route_notes(&route).iter().for_each(|note| println!("{note}"));
    }
}

pub fn egress_dev(route: &str) -> Option<&str> {
    route.split_whitespace().skip_while(|word| *word != "dev").nth(1)
}

pub fn is_tunnel(dev: &str) -> bool {
    TUNNEL_PREFIXES.iter().any(|prefix| dev.starts_with(prefix))
}

fn load_json(file_at: &Path) -> Result<Value, String> {
    let shown = file_at.display();
    let body = std::fs::read_to_string(file_at).map_err(|error| format!("reading {shown}: {error}"))?;
    serde_json::from_str(&body).map_err(|error| format!("{shown}: {error}"))
}

/// A prepared release that has been checked against its own descriptor.
struct Release {
    folder: PathBuf,
    id: String,
    row_list: Vec<Row>,
}

impl Release {
    fn row_by_kind(&self, slot_kind: &str) -> &Row {
        self.row_list
            .iter()
            .find(|row_by_kind| row_by_kind.has_role(slot_kind))
            .expect("load_release only returns releases with a manifest and a master")
    }

    fn manifest(&self) -> &Row {
        self.row_by_kind("manifest")
    }

    fn master(&self) -> &Row {
        self.row_by_kind("master")
    }

    fn near(&self, row_by_kind: &Row) -> PathBuf {
        self.folder.join(&row_by_kind.near)
    }
}

fn row_with<'a>(row_list: &'a [Row], slot_kind: &str) -> Option<&'a Row> {
    row_list.iter().find(|row_by_kind| row_by_kind.has_role(slot_kind))
}

/// The directory's name is the release id.
fn release_id(base_dir: &Path) -> Result<String, String> {
    base_dir.file_name()
        .and_then(std::ffi::OsStr::to_str)
        .map(String::from)
        .ok_or_else(|| "release directory has no name".to_owned())
}

/// Every staged file still has the bytes the descriptor lists.
fn check_files_unchanged(base_dir: &Path, row_list: &[Row]) -> Result<(), String> {
    row_list.iter().try_for_each(|row_by_kind| {
        match digest_file_hex(&base_dir.join(&row_by_kind.near))? == row_by_kind.sha256 {
            true => Ok(()),
            false => Err(format!("{} changed since the release was made", row_by_kind.near)),
        }
    })
}

/// The client manifest names the same protocol and git revision as the
/// descriptor.
fn check_manifest_agrees(base_dir: &Path, row_list: &[Row], recipe: &Value) -> Result<(), String> {
    let manifest_row = row_with(row_list, "manifest").ok_or("deployment.json has no manifest")?;
    let manifest = load_json(&base_dir.join(&manifest_row.near))?;
    let agree = ["protocol", "git"]
        .iter()
        .all(|knob| manifest[*knob] == recipe[*knob]);
    match agree {
        true => Ok(()),
        false => Err("client manifest and deployment.json disagree on protocol or git".to_owned()),
    }
}

/// The release directory's `deployment.json`, parsed.
fn descriptor_in(base_dir: &Path) -> Result<Value, String> {
    let file_at = base_dir.join("deployment.json");
    match file_at.is_file() {
        true => load_json(&file_at),
        false => Err(format!("{} is not a prepared release", base_dir.display())),
    }
}

/// Reads `deployment.json` from the chosen release directory and proves the
/// staged files still are what it says.
fn load_release(repo_root: &Path, dot_conf: &DotVars, svc: Channel) -> Result<Release, String> {
    let base_dir = release_folder(repo_root, dot_conf.value_of("RELEASE"), svc)?;
    let recipe = descriptor_in(&base_dir)?;
    let id = release_id(&base_dir)?;
    check_descriptor(&recipe, &id, svc)?;
    let row_list = rows_from(&recipe, svc)?;
    check_files_unchanged(&base_dir, &row_list)?;
    check_manifest_agrees(&base_dir, &row_list, &recipe)?;
    row_with(&row_list, "master").ok_or("deployment.json has no master")?;
    Ok(Release { folder: base_dir, id, row_list })
}

/// The host-side directories one publish works in.
struct Dirs<'a> {
    repo_root: &'a str,
    staging: String,
    release_root: String,
}

impl<'a> Dirs<'a> {
    fn new(repo_root: &'a str, svc: Channel, id: &str) -> Self {
        Self {
            repo_root,
            staging: format!("{repo_root}/staging/{svc}/{id}"),
            release_root: format!("{repo_root}/releases/{svc}"),
        }
    }

    fn live_manifest(&self) -> String {
        format!("{}/manifest.json", self.release_root)
    }

    fn staged_manifest(&self) -> String {
        format!("{}/manifest.json", self.staging)
    }
}

/// The `upload` step's detail: what is sent, what was already there, and how
/// long the inventory took. The manifest is never counted as skipped.
fn upload_summary(total: usize, upload: &[&Row], inventory_secs: u64) -> String {
    let bytes: u64 = upload.iter().map(|row_by_kind| row_by_kind.size).sum();
    let skipped = total.saturating_sub(1 + upload.len());
    format!(
        "files={} bytes={bytes} skipped={skipped} inventory={inventory_secs}s",
        upload.len()
    )
}

/// The first staged path whose landed hash is not its row's.
fn first_mismatch<'a>(
    upload: &[&Row],
    staged: &'a [String],
    landed: &BTreeMap<String, String>,
) -> Option<&'a String> {
    upload
        .iter()
        .zip(staged)
        .find(|(row_by_kind, file_at)| landed.get(*file_at) != Some(&row_by_kind.sha256))
        .map(|(_, file_at)| file_at)
}

/// Sends what is missing into staging, proves it arrived intact, and moves
/// it to its final place.
fn upload_files(shell_link: &RemoteShellLink, release: &Release, dirs: &Dirs) -> Result<(), String> {
    let began = Instant::now();
    let finals: Vec<String> = release.row_list.iter().map(|row_by_kind| row_by_kind.placed(dirs.repo_root)).collect();
    let upload = plan(&release.row_list, dirs.repo_root, &inventory(shell_link, &finals)?)?;
    let note_text = upload_summary(release.row_list.len(), &upload, began.elapsed().as_secs());
    let step = Phase::start("upload", &note_text);
    let staged: Vec<String> = upload.iter().map(|row_by_kind| row_by_kind.staged(&dirs.staging)).collect();
    upload.iter().zip(&staged).try_for_each(|(row_by_kind, dest)| {
        let flags = ["--info=progress2", "--stats", "--chmod=F644"];
        shell_link.mirror_up(&flags, &release.near(row_by_kind), dest)
    })?;
    let landed = inventory(shell_link, &staged)?;
    if let Some(file_at) = first_mismatch(&upload, &staged, &landed) {
        return Err(format!("staged sha256 mismatch for {file_at}"));
    }
    step.wrap(NO_DETAIL);
    match upload.is_empty() {
        true => Ok(()),
        false => shell_link
            .pour(MOVE_INTO_PLACE_SCRIPT, &promote_lines(&upload, dirs.repo_root, &dirs.staging))
            .map(drop),
    }
}

/// Makes the release's manifest the live one, unless it already is.
fn go_live(shell_link: &RemoteShellLink, release: &Release, dirs: &Dirs) -> Result<(), String> {
    let live = dirs.live_manifest();
    let row_by_kind = release.manifest();
    let on_host = inventory(shell_link, &[live.clone()])?;
    if on_host.get(&live) == Some(&row_by_kind.sha256) {
        println!("activate.manifest: unchanged");
        return Ok(());
    }
    let staged = dirs.staged_manifest();
    shell_link.mirror_up(&["--chmod=F644"], &release.near(row_by_kind), &staged)?;
    shell_link.launch(&sh::publish_manifest(&staged, &row_by_kind.placed(dirs.repo_root), &live))?;
    println!("activate.manifest: live release={}", release.id);
    Ok(())
}

/// The `curl` that fetches `url` into `out`, trusting only the release CA.
fn curl_fetch(url: &str, ca: &Path, dest_file: &Path) -> Command {
    let mut curl = Command::new("curl");
    curl.args(["--fail", "--show-error", "--silent"])
        .args(["--connect-timeout", "3", "--max-time", "10"])
        .arg("--cacert")
        .arg(ca)
        .arg("-o")
        .arg(dest_file)
        .arg(url);
    curl
}

/// Downloads the live manifest over HTTPS, trusting only the release CA, and
/// checks it is the one just published.
fn verify_served(url: &str, svc: Channel, ca: &Path, expected: &str) -> Result<(), String> {
    let fetched = std::env::temp_dir().join(format!("bo2mp-manifest-{svc}-{}", std::process::id()));
    let served = launch(&mut curl_fetch(url, ca, &fetched)).and_then(|()| digest_file_hex(&fetched));
    let _ = std::fs::remove_file(&fetched);
    match served? == expected {
        true => Ok(()),
        false => Err(format!("{url} is not the manifest just published")),
    }
}

/// The master that answers speaks the protocol this tool was built with.
fn check_running_protocol(running: u16) -> Result<(), String> {
    match running == WIRE_REV {
        true => Ok(()),
        false => Err(format!(
            "publish: running master protocol {running} != client {WIRE_REV}"
        )),
    }
}

fn healthy_line(svc: Channel, host_name: &str) -> String {
    let (udp, https) = (svc.port(), RELEASES_PORT);
    format!("publish: {svc} healthy: master udp/{udp}, releases https://{host_name}:{https}/{svc}/")
}

/// The deploy host and its checked root directory, once the tools exist.
fn deploy_target(dot_conf: &DotVars, tools: &[&str]) -> Result<(RemoteShellLink, String), String> {
    demand_host_programs(tools)?;
    let shell_link = RemoteShellLink::new(&dot_conf.must_get("BO2MP_DEPLOY_HOST")?)?;
    let repo_root = dot_conf.must_get("BO2MP_DEPLOY_ROOT")?;
    validate_install_dir(&repo_root)?;
    Ok((shell_link, repo_root))
}

pub fn ship(repo_root: &Path, dot_conf: &DotVars, svc: Channel) -> Result<(), String> {
    let (shell_link, deploy_root) = deploy_target(dot_conf, &NEEDED_TOOLS)?;

    let release = load_release(repo_root, dot_conf, svc)?;
    let dirs = Dirs::new(&deploy_root, svc, &release.id);
    let _lock = ChannelLock::take(&shell_link, dirs.repo_root, svc)?;
    shell_link.launch(&sh::prepare_dirs(&dirs.staging, &dirs.release_root, dirs.repo_root))?;

    upload_files(&shell_link, &release, &dirs)?;

    let switch = MasterControl::new(&shell_link, dirs.repo_root, svc);
    check_running_protocol(switch.activate(&release.master().sha256)?)?;

    go_live(&shell_link, &release, &dirs)?;

    let host_name = shell_link.host_name().to_string();
    let ca = release.folder.join("client/bo2mp-ca.pem");
    let url = format!("{}/manifest.json", updates_base(&host_name, svc));
    verify_served(&url, svc, &ca, &release.manifest().sha256)?;
    println!("verify.manifest: ok release={}", release.id);

    shell_link.launch(&switch.status_script())?;
    let require_udp = dot_conf.value_of("BO2MP_DEPLOY_REQUIRE_LOCAL_UDP").is_some_and(|flag| flag == "1");
    let master = release.near(release.master());
    Reach { master: &master, host_name: &host_name, svc, ca: &ca }.check(require_udp)?;
    let ok_line = healthy_line(svc, &host_name);
    println!("{ok_line}");
    shell_link.launch(&sh::remove(&dirs.staging))
}

fn channel_arg(arg: Option<&String>, usage: &str) -> Result<Channel, String> {
    let word = arg.ok_or_else(|| usage.to_owned())?;
    word.parse().map_err(|_| usage.to_owned())
}

pub fn dispatch_subcommand(repo_root: &Path, dot_conf: &DotVars, cli_words: &[String]) -> Result<(), String> {
    let [only] = cli_words else {
        return Err(PUBLISH_USAGE.to_owned());
    };
    ship(repo_root, dot_conf, channel_arg(Some(only), PUBLISH_USAGE)?)
}

/// The `--since` window: the flag if given, else `SINCE=`, else the default.
fn since_window(dot_conf: &DotVars, flags: &[String]) -> Result<String, String> {
    match flags {
        [] => Ok(dot_conf.value_of("SINCE").unwrap_or_else(|| DEFAULT_SINCE.to_owned())),
        [flag, given] if flag == "--since" => Ok(given.clone()),
        _ => Err(LOGS_USAGE.to_owned()),
    }
}

/// `cargo xtask logs <prod|dev> [--since 2h]`, or `SINCE=` from make.
pub fn tail_logs(dot_conf: &DotVars, cli_words: &[String]) -> Result<(), String> {
    let svc = channel_arg(cli_words.first(), LOGS_USAGE)?;
    let from_time = since_window(dot_conf, &cli_words[1..])?;
    validate_log_window(&from_time).and_then(|()| demand_host_programs(&["ssh"]))?;
    journal(&RemoteShellLink::new(&dot_conf.must_get("BO2MP_DEPLOY_HOST")?)?, svc, &from_time)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    const ID: &str = "0123456789abcdef";
    const ROOT: &str = "/srv/bo2mp";

    fn file(slot_kind: &str, near: &str, far_end: &str, hash_hex: &str, size: u64, frozen: bool) -> Value {
        json!({"role": slot_kind, "local": near, "remote": far_end, "sha256": hash_hex, "size": size, "immutable": frozen})
    }

    fn recipe() -> Value {
        let manifest = format!("manifests/{ID}.json");
        json!({
            "id": ID,
            "channel": "prod",
            "protocol": WIRE_REV,
            "master_port": 4433,
            "git": "abc",
            "files": [
                file("game-blob", "client/bo2mp-b.exe.zst", "bo2mp-b.exe.zst", "b1", 40, true),
                file("manifest", "client/manifest.json", &manifest, "m1", 5, false),
                file("launcher", "client/bo2mp-launcher.exe", "bo2mp-launcher.exe", "l1", 30, false),
                file("master", "master/bo2mp-master", "masters/s1/bo2mp-master", "s1", 70, true),
            ],
        })
    }

    fn prod_rows() -> Vec<Row> {
        rows_from(&recipe(), Channel::Prod).unwrap()
    }

    fn roles<'a>(row_list: &[&'a Row]) -> Vec<&'a str> {
        row_list.iter().map(|row_by_kind| row_by_kind.slot_kind.as_str()).collect()
    }

    fn remote_with(file_at: &str) -> Value {
        let mut val = recipe();
        val["files"][2]["remote"] = json!(file_at);
        val
    }

    #[test]
    fn client_files_land_under_the_channel_and_the_master_does_not() {
        let row_list = prod_rows();
        let remotes: Vec<&str> = row_list.iter().map(|row_by_kind| row_by_kind.far_end.as_str()).collect();
        assert_eq!(
            remotes,
            [
                "releases/prod/bo2mp-b.exe.zst",
                "releases/prod/manifests/0123456789abcdef.json",
                "releases/prod/bo2mp-launcher.exe",
                "masters/s1/bo2mp-master",
            ]
        );
        let flags: Vec<bool> = row_list.iter().map(|row_by_kind| row_by_kind.write_once).collect();
        assert_eq!(flags, [true, false, false, true]);
        let modes: Vec<&str> = row_list.iter().map(Row::mode).collect();
        assert_eq!(modes, ["644", "644", "644", "755"]);
    }

    #[test]
    fn odd_remote_paths_are_refused() {
        for bad in ["../etc/passwd", "a//b", "it's", "/abs", ""] {
            for svc in [Channel::Prod, Channel::PreReleaseBuild] {
                let verdict = rows_from(&remote_with(bad), svc);
                assert!(verdict.is_err(), "{bad} accepted on {svc}");
            }
        }
        assert!(rows_from(&remote_with("sub/dir-1_x.bin"), Channel::Prod).is_ok());
    }

    #[test]
    fn plain_paths_are_split_by_part() {
        let verdicts = [
            ("a/b.c-d_e", true),
            ("a/../b", false),
            ("a/", false),
            ("a b", false),
            ("..", false),
            (".hidden", true),
        ];
        for (file_at, hoped) in verdicts {
            assert_eq!(plain_relative(file_at), hoped, "{file_at:?}");
        }
    }

    #[test]
    fn descriptor_must_match_dir_channel_and_protocol() {
        let val = recipe();
        let accepts = |v: &Value, id: &str, ch: Channel| check_descriptor(v, id, ch).is_ok();
        assert!(accepts(&val, ID, Channel::Prod));
        assert!(!accepts(&val, "fedcba9876543210", Channel::Prod));
        assert!(!accepts(&val, ID, Channel::PreReleaseBuild));
        let mut old = val.clone();
        old["protocol"] = json!(WIRE_REV - 1);
        assert!(!accepts(&old, ID, Channel::Prod));
        let message = check_descriptor(&old, ID, Channel::Prod).unwrap_err();
        let lead = "deployment.json protocol is ";
        std::assert_eq!(message.get(..lead.len()), Some(lead), "error: {message}");
    }

    #[test]
    fn plan_skips_what_is_there_and_stops_on_immutable_collisions() {
        let row_list = prod_rows();
        let mut there = parse_inventory(
            "/srv/bo2mp/releases/prod/bo2mp-b.exe.zst\tb1\n\
             /srv/bo2mp/releases/prod/bo2mp-launcher.exe\told\n\
             /srv/bo2mp/masters/s1/bo2mp-master\tMISSING\n",
        );
        let wanted = plan(&row_list, ROOT, &there).unwrap();
        assert_eq!(
            roles(&wanted),
            ["launcher", "master"],
            "manifest goes live separately"
        );

        there.insert(format!("{ROOT}/masters/s1/bo2mp-master"), "other".to_string());
        let error = plan(&row_list, ROOT, &there).unwrap_err();
        assert!(error.starts_with("immutable collision at /srv/bo2mp/masters/s1/bo2mp-master"));
    }

    #[test]
    fn needs_upload_decides_per_row() {
        let row_list = prod_rows();
        let (zst_blob, launcher) = (&row_list[0], &row_list[2]);
        let held = |file_at: &str, hash_hex: &str| BTreeMap::from([(file_at.to_owned(), hash_hex.to_owned())]);
        let blob_path = format!("{ROOT}/{}", zst_blob.far_end);
        let launcher_path = format!("{ROOT}/{}", launcher.far_end);
        assert_eq!(needs_upload(zst_blob, ROOT, &BTreeMap::new()), Ok(true));
        assert_eq!(needs_upload(zst_blob, ROOT, &held(&blob_path, "b1")), Ok(false));
        assert!(needs_upload(zst_blob, ROOT, &held(&blob_path, "zz")).is_err());
        assert_eq!(needs_upload(launcher, ROOT, &held(&launcher_path, "zz")), Ok(true));
    }

    #[test]
    fn promote_lines_are_staged_final_mode() {
        let row_list = prod_rows();
        let picked = [&row_list[2], &row_list[3]];
        let expected = [
            "/srv/bo2mp/staging/prod/id/launcher\t/srv/bo2mp/releases/prod/bo2mp-launcher.exe\t644",
            "/srv/bo2mp/staging/prod/id/master\t/srv/bo2mp/masters/s1/bo2mp-master\t755",
        ]
        .map(|line| format!("{line}\n"))
        .concat();
        assert_eq!(promote_lines(&picked, ROOT, "/srv/bo2mp/staging/prod/id"), expected);
        assert_eq!(promote_lines(&[], ROOT, "/x"), "");
    }

    #[test]
    fn upload_report_counts_and_staged_check() {
        let row_list = prod_rows();
        let sent = [&row_list[2], &row_list[3]];
        assert_eq!(
            upload_summary(row_list.len(), &sent, 3),
            "files=2 bytes=100 skipped=1 inventory=3s"
        );
        assert_eq!(upload_summary(1, &[], 0), "files=0 bytes=0 skipped=0 inventory=0s");

        let staged: Vec<String> = sent.iter().map(|row_by_kind| row_by_kind.staged("/stg")).collect();
        let mut landed = BTreeMap::from([
            ("/stg/launcher".to_owned(), "l1".to_owned()),
            ("/stg/master".to_owned(), "s1".to_owned()),
        ]);
        assert_eq!(first_mismatch(&sent, &staged, &landed), None);
        landed.insert("/stg/master".to_owned(), "bad".to_owned());
        assert_eq!(first_mismatch(&sent, &staged, &landed), Some(&staged[1]));
        landed.remove("/stg/launcher");
        assert_eq!(first_mismatch(&sent, &staged, &landed), Some(&staged[0]));
    }

    #[test]
    fn protocol_is_read_from_master_status() {
        let cases = [
            ("master ok protocol=9 sessions=3", Some(9)),
            ("ok\nprotocol=10,\n", Some(10)),
            ("connection refused", None),
            ("protocol=x", None),
        ];
        for (said, hoped) in cases {
            assert_eq!(parse_protocol(said), hoped, "{said:?}");
        }
        assert!(check_running_protocol(WIRE_REV).is_ok());
        assert!(check_running_protocol(WIRE_REV + 1).is_err());
    }

    #[test]
    fn release_dir_prefers_release_then_latest() {
        let repo_root = std::env::temp_dir().join(format!("xtask-publish-{}", std::process::id()));
        let release_root = repo_root.join("dist/releases/dev");
        std::fs::create_dir_all(&release_root).unwrap();
        let pick = |given: Option<String>| release_folder(&repo_root, given, Channel::PreReleaseBuild);
        assert!(pick(None).unwrap_err().contains("cargo xtask release dev"));
        std::fs::write(release_root.join("LATEST"), "abc\n").unwrap();
        assert_eq!(pick(None).unwrap(), release_root.join("abc"));
        assert_eq!(pick(Some("dist/x".to_string())).unwrap(), repo_root.join("dist/x"));
        let absolute = std::env::temp_dir().join("elsewhere");
        assert_eq!(pick(Some(absolute.display().to_string())).unwrap(), absolute);
        std::fs::remove_dir_all(&repo_root).unwrap();
    }

    #[test]
    fn egress_interface_is_named_and_tunnels_spotted() {
        let route = "203.0.113.7 via 10.8.0.1 dev tun0 src 10.8.0.2 uid 1000 \\    cache";
        assert_eq!(egress_dev(route), Some("tun0"));
        assert_eq!(egress_dev("x dev"), None);
        assert_eq!(egress_dev(""), None);
        for dev in ["tun0", "wg0", "tap1", "ppp0"] {
            assert!(is_tunnel(dev), "{dev}");
        }
        assert!(!is_tunnel("eth0"));
    }

    #[test]
    fn route_notes_name_the_interface_and_flag_tunnels() {
        assert_eq!(
            route_notes("1.2.3.4 dev eth0 src 10.0.0.2"),
            ["reachability: traffic leaves through eth0"]
        );
        let through_vpn = route_notes("1.2.3.4 dev wg0 src 10.0.0.2");
        assert_eq!(through_vpn.len(), 2);
        assert!(through_vpn[1].starts_with("reachability: wg0 looks like a VPN"));
        assert!(route_notes("unreachable").is_empty());
    }

    #[test]
    fn remote_commands_keep_their_exact_text() {
        assert_eq!(sh::locks_parent(ROOT), "install -d -m0755 '/srv/bo2mp/locks'");
        assert_eq!(sh::drop_lock("/l"), "rmdir '/l' 2>/dev/null || true");
        assert_eq!(
            sh::prepare_dirs("/s", "/r", ROOT),
            "install -d -m0755 '/s' '/r/manifests' '/srv/bo2mp/bin'"
        );
        assert_eq!(
            sh::point_at("abc", "/b/link", "u.service"),
            "ln -sfn '../masters/abc/bo2mp-master' '/b/link.new' && mv -T '/b/link.new' '/b/link' && systemctl restart 'u.service'"
        );
        assert_eq!(
            sh::publish_manifest("/s/m", "/r/v", "/r/manifest.json"),
            "cp '/s/m' '/r/v' && cp '/s/m' '/r/manifest.json.new' && mv -T '/r/manifest.json.new' '/r/manifest.json'"
        );
        assert_eq!(
            sh::status("/b/link", 4433, "bo2mp-prod"),
            "'/b/link' status --connect 127.0.0.1:4433 --server-name 'bo2mp-prod' --ca-cert '/etc/bo2mp/bo2mp-ca.pem'"
        );
        assert_eq!(
            sh::keep_copy("/b/link", "/r/masters/old"),
            "[ -e '/r/masters/old/bo2mp-master' ] || { install -d '/r/masters/old' && cp -L '/b/link' '/r/masters/old/bo2mp-master'; }"
        );
        assert_eq!(sh::remove("/s"), "rm -rf '/s'");
    }

    #[test]
    fn host_directories_and_closing_line() {
        let dirs = Dirs::new(ROOT, Channel::PreReleaseBuild, "id1");
        assert_eq!(dirs.staging, "/srv/bo2mp/staging/dev/id1");
        assert_eq!(dirs.live_manifest(), "/srv/bo2mp/releases/dev/manifest.json");
        assert_eq!(dirs.staged_manifest(), "/srv/bo2mp/staging/dev/id1/manifest.json");
        let line = healthy_line(Channel::PreReleaseBuild, "h.example");
        assert!(line.starts_with("publish: dev healthy: master udp/4434, releases https://h.example:"));
        assert!(line.ends_with("/dev/"));
    }
}
