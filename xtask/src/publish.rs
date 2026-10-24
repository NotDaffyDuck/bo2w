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

use master_protocol::{Channel, PROTOCOL_VERSION};
use serde_json::Value;

use crate::dotenv::Env;
use crate::master::{DEFAULT_SINCE, check_since, journal};
use crate::provision::{RELEASES_PORT, master_link};
use crate::release::{file_sha256, update_url};
use crate::shell::{Res, Ssh, Step, check_deploy_root, require_tools, run};

const HEALTH_DEADLINE: Duration = Duration::from_secs(15);
const MISSING: &str = "MISSING";
const REMOTE_CA: &str = "/etc/bo2mp/bo2mp-ca.pem";

/// Reads absolute paths on stdin, prints `path<TAB>sha256` or
/// `path<TAB>MISSING` for each.
const REMOTE_SHA256_LIST: &str = r#"while IFS= read -r path; do
  if [ -f "$path" ]; then
    printf '%s\t%s\n' "$path" "$(sha256sum < "$path" | cut -d' ' -f1)"
  else
    printf '%s\tMISSING\n' "$path"
  fi
done"#;

/// Reads `staged<TAB>final<TAB>mode` lines and moves each file into place.
const REMOTE_PROMOTE: &str = r#"tab="$(printf '\t')"
while IFS="$tab" read -r staged final mode; do
  mkdir -p "$(dirname "$final")" && mv -f "$staged" "$final" && chmod "$mode" "$final" || exit 1
done"#;

/// One file of `deployment.json`. `remote` is relative to the deploy root.
#[derive(Debug, PartialEq)]
pub struct Row {
    pub role: String,
    pub local: String,
    pub remote: String,
    pub sha256: String,
    pub size: u64,
    pub immutable: bool,
}

impl Row {
    fn is_manifest(&self) -> bool {
        self.role == "manifest"
    }

    fn mode(&self) -> &'static str {
        if self.role == "master" { "755" } else { "644" }
    }
}

/// Remote paths go into single-quoted shell words, so they stay plain.
fn plain_relative(path: &str) -> bool {
    !path.is_empty()
        && !path.starts_with('/')
        && path
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "._/-".contains(c))
        && !path.split('/').any(|part| part.is_empty() || part == "..")
}

fn text<'a>(value: &'a Value, key: &str) -> Res<&'a str> {
    value[key]
        .as_str()
        .ok_or_else(|| format!("deployment.json: {key} is not text"))
}

/// The descriptor's files, with every non-master path moved under the
/// channel's release tree.
pub fn rows_of(descriptor: &Value, channel: Channel) -> Res<Vec<Row>> {
    let files = descriptor["files"]
        .as_array()
        .ok_or("deployment.json has no files")?;
    files
        .iter()
        .map(|file| {
            let role = text(file, "role")?.to_string();
            let remote = text(file, "remote")?;
            let remote = if role == "master" {
                remote.to_string()
            } else {
                format!("releases/{channel}/{remote}")
            };
            if !plain_relative(&remote) {
                return Err(format!("deployment.json: odd remote path {remote:?}"));
            }
            Ok(Row {
                role,
                local: text(file, "local")?.to_string(),
                remote,
                sha256: text(file, "sha256")?.to_string(),
                size: file["size"]
                    .as_u64()
                    .ok_or("deployment.json: size is not a number")?,
                immutable: file["immutable"].as_bool().unwrap_or(false),
            })
        })
        .collect()
}

/// The descriptor belongs to this channel and this build of the protocol.
pub fn check_descriptor(descriptor: &Value, dir_name: &str, channel: Channel) -> Res<()> {
    let checks = [
        ("id", descriptor["id"] == dir_name),
        ("channel", descriptor["channel"] == channel.as_str()),
        ("protocol", descriptor["protocol"] == PROTOCOL_VERSION),
        ("master_port", descriptor["master_port"] == channel.port()),
    ];
    match checks.iter().find(|(_, ok)| !ok) {
        Some((key, _)) => Err(format!(
            "deployment.json {key} is {} but this is {dir_name} on {channel} with protocol {PROTOCOL_VERSION}",
            descriptor[*key]
        )),
        None => Ok(()),
    }
}

pub fn parse_inventory(listing: &str) -> BTreeMap<String, String> {
    listing
        .lines()
        .filter_map(|line| line.split_once('\t'))
        .map(|(path, sha)| (path.to_string(), sha.trim().to_string()))
        .collect()
}

