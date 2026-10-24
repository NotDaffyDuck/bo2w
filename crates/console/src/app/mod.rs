//! The console as a Bevy plugin: the overlay, the keys that open it, the
//! command queue, and the routing of every command to the feature that
//! answers it. It also turns the player's devices into the client's input.

use bevy::{input::InputSystems, input_focus::InputFocusSystems, prelude::*};
use net::ClientSet;

use crate::{ConsoleCommand, ConsoleInputState, KeyBinds, registry::ConsoleRegistry};

mod commands;
mod gameplay;
mod panel;
mod prompt;
mod queue;
mod scripted;
mod scrollback;
mod state;

pub use queue::{ConsoleCommandQueue, ConsoleDispatch, Hold, WaitMovePose};
pub use state::{ConsoleFont, ConsoleSettings, ConsoleState};

/// The set the console's own command queue runs in. A feature that must see a
/// command before, or after, the console has answered it orders against this.
#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub struct ConsoleDispatchSet;

/// The order each frame's console work runs in, within [`ClientSet::Diag`].
#[derive(SystemSet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum Stage {
    /// Commands come in: stdin, the startup script, the queue, the menus.
    Intake,
    /// Each feature takes the commands it answers.
    Routes,
    /// The debug and tuning commands.
    Debug,
    /// Settings, and the overlay's redraw.
    Finish,
}

pub struct ConsolePlugin;

impl Plugin for ConsolePlugin {
    fn build(&self, app: &mut App) {
        crate::startup::install_stdin(app);
        add_state(app);
        add_input(app);
        add_commands(app);
        app.add_systems(
            Last,
            (
                scrollback::paint_scrollback_selection,
                crate::feature_dispatch::exit_process,
            )
                .chain(),
        );
        crate::diagnostics::register_diagnostics_mirror(app);
    }
}

fn add_state(app: &mut App) {
    app.init_resource::<ConsoleSettings>()
        .init_resource::<ConsoleState>()
        .init_resource::<frame::HudInputView>()
        .init_resource::<ConsoleInputState>()
        .init_resource::<crate::ScriptedKeys>()
        .init_resource::<scripted::ScriptedMouse>()
        .init_resource::<KeyBinds>()
        .init_resource::<ConsoleCommandQueue>()
        .init_resource::<ConsoleDispatch>()
        .init_resource::<ConsoleRegistry>()
        .init_resource::<crate::ConsoleQueue>()
        .init_resource::<crate::ConsoleLine>()
        .init_resource::<crate::weapon_dispatch::WeaponArgCompletions>()
        .init_resource::<crate::user_settings::PendingMenuBinding>()
        .init_resource::<crate::user_settings::UserSettingsPersistence>()
        .init_resource::<frame::ActivePad>()
        .init_resource::<frame::InputDevices>()
        .add_message::<ConsoleCommand>()
        .add_message::<frame::TestControllerRumble>()
        .add_systems(
            Startup,
            (
                commands::register_commands,
                panel::build_panel,
                crate::user_settings::load_user_settings,
            )
                .chain(),
        );
}

/// Keys, mouse and pad: read before the engine's own input systems, then
/// turned into the client's action input.
fn add_input(app: &mut App) {
    app.add_systems(
        PreUpdate,
        prompt::feed_console_keyboard.before(InputSystems),
    )
    .add_systems(
        PreUpdate,
        (
            crate::gamepad::bo2_trigger_thresholds,
            crate::gamepad::track_active_pad,
        )
            .chain()
            .after(InputSystems)
            .before(gameplay::publish_client_action_input),
    )
    .add_systems(
        PreUpdate,
        (
            prompt::handle_console_input,
            scrollback::handle_scrollback_pointer,
            scrollback::copy_selection_on_release,
            gameplay::isolate_gameplay_input,
            scripted::drive_scripted_keys,
            scripted::drive_scripted_mouse,
            scripted::expire_pressed_inputs,
            gameplay::publish_client_action_input,
            gameplay::sync_cursor_grab,
        )
            .chain()
            .after(InputSystems)
            .after(InputFocusSystems::Dispatch),
    );
}

fn add_commands(app: &mut App) {
    app.configure_sets(
        Update,
        (Stage::Intake, Stage::Routes, Stage::Debug, Stage::Finish)
            .chain()
            .in_set(ClientSet::Diag),
    )
    .add_systems(
        Update,
        crate::user_settings::apply_pad_layout_setting
            .after(crate::user_settings::consume_menu_binding)
            .before(crate::user_settings::sync_binding_view),
    )
    .add_systems(Update, crate::user_settings::push_omni_movement_setting)
    .add_systems(
        Update,
        (
            crate::startup::drain_stdin,
            queue::drain_startup_queue,
            queue::run_console_queue.in_set(ConsoleDispatchSet),
            queue::run_menu_commands,
            scripted::handle_input_commands,
            prompt::apply_console_os_paste,
        )
            .chain()
            .in_set(Stage::Intake),
    )
    .add_systems(
        Update,
        (
            crate::feature_dispatch::route_replay_commands,
            crate::feature_dispatch::route_ui_commands,
            (crate::frontend::route, crate::class_menu::route).chain(),
            crate::feature_dispatch::route_capture_commands,
            crate::feature_dispatch::route_state_dump_commands,
            crate::feature_dispatch::route_hitvol_commands,
            crate::feature_dispatch::route_bot_commands,
            crate::feature_dispatch::route_session_commands,
            crate::feature_dispatch::resume_lifecycle_commands,
            crate::class_dispatch::route_class_commands,
            crate::class_dispatch::complete_pending_spawn,
            crate::weapon_dispatch::clear_weapon_args_on_torn_down,
            crate::weapon_dispatch::refresh_weapon_arg_completions,
            crate::weapon_dispatch::route_weapon_commands,
        )
            .chain()
            .in_set(Stage::Routes),
    )
    .add_systems(
        Update,
        (
            (
                crate::weapon_dispatch::echo_give_results,
                crate::weapon_dispatch::echo_configuration_change_results,
            )
                .chain(),
            crate::showpos::route_showpos,
            crate::player_cmds::route,
            crate::dvars::routers(),
            crate::fog::route_fog_commands,
            crate::vision::route_vision_commands,
            crate::script_mover::route_script_mover_commands,
            crate::lock_pvs::route_lock_pvs,
            crate::reports::routers(),
        )
            .chain()
            .in_set(Stage::Debug),
    )
    .add_systems(
        Update,
        (
            (
                crate::user_settings::native_menu_settings,
                crate::user_settings::consume_menu_binding,
            )
                .chain(),
            crate::user_settings::sync_binding_view,
            crate::user_settings::apply_master_volume,
            crate::user_settings::sync_player_name,
            crate::user_settings::save_user_settings,
            panel::update_console_ui,
        )
            .chain()
            .in_set(Stage::Finish),
    );
}
