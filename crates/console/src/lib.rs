//! The console: a line typed or scripted becomes a command, the command is
//! routed to the part of the game that runs it, and what it says is logged.
//!
//! - `command` parses lines and queues them; `registry` knows each command and
//!   how to complete its arguments; `suggest` and `editor` serve the prompt.
//! - `app` is the plugin that runs the queue and draws the console.
//! - `binds`, `input`, `gamepad` and `synthetic_input` turn presses into
//!   commands, real or scripted.
//! - Every other module is a family of commands (`class_dispatch`,
//!   `weapon_dispatch`, `frontend`, `class_menu`, `user_settings`, ...).

use bevy::prelude::Resource;

pub mod app;
pub mod binds;
mod class_dispatch;
mod class_menu;
mod command;
mod diagnostics;
mod dvars;
pub mod editor;
mod feature_dispatch;
mod fog;
mod frontend;
mod gamepad;
mod host_link;
pub mod input;
mod lock_pvs;
mod player_cmds;
pub mod registry;
mod reports;
mod script_mover;
mod showpos;
mod startup;
pub mod suggest;
mod synthetic_input;
mod user_settings;
mod vision;
mod weapon_dispatch;

pub use app::{
    ConsoleCommandQueue, ConsoleDispatch, ConsoleDispatchSet, ConsoleFont, ConsolePlugin,
    ConsoleSettings, ConsoleState, Hold, WaitMovePose,
};
pub use binds::{
    BindButton, BindInputs, DEFAULT_CONTROLS, KeyBinds, PadButton, bindable_keys, display_button,
    host_keynum, parse_button_name,
};
pub use class_dispatch::class_completions;
pub use command::{ConsoleCommand, ConsoleQueue, SubmittedCommand};
pub use editor::ConsoleEditor;
pub use feature_dispatch::register_feature_commands;
pub use registry::{ArgCompleter, CommandSpec, ConsoleRegistry, StaticCompleter};
pub use startup::{scripted_run, startup_commands, strip_cmds_flag};
pub use synthetic_input::{ConsoleInputState, PRESS_SECONDS, PRESS_TICK_DT_MAX, ScriptedKeys};
pub use weapon_dispatch::{attach_completions, weapon_completions};

pub(crate) use synthetic_input::is_bind_command;

/// The last line the console said, for whoever shows it as a single line.
#[derive(Resource, Debug, Default, Clone)]
pub struct ConsoleLine(pub String);