fn inventory(ssh: &Ssh, paths: &[String]) -> Res<BTreeMap<String, String>> {
    let listing = ssh.feed(REMOTE_SHA256_LIST, &(paths.join("\n") + "\n"))?;
    Ok(parse_inventory(&listing))
}

/// Which rows need uploading. A row already in place is skipped; an immutable
/// path holding other bytes means two releases disagree, and stops the run.
pub fn plan<'a>(
    rows: &'a [Row],
    root: &str,
    in_place: &BTreeMap<String, String>,
) -> Res<Vec<&'a Row>> {
    let mut upload = Vec::new();
    for row in rows.iter().filter(|row| !row.is_manifest()) {
        let path = format!("{root}/{}", row.remote);
        let there = in_place.get(&path).map_or(MISSING, String::as_str);
        if there == row.sha256 {
            continue;
        }
        if row.immutable && there != MISSING {
            return Err(format!(
                "immutable collision at {path}: {there} there, {} here",
                row.sha256
            ));
        }
        upload.push(row);
    }
    Ok(upload)
}

pub fn promote_lines(rows: &[&Row], root: &str, staging: &str) -> String {
    rows.iter()
        .map(|row| {
            format!(
                "{staging}/{}\t{root}/{}\t{}\n",
                row.role,
                row.remote,
                row.mode()
            )
        })
        .collect()
}

/// The `protocol=N` a master's `status` prints.
pub fn parse_protocol(status: &str) -> Option<u16> {
    status
        .split_whitespace()
        .find_map(|word| word.strip_prefix("protocol="))
        .and_then(|value| value.trim_end_matches(',').parse().ok())
}

/// `RELEASE=` (absolute, or relative to the repo), else the channel's LATEST.
pub fn release_dir(root: &Path, release: Option<String>, channel: Channel) -> Res<PathBuf> {
    if let Some(release) = release {
        let path = PathBuf::from(release);
        return Ok(if path.is_absolute() {
            path
        } else {
            root.join(path)
        });
    }
    let releases = root.join("dist/releases").join(channel.as_str());
    let latest = releases.join("LATEST");
    let id = std::fs::read_to_string(&latest).map_err(|_| {
        format!(
            "no RELEASE= and no {}; run `cargo xtask release {channel}` first",
            latest.display()
        )
    })?;
    Ok(releases.join(id.trim()))
}

/// One publish per channel at a time: `mkdir` either makes the lock or fails.
struct Lock<'a> {
    ssh: &'a Ssh,
    dir: String,
}

impl<'a> Lock<'a> {
    fn take(ssh: &'a Ssh, root: &str, channel: Channel) -> Res<Self> {
        ssh.run(&format!("install -d -m0755 '{root}/locks'"))?;
        let dir = format!("{root}/locks/{channel}");
        if !ssh.try_run(&format!("mkdir '{dir}'"))? {
            return Err(format!(
                "publish: channel {channel} already being published (or {dir} was left behind)"
            ));
        }
        Ok(Self { ssh, dir })
    }
}

impl Drop for Lock<'_> {
    fn drop(&mut self) {
        let _ = self
            .ssh
            .try_run(&format!("rmdir '{}' 2>/dev/null || true", self.dir));
    }
}

/// The channel's master runs whatever `bin/bo2mp-master-<channel>` points at.
struct MasterSwitch<'a> {
    ssh: &'a Ssh,
    root: &'a str,
    channel: Channel,
}

