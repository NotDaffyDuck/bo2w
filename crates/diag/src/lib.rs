mod alloc_count;
pub mod gap;
pub mod wgsl_dump;

pub use alloc_count::{
    ProcessAllocationStats, ProcessCountingAllocator, counting_enabled,
    process_allocation_saturated, process_allocation_slots_used, process_allocations,
    process_live_heap_bytes, release_freed_heap,
};

use std::{
    fs::{File, OpenOptions},
    io::{BufWriter, Write},
    path::{Path, PathBuf},
    sync::{Mutex, OnceLock, mpsc},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

static SINK: OnceLock<Mutex<DiagState>> = OnceLock::new();
static START: OnceLock<Instant> = OnceLock::new();

/// How often a `LineFile`'s thread puts its lines on the disk.
const WRITE_EVERY: Duration = Duration::from_millis(250);


enum LineOut {
    Line(String),
    Flush(mpsc::SyncSender<()>),
}

/// Every open `LineFile`'s mailbox, so `flush` empties them all at exit.
static LINE_FILES: Mutex<Vec<mpsc::Sender<LineOut>>> = Mutex::new(Vec::new());

/// A test aid's own text file (a per-tick trace), written by a thread of its
/// own like the log: the game thread hands the line over and never waits on
/// the disk. A trace written in place held his game 8-14 s at a time when D:
/// was busy (10-15). `flush` puts every line sent so far on the disk.
pub struct LineFile(mpsc::Sender<LineOut>);

impl LineFile {
    pub fn create(path: &Path) -> Option<LineFile> {
        let file = BufWriter::with_capacity(1 << 16, File::create(path).ok()?);
        let (tx, rx) = mpsc::channel();
        std::thread::Builder::new()
            .name("diag-linefile".into())
            .spawn(move || line_writer(rx, file))
            .ok()?;
        if let Ok(mut all) = LINE_FILES.lock() {
            all.push(tx.clone());
        }
        Some(LineFile(tx))
    }

    pub fn line(&self, text: String) {
        let _ = self.0.send(LineOut::Line(text));
    }
}

fn line_writer(rx: mpsc::Receiver<LineOut>, mut file: BufWriter<File>) {
    loop {
        match rx.recv_timeout(WRITE_EVERY) {
            Ok(LineOut::Line(text)) => {
                let _ = writeln!(file, "{text}");
            }
            Ok(LineOut::Flush(done)) => {
                let _ = file.flush();
                let _ = done.send(());
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {
                let _ = file.flush();
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                let _ = file.flush();
                return;
            }
        }
    }
}

fn flush_line_files(wait: Duration) {
    let senders = LINE_FILES.lock().map(|all| all.clone()).unwrap_or_default();
    for tx in senders {
        let (done, back) = mpsc::sync_channel(1);
        if tx.send(LineOut::Flush(done)).is_ok() {
            let _ = back.recv_timeout(wait);
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Channel {
    Launch,
    Zone,
    World,
    Fpv,
    Input,
    Sim,
    Net,
    Ui,
    Audio,
    Console,
}

impl Channel {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Launch => "launch",
            Self::Zone => "zone",
            Self::World => "world",
            Self::Fpv => "fpv",
            Self::Input => "input",
            Self::Sim => "sim",
            Self::Net => "net",
            Self::Ui => "ui",
            Self::Audio => "audio",
            Self::Console => "console",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Level {
    Error = 0,
    Warn = 1,
    Info = 2,
    Debug = 3,
}

impl Level {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Error => "error",
            Self::Warn => "warn",
            Self::Info => "info",
            Self::Debug => "debug",
        }
    }

    fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "error" => Some(Self::Error),
            "warn" | "warning" => Some(Self::Warn),
            "info" => Some(Self::Info),
            "debug" => Some(Self::Debug),
            _ => None,
        }
    }
}

struct DiagState {
    file: Option<File>,
    file_path: PathBuf,

    latest: Option<PathBuf>,
    traces: Option<File>,
    traces_path: Option<PathBuf>,
    stderr_threshold: Level,
    file_threshold: Level,

    last_key: Option<(Channel, Level, String)>,
    last_count: u32,

    started: Instant,
}

pub const LATEST_LOG_NAME: &str = "latest.log";

fn link_latest(path: &Path) -> Option<PathBuf> {
    let dir = path.parent()?;
    let name = path.file_name()?;
    if name == LATEST_LOG_NAME {
        return None;
    }
    let link = dir.join(LATEST_LOG_NAME);
    #[cfg(unix)]
    {
        match std::fs::symlink_metadata(&link) {
            Ok(md) if md.is_symlink() => std::fs::remove_file(&link).ok()?,
            Ok(_) => return None,
            Err(_) => {}
        }
        std::os::unix::fs::symlink(name, &link).ok()?;
        Some(link)
    }
    #[cfg(not(unix))]
    {
        // A symlink needs a privilege Windows withholds by default; a hard link does not.
        match std::fs::remove_file(&link) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => return None,
        }
        std::fs::hard_link(path, &link).ok()?;
        Some(link)
    }
}

pub fn init_log(artifacts_root: &Path) -> PathBuf {
    let _ = START.set(Instant::now());
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);

    let path = std::env::var_os("IW4L_LOG")
        .map(PathBuf::from)
        .unwrap_or_else(|| artifacts_root.join("logs").join(format!("{stamp}.log")));
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .ok();

    let (traces, traces_path) = match std::env::var_os("IW4L_TRACES_DIR") {
        Some(dir) => {
            let dir = PathBuf::from(dir);
            let _ = std::fs::create_dir_all(&dir);
            let tp = dir.join(format!("{stamp}.jsonl"));
            let f = OpenOptions::new().create(true).append(true).open(&tp).ok();
            (f, Some(tp))
        }
        None => (None, None),
    };

    let bump = std::env::var("IW4L_LOG_LEVEL")
        .ok()
        .and_then(|s| Level::parse(&s));
    let stderr_threshold = bump.unwrap_or(Level::Error);
    let file_threshold = bump.unwrap_or(Level::Info);

    let started = START.get().copied().unwrap_or_else(Instant::now);
    let mut state = DiagState {
        file,
        file_path: path.clone(),
        latest: link_latest(&path),
        traces,
        traces_path,
        stderr_threshold,
        file_threshold,
        last_key: None,
        last_count: 0,
        started,
    };

    if state.traces.is_some() {
        let zone = std::env::var("IW4L_ZONE").unwrap_or_default();
        let role = std::env::var("IW4L_ROLE").unwrap_or_else(|_| "Listen".into());
        let games = std::env::var("IW4L_GAMES").unwrap_or_default();
        let git = option_env!("IW4L_GIT_HASH").unwrap_or("unknown");
        let header = serde_json::json!({
            "t": 0,
            "ch": "launch",
            "lvl": "info",
            "msg": "session",
            "ev": "session",
            "f": {
                "format": "iw4l-traces-v1",
                "zone": zone,
                "role": role,
                "IW4L_GAMES": games,
                "git": git,
            }
        });
        if let Some(f) = state.traces.as_mut() {
            let _ = writeln!(f, "{header}");
            let _ = f.flush();
        }
    }

    let _ = SINK.set(Mutex::new(state));
    path
}

