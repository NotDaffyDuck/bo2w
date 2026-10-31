//! `cargo xtask provision`: make the release host ready once. Service
//! accounts, the directory tree, both channels' master units, Caddy serving
//! the releases on :8443, and the server identity. No master is started;
//! `publish` does that when there is a release to run.

use std::path::{Path, PathBuf};

use master_protocol::Channel;

use crate::certs::{Authority, CertAltName};
use crate::dot_vars::DotVars;
use crate::lobby_host::systemd_text;
use crate::cmd_run::{RemoteShellLink, validate_install_dir, demand_host_programs};

pub const RELEASES_PORT: u16 = 8443;
const SERVICE_USER: &str = "bo2mp";
/// The masters and Caddy both read the server key through this group.
const RELEASE_GROUP: &str = "bo2mp-release";

pub fn remote_dirs(repo_root: &str) -> Vec<String> {
    let shared = ["bin", "masters", "locks"].map(|title| format!("{repo_root}/{title}"));
    let per_channel = Channel::ALL.into_iter().flat_map(|svc| {
        [
            format!("{repo_root}/releases/{svc}/manifests"),
            format!("{repo_root}/staging/{svc}"),
        ]
    });
    let system = ETC_DIRS.map(String::from);
    shared.into_iter().chain(per_channel).chain(system).collect()
}

const ETC_DIRS: [&str; 3] = [
    "/etc/bo2mp",
    "/etc/systemd/system",
    "/etc/systemd/journald.conf.d",
];

pub fn master_link(repo_root: &str, svc: Channel) -> String {
    format!("{repo_root}/bin/bo2mp-master-{svc}")
}

pub fn caddyfile(repo_root: &str) -> String {
    format!(
        ":{RELEASES_PORT} {{\n\
         \ttls /etc/bo2mp/server-cert.pem /etc/bo2mp/server-key.pem\n\
         \troot * {repo_root}/releases\n\
         \tfile_server\n\
         }}\n"
    )
}

/// Two masters logging every session would otherwise fill a small disk.
pub const JOURNALD: &str = "[Journal]\nSystemMaxUse=300M\nMaxRetentionSec=14day\n";

fn accounts(repo_root: &str) -> String {
    format!(
        "getent group {RELEASE_GROUP} >/dev/null || groupadd --system {RELEASE_GROUP}; \
         id {SERVICE_USER} >/dev/null 2>&1 || useradd --system --gid {RELEASE_GROUP} \
         --home-dir '{repo_root}' --shell /usr/sbin/nologin {SERVICE_USER}; \
         usermod -a -G {RELEASE_GROUP} caddy"
    )
}

/// Retires the old per-channel dedicated servers, which the masters replaced,
/// then loads the new config and opens the ports.
pub fn finish(repo_root: &str) -> String {
    let ports: String = Channel::ALL
        .iter()
        .map(|svc| format!("ufw allow {}/udp; ", svc.port()))
        .collect();
    let units: Vec<&str> = Channel::ALL.iter().map(|svc| svc.systemd_unit()).collect();
    format!(
        "systemctl disable --now bo2mp-dedicated@prod.service bo2mp-dedicated@dev.service 2>/dev/null || true\n\
         rm -f /etc/systemd/system/bo2mp-dedicated@.service '{repo_root}/bin/bo2mp-dedicated-prod' \
         '{repo_root}/bin/bo2mp-dedicated-dev' '{repo_root}/config/prod.env' '{repo_root}/config/dev.env'\n\
         chown root:{RELEASE_GROUP} /etc/bo2mp/server-key.pem\n\
         chmod 640 /etc/bo2mp/server-key.pem\n\
         caddy validate --config /etc/caddy/Caddyfile\n\
         systemctl daemon-reload\n\
         systemctl restart systemd-journald caddy\n\
         if command -v ufw >/dev/null && ufw status | grep -q '^Status: active'; then \
         {ports}ufw allow {RELEASES_PORT}/tcp; fi\n\
         systemctl enable caddy {units}\n",
        units = units.join(" "),
    )
}

