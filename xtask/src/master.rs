//! A master on any Linux box you can ssh into as root:
//! `cargo xtask master <install|update|status|logs|uninstall> user@host`.
//! The release host's two masters are `provision` and `publish`'s job.

use std::path::{Path, PathBuf};
use std::process::Command;

use master_protocol::Channel;

use crate::certs::{Ca, San};
use crate::dotenv::Env;
use crate::release::build_master;
use crate::shell::{Res, Ssh, capture, require_tools};
use crate::windows;

const LIB_DIR: &str = "/usr/local/lib/bo2mp";
const ETC_DIR: &str = "/etc/bo2mp";
const SERVICE_ACCOUNT: &str = "bo2mp";
pub const DEFAULT_SINCE: &str = "2h";

/// The units `journalctl --since -N<unit>` understands.
const SINCE_UNITS: [&str; 18] = [
    "s", "sec", "second", "seconds", "m", "min", "minute", "minutes", "h", "hr", "hour", "hours",
    "d", "day", "days", "w", "week", "weeks",
];

fn master_bin() -> String {
    format!("{LIB_DIR}/bo2mp-master")
}

fn etc(name: &str) -> String {
    format!("{ETC_DIR}/{name}")
}

/// A journal window such as `2h` or `30min`. It ends up inside single quotes
/// on the remote shell, so nothing but digits and a unit gets through.
pub fn check_since(since: &str) -> Res<()> {
    let digits = since
        .find(|c: char| !c.is_ascii_digit())
        .unwrap_or(since.len());
    let (count, unit) = since.split_at(digits);
    if !count.is_empty() && SINCE_UNITS.contains(&unit) {
        Ok(())
    } else {
        Err(format!(
            "--since wants a number and a unit such as 2h or 30min, not {since:?}"
        ))
    }
}

/// The systemd unit for one channel's master, as `bo2mp-master print-unit`
/// writes it, so the unit always matches the binary's own flags.
pub fn unit_text(
    root: &Path,
    channel: Channel,
    exec: &str,
    cert: &str,
    key: &str,
    user: &str,
    group: &str,
) -> Res<String> {
    capture(
        Command::new("cargo")
            .current_dir(root)
            .args(["run", "--quiet", "-p", "bo2mp-master", "--", "print-unit"])
            .args([
                "--channel",
                channel.as_str(),
                "--exec",
                exec,
                "--cert",
                cert,
                "--key",
                key,
            ])
            .args(["--user", user, "--group", group]),
    )
}

pub fn journal(ssh: &Ssh, channel: Channel, since: &str) -> Res<()> {
    ssh.run(&format!(
        "journalctl -u '{}' --since '-{since}' --no-pager",
        channel.unit()
    ))
}

struct Args {
    ssh: Ssh,
    channel: Channel,
    since: String,
    ca_dir: Option<PathBuf>,
}

fn parse_args(args: &[String]) -> Res<Args> {
    let mut target = None;
    let mut channel = Channel::Prod;
    let mut since = DEFAULT_SINCE.to_string();
    let mut ca_dir = None;
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        let mut value = |what: &str| {
            args.next()
                .cloned()
                .ok_or_else(|| format!("{arg} needs {what}"))
        };
        match arg.as_str() {
            "--channel" => {
                channel = value("prod or dev")?
                    .parse()
                    .map_err(|_| "--channel is prod or dev".to_string())?;
            }
            "--since" => since = value("a window such as 2h")?,
            "--ca" => ca_dir = Some(PathBuf::from(value("a directory")?)),
            flag if flag.starts_with('-') => return Err(format!("unknown option {flag}")),
            host if target.is_none() => target = Some(host.to_string()),
            extra => return Err(format!("one user@host only; {extra:?} is extra")),
        }
    }
    check_since(&since)?;
    let target = target.ok_or("name the box as user@host")?;
    Ok(Args {
        ssh: Ssh::new(&target)?,
        channel,
        since,
        ca_dir,
    })
}

