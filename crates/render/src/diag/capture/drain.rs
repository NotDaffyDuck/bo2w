//! Letting screenshot writes finish before the process goes away.
//!
//! Leaving the process kills the I/O workers, so a file still being encoded
//! would be absent or half a PNG. The exit waits for the workers it started,
//! for a bounded time: a stuck write may cost seconds and a log line, never
//! the run.

use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::time::Duration;

/// How long a scripted exit waits on a write still running.
const SCRIPTED_EXIT_BUDGET: Duration = Duration::from_secs(20);

/// A person closing the window is not worth waiting for.
const USER_QUIT_BUDGET: Duration = Duration::from_millis(100);

static BUDGET_MS: AtomicU64 = AtomicU64::new(SCRIPTED_EXIT_BUDGET.as_millis() as u64);

/// Writes handed to a worker and not finished, across the whole process.
///
/// The queue resource cannot see these when it matters: a demo ends in
/// `std::process::exit` from `Last`, and a system reading `AppExit` may never
/// run again. This count is what the exit handler waits on.
pub(super) static WRITES_IN_FLIGHT: AtomicUsize = AtomicUsize::new(0);

pub(super) fn budget() -> Duration {
    Duration::from_millis(BUDGET_MS.load(Ordering::Relaxed))
}

/// Called when the exit is the player's own: shrinks the wait to almost none.
pub fn exit_is_user_quit() {
    BUDGET_MS.fetch_min(USER_QUIT_BUDGET.as_millis() as u64, Ordering::Relaxed);
}

/// Waits for running encodes however the process leaves, armed once on the
/// first readback. If a write is stuck the file it was making stays
/// incomplete, which the queue's own report says.
#[cfg(unix)]
pub(super) fn arm_at_exit() {
    use std::sync::atomic::AtomicBool;
    use std::time::Instant;

    static ARMED: AtomicBool = AtomicBool::new(false);
    unsafe extern "C" {
        fn atexit(callback: extern "C" fn()) -> i32;
    }
    extern "C" fn wait_for_writes() {
        let deadline = Instant::now() + budget();
        while WRITES_IN_FLIGHT.load(Ordering::Relaxed) > 0 && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(2));
        }
    }
    if !ARMED.swap(true, Ordering::SeqCst) {
        let _ = unsafe { atexit(wait_for_writes) };
    }
}

#[cfg(not(unix))]
pub(super) fn arm_at_exit() {}