impl MasterSwitch<'_> {
    fn link(&self) -> String {
        master_link(self.root, self.channel)
    }

    fn probe(&self) -> Res<Option<u16>> {
        let status = self.ssh.capture(&format!(
            "'{link}' status --connect 127.0.0.1:{port} --server-name '{name}' --ca-cert '{REMOTE_CA}' 2>&1 || true",
            link = self.link(),
            port = self.channel.port(),
            name = self.channel.server_name(),
        ))?;
        Ok(parse_protocol(&status))
    }

    fn main_pid(&self) -> Res<String> {
        Ok(self
            .ssh
            .capture(&format!(
                "systemctl show -p MainPID --value '{}'",
                self.channel.unit()
            ))?
            .trim()
            .to_string())
    }

    fn point_at(&self, sha: &str) -> Res<()> {
        let link = self.link();
        self.ssh.run(&format!(
            "ln -sfn '../masters/{sha}/bo2mp-master' '{link}.new' && mv -T '{link}.new' '{link}' && systemctl restart '{}'",
            self.channel.unit()
        ))
    }

    fn wait_until_up(&self) -> Res<Option<u16>> {
        let deadline = Instant::now() + HEALTH_DEADLINE;
        loop {
            if let Some(protocol) = self.probe()? {
                return Ok(Some(protocol));
            }
            if Instant::now() >= deadline {
                return Ok(None);
            }
            std::thread::sleep(Duration::from_secs(1));
        }
    }

    /// Switch to `new_sha` and wait for it to answer, or go back to the
    /// master that ran before.
    fn activate(&self, new_sha: &str) -> Res<u16> {
        let link = self.link();
        let old_pid = self.main_pid()?;
        let current = self
            .ssh
            .capture(&format!(
                "if [ -e '{link}' ]; then sha256sum < '{link}' | cut -d' ' -f1; fi"
            ))?
            .trim()
            .to_string();
        if current == new_sha {
            if let Some(protocol) = self.probe()? {
                println!("activate.master: unchanged pid={old_pid}");
                return Ok(protocol);
            }
            self.ssh
                .run(&format!("systemctl restart '{}'", self.channel.unit()))?;
            return self
                .wait_until_up()?
                .ok_or_else(|| "publish: the unchanged master does not answer".to_string());
        }

        let step = Step::start("activate.master", &format!("old={current} new={new_sha}"));
        if !current.is_empty() {
            let keep = format!("{}/masters/{current}", self.root);
            self.ssh.run(&format!(
                "[ -e '{keep}/bo2mp-master' ] || {{ install -d '{keep}' && cp -L '{link}' '{keep}/bo2mp-master'; }}"
            ))?;
        }
        self.ssh.run(&format!(
            "test -x '{}/masters/{new_sha}/bo2mp-master'",
            self.root
        ))?;
        self.point_at(new_sha)?;
        match self.wait_until_up()? {
            Some(protocol) => {
                step.done(&format!("pid={} protocol={protocol}", self.main_pid()?));
                println!("activate.master: sessions on the old master dropped; clients reconnect");
                Ok(protocol)
            }
            None => {
                println!(
                    "activate.master: no answer in {}s",
                    HEALTH_DEADLINE.as_secs()
                );
                if !current.is_empty() {
                    self.point_at(&current)?;
                    println!("activate.master: restored {current}");
                }
                Err("publish: master activation failed; client manifest not switched".to_string())
            }
        }
    }
}

/// From this machine, as a player would: UDP to the master. A VPN often
/// carries no UDP, so a failure says which interface the traffic took.
fn local_reachability(
    master: &Path,
    host: &str,
    channel: Channel,
    ca: &Path,
    required: bool,
) -> Res<()> {
    if !cfg!(unix) {
        println!("reachability: skipped; the release master only runs on Linux");
        return Ok(());
    }
    let ok = Command::new("timeout")
        .arg("5")
        .arg(master)
        .args(["status", "--connect", &format!("{host}:{}", channel.port())])
        .args(["--server-name", channel.server_name(), "--ca-cert"])
        .arg(ca)
        .status()
        .is_ok_and(|status| status.success());
    if ok {
        return Ok(());
    }
    println!(
        "reachability: this machine cannot reach udp/{} on {host}",
        channel.port()
    );
    let route = Command::new("sh")
        .arg("-c")
        .arg(format!(
            "ip -o route get \"$(getent ahostsv4 '{host}' | head -n1 | cut -d' ' -f1)\""
        ))
        .output()
        .ok()
        .map(|out| String::from_utf8_lossy(&out.stdout).into_owned());
    if let Some(dev) = route.as_deref().and_then(egress_dev) {
        println!("reachability: traffic leaves through {dev}");
        if is_tunnel(dev) {
            println!("reachability: {dev} looks like a VPN; players outside it may still connect");
        }
    }
    if required {
        Err(
            "publish: BO2MP_DEPLOY_REQUIRE_LOCAL_UDP=1 and the master is unreachable from here"
                .to_string(),
        )
    } else {
        Ok(())
    }
}

pub fn egress_dev(route: &str) -> Option<&str> {
    let mut words = route.split_whitespace();
    words.find(|word| *word == "dev")?;
    words.next()
}

pub fn is_tunnel(dev: &str) -> bool {
    ["tun", "wg", "tap", "ppp"]
        .iter()
        .any(|prefix| dev.starts_with(prefix))
}

fn read_json(path: &Path) -> Res<Value> {
    let text = std::fs::read_to_string(path)
        .map_err(|error| format!("reading {}: {error}", path.display()))?;
    serde_json::from_str(&text).map_err(|error| format!("{}: {error}", path.display()))
}

