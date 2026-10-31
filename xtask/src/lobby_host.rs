//! A master on any Linux box you can ssh into as root:
//! `cargo xtask master <install|update|status|logs|uninstall> user@host`.
//! The release host's two masters are `provision` and `publish`'s job.

use std::path::{Path, PathBuf};
use std::process::Command;

use master_protocol::Channel;

use crate::certs::{Authority, CertAltName};
use crate::dot_vars::DotVars;
use crate::release::compile_lobby_server;
use crate::cmd_run::{RemoteShellLink, grab, demand_host_programs};
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
    [LIB_DIR, "bo2mp-master"].join("/")
}

fn etc(title: &str) -> String {
    [ETC_DIR, title].join("/")
}

/// A journal window such as `2h` or `30min`. It ends up inside single quotes
/// on the remote shell, so nothing but digits and a unit gets through.
pub fn validate_log_window(from_time: &str) -> Result<(), String> {
    let unit = from_time.trim_start_matches(|c: char| c.is_ascii_digit());
    let has_count = unit.len() < from_time.len();
    if has_count && SINCE_UNITS.contains(&unit) {
        return Ok(());
    }
    Err(format!(
        "--since wants a number and a unit such as 2h or 30min, not {from_time:?}"
    ))
}

/// The systemd unit for one channel's master, as `bo2mp-master print-unit`
/// writes it, so the unit always matches the binary's own flags.
pub fn systemd_text(
    repo_root: &Path,
    svc: Channel,
    exec: &str,
    cert: &str,
    knob: &str,
    user: &str,
    unix_group: &str,
) -> Result<String, String> {
    let options = [
        ("--channel", svc.as_str()),
        ("--exec", exec),
        ("--cert", cert),
        ("--key", knob),
        ("--user", user),
        ("--group", unix_group),
    ];
    let mut command = Command::new("cargo");
    command.current_dir(repo_root);
    command.args(["run", "--quiet", "-p", "bo2mp-master", "--", "print-unit"]);
    command.args(options.into_iter().flat_map(|(flag, val)| [flag, val]));
    grab(&mut command)
}

pub fn journal(shell_link: &RemoteShellLink, svc: Channel, from_time: &str) -> Result<(), String> {
    shell_link.launch(&journal_script(svc, from_time))
}

fn journal_script(svc: Channel, from_time: &str) -> String {
    format!(
        "journalctl -u '{}' --since '-{from_time}' --no-pager",
        svc.systemd_unit()
    )
}

/// The box and the master channel on it that a verb works on.
struct Job {
    shell_link: RemoteShellLink,
    svc: Channel,
}

/// A journal window that `validate_log_window` let through.
struct Since(String);

impl Since {
    fn parse(text: &str) -> Result<Self, String> {
        validate_log_window(text).map(|()| Self(text.to_owned()))
    }
}

struct MasterOpts {
    job: Job,
    from_time: Since,
    ca_dir: Option<PathBuf>,
}

/// The word after `flag`, or an error naming what the flag needs.
fn value_of<'a, I>(rest: &mut I, flag: &str, what: &str) -> Result<&'a str, String>
where
    I: Iterator<Item = &'a str>,
{
    rest.next().ok_or_else(|| format!("{flag} needs {what}"))
}

/// What the command line said so far; unsaid parts take their defaults last.
#[derive(Default)]
struct Picked<'a> {
    dest: Option<&'a str>,
    svc: Option<Channel>,
    ca_dir: Option<PathBuf>,
    window: Option<String>,
}

fn parse_args(cli_words: &[String]) -> Result<MasterOpts, String> {
    let mut words = cli_words.iter().map(String::as_str);
    let mut picked = Picked::default();
    while let Some(word) = words.next() {
        match word.strip_prefix("--") {
            Some("channel") => {
                let title = value_of(&mut words, word, "prod or dev")?;
                let parsed = title.parse::<Channel>();
                picked.svc = Some(parsed.map_err(|_| String::from("--channel is prod or dev"))?);
            }
            Some("since") => {
                let text = value_of(&mut words, word, "a window such as 2h")?;
                picked.window = Some(text.to_owned());
            }
            Some("ca") => {
                let base_dir = value_of(&mut words, word, "a directory")?;
                picked.ca_dir = Some(PathBuf::from(base_dir));
            }
            _ if word.starts_with('-') => return Err(format!("unknown option {word}")),
            _ if picked.dest.is_some() => {
                return Err(format!("one user@host only; {word:?} is extra"));
            }
            _ => picked.dest = Some(word),
        }
    }
    let window = picked.window.as_deref().unwrap_or(DEFAULT_SINCE);
    let from_time = Since::parse(window)?;
    let dest = picked.dest.ok_or("name the box as user@host")?;
    let job = Job {
        shell_link: RemoteShellLink::new(dest)?,
        svc: picked.svc.unwrap_or(Channel::Prod),
    };
    Ok(MasterOpts {
        job,
        from_time,
        ca_dir: picked.ca_dir,
    })
}