const SERVER_CERT_PATH: &str = "/etc/bo2mp/server-cert.pem";
const SERVER_KEY_PATH: &str = "/etc/bo2mp/server-key.pem";
const LOCAL_TOOLS: [&str; 4] = ["cargo", "openssl", "rsync", "ssh"];
const CADDY_STATUS: &str = "systemctl --no-pager --full status caddy";

/// The one shell line that makes every directory, quoted.
fn install_dirs(repo_root: &str) -> String {
    let quoted: Vec<String> = remote_dirs(repo_root)
        .into_iter()
        .map(|base_dir| format!("'{base_dir}'"))
        .collect();
    format!("install -d -m0755 {}", quoted.join(" "))
}

/// One identity file, where it goes on the host and the rsync mode it gets there.
struct IdentityUpload {
    local: PathBuf,
    remote: &'static str,
    mode: &'static str,
}

/// Every identity file to upload; everything but the key stays readable by others.
fn identity_uploads(authority: &Authority) -> Vec<IdentityUpload> {
    let readable = "--chmod=F644";
    vec![
        IdentityUpload { local: authority.cert(), remote: "/etc/bo2mp/bo2mp-ca.pem", mode: readable },
        IdentityUpload { local: authority.server_cert(), remote: SERVER_CERT_PATH, mode: readable },
        IdentityUpload { local: authority.server_key(), remote: SERVER_KEY_PATH, mode: "--chmod=F640" },
    ]
}

fn write_unit(local_root: &Path, shell_link: &RemoteShellLink, repo_root: &str, svc: Channel) -> Result<(), String> {
    let link = master_link(repo_root, svc);
    let text = systemd_text(
        local_root,
        svc,
        &link,
        SERVER_CERT_PATH,
        SERVER_KEY_PATH,
        SERVICE_USER,
        RELEASE_GROUP,
    )?;
    let cmd_text = format!("cat > '/etc/systemd/system/{}'", svc.systemd_unit());
    shell_link.pour(&cmd_text, &text).map(drop)
}