pub fn latest_log_path() -> Option<PathBuf> {
    SINK.get()
        .and_then(|s| s.lock().ok())
        .and_then(|g| g.latest.clone())
}

pub fn exit_launch_error(message: &str) -> ! {
    write_event(Channel::Launch, Level::Error, message, None, None);
    if let Some(sink) = SINK.get()
        && let Ok(mut state) = sink.lock()
    {
        if let Some(file) = state.file.as_mut() {
            let _ = file.flush();
            eprintln!("log: {}", state.file_path.display());
        } else {
            eprintln!("log unavailable: {}", state.file_path.display());
        }
    }
    #[cfg(windows)]
    {
        use std::io::IsTerminal;
        if std::io::stdin().is_terminal() {
            eprintln!("\nPress Enter to close.");
            let _ = std::io::stderr().flush();
            let _ = std::io::stdin().read_line(&mut String::new());
        }
    }
    std::process::exit(2);
}

pub fn traces_path() -> Option<PathBuf> {
    SINK.get()
        .and_then(|s| s.lock().ok())
        .and_then(|g| g.traces_path.clone())
}

fn now_ms(started: Instant) -> u64 {
    started.elapsed().as_millis() as u64
}

fn flush_collapsed(state: &mut DiagState) {
    if state.last_count <= 1 {
        return;
    }
    let Some((ch, lvl, msg)) = state.last_key.clone() else {
        return;
    };
    let n = state.last_count;
    let line = format!("{}  {}  ↑ repeated {n}× — {msg}", ch.as_str(), lvl.as_str());
    emit_raw(state, ch, lvl, &line, Some(n), None, None, true);
    state.last_count = 1;
}

fn emit_raw(
    state: &mut DiagState,
    ch: Channel,
    lvl: Level,
    msg: &str,
    n: Option<u32>,
    ev: Option<&str>,
    fields: Option<&serde_json::Value>,
    collapsed_banner: bool,
) {
    let text = if collapsed_banner {
        msg.to_owned()
    } else {
        format!("{}  {}  {msg}", ch.as_str(), lvl.as_str())
    };

    if lvl <= state.stderr_threshold {
        eprintln!("{text}");
    }
    if lvl <= state.file_threshold
        && let Some(file) = state.file.as_mut()
    {
        let _ = writeln!(file, "{text}");
        let _ = file.flush();
    }
    if let Some(file) = state.traces.as_mut() {
        let t = now_ms(state.started);
        let mut obj = serde_json::json!({
            "t": t,
            "ch": ch.as_str(),
            "lvl": lvl.as_str(),
            "msg": if collapsed_banner { msg } else { msg },
        });
        if let Some(ev) = ev {
            obj["ev"] = serde_json::Value::String(ev.to_owned());
        }
        if let Some(n) = n.filter(|&n| n > 1) {
            obj["n"] = serde_json::Value::from(n);
        }
        if let Some(f) = fields {
            obj["f"] = f.clone();
        }

        if !collapsed_banner {
            obj["msg"] = serde_json::Value::String(msg.to_owned());
        }
        let _ = writeln!(file, "{obj}");
        let _ = file.flush();
    }
}