/// The CA lives outside the clone: its private key is everyone's trust
/// anchor, not a repository file.
fn ca(dot_conf: &DotVars, cli_words: &MasterOpts) -> Result<Authority, String> {
    let given = cli_words
        .ca_dir
        .clone()
        .or_else(|| dot_conf.value_of("BO2MP_MASTER_CA_DIR").map(PathBuf::from));
    if let Some(base_dir) = given {
        return Ok(Authority::new(base_dir));
    }
    let home = ["HOME", "USERPROFILE"]
        .into_iter()
        .find_map(|title| dot_conf.value_of(title))
        .ok_or("no home directory; pass --ca DIR or set BO2MP_MASTER_CA_DIR")?;
    Ok(Authority::new(Path::new(&home).join(".bo2mp/ca")))
}

/// A CA that exists on disk and already covers the channel labels.
fn ready_ca(dot_conf: &DotVars, cli_words: &MasterOpts) -> Result<Authority, String> {
    let ca = ca(dot_conf, cli_words)?;
    ca.make_ready(&CertAltName::Labels).map(|()| ca)
}

fn make_remote_dir(shell_link: &RemoteShellLink, base_dir: &str) -> Result<(), String> {
    shell_link.launch(&format!("install -d -m0755 '{base_dir}'"))
}

fn upload_master(repo_root: &Path, dot_conf: &DotVars, shell_link: &RemoteShellLink) -> Result<(), String> {
    let bin = compile_lobby_server(repo_root, &windows::flavour(dot_conf)?)?;
    make_remote_dir(shell_link, LIB_DIR)?;
    shell_link.mirror_up(&["--chmod=F755"], &bin, &master_bin())
}

fn upload_identity(shell_link: &RemoteShellLink, ca: &Authority) -> Result<(), String> {
    make_remote_dir(shell_link, ETC_DIR)?;
    let key_path = etc("server-key.pem");
    let parcels = [
        ("--chmod=F644", ca.cert(), "bo2mp-ca.pem"),
        ("--chmod=F644", ca.server_cert(), "server-cert.pem"),
        ("--chmod=F640", ca.server_key(), "server-key.pem"),
    ];
    for (mode, near, title) in &parcels {
        shell_link.mirror_up(&[*mode], near, &etc(title))?;
    }
    let account = SERVICE_ACCOUNT;
    shell_link.launch(&format!(
        "chown root:{account} '{key_path}' && chmod 0640 '{key_path}'"
    ))
}

fn report(job: &Job) -> Result<(), String> {
    let Job { shell_link, svc } = job;
    shell_link.launch(&format!(
        "systemctl --no-pager --full status '{}' || true",
        svc.systemd_unit()
    ))?;
    let (bin, ca) = (master_bin(), etc("bo2mp-ca.pem"));
    let (port, title) = (svc.port(), svc.tls_name());
    shell_link.launch(&format!(
        "'{bin}' status --connect 127.0.0.1:{port} --server-name '{title}' --ca-cert '{ca}'"
    ))
}

/// What the operator hands to players, one printed line per entry.
fn trust_notice(host_name: &str, svc: Channel, trust_anchor_cert: &Path) -> Vec<String> {
    let lines = [
        format!(
            "For your players: these three .env lines, and {}",
            trust_anchor_cert.display()
        ),
        format!("  BO2MP_MASTER_ADDR={host_name}:{}", svc.port()),
        format!("  BO2MP_MASTER_SERVER_NAME={}", svc.tls_name()),
        "  BO2MP_MASTER_CA_CERT=/path/to/bo2mp-ca.pem".to_owned(),
        "Then `make map` hosts a match and `make menu` finds it.".to_owned(),
    ];
    std::iter::once(String::new()).chain(lines).collect()
}

/// The address and label are the same for everyone, so `bo2mp-ca.pem` is the
/// whole of the trust: hand it over a channel the players already trust.
fn hand_out(job: &Job, ca: &Authority) {
    let notice = trust_notice(job.shell_link.host_name(), job.svc, &ca.cert());
    for line in notice {
        println!("{line}");
    }
}

/// Creates the unprivileged account the master runs as, unless it exists.
fn ensure_service_account(shell_link: &RemoteShellLink) -> Result<(), String> {
    let title = SERVICE_ACCOUNT;
    let make_group = format!("getent group {title} >/dev/null || groupadd --system {title}");
    let make_user = format!(
        "id {title} >/dev/null 2>&1 || useradd --system --gid {title} --home-dir /nonexistent --shell /usr/sbin/nologin {title}"
    );
    shell_link.launch(&format!("{make_group}; {make_user}"))
}

