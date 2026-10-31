//! Console lines that wait for the console to be ready for them.

use bevy::prelude::*;

/// The commands the process was started with (`+map ...`). The console holds
/// them back until the main menu is up, then runs them.
#[derive(Resource, Default)]
pub struct BootConsoleScript {
    pub commands: Vec<String>,
}

/// Lines other systems want run or shown by the console. The console empties
/// this every frame.
#[derive(Resource, Default)]
pub struct StagedConsoleText(pub Vec<String>);