pub fn publish(root: &Path, env: &Env, channel: Channel) -> Res<()> {
    require_tools(&["curl", "rsync", "ssh"])?;
    let ssh = Ssh::new(&env.require("BO2MP_DEPLOY_HOST")?)?;
    let deploy_root = env.require("BO2MP_DEPLOY_ROOT")?;
    check_deploy_root(&deploy_root)?;
    let deploy_root = deploy_root.as_str();

    let dir = release_dir(root, env.get("RELEASE"), channel)?;
    let descriptor_path = dir.join("deployment.json");
    if !descriptor_path.is_file() {
        return Err(format!("{} is not a prepared release", dir.display()));
    }
    let descriptor = read_json(&descriptor_path)?;
    let id = dir
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or("release directory has no name")?
        .to_string();
    check_descriptor(&descriptor, &id, channel)?;
    let rows = rows_of(&descriptor, channel)?;
    for row in &rows {
        if file_sha256(&dir.join(&row.local))? != row.sha256 {
            return Err(format!("{} changed since the release was made", row.local));
        }
    }
    let manifest_row = rows
        .iter()
        .find(|row| row.is_manifest())
        .ok_or("deployment.json has no manifest")?;
    let manifest = read_json(&dir.join(&manifest_row.local))?;
    if manifest["protocol"] != descriptor["protocol"] || manifest["git"] != descriptor["git"] {
        return Err("client manifest and deployment.json disagree on protocol or git".to_string());
    }
    let master_row = rows
        .iter()
        .find(|row| row.role == "master")
        .ok_or("deployment.json has no master")?;

    let staging = format!("{deploy_root}/staging/{channel}/{id}");
    let releases = format!("{deploy_root}/releases/{channel}");
    let _lock = Lock::take(&ssh, deploy_root, channel)?;
    ssh.run(&format!(
        "install -d -m0755 '{staging}' '{releases}/manifests' '{deploy_root}/bin'"
    ))?;

    let started = Instant::now();
    let finals: Vec<String> = rows
        .iter()
        .map(|row| format!("{deploy_root}/{}", row.remote))
        .collect();
    let upload = plan(&rows, deploy_root, &inventory(&ssh, &finals)?)?;
    let bytes: u64 = upload.iter().map(|row| row.size).sum();
    let step = Step::start(
        "upload",
        &format!(
            "files={} bytes={bytes} skipped={} inventory={}s",
            upload.len(),
            rows.len() - 1 - upload.len(),
            started.elapsed().as_secs()
        ),
    );
    for row in &upload {
        ssh.rsync(
            &["--info=progress2", "--stats", "--chmod=F644"],
            &dir.join(&row.local),
            &format!("{staging}/{}", row.role),
        )?;
    }
    let staged: Vec<String> = upload
        .iter()
        .map(|row| format!("{staging}/{}", row.role))
        .collect();
    let landed = inventory(&ssh, &staged)?;
    for (row, path) in upload.iter().zip(&staged) {
        if landed.get(path) != Some(&row.sha256) {
            return Err(format!("staged sha256 mismatch for {path}"));
        }
    }
    step.done("");
    if !upload.is_empty() {
        ssh.feed(
            REMOTE_PROMOTE,
            &promote_lines(&upload, deploy_root, &staging),
        )?;
    }

    let switch = MasterSwitch {
        ssh: &ssh,
        root: deploy_root,
        channel,
    };
    let running = switch.activate(&master_row.sha256)?;
    if running != PROTOCOL_VERSION {
        return Err(format!(
            "publish: running master protocol {running} != client {PROTOCOL_VERSION}"
        ));
    }

    let live = format!("{releases}/manifest.json");
    let host = ssh.host().to_string();
    if inventory(&ssh, std::slice::from_ref(&live))?.get(&live) == Some(&manifest_row.sha256) {
        println!("activate.manifest: unchanged");
    } else {
        ssh.rsync(
            &["--chmod=F644"],
            &dir.join(&manifest_row.local),
            &format!("{staging}/manifest.json"),
        )?;
        ssh.run(&format!(
            "cp '{staging}/manifest.json' '{deploy_root}/{versioned}' && \
             cp '{staging}/manifest.json' '{live}.new' && mv -T '{live}.new' '{live}'",
            versioned = manifest_row.remote,
        ))?;
        println!("activate.manifest: live release={id}");
    }

    let ca = dir.join("client/bo2mp-ca.pem");
    let fetched =
        std::env::temp_dir().join(format!("bo2mp-manifest-{channel}-{}", std::process::id()));
    let url = format!("{}/manifest.json", update_url(&host, channel));
    let got = run(Command::new("curl")
        .args([
            "--fail",
            "--show-error",
            "--silent",
            "--connect-timeout",
            "3",
            "--max-time",
            "10",
        ])
        .arg("--cacert")
        .arg(&ca)
        .arg("-o")
        .arg(&fetched)
        .arg(&url))
    .and_then(|()| file_sha256(&fetched));
    let _ = std::fs::remove_file(&fetched);
    if got? != manifest_row.sha256 {
        return Err(format!("{url} is not the manifest just published"));
    }
    println!("verify.manifest: ok release={id}");

    ssh.run(&format!(
        "'{}' status --connect 127.0.0.1:{} --server-name '{}' --ca-cert '{REMOTE_CA}'",
        switch.link(),
        channel.port(),
        channel.server_name()
    ))?;
    local_reachability(
        &dir.join(&master_row.local),
        &host,
        channel,
        &ca,
        env.get("BO2MP_DEPLOY_REQUIRE_LOCAL_UDP").as_deref() == Some("1"),
    )?;
    println!(
        "publish: {channel} healthy: master udp/{}, releases https://{host}:{RELEASES_PORT}/{channel}/",
        channel.port()
    );
    ssh.run(&format!("rm -rf '{staging}'"))
}

