//! The log: one line per call, on a channel and at a level, put on the disk
//! by a thread of its own. A frame that logs only hands the line over; the
//! writer waits on the disk, so a busy disk never holds up the game.

use std::fmt;
use std::fs::{File, OpenOptions};
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Mutex, OnceLock, mpsc};
use std::time::{Duration, Instant, UNIX_EPOCH};

/// The folder bo2mp writes its own output under (logs, caches, runs,
/// screenshots, demos), relative to where it runs.
pub const ARTIFACTS_DIR: &str = "bo2mp-artifacts";

/// The name beside each new log that always opens the newest one.
const NEWEST_LOG_FILE: &str = "latest.log";

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
    ViewModelPerspective,
    Input,
    Sim,
    WireLink,
    Ui,
    Audio,
    Console,
}

/// The word each channel writes, in declaration order.
const CHANNEL_WORDS: [&str; 10] =
    ["launch", "zone", "world", "fpv", "input", "sim", "net", "ui", "audio", "console"];

impl Channel {
    pub const fn as_str(self) -> &'static str {
        CHANNEL_WORDS[self as usize]
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

/// Every level with its word, most important first.
const LEVELS: [(Level, &str); 4] = [
    (Level::Error, "error"),
    (Level::Warn, "warn"),
    (Level::Info, "info"),
    (Level::Debug, "debug"),
];

impl Level {
    pub const fn as_str(self) -> &'static str {
        LEVELS[self as usize].1
    }

    /// A level from its word, any case and padding; `warning` means warn.
    fn parse(text: &str) -> Option<Self> {
        let word = text.trim().to_ascii_lowercase();
        let word = if word == "warning" { "warn" } else { word.as_str() };
        LEVELS.iter().find(|(_, name)| *name == word).map(|(level, _)| *level)
    }
}

/// Where one log line goes, and whether it must reach the disk right away.
#[derive(Clone, Copy)]
struct Route {
    stderr: bool,
    file: bool,
    at_once: bool,
}

/// What the writer thread is asked to do, in the order asked.
enum Job {
    Log(String, Route),
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

/// Write buffer each open file gets.
const FILE_BUFFER: usize = 64 * 1024;

/// The files the writer thread owns, and when it last put them on the disk.
/// Write failures are dropped on purpose: the writer must outlive them.
struct Sinks {
    log: Option<BufWriter<File>>,
    /// `LineFile`s by slot; a slot is empty until its file opens.
    lines: Vec<Option<BufWriter<File>>>,
    last_sync: Instant,
}

impl Sinks {
    fn every_file(&mut self) -> impl Iterator<Item = &mut BufWriter<File>> {
        self.log.iter_mut().chain(self.lines.iter_mut().flatten())
    }

    fn sync(&mut self) {
        for open in self.every_file() {
            open.flush().ok();
        }
        self.last_sync = Instant::now();
    }

    fn log_line(&mut self, text: &str, route: Route) {
        if route.stderr {
            writeln!(std::io::stderr(), "{text}").ok();
        }
        match self.log.as_mut() {
            Some(log) if route.file => {
                writeln!(log, "{text}").ok();
            }
            _ => {}
        }
    }

    fn open_lines(&mut self, at: usize, file: File) {
        let needed = at + 1;
        if self.lines.len() < needed {
            self.lines.resize_with(needed, || None);
        }
        self.lines[at] = Some(BufWriter::with_capacity(FILE_BUFFER, file));
    }

    fn add_line(&mut self, line_slot: usize, line_text: &str) {
        if let Some(target) = self.lines.get_mut(line_slot).and_then(Option::as_mut) {
            writeln!(target, "{line_text}").ok();
        }
    }

