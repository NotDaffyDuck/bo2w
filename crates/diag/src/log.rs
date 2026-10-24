//! The log: one line per call, on a channel and at a level, put on the disk
//! by a thread of its own. A frame that logs only hands the line over; the
//! writer waits on the disk, so a busy disk never holds up the game.

use std::fmt;
use std::fs::{File, OpenOptions};
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Mutex, OnceLock, mpsc};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// The folder bo2mp writes its own output under (logs, caches, runs,
/// screenshots, demos), relative to where it runs.
pub const ARTIFACTS_DIR: &str = "bo2mp-artifacts";

/// The name beside each new log that always opens the newest one.
const LATEST_LOG_NAME: &str = "latest.log";

const WRITER_NAME: &str = "diag-writer";

/// How long written lines may sit in memory before they go to the disk. A
/// warning or an error goes at once.
const BATCH: Duration = Duration::from_millis(250);

/// The part of the game a line is about: the first word of the line.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
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

/// How much a line matters, the most first. A place (stderr, the file)
/// takes the lines up to its own level.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Level {
    Error,
    Warn,
    Info,
    Debug,
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

    fn parse(text: &str) -> Option<Self> {
        match text.trim().to_ascii_lowercase().as_str() {
            "error" => Some(Self::Error),
            "warn" | "warning" => Some(Self::Warn),
            "info" => Some(Self::Info),
            "debug" => Some(Self::Debug),
            _ => None,
        }
    }
}

/// What the writer thread is asked to do, in the order asked.
enum Job {
    Log {
        text: String,
        stderr: bool,
        file: bool,
        at_once: bool,
    },
    OpenLog(File),
    OpenLines(usize, File),
    Line(usize, String),
    /// Put everything asked before this on the disk, then answer.
    Flush(mpsc::SyncSender<()>),
}

static JOBS: OnceLock<Option<mpsc::Sender<Job>>> = OnceLock::new();

/// The writer's mailbox, starting the writer on first use.
fn start_writer() -> Option<&'static mpsc::Sender<Job>> {
    JOBS.get_or_init(|| {
        let (jobs, inbox) = mpsc::channel();
        std::thread::Builder::new()
            .name(WRITER_NAME.into())
            .spawn(move || run_writer(inbox))
            .ok()?;
        Some(jobs)
    })
    .as_ref()
}

/// The writer's mailbox, if it has started.
fn writer() -> Option<&'static mpsc::Sender<Job>> {
    JOBS.get().and_then(Option::as_ref)
}

fn run_writer(inbox: mpsc::Receiver<Job>) {
    let mut log: Option<BufWriter<File>> = None;
    let mut line_files: Vec<Option<BufWriter<File>>> = Vec::new();
    let mut on_disk = Instant::now();
    loop {
        let job = inbox.recv_timeout(BATCH);
        let at_once = match job {
            Ok(Job::Log {
                text,
                stderr,
                file,
                at_once,
            }) => {
                // A failed write is dropped: the writer must outlive it.
                if stderr {
                    let _ = writeln!(std::io::stderr(), "{text}");
                }
                if file && let Some(log) = log.as_mut() {
                    let _ = writeln!(log, "{text}");
                }
                at_once
            }
            Ok(Job::OpenLog(file)) => {
                log = Some(BufWriter::with_capacity(1 << 16, file));
                false
            }
            Ok(Job::OpenLines(slot, file)) => {
                if line_files.len() <= slot {
                    line_files.resize_with(slot + 1, || None);
                }
                line_files[slot] = Some(BufWriter::with_capacity(1 << 16, file));
                false
            }
            Ok(Job::Line(slot, text)) => {
                if let Some(Some(file)) = line_files.get_mut(slot) {
                    let _ = writeln!(file, "{text}");
                }
                false
            }
            Ok(Job::Flush(answer)) => {
                put_on_disk(&mut log, &mut line_files);
                on_disk = Instant::now();
                let _ = answer.send(());
                continue;
            }
            Err(mpsc::RecvTimeoutError::Timeout) => true,
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                put_on_disk(&mut log, &mut line_files);
                return;
            }
        };
        if at_once || on_disk.elapsed() >= BATCH {
            put_on_disk(&mut log, &mut line_files);
            on_disk = Instant::now();
        }
    }
}

