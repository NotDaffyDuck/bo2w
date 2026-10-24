//! The composition root: reads the command line and builds the bevy `App`
//! for a menu, map, serve or play run out of every other crate's plugin.

pub mod args;
mod frame_owner;
mod launch;
mod plugins;
#[cfg(feature = "span-stats")]
mod span_stats;

pub use args::{AcceptanceLaunch, LaunchMode, parse_cli};
pub use launch::launch;
pub use plugins::assemble_listen_app;
