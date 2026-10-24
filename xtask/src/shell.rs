//! Running local tools, and reaching a host over ssh and rsync without ever
//! stopping at a password prompt.

use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::Instant;

pub type Res<T> = Result<T, String>;

/// Fail fast instead of hanging: no prompts, and a dead link is noticed in
/// about fifteen seconds.
const SSH_OPTIONS: [&str; 8] = [
    "-o",
    "BatchMode=yes",
    "-o",
    "ConnectTimeout=10",
    "-o",
    "ServerAliveInterval=5",
    "-o",
    "ServerAliveCountMax=3",
];

pub fn tool_on_path(tool: &str) -> bool {
    let Some(path) = std::env::var_os("PATH") else {
        return false;
    };
    std::env::split_paths(&path).any(|dir| {
        let candidate = dir.join(tool);
        candidate.is_file() || (cfg!(windows) && candidate.with_extension("exe").is_file())
    })
}

pub fn require_tools(tools: &[&str]) -> Res<()> {
    match tools.iter().find(|tool| !tool_on_path(tool)) {
        Some(missing) => Err(format!("missing tool: {missing}")),
        None => Ok(()),
    }
}

fn program(command: &Command) -> String {
    command.get_program().to_string_lossy().into_owned()
}

/// Output goes straight to the terminal; a non-zero exit is an error.
pub fn run(command: &mut Command) -> Res<()> {
    let status = command
        .status()
        .map_err(|error| format!("{}: {error}", program(command)))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("{}: {status}", program(command)))
    }
}

/// stdout as text; stderr still reaches the terminal.
pub fn capture(command: &mut Command) -> Res<String> {
    let output = command
        .stderr(Stdio::inherit())
        .output()
        .map_err(|error| format!("{}: {error}", program(command)))?;
    if !output.status.success() {
        return Err(format!("{}: {}", program(command), output.status));
    }
    String::from_utf8(output.stdout)
        .map_err(|_| format!("{}: output is not UTF-8", program(command)))
}

/// Like [`capture`], with `input` on stdin.
pub fn feed(command: &mut Command, input: &str) -> Res<String> {
    let mut child = command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .map_err(|error| format!("{}: {error}", program(command)))?;
    let mut stdin = child.stdin.take().ok_or("child has no stdin")?;
    let input = input.to_string();
    // A separate writer, so a child that answers while it reads cannot fill
    // its stdout pipe and stall against us.
    let writer = std::thread::spawn(move || stdin.write_all(input.as_bytes()));
    let output = child
        .wait_with_output()
        .map_err(|error| format!("{}: {error}", program(command)))?;
    let wrote = writer
        .join()
        .map_err(|_| "stdin writer panicked".to_string())?;
    if !output.status.success() {
        return Err(format!("{}: {}", program(command), output.status));
    }
    wrote.map_err(|error| format!("{}: writing stdin: {error}", program(command)))?;
    String::from_utf8(output.stdout)
        .map_err(|_| format!("{}: output is not UTF-8", program(command)))
}

/// Owner-only (`0o700`/`0o600`) or group-readable modes for keys and their
/// directory. Windows has no such bits; the files stay as created.
pub fn set_mode(path: &Path, mode: u32) -> Res<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode))
            .map_err(|error| format!("chmod {mode:o} {}: {error}", path.display()))
    }
    #[cfg(not(unix))]
    {
        let _ = (path, mode);
        Ok(())
    }
}

/// `name: start ...` now and `name: done elapsed=Ns ...` at the end, so a slow
/// step is visible in the log without a profiler.
pub struct Step {
    name: String,
    started: Instant,
}

impl Step {
    pub fn start(name: &str, detail: &str) -> Self {
        println!("{name}: start {detail}");
        Self {
            name: name.to_string(),
            started: Instant::now(),
        }
    }

    pub fn done(self, detail: &str) {
        println!(
            "{}: done elapsed={}s {detail}",
            self.name,
            self.started.elapsed().as_secs()
        );
    }
}

/// `user@host`, checked so neither half can smuggle an option or shell text
/// into the ssh and rsync command lines.
pub struct Ssh {
    user: String,
    host: String,
}