fn put_on_disk(log: &mut Option<BufWriter<File>>, line_files: &mut [Option<BufWriter<File>>]) {
    for file in log.iter_mut().chain(line_files.iter_mut().flatten()) {
        let _ = file.flush();
    }
}

/// Waits, at most `limit`, until every line handed over so far is on the
/// disk. The writer thread itself returns at once.
pub(crate) fn wait_for_disk(limit: Duration) {
    let Some(jobs) = writer() else {
        return;
    };
    if std::thread::current().name() == Some(WRITER_NAME) {
        return;
    }
    let (answer, heard) = mpsc::sync_channel(1);
    if jobs.send(Job::Flush(answer)).is_ok() {
        let _ = heard.recv_timeout(limit);
    }
}

/// Where the log goes, fixed by `init_log`.
struct Output {
    path: PathBuf,
    open: bool,
    latest: Option<PathBuf>,
    stderr_up_to: Level,
    file_up_to: Level,
}

impl Output {
    fn takes(&self, level: Level) -> bool {
        level <= self.stderr_up_to || (self.open && level <= self.file_up_to)
    }

    fn send(&self, level: Level, text: String) {
        let stderr = level <= self.stderr_up_to;
        let file = self.open && level <= self.file_up_to;
        match writer() {
            Some(jobs) => {
                let _ = jobs.send(Job::Log {
                    text,
                    stderr,
                    file,
                    at_once: level <= Level::Warn,
                });
            }
            None if stderr => eprintln!("{text}"),
            None => {}
        }
    }
}

static OUTPUT: OnceLock<Output> = OnceLock::new();
static STARTED: OnceLock<Instant> = OnceLock::new();
static REPEATS: Mutex<Repeats> = Mutex::new(Repeats::new());

/// Holds back a line that only repeats the one written before it, and
/// counts it. The count goes out as one note when a different line comes,
/// or at `flush`.
struct Repeats {
    last: Option<(Level, String)>,
    more: u32,
}

impl Repeats {
    const fn new() -> Self {
        Self {
            last: None,
            more: 0,
        }
    }

    /// Counts `line` when it repeats the last line written; true if it did.
    fn count(&mut self, line: &str) -> bool {
        let repeat = self.last.as_ref().is_some_and(|(_, last)| last == line);
        if repeat {
            self.more = self.more.saturating_add(1);
        }
        repeat
    }

    /// The note for the repeats counted since the last line went out.
    fn note(&mut self) -> Option<(Level, String)> {
        let more = std::mem::take(&mut self.more);
        let (level, last) = self.last.as_ref()?;
        (more > 0).then(|| (*level, format!("{last}  (+{more} more)")))
    }

    fn wrote(&mut self, level: Level, line: String) {
        self.last = Some((level, line));
    }
}

/// Opens the log: `BO2MP_LOG`, or `<artifacts_root>/logs/<unix seconds>.log`,
/// and points `latest.log` beside it at it. `BO2MP_LOG_LEVEL` (error, warn,
/// info, debug) sets what both stderr and the file take; without it stderr
/// takes errors and the file takes up to info. From here on a crash leaves
/// its reason in the log. Returns the log's path.
pub fn init_log(artifacts_root: &Path) -> PathBuf {
    if let Some(output) = OUTPUT.get() {
        return output.path.clone();
    }
    let _ = STARTED.set(Instant::now());
    let path = std::env::var_os("BO2MP_LOG")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            let secs = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_or(0, |since| since.as_secs());
            artifacts_root.join("logs").join(format!("{secs}.log"))
        });
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let file = OpenOptions::new().create(true).append(true).open(&path);
    let open = match (file, start_writer()) {
        (Ok(file), Some(jobs)) => jobs.send(Job::OpenLog(file)).is_ok(),
        _ => false,
    };
    let chosen = std::env::var("BO2MP_LOG_LEVEL")
        .ok()
        .and_then(|text| Level::parse(&text));
    let output = Output {
        latest: link_latest(&path),
        path,
        open,
        stderr_up_to: chosen.unwrap_or(Level::Error),
        file_up_to: chosen.unwrap_or(Level::Info),
    };
    let path = OUTPUT.get_or_init(|| output).path.clone();
    crate::crash::install(&path);
    path
}

