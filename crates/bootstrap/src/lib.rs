//! The composition root: reads the command line and builds the bevy `App`
//! for a menu, map, serve or play run out of every other crate's plugin.

mod app_stack;
mod cli;
mod fast_cores;
mod start;
#[cfg(feature = "span-stats")]
mod span_stats;

pub use app_stack::build_hosting_app;
pub use cli::{SignoffStartup, StartupFlavor, read_flags};
pub use start::run_mode;
