//! The console as a Bevy plugin: the overlay, the keys that open it, the
//! command queue, and the routing of every command to the feature that
//! answers it. It also turns the player's devices into the client's input.

use bevy::input::InputSystems;
use bevy::input_focus::InputFocusSystems;
use bevy::prelude::*;

use crate::TypedOrder;
use crate::ScriptedControlsStatus;
use crate::BindingTable;
use crate::order_book::OrderCatalog;
use net::ClientStep;
use bevy::app::App;
use bevy::app::Last;
use bevy::app::Update;

mod commands;
mod gameplay;
mod panel;
mod prompt;
mod queue;
mod scripted;
mod scrollback;
mod state;

pub use queue::{TypedOrderBacklog, TypedOrderRouter, Hold, AwaitMotionPose};
pub use state::PromptTypeface;
pub use state::PromptPanelOptions;
pub use state::PromptPanelStatus;

/// The set the console's own command queue runs in. A feature that must see a
/// command before, or after, the console has answered it orders against this.
#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub struct TypedOrderStage;

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

pub struct PromptPanelPlugin;

impl Plugin for PromptPanelPlugin {
    fn build(&self, app: &mut App) {
        crate::startup::install_stdin(app);
        add_state(app);
        add_input(app);
        add_commands(app);
        let late = (
            scrollback::paint_scrollback_selection,
            crate::tool_cmds::exit_process,
        );
        app.add_systems(Last, late.chain());
        crate::probes::install_metrics_echo(app);
    }
}

fn add_state(app: &mut App) {
    use crate::prefs::AwaitedMenuControl;
    use crate::prefs::PrefsStorage;
    use crate::prefs::read_player_prefs;
    use crate::weapon_cmds::GunParamHints;

    app.init_resource::<PromptPanelOptions>();
    app.init_resource::<PromptPanelStatus>();
    app.init_resource::<frame::OverlayControlSnapshot>();
    app.init_resource::<ScriptedControlsStatus>();
    app.init_resource::<crate::ScriptedKeys>();
    app.init_resource::<scripted::ScriptedMouse>();
    app.init_resource::<BindingTable>();
    app.init_resource::<TypedOrderBacklog>();
    app.init_resource::<TypedOrderRouter>();
    app.init_resource::<OrderCatalog>();
    app.init_resource::<crate::OrderInbox>();
    app.init_resource::<crate::PromptOutputRow>();
    app.init_resource::<GunParamHints>();
    app.init_resource::<AwaitedMenuControl>();
    app.init_resource::<PrefsStorage>();
    app.init_resource::<frame::ActivePad>();
    app.init_resource::<frame::InputDevices>();
    app.add_message::<TypedOrder>();
    app.add_message::<frame::TestControllerRumble>();

    let startup_steps = (
        commands::register_commands,
        panel::build_panel,
        read_player_prefs,
    );
    app.add_systems(Startup, startup_steps.chain());
}

/// Keys, mouse and pad: read before the engine's own input systems, then
/// turned into the client's action input.
fn add_input(app: &mut App) {
    // The keyboard feed has to see raw events before the engine consumes them.
    let keyboard = prompt::feed_console_keyboard.before(InputSystems);
    app.add_systems(PreUpdate, keyboard);

    let pad_steps = (
        crate::gamepad::bo2_trigger_thresholds,
        crate::gamepad::track_active_pad,
    );
    let pad_order = pad_steps
        .chain()
            .after(InputSystems)
        .before(gameplay::publish_client_action_input);
    app.add_systems(PreUpdate, pad_order);

    let device_steps = (
        prompt::handle_console_input,
        scrollback::handle_scrollback_pointer,
        scrollback::copy_selection_on_release,
        gameplay::isolate_gameplay_input,
        scripted::drive_scripted_keys,
        scripted::drive_scripted_mouse,
        scripted::expire_pressed_inputs,
        gameplay::publish_client_action_input,
        gameplay::sync_cursor_grab,
    );
    let device_order = device_steps
        .chain()
            .after(InputSystems)
        .after(InputFocusSystems::Dispatch);
    app.add_systems(PreUpdate, device_order);
}