fn write_unit(repo_root: &Path, shell_link: &RemoteShellLink, svc: Channel) -> Result<(), String> {
    let text = systemd_text(
        repo_root,
        svc,
        &master_bin(),
        &etc("server-cert.pem"),
        &etc("server-key.pem"),
        SERVICE_ACCOUNT,
        SERVICE_ACCOUNT,
    )?;
    let cmd_text = format!("cat > '/etc/systemd/system/{}'", svc.systemd_unit());
    shell_link.pour(&cmd_text, &text).map(drop)
}

fn start_unit(shell_link: &RemoteShellLink, svc: Channel) -> Result<(), String> {
    let (port, unit) = (svc.port(), svc.systemd_unit());
    let open_firewall =
        format!("if command -v ufw >/dev/null && ufw status | grep -q '^Status: active'; then ufw allow {port}/udp; fi");
    let enable = format!("systemctl daemon-reload && systemctl enable --now '{unit}'");
    shell_link.launch(&format!("{open_firewall}; {enable}"))
}

/// What the local side needs before it can build and upload.
const LOCAL_TOOLS: [&str; 3] = ["cargo", "rsync", "ssh"];

fn install(repo_root: &Path, dot_conf: &DotVars, cli_words: &MasterOpts) -> Result<(), String> {
    demand_host_programs(&LOCAL_TOOLS)?;
    let Job { shell_link, svc } = &cli_words.job;
    let ca = ready_ca(dot_conf, cli_words)?;
    ensure_service_account(shell_link)?;
    upload_master(repo_root, dot_conf, shell_link)?;
    upload_identity(shell_link, &ca)?;
    write_unit(repo_root, shell_link, *svc)?;
    start_unit(shell_link, *svc)?;
    report(&cli_words.job)?;
    hand_out(&cli_words.job, &ca);
    Ok(())
}

fn restart_script(svc: Channel) -> String {
    ["systemctl restart '", svc.systemd_unit(), "'"].concat()
}

fn update(repo_root: &Path, dot_conf: &DotVars, cli_words: &MasterOpts) -> Result<(), String> {
    demand_host_programs(&LOCAL_TOOLS)?;
    let job = &cli_words.job;
    upload_master(repo_root, dot_conf, &job.shell_link)?;
    job.shell_link.launch(&restart_script(job.svc))?;
    report(job)
}

/// The shell test that is true while another channel's unit file is installed.
fn other_unit_exists(svc: Channel) -> String {
    let others = Channel::ALL.into_iter().filter(|other| *other != svc);
    let tests: Vec<String> = others
        .map(|other| format!("[ -e '/etc/systemd/system/{}' ]", other.systemd_unit()))
        .collect();
    tests.join(" || ")
}

/// Both channels run the one binary, so it goes only with the last unit. The
/// identity under /etc/bo2mp and the local CA stay.
fn uninstall(cli_words: &MasterOpts) -> Result<(), String> {
    let Job { shell_link, svc } = &cli_words.job;
    let unit = svc.systemd_unit();
    let other_unit_exists = other_unit_exists(*svc);
    let bin = master_bin();
    shell_link.launch(&format!(
        "systemctl disable --now '{unit}' 2>/dev/null || true; \
         rm -f '/etc/systemd/system/{unit}'; systemctl daemon-reload; \
         if ! {{ {other_unit_exists}; }}; then rm -f '{bin}'; rmdir '{LIB_DIR}' 2>/dev/null || true; fi"
    ))?;
    let dest = shell_link.dest();
    println!("master: {unit} removed from {dest}; {ETC_DIR} and the local CA were kept");
    Ok(())
}

const VERBS: [(&str, fn(&Path, &DotVars, &MasterOpts) -> Result<(), String>); 5] = [
    ("status", |_, _, cli_words| report(&cli_words.job)),
    ("logs", |_, _, cli_words| {
        journal(&cli_words.job.shell_link, cli_words.job.svc, &cli_words.from_time.0)
    }),
    ("uninstall", |_, _, cli_words| uninstall(cli_words)),
    ("install", install),
    ("update", update),
];