/// Points `latest.log` beside `log` at it. Returns the link, or None when
/// none was made.
fn link_latest(log: &Path) -> Option<PathBuf> {
    let name = log.file_name()?;
    if name == LATEST_LOG_NAME {
        return None;
    }
    let link = log.with_file_name(LATEST_LOG_NAME);
    #[cfg(unix)]
    {
        // Only a link is replaced; a real file of that name is left alone.
        if let Ok(meta) = std::fs::symlink_metadata(&link) {
            if !meta.is_symlink() {
                return None;
            }
            std::fs::remove_file(&link).ok()?;
        }
        std::os::unix::fs::symlink(name, &link).ok()?;
    }
    #[cfg(not(unix))]
    {
        // Windows: a symbolic link needs a privilege most accounts do not
        // have, and a hard link does not.
        if let Err(error) = std::fs::remove_file(&link)
            && error.kind() != std::io::ErrorKind::NotFound
        {
            return None;
        }
        std::fs::hard_link(log, &link).ok()?;
    }
    Some(link)
}

/// The `latest.log` link `init_log` made, if it made one.
pub fn latest_log_path() -> Option<PathBuf> {
    OUTPUT.get()?.latest.clone()
}

/// Writes one line, `<channel>  <level>  <message>`. The `error!`, `warn!`,
/// `info!` and `event!` macros call it. Before `init_log` every line goes
/// to stderr.
#[doc(hidden)]
pub fn write_event(channel: Channel, level: Level, message: fmt::Arguments<'_>) {
    let Some(output) = OUTPUT.get() else {
        eprintln!("{}  {}  {message}", channel.as_str(), level.as_str());
        return;
    };
    if !output.takes(level) {
        return;
    }
    // Formatted before the lock: a value whose formatting logs must not
    // find it held.
    let line = format!("{}  {}  {message}", channel.as_str(), level.as_str());
    let Ok(mut repeats) = REPEATS.lock() else {
        return;
    };
    if repeats.count(&line) {
        return;
    }
    if let Some((level, note)) = repeats.note() {
        output.send(level, note);
    }
    output.send(level, line.clone());
    repeats.wrote(level, line);
}

/// Puts everything logged so far, and every `LineFile`'s lines, on the
/// disk. Called before the process ends.
pub fn flush() {
    if let Some(output) = OUTPUT.get()
        && let Ok(mut repeats) = REPEATS.lock()
        && let Some((level, note)) = repeats.note()
    {
        output.send(level, note);
    }
    wait_for_disk(Duration::from_secs(2));
}

/// Ends the process on an error found while starting up: logs it, says
/// where the log is, and exits with code 2. On Windows a console window
/// waits for Enter first, so the message can be read before it closes.
pub fn exit_launch_error(message: &str) -> ! {
    write_event(Channel::Launch, Level::Error, format_args!("{message}"));
    flush();
    if let Some(output) = OUTPUT.get() {
        let state = if output.open {
            "log"
        } else {
            "log unavailable"
        };
        eprintln!("{state}: {}", output.path.display());
    }
    #[cfg(windows)]
    {
        use std::io::IsTerminal as _;
        if std::io::stdin().is_terminal() {
            eprintln!("\nPress Enter to close.");
            let _ = std::io::stdin().read_line(&mut String::new());
        }
    }
    std::process::exit(2);
}

/// Prints where the log is on stdout, for whoever started the process.
pub fn announce_log_stdout(path: &Path, latest: Option<&Path>) {
    match latest {
        Some(latest) => to_stdout(&format!("log: {} ({})", path.display(), latest.display())),
        None => to_stdout(&format!("log: {}", path.display())),
    }
}

fn to_stdout(line: &str) {
    let mut stdout = std::io::stdout().lock();
    let _ = writeln!(stdout, "{line}");
    let _ = stdout.flush();
}

pub(crate) fn since_start() -> Duration {
    STARTED.get().map_or(Duration::ZERO, Instant::elapsed)
}

/// Nanoseconds since `init_log`; 0 before it.
pub fn process_elapsed_ns() -> u128 {
    since_start().as_nanos()
}

/// A step in the process's life (started, quitting, exiting), as
/// `lifecycle: <name> pid=<pid> ns=<ns><detail>` in the log and on stdout,
/// where a test runner watches for it.
pub fn lifecycle_boundary(name: &str, detail: &str) {
    boundary("lifecycle", name, detail);
}