fn channel_arg(arg: Option<&String>, usage: &str) -> Res<Channel> {
    arg.ok_or(usage)?.parse().map_err(|_| usage.to_string())
}

pub fn run_cli(root: &Path, env: &Env, args: &[String]) -> Res<()> {
    let usage = "usage: cargo xtask publish <prod|dev>";
    if args.len() != 1 {
        return Err(usage.to_string());
    }
    publish(root, env, channel_arg(args.first(), usage)?)
}

/// `cargo xtask logs <prod|dev> [--since 2h]`, or `SINCE=` from make.
pub fn logs(env: &Env, args: &[String]) -> Res<()> {
    let usage = "usage: cargo xtask logs <prod|dev> [--since 2h]";
    let channel = channel_arg(args.first(), usage)?;
    let since = match &args[1..] {
        [] => env
            .get("SINCE")
            .unwrap_or_else(|| DEFAULT_SINCE.to_string()),
        [flag, since] if flag == "--since" => since.clone(),
        _ => return Err(usage.to_string()),
    };
    check_since(&since)?;
    require_tools(&["ssh"])?;
    journal(
        &Ssh::new(&env.require("BO2MP_DEPLOY_HOST")?)?,
        channel,
        &since,
    )
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn descriptor() -> Value {
        json!({
            "id": "0123456789abcdef",
            "channel": "prod",
            "protocol": PROTOCOL_VERSION,
            "master_port": 4433,
            "git": "abc",
            "files": [
                {"role": "game-blob", "local": "client/bo2mp-b.exe.zst", "remote": "bo2mp-b.exe.zst", "sha256": "b1", "size": 40, "immutable": true},
                {"role": "manifest", "local": "client/manifest.json", "remote": "manifests/0123456789abcdef.json", "sha256": "m1", "size": 5, "immutable": false, "live": "manifest.json"},
                {"role": "launcher", "local": "client/bo2mp-launcher.exe", "remote": "bo2mp-launcher.exe", "sha256": "l1", "size": 30, "immutable": false},
                {"role": "master", "local": "master/bo2mp-master", "remote": "masters/s1/bo2mp-master", "sha256": "s1", "size": 70, "immutable": true},
            ],
        })
    }

    #[test]
    fn client_files_land_under_the_channel_and_the_master_does_not() {
        let rows = rows_of(&descriptor(), Channel::Prod).unwrap();
        assert_eq!(rows[0].remote, "releases/prod/bo2mp-b.exe.zst");
        assert_eq!(
            rows[1].remote,
            "releases/prod/manifests/0123456789abcdef.json"
        );
        assert_eq!(rows[3].remote, "masters/s1/bo2mp-master");
        assert!(rows[0].immutable && !rows[2].immutable);
        assert_eq!(rows[3].mode(), "755");
        assert_eq!(rows[2].mode(), "644");
    }

    #[test]
    fn odd_remote_paths_are_refused() {
        for bad in ["../etc/passwd", "a//b", "it's", "/abs", ""] {
            let mut value = descriptor();
            value["files"][2]["remote"] = json!(bad);
            let channel = if bad == "/abs" {
                Channel::Dev
            } else {
                Channel::Prod
            };
            let rows = rows_of(&value, channel);
            if bad == "/abs" {
                // Under releases/<channel>/ the slash doubles up.
                assert!(rows.is_err());
            } else {
                assert!(rows.is_err(), "{bad} accepted");
            }
        }
    }

    #[test]
    fn descriptor_must_match_dir_channel_and_protocol() {
        let value = descriptor();
        assert!(check_descriptor(&value, "0123456789abcdef", Channel::Prod).is_ok());
        assert!(check_descriptor(&value, "fedcba9876543210", Channel::Prod).is_err());
        assert!(check_descriptor(&value, "0123456789abcdef", Channel::Dev).is_err());
        let mut old = descriptor();
        old["protocol"] = json!(PROTOCOL_VERSION - 1);
        assert!(check_descriptor(&old, "0123456789abcdef", Channel::Prod).is_err());
    }

    #[test]
    fn plan_skips_what_is_there_and_stops_on_immutable_collisions() {
        let rows = rows_of(&descriptor(), Channel::Prod).unwrap();
        let root = "/srv/bo2mp";
        let mut there = parse_inventory(
            "/srv/bo2mp/releases/prod/bo2mp-b.exe.zst\tb1\n\
             /srv/bo2mp/releases/prod/bo2mp-launcher.exe\told\n\
             /srv/bo2mp/masters/s1/bo2mp-master\tMISSING\n",
        );
        let roles: Vec<&str> = plan(&rows, root, &there)
            .unwrap()
            .iter()
            .map(|row| row.role.as_str())
            .collect();
        assert_eq!(
            roles,
            ["launcher", "master"],
            "manifest goes live separately"
        );

        there.insert(
            "/srv/bo2mp/masters/s1/bo2mp-master".to_string(),
            "other".to_string(),
        );
        let error = plan(&rows, root, &there).unwrap_err();
        assert!(error.starts_with("immutable collision at /srv/bo2mp/masters/s1/bo2mp-master"));
    }

    #[test]
    fn promote_lines_are_staged_final_mode() {
        let rows = rows_of(&descriptor(), Channel::Prod).unwrap();
        let picked = [&rows[2], &rows[3]];
        assert_eq!(
            promote_lines(&picked, "/srv/bo2mp", "/srv/bo2mp/staging/prod/id"),
            "/srv/bo2mp/staging/prod/id/launcher\t/srv/bo2mp/releases/prod/bo2mp-launcher.exe\t644\n\
             /srv/bo2mp/staging/prod/id/master\t/srv/bo2mp/masters/s1/bo2mp-master\t755\n"
        );
    }

    #[test]
    fn protocol_is_read_from_master_status() {
        assert_eq!(parse_protocol("master ok protocol=9 sessions=3"), Some(9));
        assert_eq!(parse_protocol("ok\nprotocol=10,\n"), Some(10));
        assert_eq!(parse_protocol("connection refused"), None);
        assert_eq!(parse_protocol("protocol=x"), None);
    }

    #[test]
    fn release_dir_prefers_release_then_latest() {
        let root = std::env::temp_dir().join(format!("xtask-publish-{}", std::process::id()));
        let releases = root.join("dist/releases/dev");
        std::fs::create_dir_all(&releases).unwrap();
        assert!(
            release_dir(&root, None, Channel::Dev)
                .unwrap_err()
                .contains("cargo xtask release dev")
        );
        std::fs::write(releases.join("LATEST"), "abc\n").unwrap();
        assert_eq!(
            release_dir(&root, None, Channel::Dev).unwrap(),
            releases.join("abc")
        );
        assert_eq!(
            release_dir(&root, Some("dist/x".to_string()), Channel::Dev).unwrap(),
            root.join("dist/x")
        );
        let absolute = std::env::temp_dir().join("elsewhere");
        assert_eq!(
            release_dir(&root, Some(absolute.display().to_string()), Channel::Dev).unwrap(),
            absolute
        );
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn egress_interface_is_named_and_tunnels_spotted() {
        let route = "203.0.113.7 via 10.8.0.1 dev tun0 src 10.8.0.2 uid 1000 \\    cache";
        assert_eq!(egress_dev(route), Some("tun0"));
        assert!(is_tunnel("tun0") && is_tunnel("wg0"));
        assert!(!is_tunnel("eth0"));
        assert_eq!(egress_dev(""), None);
    }
}
