//! Running local tools, and reaching a host over ssh and rsync without ever
//! stopping at a password prompt.

use std::io::Write;
use std::path::Path;
use std::process::{Command, ExitStatus, Stdio};
use std::time::Instant;

/// Fail fast instead of hanging: no prompts, and a dead link is noticed in
/// about fifteen seconds.
const SSH_SETTINGS: [(&str, &str); 4] = [
    ("BatchMode", "yes"),
    ("ConnectTimeout", "10"),
    ("ServerAliveInterval", "5"),
    ("ServerAliveCountMax", "3"),
];

/// Each setting as the two words `-o Key=Value`.
fn ssh_options() -> Vec<String> {
    SSH_SETTINGS
        .iter()
        .flat_map(|(knob, val)| ["-o".to_string(), format!("{knob}={val}")])
        .collect()
}

fn found_in(base_dir: &Path, tool: &str) -> bool {
    let plain = base_dir.join(tool);
    let with_exe = cfg!(windows).then(|| plain.with_extension("exe"));
    std::iter::once(plain).chain(with_exe).any(|file| file.is_file())
}

pub fn on_path(tool: &str) -> bool {
    let dirs = std::env::var_os("PATH").unwrap_or_default();
    std::env::split_paths(&dirs).any(|base_dir| found_in(&base_dir, tool))
}

pub fn demand_host_programs(tools: &[&str]) -> Result<(), String> {
    let absent = tools.iter().find(|tool| !on_path(tool));
    absent.map_or(Ok(()), |tool| Err(format!("missing tool: {tool}")))
}

fn program(command: &Command) -> String {
    command.get_program().to_string_lossy().to_string()
}

/// `<program>: <what went wrong>`, the shape every error here takes.
fn blame(command: &Command, what: impl std::fmt::Display) -> String {
    format!("{}: {what}", program(command))
}

/// A non-zero exit becomes the error text `<program>: <status>`.
fn exited_ok(command: &Command, status: ExitStatus) -> Result<(), String> {
    status.success().then_some(()).ok_or_else(|| blame(command, status))
}

fn as_text(command: &Command, bytes: Vec<u8>) -> Result<String, String> {
    String::from_utf8(bytes).map_err(|_| blame(command, "output is not UTF-8"))
}

/// Output goes straight to the terminal; a non-zero exit is an error.
pub fn launch(command: &mut Command) -> Result<(), String> {
    let status = command.status();
    let status = status.map_err(|error| blame(command, error))?;
    exited_ok(command, status)
}

/// Runs `command` and hands back what it printed; its error output goes
/// straight to the console as usual.
pub fn grab(command: &mut Command) -> Result<String, String> {
    let output = command.stderr(Stdio::inherit()).output();
    let output = output.map_err(|error| blame(command, error))?;
    exited_ok(command, output.status).and_then(|()| as_text(command, output.stdout))
}

/// Like [`capture`], with `input` on stdin.
pub fn pour(command: &mut Command, stdin_text: &str) -> Result<String, String> {
    command.stdin(Stdio::piped()).stderr(Stdio::inherit());
    command.stdout(Stdio::piped());
    let mut child = command.spawn().map_err(|error| blame(command, error))?;
    let mut pipe = child.stdin.take().ok_or("child has no stdin")?;
    // The writer runs beside the wait, so a child that answers while it reads
    // cannot fill its stdout pipe and stall against us.
    let (output, wrote) = std::thread::scope(|scope| {
        let writer = scope.spawn(move || pipe.write_all(stdin_text.as_bytes()));
        (child.wait_with_output(), writer.join())
    });
    let output = output.map_err(|error| blame(command, error))?;
    let wrote = wrote.map_err(|_| "stdin writer panicked".to_string())?;
    exited_ok(command, output.status)?;
    wrote.map_err(|error| blame(command, format_args!("writing stdin: {error}")))?;
    as_text(command, output.stdout)
}

/// Owner-only (`0o700`/`0o600`) or group-readable modes for keys and their
/// directory. Windows has no such bits; the files stay as created.
pub fn set_mode(file_at: &Path, mode: u32) -> Result<(), String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let applied = std::fs::set_permissions(file_at, PermissionsExt::from_mode(mode));
        applied.map_err(|e| format!("chmod {mode:o} {}: {e}", file_at.display()))
    }
    #[cfg(not(unix))]
    {
        let _ = (file_at, mode);
        Ok(())
    }
}

/// `name: start ...` now and `name: done elapsed=Ns ...` at the end, so a slow
/// step is visible in the log without a profiler.
pub struct Phase {
    title: Box<str>,
    begun_at: Instant,
}

impl Phase {
    pub fn start(title: &str, note_text: &str) -> Self {
        let line = format!("{title}: start {note_text}");
        println!("{line}");
        Self {
            title: Box::from(title),
            begun_at: std::time::Instant::now(),
        }
    }

    pub fn wrap(self, note_text: &str) {
        let Self { title, begun_at } = self;
        let secs = begun_at.elapsed().as_secs();
        println!("{title}: done elapsed={secs}s {note_text}");
    }
}

/// `user@host`, checked so neither half can smuggle an option or shell text
/// into the ssh and rsync command lines.
pub struct RemoteShellLink {
    /// The whole `user@host`; `at` is the index of its one `@`.
    joined: String,
    at: usize,
}

/// Starts alphanumeric and continues with alphanumerics or one of `extra`.
fn plain_word(word: &str, more_flags: &str) -> bool {
    let fits = |(place, c): (usize, char)| {
        c.is_ascii_alphanumeric() || (place > 0 && more_flags.contains(c))
    };
    !word.is_empty() && word.char_indices().all(fits)
}