pub fn dispatch_subcommand(local_root: &Path, dot_conf: &DotVars, cli_words: &[String]) -> Result<(), String> {
    if !cli_words.is_empty() {
        return Err("usage: cargo xtask provision (settings come from .env)".to_string());
    }
    demand_host_programs(&LOCAL_TOOLS)?;
    let shell_link = RemoteShellLink::new(&dot_conf.must_get("BO2MP_DEPLOY_HOST")?)?;
    let repo_root = dot_conf.must_get("BO2MP_DEPLOY_ROOT")?;
    validate_install_dir(&repo_root)?;
    let ca = Authority::new(dot_conf.must_get("BO2MP_RELEASE_KEY")?);
    ca.make_ready(&CertAltName::WithHost(shell_link.host_name()))?;

    shell_link.launch(
        "command -v caddy >/dev/null && command -v rsync >/dev/null || \
         { apt-get update && DEBIAN_FRONTEND=noninteractive apt-get install -y caddy rsync; }",
    )?;
    shell_link.launch(&accounts(&repo_root))?;
    shell_link.launch(&install_dirs(&repo_root))?;

    Channel::ALL
        .into_iter()
        .try_for_each(|svc| write_unit(local_root, &shell_link, &repo_root, svc))?;
    shell_link.pour("cat > /etc/caddy/Caddyfile", &caddyfile(&repo_root))?;
    shell_link.pour("cat > /etc/systemd/journald.conf.d/bo2mp.conf", JOURNALD)?;

    for upload in &identity_uploads(&ca) {
        shell_link.mirror_up(&[upload.mode], &upload.local, upload.remote)?;
    }
    shell_link.launch(&finish(&repo_root))?;
    shell_link.launch(CADDY_STATUS)?;
    let dest = shell_link.dest();
    println!(
        "provision: {dest} ready; next `cargo xtask release prod` and `cargo xtask publish prod`"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn caddy_serves_the_release_tree_with_our_identity() {
        let text = caddyfile("/srv/bo2mp");
        assert!(text.starts_with(":8443 {\n"));
        assert!(text.contains("\ttls /etc/bo2mp/server-cert.pem /etc/bo2mp/server-key.pem\n"));
        assert!(text.contains("\troot * /srv/bo2mp/releases\n"));
        assert!(text.ends_with("\tfile_server\n}\n"));
    }

    #[test]
    fn every_channel_gets_manifests_and_staging() {
        let dirs = remote_dirs("/srv/bo2mp");
        for hoped in [
            "/srv/bo2mp/bin",
            "/srv/bo2mp/masters",
            "/srv/bo2mp/locks",
            "/srv/bo2mp/releases/prod/manifests",
            "/srv/bo2mp/releases/dev/manifests",
            "/srv/bo2mp/staging/prod",
            "/srv/bo2mp/staging/dev",
            "/etc/bo2mp",
            "/etc/systemd/journald.conf.d",
        ] {
            assert!(dirs.iter().any(|base_dir| base_dir == hoped), "{hoped} missing");
        }
        assert_eq!(
            master_link("/srv/bo2mp", Channel::PreReleaseBuild),
            "/srv/bo2mp/bin/bo2mp-master-dev"
        );
    }

    #[test]
    fn finish_opens_both_master_ports_and_starts_no_master() {
        let cmd_text = finish("/srv/bo2mp");
        assert!(cmd_text.contains("ufw allow 4433/udp; ufw allow 4434/udp; ufw allow 8443/tcp"));
        assert!(
            cmd_text
                .contains("systemctl enable caddy bo2mp-master.service bo2mp-master-dev.service\n")
        );
        assert!(!cmd_text.contains("enable --now"));
        assert!(!cmd_text.contains("start bo2mp-master"));
        assert!(cmd_text.contains("caddy validate --config /etc/caddy/Caddyfile\n"));
    }

    #[test]
    fn directories_are_listed_shared_first_then_per_channel_then_system() {
        let hoped = [
            "/r/bin",
            "/r/masters",
            "/r/locks",
            "/r/releases/prod/manifests",
            "/r/staging/prod",
            "/r/releases/dev/manifests",
            "/r/staging/dev",
            "/etc/bo2mp",
            "/etc/systemd/system",
            "/etc/systemd/journald.conf.d",
        ];
        assert_eq!(remote_dirs("/r"), hoped);
        let line = install_dirs("/r");
        assert!(line.starts_with("install -d -m0755 '/r/bin' '/r/masters' "));
        assert!(line.ends_with(" '/etc/systemd/journald.conf.d'"));
    }

    #[test]
    fn only_the_server_key_is_closed_to_others() {
        let uploads = identity_uploads(&Authority::new("/keys"));
        let modes: Vec<&str> = uploads.iter().map(|upload| upload.mode).collect();
        assert_eq!(modes, ["--chmod=F644", "--chmod=F644", "--chmod=F640"]);
        let remotes: Vec<&str> = uploads.iter().map(|upload| upload.remote).collect();
        assert_eq!(
            remotes,
            [
                "/etc/bo2mp/bo2mp-ca.pem",
                "/etc/bo2mp/server-cert.pem",
                "/etc/bo2mp/server-key.pem"
            ]
        );
        assert!(uploads[2].local.ends_with("server-key.pem"));
    }

    #[test]
    fn provision_takes_no_arguments() {
        let dot_conf = DotVars::parse("");
        let error = dispatch_subcommand(Path::new("."), &dot_conf, &["now".to_string()]).unwrap_err();
        assert_eq!(error, "usage: cargo xtask provision (settings come from .env)");
    }

    #[test]
    fn journal_is_capped() {
        assert!(JOURNALD.starts_with("[Journal]\n"));
        assert!(JOURNALD.contains("SystemMaxUse=300M\n"));
    }
}
