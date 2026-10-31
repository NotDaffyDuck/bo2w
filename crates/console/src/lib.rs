//! Everything behind the `~` prompt. A typed or scripted line is split into
//! commands; each command goes to the system that owns it; the answers land in
//! the scrollback.
//!
//! Where to look:
//! - parsing and the order queue: `command`; the list of known commands and
//!   their argument completers: `order_book`; prompt helpers: `completion`,
//!   `line_edit`.
//! - the panel, the queue runner and the holds: `app`.
//! - presses to commands: `keymap`, `key_input`, `gamepad`, `synthetic_input`.
//! - the rest are command families: loadouts, weapons, menus, the class
//!   editor, player preferences and so on.

// Modules other crates reach into.
pub mod app;
pub mod completion;
pub mod key_input;
pub mod keymap;
pub mod line_edit;
pub mod order_book;

// Modules that stay inside the crate.
mod class_menu;
mod command;
mod dvars;
mod fog;
mod gamepad;
mod host_link;
mod loadout_cmds;
mod lock_sight;
mod menus;
mod mover_cmds;
mod player_cmds;
mod prefs;
mod probes;
mod reports;
mod showpos;
mod startup;
mod synthetic_input;
mod tool_cmds;
mod vision;
mod weapon_cmds;

pub use app::{AwaitMotionPose, Hold, PromptPanelOptions, PromptPanelPlugin, PromptPanelStatus};
pub use app::{PromptTypeface, TypedOrderBacklog, TypedOrderRouter, TypedOrderStage};
pub use order_book::{FixedListSuggester, OrderBlueprint, OrderCatalog, ParamSuggester};
pub use command::{OrderInbox, TypedOrder};
pub use keymap::{key_name_list, button_label, host_key_code, button_from_name};
pub use keymap::{ActionBinding, BindingTable, BindingTableFeed, GamepadKey, STOCK_BINDINGS};
pub use line_edit::PromptLineEditor;
pub use loadout_cmds::class_completions;
pub use startup::{boot_orders, scripted_run, strip_cmds_flag};
pub use synthetic_input::{ScriptedControlsStatus, ScriptedKeys};
pub use synthetic_input::{TAP_HOLD_SECONDS, TAP_STEP_SECONDS_CAP};
pub use tool_cmds::install_tool_orders;
pub use weapon_cmds::weapon_completions;
pub use weapon_cmds::attach_completions;

pub(crate) use synthetic_input::names_control_order;

use bevy::prelude::Resource;

/// Whatever the console printed most recently, kept for the one-line readouts.
#[derive(Resource, Clone, Debug, Default)]
pub struct PromptOutputRow(pub String);