impl RemoteShellLink {
    pub fn new(dest: &str) -> Result<Self, String> {
        let parts = dest.split_once('@');
        match parts.filter(|(user, host_name)| plain_word(user, "._-") && plain_word(host_name, ".-")) {
            Some((user, _)) => Ok(Self {
                joined: dest.to_string(),
                at: user.len(),
            }),
            None => Err(format!("expected user@host, got {dest:?}")),
        }
    }

    pub fn host_name(&self) -> &str {
        &self.joined[self.at + 1..]
    }

    pub fn dest(&self) -> String {
        self.joined.clone()
    }

    fn shell_link(&self, cmd_text: &str, with_stdin: bool) -> Command {
        let no_stdin = (!with_stdin).then(|| "-n".to_string());
        let words = no_stdin
            .into_iter()
            .chain(ssh_options())
            .chain([self.joined.clone(), cmd_text.to_string()]);
        let mut command = Command::new("ssh");
        command.args(words);
        command
    }

    pub fn launch(&self, cmd_text: &str) -> Result<(), String> {
        launch(&mut self.shell_link(cmd_text, false))
    }

    /// `Ok(false)` when the script ran and failed; `Err` when ssh itself could
    /// not start.
    pub fn try_launch(&self, cmd_text: &str) -> Result<bool, String> {
        match self.shell_link(cmd_text, false).status() {
            Ok(wrap) => Ok(wrap.success()),
            Err(error) => Err(format!("ssh: {error}")),
        }
    }

    pub fn grab(&self, cmd_text: &str) -> Result<String, String> {
        grab(&mut self.shell_link(cmd_text, false))
    }

    pub fn pour(&self, cmd_text: &str, stdin_text: &str) -> Result<String, String> {
        pour(&mut self.shell_link(cmd_text, true), stdin_text)
    }

    pub fn mirror_up(&self, more_flags: &[&str], near: &Path, far_end: &str) -> Result<(), String> {
        let flags = SSH_SETTINGS.map(|(knob, val)| format!("-o {knob}={val}"));
        let mut command = Command::new("rsync");
        command
            .args(["-a", "--timeout=60", "-e", &format!("ssh {}", flags.join(" "))])
            .args(more_flags);
        command.arg(near);
        command.arg(format!("{}:{far_end}", self.joined));
        launch(&mut command)
    }
}

/// `BO2MP_DEPLOY_ROOT` is pasted into remote shell lines inside single quotes,
/// so it may only be a plain absolute path.
pub fn validate_install_dir(repo_root: &str) -> Result<(), String> {
    let plain_char = |c: char| c.is_ascii_alphanumeric() || ".-_/".contains(c);
    let rooted = repo_root.strip_prefix('/').is_some();
    let simple = rooted
        && repo_root.chars().all(plain_char)
        && !repo_root.split('/').any(|partial| partial == "..");
    if simple {
        Ok(())
    } else {
        Err("BO2MP_DEPLOY_ROOT must be a simple absolute path".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every case must give the verdict `want`; the failure names the case.
    fn all_verdicts(cases: &[&str], hoped: bool, check: impl Fn(&str) -> bool) {
        for case in cases {
            assert_eq!(check(case), hoped, "{case:?} got the wrong verdict");
        }
    }

    #[test]
    fn ssh_targets_are_plain_user_at_host() {
        let shell_link = RemoteShellLink::new("deploy_1.x@relay-2.example.net").unwrap();
        assert_eq!(shell_link.host_name(), "relay-2.example.net");
        assert_eq!(shell_link.dest(), "deploy_1.x@relay-2.example.net");
        assert_eq!(RemoteShellLink::new("root@203.0.113.7").unwrap().host_name(), "203.0.113.7");
        let refused = [
            "nohost",
            "@host",
            "user@",
            "-oProxyCommand@host",
            "user@-host",
            "us er@host",
            "user@host;rm",
            "user@host_name",
            "a@b@c",
        ];
        all_verdicts(&refused, false, |t| RemoteShellLink::new(t).is_ok());
    }

    #[test]
    fn deploy_root_is_a_plain_absolute_path() {
        let ok = |repo_root: &str| validate_install_dir(repo_root).is_ok();
        all_verdicts(&["/srv/bo2mp", "/opt/bo2mp-release_2/v1.0"], true, ok);
        let refused = ["srv/bo2mp", "/srv/../etc", "/srv/bo 2", "/srv/'x'", "/srv/$HOME", ""];
        all_verdicts(&refused, false, ok);
        assert_eq!(
            validate_install_dir("x").unwrap_err(),
            "BO2MP_DEPLOY_ROOT must be a simple absolute path"
        );
    }

    #[test]
    fn ssh_options_are_flag_value_pairs() {
        let words = ssh_options();
        assert_eq!(words.len(), 2 * SSH_SETTINGS.len());
        assert!(words.chunks(2).all(|pair| pair[0] == "-o" && pair[1].contains('=')));
        assert!(words.contains(&"BatchMode=yes".to_string()));
    }

    #[test]
    fn a_tool_that_is_nowhere_is_reported_by_name() {
        let ghost = "xtask-no-such-tool-7f3a";
        assert!(!on_path(ghost));
        assert_eq!(
            demand_host_programs(&[ghost]).unwrap_err(),
            format!("missing tool: {ghost}")
        );
    }
}