/// The CA lives outside the clone: its private key is everyone's trust
/// anchor, not a repository file.
fn ca(env: &Env, args: &Args) -> Res<Ca> {
    if let Some(dir) = &args.ca_dir {
        return Ok(Ca::new(dir));
    }
    if let Some(dir) = env.get("BO2MP_MASTER_CA_DIR") {
        return Ok(Ca::new(dir));
    }
    let home = env
        .get("HOME")
        .or_else(|| env.get("USERPROFILE"))
        .ok_or("no home directory; pass --ca DIR or set BO2MP_MASTER_CA_DIR")?;
    Ok(Ca::new(Path::new(&home).join(".bo2mp/ca")))
}

fn upload_master(root: &Path, env: &Env, ssh: &Ssh) -> Res<()> {
    let bin = build_master(root, &windows::profile(env)?)?;
    ssh.run(&format!("install -d -m0755 '{LIB_DIR}'"))?;
    ssh.rsync(&["--chmod=F755"], &bin, &master_bin())
}

fn upload_identity(ssh: &Ssh, ca: &Ca) -> Res<()> {
    ssh.run(&format!("install -d -m0755 '{ETC_DIR}'"))?;
    ssh.rsync(&["--chmod=F644"], &ca.cert(), &etc("bo2mp-ca.pem"))?;
    ssh.rsync(
        &["--chmod=F644"],
        &ca.server_cert(),
        &etc("server-cert.pem"),
    )?;
    ssh.rsync(&["--chmod=F640"], &ca.server_key(), &etc("server-key.pem"))?;
    ssh.run(&format!(
        "chown root:{SERVICE_ACCOUNT} '{key}' && chmod 0640 '{key}'",
        key = etc("server-key.pem")
    ))
}

fn report(ssh: &Ssh, channel: Channel) -> Res<()> {
    ssh.run(&format!(
        "systemctl --no-pager --full status '{}' || true",
        channel.unit()
    ))?;
    ssh.run(&format!(
        "'{bin}' status --connect 127.0.0.1:{port} --server-name '{name}' --ca-cert '{ca}'",
        bin = master_bin(),
        port = channel.port(),
        name = channel.server_name(),
        ca = etc("bo2mp-ca.pem"),
    ))
}

/// The address and label are the same for everyone, so `bo2mp-ca.pem` is the
/// whole of the trust: hand it over a channel the players already trust.
fn hand_out(ssh: &Ssh, channel: Channel, ca: &Ca) {
    println!();
    println!(
        "For your players: these three .env lines, and {}",
        ca.cert().display()
    );
    println!("  BO2MP_MASTER_ADDR={}:{}", ssh.host(), channel.port());
    println!("  BO2MP_MASTER_SERVER_NAME={}", channel.server_name());
    println!("  BO2MP_MASTER_CA_CERT=/path/to/bo2mp-ca.pem");
    println!("Then `make map` hosts a match and `make menu` finds it.");
}

fn install(root: &Path, env: &Env, args: &Args) -> Res<()> {
    require_tools(&["cargo", "rsync", "ssh"])?;
    let (ssh, channel) = (&args.ssh, args.channel);
    let ca = ca(env, args)?;
    ca.ensure(&San::Labels)?;
    ssh.run(&format!(
        "getent group {SERVICE_ACCOUNT} >/dev/null || groupadd --system {SERVICE_ACCOUNT}; \
         id {SERVICE_ACCOUNT} >/dev/null 2>&1 || useradd --system --gid {SERVICE_ACCOUNT} \
         --home-dir /nonexistent --shell /usr/sbin/nologin {SERVICE_ACCOUNT}"
    ))?;
    upload_master(root, env, ssh)?;
    upload_identity(ssh, &ca)?;
    let unit = unit_text(
        root,
        channel,
        &master_bin(),
        &etc("server-cert.pem"),
        &etc("server-key.pem"),
        SERVICE_ACCOUNT,
        SERVICE_ACCOUNT,
    )?;
    ssh.feed(
        &format!("cat > '/etc/systemd/system/{}'", channel.unit()),
        &unit,
    )?;
    ssh.run(&format!(
        "if command -v ufw >/dev/null && ufw status | grep -q '^Status: active'; then \
         ufw allow {}/udp; fi; \
         systemctl daemon-reload && systemctl enable --now '{}'",
        channel.port(),
        channel.unit()
    ))?;
    report(ssh, channel)?;
    hand_out(ssh, channel, &ca);
    Ok(())
}