pub fn dispatch_subcommand(repo_root: &Path, dot_conf: &DotVars, cli_words: &[String]) -> Result<(), String> {
    let usage = "usage: cargo xtask master <install|update|status|logs|uninstall> \
                 [--channel prod|dev] [--since 2h] [--ca DIR] user@host";
    let Some((verb, rest)) = cli_words.split_first() else {
        return Err(usage.to_owned());
    };
    let parsed = parse_args(rest).map_err(|why| format!("{why}\n{usage}"))?;
    match VERBS.iter().find(|(title, _)| *title == verb.as_str()) {
        Some((_, run_verb)) => run_verb(repo_root, dot_conf, &parsed),
        None => Err(format!("unknown verb {verb:?}\n{usage}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strings(cli_words: &[&str]) -> Vec<String> {
        cli_words.iter().map(|arg| arg.to_string()).collect()
    }

    #[test]
    fn since_is_a_count_and_a_journal_unit() {
        for good in ["2h", "30min", "1day", "90s", "2weeks", "15m"] {
            assert!(validate_log_window(good).is_ok(), "{good} refused");
        }
        for bad in ["", "h", "2", "2 h", "2mins", "2h;reboot", "-2h", "2h'"] {
            assert!(validate_log_window(bad).is_err(), "{bad} accepted");
        }
    }

    #[test]
    fn options_go_anywhere_and_default_to_prod() {
        let cli_words = parse_args(&strings(&["root@203.0.113.7"])).unwrap();
        assert_eq!(cli_words.job.svc, Channel::Prod);
        assert_eq!(cli_words.from_time.0, DEFAULT_SINCE);
        assert!(cli_words.ca_dir.is_none());

        let cli_words = parse_args(&strings(&[
            "--since",
            "30min",
            "root@203.0.113.7",
            "--channel",
            "dev",
            "--ca",
            "/keys",
        ]))
        .unwrap();
        assert_eq!(cli_words.job.svc, Channel::PreReleaseBuild);
        assert_eq!(cli_words.from_time.0, "30min");
        assert_eq!(cli_words.job.shell_link.host_name(), "203.0.113.7");
        assert_eq!(cli_words.ca_dir, Some(PathBuf::from("/keys")));
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

    #[test]
    fn remote_commands_are_built_from_the_channel() {
        assert_eq!(
            journal_script(Channel::PreReleaseBuild, "2h"),
            "journalctl -u 'bo2mp-master-dev.service' --since '-2h' --no-pager"
        );
        assert_eq!(
            restart_script(Channel::Prod),
            "systemctl restart 'bo2mp-master.service'"
        );
        assert_eq!(
            other_unit_exists(Channel::Prod),
            "[ -e '/etc/systemd/system/bo2mp-master-dev.service' ]"
        );
        assert_eq!(
            other_unit_exists(Channel::PreReleaseBuild),
            "[ -e '/etc/systemd/system/bo2mp-master.service' ]"
        );
    }

    #[test]
    fn players_get_the_address_the_label_and_the_ca() {
        let notice = trust_notice("203.0.113.7", Channel::PreReleaseBuild, Path::new("/ca/bo2mp-ca.pem"));
        assert_eq!(notice.len(), 6);
        assert_eq!(notice[0], "");
        assert!(notice[1].starts_with("For your players: "));
        assert!(notice[1].ends_with("bo2mp-ca.pem"));
        assert_eq!(notice[2], "  BO2MP_MASTER_ADDR=203.0.113.7:4434");
        assert_eq!(notice[3], "  BO2MP_MASTER_SERVER_NAME=bo2mp-dev");
        assert_eq!(notice[4], "  BO2MP_MASTER_CA_CERT=/path/to/bo2mp-ca.pem");
        assert!(notice[5].contains("`make menu`"));
    }

    #[test]
    fn run_cli_names_usage_for_every_kind_of_refusal() {
        let dot_conf = DotVars::parse("");
        let repo_root = Path::new(".");
        let usage_tail = "usage: cargo xtask master <install|update|status|logs|uninstall> [--channel prod|dev] [--since 2h] [--ca DIR] user@host";
        assert_eq!(dispatch_subcommand(repo_root, &dot_conf, &[]).unwrap_err(), usage_tail);
        let bad_option = dispatch_subcommand(repo_root, &dot_conf, &strings(&["status", "--nope"])).unwrap_err();
        assert_eq!(bad_option, format!("unknown option --nope\n{usage_tail}"));
        let unknown = dispatch_subcommand(repo_root, &dot_conf, &strings(&["restart", "root@h"])).unwrap_err();
        assert_eq!(unknown, format!("unknown verb \"restart\"\n{usage_tail}"));
    }

    #[test]
    fn the_since_window_is_checked_after_the_words_are_read() {
        let err = parse_args(&strings(&["--since", "2x", "root@h"])).err().unwrap();
        assert!(err.starts_with("--since wants a number and a unit"), "{err}");
        let err = parse_args(&strings(&["--ca"])).err().unwrap();
        assert_eq!(err, "--ca needs a directory");
        let chosen = parse_args(&strings(&["root@h"])).map(|cli_words| cli_words.job.svc);
        assert_eq!(chosen, Ok(Channel::Prod));
    }
}
