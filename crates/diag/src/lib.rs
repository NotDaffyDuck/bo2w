//! What the engine says about itself while it runs.
//!
//! - The log: `error!`, `warn!`, `info!` and `event!` write one line on a
//!   [`Channel`]. [`init_log`] opens the file; a thread of its own writes it.
//! - A crash leaves its reason as the log's last line.
//! - [`LineFile`]: a test aid's own text file, written the same way.
//! - Counters: heap allocations ([`ProcessCountingAllocator`]) and CPU time.
//! - [`gap`]: ledgers of the parts a subsystem knows it does not do yet.
//! - [`wgsl_dump`]: shader source written out for render debugging.

mod alloc;
mod cpu;
mod crash;
pub mod gap;
mod log;
pub mod wgsl_dump;

pub use alloc::{
    ProcessAllocationStats, ProcessCountingAllocator, counting_enabled, process_allocations,
    process_live_heap_bytes, release_freed_heap,
};
pub use cpu::{busy_note, process_cpu_ms, thread_cpu_ms};
pub use log::{
    ARTIFACTS_DIR, Channel, Level, LineFile, StderrLine, announce_log_stdout, exit_launch_error,
    flush, init_log, latest_log_path, lifecycle_boundary, process_elapsed_ns, script_boundary,
    stderr_writer, write_event,
};

/// Logs an error on a channel: `error!(Net, "lost {peer}")`.
#[macro_export]
macro_rules! error {
    ($channel:ident, $($message:tt)+) => {
        $crate::write_event($crate::Channel::$channel, $crate::Level::Error, format_args!($($message)+))
    };
}

/// Logs a warning on a channel: `warn!(Sim, "stall {ms} ms")`.
#[macro_export]
macro_rules! warn {
    ($channel:ident, $($message:tt)+) => {
        $crate::write_event($crate::Channel::$channel, $crate::Level::Warn, format_args!($($message)+))
    };
}

/// Logs a line of information on a channel: `info!(Zone, "loaded {name}")`.
#[macro_export]
macro_rules! info {
    ($channel:ident, $($message:tt)+) => {
        $crate::write_event($crate::Channel::$channel, $crate::Level::Info, format_args!($($message)+))
    };
}

/// Logs a named event at a chosen level: `event!(Input, Debug, "cursor",
/// "cursor grab={grab}")`. The name is for a trace recorder; with none
/// attached only the line is written.
#[macro_export]
macro_rules! event {
    ($channel:ident, $level:ident, $name:literal, $($message:tt)+) => {
        $crate::write_event($crate::Channel::$channel, $crate::Level::$level, format_args!($($message)+))
    };
}