pub fn write_event(
    ch: Channel,
    lvl: Level,
    msg: &str,
    ev: Option<&str>,
    fields: Option<&serde_json::Value>,
) {
    let Some(sink) = SINK.get() else {
        eprintln!("{}  {}  {msg}", ch.as_str(), lvl.as_str());
        return;
    };
    let Ok(mut state) = sink.lock() else {
        return;
    };

    let key = (ch, lvl, msg.to_owned());
    if state.last_key.as_ref() == Some(&key) {
        state.last_count = state.last_count.saturating_add(1);

        if state.last_count.is_multiple_of(64) {
            let t = now_ms(state.started);
            let n = state.last_count;
            let obj = serde_json::json!({
                "t": t,
                "ch": ch.as_str(),
                "lvl": lvl.as_str(),
                "msg": msg,
                "n": n,
            });
            if let Some(file) = state.traces.as_mut() {
                let _ = writeln!(file, "{obj}");
            }
        }
        return;
    }

    flush_collapsed(&mut state);
    state.last_key = Some(key);
    state.last_count = 1;
    emit_raw(&mut state, ch, lvl, msg, None, ev, fields, false);
}

pub fn flush() {
    if let Some(sink) = SINK.get()
        && let Ok(mut state) = sink.lock()
    {
        flush_collapsed(&mut state);
    }
    flush_line_files(Duration::from_secs(2));
}

pub fn announce_log_stdout(path: &Path, latest: Option<&Path>) {
    let line = match latest {
        Some(latest) => format!("log: {} ({})", path.display(), latest.display()),
        None => format!("log: {}", path.display()),
    };
    announce_stdout(&line);
}

pub fn process_elapsed_ns() -> u128 {
    START
        .get()
        .map(|start| start.elapsed().as_nanos())
        .unwrap_or(0)
}

pub fn lifecycle_boundary(name: &str, detail: &str) {
    controller_line("lifecycle", name, detail);
}

pub fn script_boundary(name: &str, detail: &str) {
    controller_line("gsc", name, detail);
}

fn controller_line(prefix: &str, name: &str, detail: &str) {
    let line = format!(
        "{prefix}: {name} pid={} ns={}{detail}",
        std::process::id(),
        process_elapsed_ns()
    );
    write_event(Channel::Launch, Level::Info, &line, None, None);
    announce_stdout(&line);
}

pub fn announce_stdout(line: &str) {
    let _ = writeln!(std::io::stdout(), "{line}");
    let _ = std::io::stdout().flush();
}

pub fn write_line(category: &str, message: &str) {
    let ch = match category {
        "launch" => Channel::Launch,
        "zone" => Channel::Zone,
        "world" => Channel::World,
        "fpv" => Channel::Fpv,
        "input" => Channel::Input,
        "sim" | "spawn" | "score" => Channel::Sim,
        "net" => Channel::Net,
        "ui" | "menu" | "hud" => Channel::Ui,
        "audio" => Channel::Audio,
        "console" => Channel::Console,
        _ => Channel::Launch,
    };
    let msg = if category.is_empty() {
        message.to_owned()
    } else if message.starts_with(category) {
        message.to_owned()
    } else {
        format!("{category}: {message}")
    };
    write_event(ch, Level::Info, &msg, None, None);
}

#[macro_export]
macro_rules! log_line {
    ($cat:ident, $($arg:tt)*) => {{
        $crate::write_line(stringify!($cat), &format!($($arg)*));
    }};
    ($($arg:tt)*) => {{
        $crate::write_line("", &format!($($arg)*));
    }};
}

#[macro_export]
macro_rules! error {
    ($ch:ident, $($arg:tt)*) => {{
        $crate::write_event(
            $crate::Channel::$ch,
            $crate::Level::Error,
            &format!($($arg)*),
            None,
            None,
        );
    }};
}

#[macro_export]
macro_rules! warn {
    ($ch:ident, $($arg:tt)*) => {{
        $crate::write_event(
            $crate::Channel::$ch,
            $crate::Level::Warn,
            &format!($($arg)*),
            None,
            None,
        );
    }};
}

#[macro_export]
macro_rules! info {
    ($ch:ident, $($arg:tt)*) => {{
        $crate::write_event(
            $crate::Channel::$ch,
            $crate::Level::Info,
            &format!($($arg)*),
            None,
            None,
        );
    }};
}

#[macro_export]
macro_rules! debug {
    ($ch:ident, $($arg:tt)*) => {{
        $crate::write_event(
            $crate::Channel::$ch,
            $crate::Level::Debug,
            &format!($($arg)*),
            None,
            None,
        );
    }};
}

#[macro_export]
macro_rules! event {
    ($ch:ident, $lvl:ident, $ev:literal, $($arg:tt)*) => {{
        $crate::write_event(
            $crate::Channel::$ch,
            $crate::Level::$lvl,
            &format!($($arg)*),
            Some($ev),
            None,
        );
    }};
}