impl Ssh {
    pub fn new(target: &str) -> Res<Self> {
        let bad = || format!("expected user@host, got {target:?}");
        let (user, host) = target.split_once('@').ok_or_else(bad)?;
        let user_ok = user.starts_with(|c: char| c.is_ascii_alphanumeric())
            && user
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || "._-".contains(c));
        let host_ok = host.starts_with(|c: char| c.is_ascii_alphanumeric())
            && host
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || ".-".contains(c));
        if !(user_ok && host_ok) {
            return Err(bad());
        }
        Ok(Self {
            user: user.to_string(),
            host: host.to_string(),
        })
    }

    pub fn host(&self) -> &str {
        &self.host
    }

    pub fn target(&self) -> String {
        format!("{}@{}", self.user, self.host)
    }

    fn ssh(&self, script: &str, with_stdin: bool) -> Command {
        let mut command = Command::new("ssh");
        if !with_stdin {
            command.arg("-n");
        }
        command.args(SSH_OPTIONS).arg(self.target()).arg(script);
        command
    }

    pub fn run(&self, script: &str) -> Res<()> {
        run(&mut self.ssh(script, false))
    }

    /// `Ok(false)` when the script ran and failed; `Err` when ssh itself could
    /// not start.
    pub fn try_run(&self, script: &str) -> Res<bool> {
        self.ssh(script, false)
            .status()
            .map(|status| status.success())
            .map_err(|error| format!("ssh: {error}"))
    }

    pub fn capture(&self, script: &str) -> Res<String> {
        capture(&mut self.ssh(script, false))
    }

    pub fn feed(&self, script: &str, input: &str) -> Res<String> {
        feed(&mut self.ssh(script, true), input)
    }

    pub fn rsync(&self, extra: &[&str], local: &Path, remote: &str) -> Res<()> {
        let transport = format!("ssh {}", SSH_OPTIONS.join(" "));
        run(Command::new("rsync")
            .args(["-a", "--timeout=60", "-e", &transport])
            .args(extra)
            .arg(local)
            .arg(format!("{}:{remote}", self.target())))
    }
}

/// `BO2MP_DEPLOY_ROOT` is pasted into remote shell lines inside single quotes,
/// so it may only be a plain absolute path.
pub fn check_deploy_root(root: &str) -> Res<()> {
    let plain = root.starts_with('/')
        && root
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "._/-".contains(c))
        && !root.split('/').any(|part| part == "..");
    if plain {
        Ok(())
    } else {
        Err("BO2MP_DEPLOY_ROOT must be a simple absolute path".to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ssh_targets_are_plain_user_at_host() {
        let ssh = Ssh::new("deploy_1.x@relay-2.example.net").unwrap();
        assert_eq!(ssh.host(), "relay-2.example.net");
        assert_eq!(ssh.target(), "deploy_1.x@relay-2.example.net");
        assert_eq!(Ssh::new("root@203.0.113.7").unwrap().host(), "203.0.113.7");
        for bad in [
            "nohost",
            "@host",
            "user@",
            "-oProxyCommand@host",
            "user@-host",
            "us er@host",
            "user@host;rm",
            "user@host_name",
            "a@b@c",
        ] {
            assert!(Ssh::new(bad).is_err(), "{bad} accepted");
        }
    }

    #[test]
    fn deploy_root_is_a_plain_absolute_path() {
        for good in ["/srv/bo2mp", "/opt/bo2mp-release_2/v1.0"] {
            assert!(check_deploy_root(good).is_ok(), "{good} refused");
        }
        for bad in [
            "srv/bo2mp",
            "/srv/../etc",
            "/srv/bo 2",
            "/srv/'x'",
            "/srv/$HOME",
            "",
        ] {
            assert!(check_deploy_root(bad).is_err(), "{bad} accepted");
        }
    }

    #[test]
    fn a_tool_that_is_nowhere_is_reported_by_name() {
        assert!(!tool_on_path("xtask-no-such-tool-7f3a"));
        assert_eq!(
            require_tools(&["xtask-no-such-tool-7f3a"]).unwrap_err(),
            "missing tool: xtask-no-such-tool-7f3a"
        );
    }
}