    /// Does one job; true when it wants the files on the disk now.
    fn run_job(&mut self, job: Job) -> bool {
        match job {
            Job::Log(text, route) => {
                self.log_line(&text, route);
                route.at_once
            }
            Job::OpenLog(file) => {
                self.log = Some(BufWriter::with_capacity(FILE_BUFFER, file));
                false
            }
            Job::OpenLines(slot, file) => {
                self.open_lines(slot, file);
                false
            }
            Job::Line(slot, text) => {
                self.add_line(slot, &text);
                false
            }
            Job::Flush(answer) => {
                self.sync();
                answer.send(()).ok();
                false
            }
        }
    }
}

fn run_writer(inbox: mpsc::Receiver<Job>) {
    let mut sinks = Sinks { log: None, lines: Vec::new(), last_sync: Instant::now() };
    loop {
        let urgent = match inbox.recv_timeout(BATCH) {
            Ok(job) => sinks.run_job(job),
            // A quiet stretch: whatever waits goes out.
            Err(mpsc::RecvTimeoutError::Timeout) => true,
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        };
        if urgent || sinks.last_sync.elapsed() >= BATCH {
            sinks.sync();
        }
    }
    sinks.sync();
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
    newest_alias: Option<PathBuf>,
    stderr_up_to: Level,
    file_up_to: Level,
}

impl Output {
    fn takes(&self, level: Level) -> bool {
        level <= self.stderr_up_to || (self.open && level <= self.file_up_to)
    }