/// A step in the match scripts' life, as `gsc: <name> pid=<pid>
/// ns=<ns><detail>` in the log and on stdout.
pub fn script_boundary(name: &str, detail: &str) {
    boundary("gsc", name, detail);
}

fn boundary(kind: &str, name: &str, detail: &str) {
    let line = format!(
        "{kind}: {name} pid={} ns={}{detail}",
        std::process::id(),
        process_elapsed_ns()
    );
    write_event(Channel::Launch, Level::Info, format_args!("{line}"));
    to_stdout(&line);
}

/// One line of Bevy's own log output for stderr, handed to the writer
/// thread when dropped: a stderr write Windows held up used to stall the
/// frame that logged it.
pub struct StderrLine(Vec<u8>);

/// Bevy's log writer (a `tracing_subscriber` `MakeWriter`): one
/// `StderrLine` per event.
pub fn stderr_writer() -> StderrLine {
    StderrLine(Vec::new())
}

impl Write for StderrLine {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0.extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl Drop for StderrLine {
    fn drop(&mut self) {
        let text = String::from_utf8_lossy(&self.0);
        let text = text.trim_end_matches(['\r', '\n']);
        if text.is_empty() {
            return;
        }
        match writer() {
            Some(jobs) => {
                let _ = jobs.send(Job::Log {
                    text: text.to_owned(),
                    stderr: true,
                    file: false,
                    at_once: false,
                });
            }
            None => {
                let _ = writeln!(std::io::stderr(), "{text}");
            }
        }
    }
}

/// A test aid's own text file (a per-tick trace). Its lines go through the
/// log's writer thread, so the caller never waits on the disk; `flush`
/// puts them all on it.
pub struct LineFile(usize);

impl LineFile {
    /// Creates `path`, emptying a file already there. None when it cannot.
    pub fn create(path: &Path) -> Option<Self> {
        static NEXT_SLOT: AtomicUsize = AtomicUsize::new(0);
        let file = File::create(path).ok()?;
        let slot = NEXT_SLOT.fetch_add(1, Ordering::Relaxed);
        start_writer()?.send(Job::OpenLines(slot, file)).ok()?;
        Some(Self(slot))
    }

    /// Appends `text` as one line.
    pub fn line(&self, text: String) {
        if let Some(jobs) = writer() {
            let _ = jobs.send(Job::Line(self.0, text));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn levels_order_from_most_to_least_important_and_parse_loosely() {
        assert!(Level::Error < Level::Warn && Level::Warn < Level::Info);
        assert!(Level::Info < Level::Debug);
        assert_eq!(Level::parse(" Warning "), Some(Level::Warn));
        assert_eq!(Level::parse("DEBUG"), Some(Level::Debug));
        assert_eq!(Level::parse("loud"), None);
        assert_eq!(Channel::Console.as_str(), "console");
    }

    #[test]
    fn a_repeated_line_is_counted_and_noted_once() {
        let mut repeats = Repeats::new();
        assert!(!repeats.count("sim  warn  stall"));
        repeats.wrote(Level::Warn, "sim  warn  stall".into());
        assert_eq!(repeats.note(), None);
        assert!(repeats.count("sim  warn  stall"));
        assert!(repeats.count("sim  warn  stall"));
        assert!(!repeats.count("sim  info  moved on"));
        assert_eq!(
            repeats.note(),
            Some((Level::Warn, "sim  warn  stall  (+2 more)".into()))
        );
        assert_eq!(repeats.note(), None);
    }

    #[test]
    fn line_file_lines_are_on_the_disk_after_flush() {
        let path = std::env::temp_dir().join(format!("diag-linefile-{}.txt", std::process::id()));
        let file = LineFile::create(&path).expect("temp file");
        file.line("first".into());
        file.line("second".into());
        flush();
        let text = std::fs::read_to_string(&path).expect("written");
        let _ = std::fs::remove_file(&path);
        assert_eq!(text, "first\nsecond\n");
    }

    #[test]
    fn stderr_line_takes_bytes_and_drops_quietly_when_empty() {
        let mut line = stderr_writer();
        assert_eq!(line.write(b"").expect("buffer"), 0);
        drop(line);
    }
}