fn add_commands(app: &mut App) {
    use crate::loadout_cmds::finish_queued_respawn;
    use crate::loadout_cmds::steer_kit_orders;
    use crate::tool_cmds::carry_on_session_orders;
    use crate::tool_cmds::route_bot_commands;
    use crate::tool_cmds::route_hitvol_commands;
    use crate::tool_cmds::steer_lobby_orders;
    use crate::tool_cmds::steer_menu_orders;
    use crate::tool_cmds::steer_playback_orders;
    use crate::tool_cmds::steer_screenshot_orders;
    use crate::tool_cmds::steer_snapshot_orders;
    use crate::prefs::apply_pad_layout_setting;
    use crate::prefs::native_menu_settings;
    use crate::prefs::push_omni_movement_setting;
    use crate::prefs::rename_local_player;
    use crate::prefs::set_overall_loudness;
    use crate::prefs::refresh_binding_rows;
    use crate::prefs::take_menu_control;
    use crate::prefs::write_player_prefs;
    use crate::weapon_cmds::drop_gun_hints_on_close;
    use crate::weapon_cmds::echo_configuration_change_results;
    use crate::weapon_cmds::rebuild_gun_hints;
    use crate::weapon_cmds::report_handout_outcome;
    use crate::weapon_cmds::steer_gun_orders;

    let stages = (Stage::Intake, Stage::Routes, Stage::Debug, Stage::Finish)
        .chain()
        .in_set(ClientStep::Diag);
    app.configure_sets(Update, stages);

    let pad_layout = apply_pad_layout_setting
        .after(take_menu_control)
        .before(refresh_binding_rows);
    app.add_systems(bevy::app::Update, pad_layout);
    app.add_systems(bevy::app::Update, push_omni_movement_setting);

    let intake = (
        crate::startup::drain_stdin,
        queue::drain_startup_queue,
        queue::run_console_queue.in_set(TypedOrderStage),
        queue::run_menu_commands,
        scripted::handle_input_commands,
        prompt::apply_console_os_paste,
    );
    app.add_systems(Update, intake.chain().in_set(Stage::Intake));

    let routes = (
        steer_playback_orders,
        steer_menu_orders,
        (crate::menus::route_menu_orders, crate::class_menu::route_class_menu).chain(),
        steer_screenshot_orders,
        steer_snapshot_orders,
        route_hitvol_commands,
        route_bot_commands,
        steer_lobby_orders,
        carry_on_session_orders,
        steer_kit_orders,
        finish_queued_respawn,
        drop_gun_hints_on_close,
        rebuild_gun_hints,
        steer_gun_orders,
    );
    app.add_systems(Update, routes.chain().in_set(Stage::Routes));

    let give_echoes = (
        report_handout_outcome,
        echo_configuration_change_results,
    );
    let debug_tools = (
        give_echoes.chain(),
        crate::showpos::route_showpos,
        crate::player_cmds::route_player_orders,
        crate::dvars::routers(),
        crate::fog::route_fog_commands,
        crate::vision::route_vision_commands,
        crate::mover_cmds::route_script_mover_commands,
        crate::lock_sight::route_lock_pvs,
        crate::reports::routers(),
    );
    app.add_systems(Update, debug_tools.chain().in_set(Stage::Debug));

    let menu_bindings = (
        native_menu_settings,
        take_menu_control,
    );
    let finish = (
        menu_bindings.chain(),
        refresh_binding_rows,
        set_overall_loudness,
        rename_local_player,
        write_player_prefs,
        panel::update_console_ui,
    );
    app.add_systems(Update, finish.chain().in_set(Stage::Finish));
}