    fn deliver(&self, level: Level, line_text: String) {
        let route = Route {
            stderr: level <= self.stderr_up_to,
            file: self.open && level <= self.file_up_to,
            at_once: level <= Level::Warn,
        };
        hand_over(line_text, route);
    }
}

/// Gives a line to the writer thread; with no writer running, only its
/// stderr part happens, straight away.
fn hand_over(text: String, route: Route) {
    if let Some(jobs) = writer() {
        jobs.send(Job::Log(text, route)).ok();
    } else if route.stderr {
        writeln!(std::io::stderr(), "{text}").ok();
    }
}

static OUTPUT: OnceLock<Output> = OnceLock::new();
static STARTED: OnceLock<Instant> = OnceLock::new();
static REPEATS: Mutex<Repeats> = Mutex::new(Repeats::NONE);

/// Holds back a line that only repeats the one written before it, and
/// counts it. The count goes out as one note when a different line comes,
/// or at `flush`.
struct Repeats {
    last: Option<(Level, String)>,
    more: u32,
}

impl Repeats {
    /// Nothing written yet, so nothing to repeat.
    const NONE: Self = Self { last: None, more: 0 };

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

/// Opens the log: `BO2MP_LOG`, or `<output_store_dir>/logs/<unix seconds>.log`,
/// and points `latest.log` beside it at it. `BO2MP_LOG_LEVEL` (error, warn,
/// info, debug) sets what both stderr and the file take; without it stderr
/// takes errors and the file takes up to info. From here on a crash leaves
/// its reason in the log. Returns the log's path.
pub fn init_log(output_store_dir: &Path) -> PathBuf {
    if let Some(output) = OUTPUT.get() {
        return output.path.clone();
    }
    let _ = STARTED.set(Instant::now());
    let path = std::env::var_os("BO2MP_LOG")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            let secs = UNIX_EPOCH.elapsed().map_or(0, |since| since.as_secs());
            output_store_dir.join("logs").join(format!("{secs}.log"))
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
    let (stderr_up_to, file_up_to) = match chosen {
        Some(both) => (both, both),
        None => (Level::Error, Level::Info),
    };
    let latest = point_newest_alias(&path);
    let output = Output { path, open, newest_alias: latest, stderr_up_to, file_up_to };
    let path = OUTPUT.get_or_init(|| output).path.clone();
    crate::crash::install(&path);
    path
}

/// Points `latest.log` beside `log` at it. Returns the link, or None when
/// none was made.
fn point_newest_alias(log: &Path) -> Option<PathBuf> {
    // A log that is itself called latest.log gets no link to itself.
    log.file_name().filter(|name| *name != NEWEST_LOG_FILE)?;
    let link = log.with_file_name(NEWEST_LOG_FILE);
    replace_link(log, &link).then_some(link)
}

/// Unix: a relative symbolic link. Only a link is replaced; a real file of
/// that name is left alone.
#[cfg(unix)]
fn replace_link(log: &Path, link: &Path) -> bool {
    let Some(target) = log.file_name() else {
        return false;
    };
    let in_the_way = std::fs::symlink_metadata(link).map(|meta| meta.is_symlink());
    let cleared = match in_the_way {
        Ok(true) => std::fs::remove_file(link).is_ok(),
        Ok(false) => false,
        Err(_) => true,
    };
    cleared && std::os::unix::fs::symlink(target, link).is_ok()
}

/// Windows: a symbolic link needs a privilege most accounts do not have, and
/// a hard link does not.
#[cfg(not(unix))]
fn replace_link(log: &Path, link: &Path) -> bool {
    let cleared = match std::fs::remove_file(link) {
        Ok(()) => true,
        Err(error) => error.kind() == std::io::ErrorKind::NotFound,
    };
    cleared && std::fs::hard_link(log, link).is_ok()
}

/// The `latest.log` link `init_log` made, if it made one.
pub fn newest_log_location() -> Option<PathBuf> {
    OUTPUT.get()?.newest_alias.clone()
}

/// Writes one line, `<channel>  <level>  <message>`. The `error!`, `warn!`,
/// `info!` and `event!` macros call it. Before `init_log` every line goes
/// to stderr.
#[doc(hidden)]
pub fn put_event(channel: Channel, level: Level, message: fmt::Arguments<'_>) {
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
        output.deliver(level, note);
    }
    output.deliver(level, line.clone());
    repeats.wrote(level, line);
}

/// Puts everything logged so far, and every `LineFile`'s lines, on the
/// disk. Called before the process ends.
pub fn flush() {
    if let Some(output) = OUTPUT.get()
        && let Ok(mut repeats) = REPEATS.lock()
        && let Some((level, note)) = repeats.note()
    {
        output.deliver(level, note);
    }
    wait_for_disk(Duration::from_secs(2));
}

/// Ends the process on an error found while starting up: logs it, says
/// where the log is, and exits with code 2. On Windows a console window
/// waits for Enter first, so the message can be read before it closes.
pub fn abort_on_startup_fault(message: &str) -> ! {
    put_event(Channel::Launch, Level::Error, format_args!("{message}"));
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
    hold_console_open();
    std::process::exit(2);
}

/// A console window opened just for this process closes with it; wait for
/// Enter so its last words can be read.
#[cfg(windows)]
fn hold_console_open() {
    use std::io::IsTerminal as _;
    let input = std::io::stdin();
    if !input.is_terminal() {
        return;
    }
    eprintln!("\nPress Enter to close.");
    let mut ignored = String::new();
    input.read_line(&mut ignored).ok();
}

/// Prints where the log is on stdout, for whoever started the process.
pub fn print_log_location(path: &Path, newest_link: Option<&Path>) {
    let shown = path.to_string_lossy();
    match newest_link {
        Some(latest) => to_stdout(&format!("log: {shown} ({})", latest.to_string_lossy())),
        None => to_stdout(&format!("log: {shown}")),
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
pub fn lifecycle_boundary(name: &str, note: &str) {
    boundary("lifecycle", name, note);
}

/// A step in the match scripts' life, as `gsc: <name> pid=<pid>
/// ns=<ns><detail>` in the log and on stdout.
pub fn script_boundary(name: &str, note: &str) {
    boundary("gsc", name, note);
}

fn boundary(kind: &str, name: &str, note: &str) {
    let pid = std::process::id();
    let ns = process_elapsed_ns();
    let line = format!("{kind}: {name} pid={pid} ns={ns}{note}");
    put_event(Channel::Launch, Level::Info, format_args!("{line}"));
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
        let shown = String::from_utf8_lossy(&self.0);
        let body = shown.trim_end_matches(['\r', '\n']);
        if !body.is_empty() {
            let stderr_only = Route { stderr: true, file: false, at_once: false };
            hand_over(body.to_owned(), stderr_only);
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
        let mut repeats = Repeats::NONE;
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
        std::fs::remove_file(&path).ok();
        assert_eq!(text, "first\nsecond\n");
    }

    #[test]
    fn stderr_line_takes_bytes_and_drops_quietly_when_empty() {
        let mut line = stderr_writer();
        assert_eq!(line.write(b"").expect("buffer"), 0);
        drop(line);
    }
}
