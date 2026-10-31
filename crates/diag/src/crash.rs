//! A crash leaves its reason as the log's last line: a panic its message,
//! place, thread and backtrace; on Windows, an exception nothing caught its
//! code and address.

use std::fs::OpenOptions;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::Duration;

static LOG: OnceLock<PathBuf> = OnceLock::new();

/// How long a crash waits for the lines logged before it to reach the disk.
const LAST_FLUSH: Duration = Duration::from_secs(1);

pub(crate) fn install(log: &Path) {
    if LOG.set(log.to_path_buf()).is_err() {
        return;
    }
    let earlier = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        crate::log::wait_for_disk(LAST_FLUSH);
        let ms = crate::log::since_start().as_millis();
        last_words(&format!(
            "[{ms}ms] CRASH panic on thread '{}': {info}\n{}",
            std::thread::current().name().unwrap_or("unnamed"),
            std::backtrace::Backtrace::force_capture()
        ));
        earlier(info);
    }));
    #[cfg(windows)]
    exception::install();
}

/// Appends `text` to the log through a handle of its own: the code that
/// crashed may be the code holding the writer up.
fn last_words(text: &str) {
    let Some(path) = LOG.get() else {
        return;
    };
    if let Ok(mut log) = OpenOptions::new().append(true).open(path) {
        let _ = writeln!(log, "{text}");
        let _ = log.flush();
    }
}

#[cfg(windows)]
mod exception {
    use std::ffi::c_void;

    /// The start of Windows' EXCEPTION_RECORD.
    #[repr(C)]
    struct Record {
        code: u32,
        flags: u32,
        chained: *const Record,
        address: *const c_void,
        parameter_count: u32,
        parameters: [usize; 15],
    }

    /// Windows' EXCEPTION_POINTERS.
    #[repr(C)]
    struct Pointers {
        record: *const Record,
        context: *const c_void,
    }

    /// Lets Windows carry on as if no filter were set: the process ends.
    const CONTINUE_SEARCH: i32 = 0;

    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn SetUnhandledExceptionFilter(filter: Option<unsafe extern "system" fn(*const Pointers) -> i32>) -> Option<unsafe extern "system" fn(*const Pointers) -> i32>;
        fn GetModuleHandleW(name: *const u16) -> *mut c_void;
    }

    pub(super) fn install() {
        // SAFETY: the filter is a plain function that lives as long as the
        // process.
        unsafe {
            SetUnhandledExceptionFilter(Some(on_exception));
        }
    }

    unsafe extern "system" fn on_exception(pointers: *const Pointers) -> i32 {
        crate::log::wait_for_disk(super::LAST_FLUSH);
        // SAFETY: Windows hands the filter its EXCEPTION_POINTERS; a null
        // pointer is read as no record.
        let record = unsafe {
            pointers
                .as_ref()
                .and_then(|pointers| pointers.record.as_ref())
        };
        let (code, address, first, second) = record.map_or((0, 0, 0, 0), |record| {
            (
                record.code,
                record.address as usize,
                record.parameters[0],
                record.parameters[1],
            )
        });
        // SAFETY: a null name asks for the exe's own module, whose handle is
        // the address it was loaded at.
        let exe = unsafe { GetModuleHandleW(std::ptr::null()) } as usize;
        super::last_words(&format!(
            "CRASH exception 0x{code:08X} at 0x{address:X} (exe+0x{:X}) on thread '{}' (info {first:X} {second:X})",
            address.wrapping_sub(exe),
            std::thread::current().name().unwrap_or("unnamed")
        ));
        CONTINUE_SEARCH
    }
}