fn update(root: &Path, env: &Env, args: &Args) -> Res<()> {
    require_tools(&["cargo", "rsync", "ssh"])?;
    upload_master(root, env, &args.ssh)?;
    args.ssh
        .run(&format!("systemctl restart '{}'", args.channel.unit()))?;
    report(&args.ssh, args.channel)
}

/// Both channels run the one binary, so it goes only with the last unit. The
/// identity under /etc/bo2mp and the local CA stay.
fn uninstall(args: &Args) -> Res<()> {
    let channel = args.channel;
    let others: Vec<String> = Channel::ALL
        .iter()
        .filter(|other| **other != channel)
        .map(|other| format!("[ -e '/etc/systemd/system/{}' ]", other.unit()))
        .collect();
    args.ssh.run(&format!(
        "systemctl disable --now '{unit}' 2>/dev/null || true; \
         rm -f '/etc/systemd/system/{unit}'; systemctl daemon-reload; \
         if ! {{ {others}; }}; then rm -f '{bin}'; rmdir '{LIB_DIR}' 2>/dev/null || true; fi",
        unit = channel.unit(),
        others = others.join(" || "),
        bin = master_bin(),
    ))?;
    println!(
        "master: {} removed from {}; {ETC_DIR} and the local CA were kept",
        channel.unit(),
        args.ssh.target()
    );
    Ok(())
}

pub fn run_cli(root: &Path, env: &Env, args: &[String]) -> Res<()> {
    let usage = "usage: cargo xtask master <install|update|status|logs|uninstall> \
                 [--channel prod|dev] [--since 2h] [--ca DIR] user@host";
    let (verb, rest) = args.split_first().ok_or(usage)?;
    let parsed = parse_args(rest).map_err(|why| format!("{why}\n{usage}"))?;
    match verb.as_str() {
        "install" => install(root, env, &parsed),
        "update" => update(root, env, &parsed),
        "status" => report(&parsed.ssh, parsed.channel),
        "logs" => journal(&parsed.ssh, parsed.channel, &parsed.since),
        "uninstall" => uninstall(&parsed),
        other => Err(format!("unknown verb {other:?}\n{usage}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strings(args: &[&str]) -> Vec<String> {
        args.iter().map(|arg| arg.to_string()).collect()
    }

    #[test]
    fn since_is_a_count_and_a_journal_unit() {
        for good in ["2h", "30min", "1day", "90s", "2weeks", "15m"] {
            assert!(check_since(good).is_ok(), "{good} refused");
        }
        for bad in ["", "h", "2", "2 h", "2mins", "2h;reboot", "-2h", "2h'"] {
            assert!(check_since(bad).is_err(), "{bad} accepted");
        }
    }

    #[test]
    fn options_go_anywhere_and_default_to_prod() {
        let args = parse_args(&strings(&["root@203.0.113.7"])).unwrap();
        assert_eq!(args.channel, Channel::Prod);
        assert_eq!(args.since, DEFAULT_SINCE);
        assert!(args.ca_dir.is_none());

        let args = parse_args(&strings(&[
            "--since",
            "30min",
            "root@203.0.113.7",
            "--channel",
            "dev",
            "--ca",
            "/keys",
        ]))
        .unwrap();
        assert_eq!(args.channel, Channel::Dev);
        assert_eq!(args.since, "30min");
        assert_eq!(args.ssh.host(), "203.0.113.7");
        assert_eq!(args.ca_dir, Some(PathBuf::from("/keys")));
    }

    #[test]
    fn bad_arguments_are_refused() {
        for bad in [
            &[][..],
            &["--channel", "beta", "root@h"][..],
            &["--channel"][..],
            &["--verbose", "root@h"][..],
            &["root@h", "root@g"][..],
            &["--since", "2x", "root@h"][..],
            &["not a target"][..],
        ] {
            assert!(parse_args(&strings(bad)).is_err(), "{bad:?} accepted");
        }
    }
}
