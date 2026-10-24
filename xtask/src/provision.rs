//! `cargo xtask provision`: make the release host ready once. Service
//! accounts, the directory tree, both channels' master units, Caddy serving
//! the releases on :8443, and the server identity. No master is started;
//! `publish` does that when there is a release to run.

use std::path::Path;

use master_protocol::Channel;

use crate::certs::{Ca, San};
use crate::dotenv::Env;
use crate::master::unit_text;
use crate::shell::{Res, Ssh, check_deploy_root, require_tools};

pub const RELEASES_PORT: u16 = 8443;
const SERVICE_USER: &str = "bo2mp";
/// The masters and Caddy both read the server key through this group.
const RELEASE_GROUP: &str = "bo2mp-release";

pub fn remote_dirs(root: &str) -> Vec<String> {
    let mut dirs = vec![
        format!("{root}/bin"),
        format!("{root}/masters"),
        format!("{root}/locks"),
    ];
    for channel in Channel::ALL {
        dirs.push(format!("{root}/releases/{channel}/manifests"));
        dirs.push(format!("{root}/staging/{channel}"));
    }
    dirs.extend(
        [
            "/etc/bo2mp",
            "/etc/systemd/system",
            "/etc/systemd/journald.conf.d",
        ]
        .map(String::from),
    );
    dirs
}

pub fn master_link(root: &str, channel: Channel) -> String {
    format!("{root}/bin/bo2mp-master-{channel}")
}

pub fn caddyfile(root: &str) -> String {
    format!(
        ":{RELEASES_PORT} {{\n\
         \ttls /etc/bo2mp/server-cert.pem /etc/bo2mp/server-key.pem\n\
         \troot * {root}/releases\n\
         \tfile_server\n\
         }}\n"
    )
}

/// Two masters logging every session would otherwise fill a small disk.
pub const JOURNALD: &str = "[Journal]\nSystemMaxUse=300M\nMaxRetentionSec=14day\n";

fn accounts(root: &str) -> String {
    format!(
        "getent group {RELEASE_GROUP} >/dev/null || groupadd --system {RELEASE_GROUP}; \
         id {SERVICE_USER} >/dev/null 2>&1 || useradd --system --gid {RELEASE_GROUP} \
         --home-dir '{root}' --shell /usr/sbin/nologin {SERVICE_USER}; \
         usermod -a -G {RELEASE_GROUP} caddy"
    )
}

/// Retires the old per-channel dedicated servers, which the masters replaced,
/// then loads the new config and opens the ports.
pub fn finish(root: &str) -> String {
    let ports: String = Channel::ALL
        .iter()
        .map(|channel| format!("ufw allow {}/udp; ", channel.port()))
        .collect();
    let units: Vec<&str> = Channel::ALL.iter().map(|channel| channel.unit()).collect();
    format!(
        "systemctl disable --now bo2mp-dedicated@prod.service bo2mp-dedicated@dev.service 2>/dev/null || true\n\
         rm -f /etc/systemd/system/bo2mp-dedicated@.service '{root}/bin/bo2mp-dedicated-prod' \
         '{root}/bin/bo2mp-dedicated-dev' '{root}/config/prod.env' '{root}/config/dev.env'\n\
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

pub fn run_cli(local_root: &Path, env: &Env, args: &[String]) -> Res<()> {
    if !args.is_empty() {
        return Err("usage: cargo xtask provision (settings come from .env)".to_string());
    }
    require_tools(&["cargo", "openssl", "rsync", "ssh"])?;
    let ssh = Ssh::new(&env.require("BO2MP_DEPLOY_HOST")?)?;
    let root = env.require("BO2MP_DEPLOY_ROOT")?;
    check_deploy_root(&root)?;
    let ca = Ca::new(env.require("BO2MP_RELEASE_KEY")?);
    ca.ensure(&San::WithHost(ssh.host()))?;

    ssh.run(
        "command -v caddy >/dev/null && command -v rsync >/dev/null || \
         { apt-get update && DEBIAN_FRONTEND=noninteractive apt-get install -y caddy rsync; }",
    )?;
    ssh.run(&accounts(&root))?;
    let dirs: Vec<String> = remote_dirs(&root)
        .iter()
        .map(|dir| format!("'{dir}'"))
        .collect();
    ssh.run(&format!("install -d -m0755 {}", dirs.join(" ")))?;

    for channel in Channel::ALL {
        let unit = unit_text(
            local_root,
            channel,
            &master_link(&root, channel),
            "/etc/bo2mp/server-cert.pem",
            "/etc/bo2mp/server-key.pem",
            SERVICE_USER,
            RELEASE_GROUP,
        )?;
        ssh.feed(
            &format!("cat > '/etc/systemd/system/{}'", channel.unit()),
            &unit,
        )?;
    }
    ssh.feed("cat > /etc/caddy/Caddyfile", &caddyfile(&root))?;
    ssh.feed("cat > /etc/systemd/journald.conf.d/bo2mp.conf", JOURNALD)?;

    ssh.rsync(&["--chmod=F644"], &ca.cert(), "/etc/bo2mp/bo2mp-ca.pem")?;
    ssh.rsync(
        &["--chmod=F644"],
        &ca.server_cert(),
        "/etc/bo2mp/server-cert.pem",
    )?;
    ssh.rsync(
        &["--chmod=F640"],
        &ca.server_key(),
        "/etc/bo2mp/server-key.pem",
    )?;
    ssh.run(&finish(&root))?;
    ssh.run("systemctl --no-pager --full status caddy")?;
    println!(
        "provision: {} ready; next `cargo xtask release prod` and `cargo xtask publish prod`",
        ssh.target()
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
        for want in [
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
            assert!(dirs.iter().any(|dir| dir == want), "{want} missing");
        }
        assert_eq!(
            master_link("/srv/bo2mp", Channel::Dev),
            "/srv/bo2mp/bin/bo2mp-master-dev"
        );
    }

    #[test]
    fn finish_opens_both_master_ports_and_starts_no_master() {
        let script = finish("/srv/bo2mp");
        assert!(script.contains("ufw allow 4433/udp; ufw allow 4434/udp; ufw allow 8443/tcp"));
        assert!(
            script
                .contains("systemctl enable caddy bo2mp-master.service bo2mp-master-dev.service\n")
        );
        assert!(!script.contains("enable --now"));
        assert!(!script.contains("start bo2mp-master"));
        assert!(script.contains("caddy validate --config /etc/caddy/Caddyfile\n"));
    }

    #[test]
    fn journal_is_capped() {
        assert!(JOURNALD.starts_with("[Journal]\n"));
        assert!(JOURNALD.contains("SystemMaxUse=300M\n"));
    }
}
