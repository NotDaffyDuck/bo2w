//! Each frame the player's devices are turned into the client's action input:
//! keys, mouse and pad go through the binds, and the pointer stays grabbed
//! while play owns it.

use std::{collections::BTreeSet, collections::HashSet};

use bevy::{ecs::system::SystemParam, input::gamepad::Gamepad, input::gamepad::GamepadButton};
use bevy::{input::mouse::MouseMotion, input::mouse::MouseWheel, window::CursorEntered};
use bevy::{window::CursorGrabMode, window::CursorOptions};
use bevy::prelude::*;
use bevy::window::WindowFocused;
use frame::ActivePad;
use frame::PlayerPrefsBundle;
use frame::OverlayControlSnapshot;
use frame::InputDevices;
use frame::PromptStyle;
use input_t6::{PlayerIntentFeed, KEYBOARD_SLOTS, SCRIPTED_KEY_SLOT, input_cmd, key_event, release_action_number};
use net::{PlayerControlSnapshot, frame_time_msec, pressed_frame_millis};

use super::scripted::Timed;
use super::state::PromptPanelStatus;
use crate::ActionBinding;
use crate::ScriptedControlsStatus;
use crate::BindingTable;
use crate::keymap::BindingTableFeed;
use crate::keymap::button_label;
use crate::keymap::host_key_code;
use crate::keymap::pulse_wheel_binding;
use crate::keymap::wheel_detents;
use bevy::window::PrimaryWindow;
use frame::ShellScreen;

/// Turn in degrees per unit of mouse movement, as in BO2's defaults.
const MOUSE_DEGREES: f32 = 0.022;
/// Movement within one window of time that tells us the mouse was picked up.
const MOUSE_NOTICED: f32 = 2.0;
const MOUSE_WINDOW_SECONDS: f32 = 0.3;

/// While the console is open, gameplay gets no keys and no mouse movement.
pub(super) fn isolate_gameplay_input(
    console: Res<PromptPanelStatus>,
    mut keyboard: ResMut<ButtonInput<KeyCode>>,
    mut moved: MessageReader<MouseMotion>,
) {
    let open = console.shown;
    if !open {
        return;
    }
    keyboard.reset_all();
    moved.clear();
}

/// The current frame's time, in the units client commands are stamped with.
#[derive(Copy, Clone)]
struct Tick {
    now: i32,
    frame: u32,
}

/// The buttons the player can press: keyboard, mouse and the active pad.
#[derive(SystemParam)]
pub(super) struct Buttons<'w, 's> {
    keys: Res<'w, ButtonInput<KeyCode>>,
    mouse: Res<'w, ButtonInput<MouseButton>>,
    gamepads: Query<'w, 's, &'static Gamepad>,
    active: Res<'w, ActivePad>,
}

/// Mouse movement and wheel events since the last frame.
#[derive(SystemParam)]
pub(super) struct MouseIn<'w, 's> {
    motion: MessageReader<'w, 's, MouseMotion>,
    wheel: MessageReader<'w, 's, MouseWheel>,
    /// Fraction of a wheel notch left over after the whole steps were taken.
    carry: Local<'s, f32>,
}

/// Things other than play that can take the input.
#[derive(SystemParam)]
pub(super) struct Modal<'w> {
    console: Res<'w, PromptPanelStatus>,
    menus: Option<Res<'w, hud::ScriptMenus>>,
}

impl Modal<'_> {
    fn grabs_input(&self) -> bool {
        self.console.shown || self.menus.as_ref().is_some_and(|menus| menus.captures_input())
    }
}

/// State the input system carries between frames.
#[derive(Default)]
pub(super) struct Physical {
    pad_owner: Option<Entity>,
    /// Buttons held since before they could count: pressed while the console
    /// had the input, or while they were bound to something else.
    blocked: HashSet<ActionBinding>,
    movement_ready: bool,
    look_ready: bool,
    mouse_activity: f32,
    window_began: f32,
    shown_prompts: Option<(bool, PromptStyle, bool)>,
}

impl Physical {
    fn start_window(&mut self, elapsed: f32) {
        let age = elapsed - self.window_began;
        if age > MOUSE_WINDOW_SECONDS {
            self.mouse_activity = 0.0;
            self.window_began = elapsed;
        }
    }

    /// Adds mouse movement; enough within one window puts the prompts back on
    /// the keyboard's.
    fn noticed_mouse(&mut self, travel: f32, devices: &mut InputDevices) {
        self.mouse_activity += travel;
        if self.mouse_activity < MOUSE_NOTICED {
            return;
        }
        devices.pad_prompts = false;
        self.mouse_activity = 0.0;
    }
}

/// Which device the command reports as last used. A scripted run counts as a
/// pad unless `BO2MP_LAST_INPUT=mouse`, so tests can exercise the mouse's
/// dual-wield triggers.
fn last_input_pad(pad_prompts: bool) -> bool {
    static OVERRIDE: std::sync::OnceLock<Option<bool>> = std::sync::OnceLock::new();
    let pinned = OVERRIDE.get_or_init(|| {
        let requested = std::env::var("BO2MP_LAST_INPUT").ok();
        forced_device(requested.as_deref())
    });
    match *pinned {
        Some(pad) => pad,
        None => pad_prompts || crate::startup::scripted_run(),
    }
}

fn forced_device(setting: Option<&str>) -> Option<bool> {
    match setting {
        Some("mouse") => Some(false),
        Some("pad") => Some(true),
        _ => None,
    }
}

/// Resets the per-frame fields and reapplies the settings-driven ones.
fn begin_frame(out: &mut PlayerControlSnapshot, clock: &Time, settings: &PlayerPrefsBundle) -> Tick {
    out.pad_aim_assist = 0;
    out.pad_deflection = 0.0;
    out.pad_move = [0.0; 2];
    out.pad_look = [0.0; 2];
    out.pad_look_delta = [0.0; 2];
    (out.pointer_delta_horizontal, out.pointer_delta_vertical) = (0.0, 0.0);
    let tick = Tick {
        now: frame_time_msec(clock.elapsed_secs()),
        frame: pressed_frame_millis(clock.delta_secs()),
    };
    out.tick_span_ms = tick.frame;
    out.present_tick_ms = tick.now;
    let PlayerPrefsBundle { sensitivity, flip_look: invert_mouse, .. } = *settings;
    out.sensitivity = sensitivity;
    out.pitch_per_count = if invert_mouse { -MOUSE_DEGREES } else { MOUSE_DEGREES };
    // Zero means never set; fall back to the stock value.
    if out.yaw_sensitivity == 0.0 {
        out.yaw_sensitivity = MOUSE_DEGREES;
    }
    if out.view_angle_multiplier == 0.0 {
        out.view_angle_multiplier = 1.0;
    }
    tick
}

/// Drops the pad's turn rate and aim assistance.
fn release_pad(out: &mut PlayerControlSnapshot, devices: &mut InputDevices) {
    devices.aiming_with_pad = false;
    out.pad_turn_rate = [0.0; 2];
    out.pad_lockon = None;
    out.pad_autoaim = None;
}

/// Records which pad is in use. A different pad starts clean: buttons it was
/// holding when it arrived do not count, and its sticks must pass through rest
/// first. Returns whether the pad changed.
fn follow_pad_owner(
    physical: &mut Physical,
    out: &mut PlayerControlSnapshot,
    devices: &mut InputDevices,
    owner: Option<Entity>,
) -> bool {
    let same_pad = physical.pad_owner == owner;
    if same_pad {
        return !same_pad;
    }
    physical.pad_owner = owner;
    physical.blocked.retain(|button| !button.is_pad());
    physical.movement_ready = true;
    physical.look_ready = true;
    release_pad(out, devices);
    true
}

/// Releases a key the client still thinks is down.
fn lift(out: &mut PlayerControlSnapshot, digit_for_key: usize, at: Tick) {
    if out.client.keys[digit_for_key].down == 0 {
        return;
    }
    key_event(&mut out.client, digit_for_key, false, at.now, at.frame);
}

/// Brings the client's key table in line with the binds. A key whose bind
/// changed is released and stays blocked until it comes up.
fn follow_binds(
    out: &mut PlayerControlSnapshot,
    binds: &BindingTable,
    inputs: &BindingTableFeed,
    physical: &mut Physical,
    tick: Tick,
    captured: bool,
    owner_changed: bool,
) {
    let mut wanted = [0u32; KEYBOARD_SLOTS];
    let in_table = binds
        .iter()
        .map(|(control, action)| (control, action, host_key_code(control)))
        .filter(|&(_, _, slot)| slot < KEYBOARD_SLOTS);
    for (control, id, slot) in in_table {
        wanted[slot] = id;
        let rebound = out.client.keys[slot].binding != id;
        let disturbed = rebound || captured || (owner_changed && control.is_pad());
        if !disturbed {
            continue;
        }
        lift(out, slot, tick);
        if inputs.is_held(control) && (captured || rebound || !inputs.went_down(control)) {
            physical.blocked.insert(control);
        }
    }
    for (slot, id) in wanted.into_iter().enumerate() {
        if out.client.keys[slot].binding == id {
            continue;
        }
        lift(out, slot, tick);
        out.client.keys[slot].binding = id;
    }
}

/// Nothing gets through to the game: every key is released and the mouse is
/// emptied.
fn capture(
    mouse: &mut MouseIn<'_, '_>,
    scripted: &mut ScriptedControlsStatus,
    physical: &mut Physical,
    out: &mut PlayerControlSnapshot,
    devices: &mut InputDevices,
    tick: Tick,
) {
    let mut travel = 0.0_f32;
    for event in mouse.motion.read() {
        travel += event.delta.x.abs() + event.delta.y.abs();
    }
    if devices.focused {
        physical.noticed_mouse(travel, devices);
    }
    mouse.wheel.clear();
    scripted.take_pointer_move();
    *mouse.carry = 0.0;
    for slot in 0..KEYBOARD_SLOTS {
        lift(out, slot, tick);
    }
    let kept_keys = out.client.keys;
    out.scenario_identifiers.clear();
    out.client = PlayerIntentFeed {
        keys: kept_keys,
        ..Default::default()
    };
    release_pad(out, devices);
    out.pad_was_ads = false;
    physical.movement_ready = false;
    physical.look_ready = false;
}

/// Reads the sticks. One that was held while the pad changed hands, or while
/// the console had the input, is ignored until it has been at rest.
fn read_pad(
    pad: &Gamepad,
    settings: &PlayerPrefsBundle,
    physical: &mut Physical,
    out: &mut PlayerControlSnapshot,
) {
    let sticks = crate::gamepad::sticks(pad, settings);
    if sticks.movement == Vec2::ZERO {
        physical.movement_ready = true;
    }
    if sticks.look == Vec2::ZERO {
        physical.look_ready = true;
    }
    if physical.movement_ready {
        out.pad_move = sticks.movement.to_array();
    }
    if physical.look_ready {
        let shaped = crate::gamepad::shaped_look(sticks.look, settings);
        out.pad_look = shaped.to_array();
        out.pad_deflection = sticks.look.length();
    }
}

/// Feeds the bound buttons' presses and releases to the client. `allowed`
/// decides which presses count: all of them normally, but a scripted run takes
/// only those its own commands hold.
fn press_binds(
    out: &mut PlayerControlSnapshot,
    binds: &BindingTable,
    inputs: &BindingTableFeed,
    physical: &Physical,
    tick: Tick,
    allowed: impl Fn(ActionBinding) -> bool,
) {
    for (control, _) in binds.iter() {
        let slot = host_key_code(control);
        if slot >= KEYBOARD_SLOTS || physical.blocked.contains(&control) {
            continue;
        }
        if inputs.went_down(control) && allowed(control) {
            key_event(&mut out.client, slot, true, tick.now, tick.frame);
        }
        if !inputs.is_held(control) {
            lift(out, slot, tick);
        }
    }
}

/// Each step of a wheel notch presses the wheel's bind once.
fn turn_wheel(
    mouse: &mut MouseIn<'_, '_>,
    binds: &BindingTable,
    out: &mut PlayerControlSnapshot,
    devices: &mut InputDevices,
    tick: Tick,
) {
    for event in mouse.wheel.read() {
        let steps = wheel_detents(event.unit, event.y, &mut mouse.carry);
        let bound = if steps > 0 {
            ActionBinding::WheelUp
        } else {
            ActionBinding::WheelDown
        };
        for _ in 0..steps.unsigned_abs() {
            pulse_wheel_binding(binds, &mut out.client, bound, tick.now, tick.frame);
        }
        if steps != 0 {
            devices.pad_prompts = false;
        }
    }
}

/// Adds up the frame's mouse movement, real and scripted, and decides whether
/// the mouse or the pad is steering the view.
fn steer(
    mouse: &mut MouseIn<'_, '_>,
    scripted: &mut ScriptedControlsStatus,
    physical: &mut Physical,
    out: &mut PlayerControlSnapshot,
    devices: &mut InputDevices,
    scripted_run: bool,
) {
    let (queued_x, queued_y) = scripted.take_pointer_move();
    let (rate_x, rate_y) = scripted.pointer_velocity().unwrap_or_default();
    out.pointer_delta_horizontal += queued_x + rate_x;
    out.pointer_delta_vertical += queued_y + rate_y;
    if !scripted_run {
        for event in mouse.motion.read() {
            out.pointer_delta_horizontal += event.delta.x;
            out.pointer_delta_vertical += event.delta.y;
        }
    } else {
        mouse.motion.clear();
    }
    let moved = out.pointer_delta_horizontal != 0.0 || out.pointer_delta_vertical != 0.0;
    if moved {
        devices.aiming_with_pad = false;
        out.pad_look = [0.0; 2];
        out.pad_deflection = 0.0;
        out.pad_turn_rate = [0.0; 2];
        physical.noticed_mouse(out.pointer_delta_horizontal.abs() + out.pointer_delta_vertical.abs(), devices);
    } else if out.pad_deflection > 0.0 {
        devices.aiming_with_pad = true;
    }
}

/// Sends the commands the console is holding: a press for each that is new
/// this frame and a release for each that went away.
fn play_scripted(out: &mut PlayerControlSnapshot, scripted: &ScriptedControlsStatus, at: Tick) {
    let now_held: BTreeSet<u32> = scripted.ids().collect();
    let was_held = std::mem::take(&mut out.scenario_identifiers);
    // A press is stamped at the start of the frame; zero means "no time" to
    // the client, so a frame starting at zero is written as minus its length.
    let frame_len = at.frame as i32;
    let pressed_at = match at.now.wrapping_sub(frame_len) {
        0 => -frame_len,
        start => start,
    };
    for &action in now_held.difference(&was_held) {
        input_cmd(&mut out.client, action, SCRIPTED_KEY_SLOT, pressed_at, at.frame);
    }
    let releases = was_held.difference(&now_held).filter_map(|&action| release_action_number(action));
    for release in releases {
        input_cmd(&mut out.client, release, SCRIPTED_KEY_SLOT, at.now, at.frame);
    }
    out.scenario_identifiers = now_held;
}

/// Records `label` as the key shown for `command` in the HUD's prompts.
fn note_command(hud: &mut OverlayControlSnapshot, order: &str, label: &str) {
    // BO2's HUD asks for the weapon-switch key by `+weapnext_inventory`
    // (`[{+weapnext_inventory}]`, e.g. the VTOL's FLIR prompt); a key bound to
    // plain `weapnext` has to answer for it as well.
    let answers_for: &[&str] = match order {
        "weapnext" | "+weapnext_inventory" => &["weapnext", "+weapnext_inventory"],
        _ => &[order],
    };
    for &name in answers_for {
        hud.binding_keys.entry(name.to_owned()).or_insert_with(|| label.to_owned());
        hud.binding_keys_all.entry(name.to_owned()).or_default().push(label.to_owned());
    }
    if hud.act_key.is_none() && (order == "+activate" || order == "+usereload") {
        hud.act_key = Some(label.to_owned());
    }
}

/// Fills in the key (or pad button) names that the HUD's prompts display.
fn fill_labels(
    hud: &mut OverlayControlSnapshot,
    binds: &BindingTable,
    pad_prompts: bool,
    style: PromptStyle,
    bo2_game: bool,
) {
    let mut relevant: Vec<_> = binds
        .iter()
        .filter(|(control, _)| control.is_pad() == pad_prompts)
        .collect::<Vec<(ActionBinding, u32)>>();
    relevant.sort_by_key(|(control, _)| host_key_code(*control));
    hud.binding_keys.clear();
    hud.binding_keys_all.clear();
    hud.act_key = None;
    hud.bound_action_buttons = Default::default();
    for (control, id) in relevant {
        let label = match control {
            // bo2zm: in a Black Ops II game, BO2's button pictures.
            ActionBinding::Pad(pad) => {
                let glyph = if bo2_game { frame::bo2_pad_glyph(pad.bo2_name(), style) } else { None };
                String::from(glyph.unwrap_or_else(|| pad.prompt(style)))
            }
            _ => button_label(control).to_uppercase(),
        };
        if let Some(command) = binds.command_on(control) {
            note_command(hud, command, &label);
        }
        for (position, slot) in hud.bound_action_buttons.iter_mut().enumerate() {
            if slot.is_none() && id == 15 + position as u32 * 2 {
                *slot = Some(label.clone());
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) fn publish_client_action_input(
    clock: Res<Time>,
    buttons: Buttons<'_, '_>,
    mut mouse: MouseIn<'_, '_>,
    mut physical: Local<Physical>,
    binds: Res<BindingTable>,
    mut scripted: ResMut<ScriptedControlsStatus>,
    timed: Timed<'_>,
    modal: Modal<'_>,
    mut overlay: ResMut<OverlayControlSnapshot>,
    settings: Res<PlayerPrefsBundle>,
    mut out: ResMut<PlayerControlSnapshot>,
    mut devices: ResMut<InputDevices>,
) {
    physical.start_window(clock.elapsed_secs());
    let active_pad = buttons.active.0;
    let pad = active_pad.and_then(|entity| buttons.gamepads.get(entity).ok());
    let inputs = BindingTableFeed::new(&buttons.keys, &buttons.mouse).with_pad(pad);
    let owner_changed = follow_pad_owner(&mut physical, &mut out, &mut devices, active_pad);
    let tick = begin_frame(&mut out, &clock, &settings);
    // bo2zm: Black Ops II's aim assist on a pad (a PC game has none).
    if devices.aiming_with_pad && pad.is_some() && frame::bo2_game() {
        out.pad_aim_assist = 2;
    }
    if binds.is_changed() {
        *mouse.carry = 0.0;
    }
    let scripted_run = crate::startup::scripted_run();
    let start_pressed = pad.is_some_and(|pad| pad.just_pressed(GamepadButton::Start));
    let modal_captured =
        modal.grabs_input() || buttons.keys.just_pressed(KeyCode::Escape) || start_pressed;
    let captured = modal_captured || !devices.focused;
    overlay.console_open = modal.console.shown;
    physical.blocked.retain(|button| inputs.is_held(*button));

    follow_binds(
        &mut out,
        &binds,
        &inputs,
        &mut physical,
        tick,
        captured,
        owner_changed,
    );
    if captured {
        capture(
            &mut mouse,
            &mut scripted,
            &mut physical,
            &mut out,
            &mut devices,
            tick,
        );
    } else {
        out.pad_sensitivity = settings.pad_sensitivity / PlayerPrefsBundle::PAD_SENSITIVITY_DEFAULT;
        out.pad_ads_sensitivity = settings.pad_ads_sensitivity;
        if let Some(pad) = pad.filter(|_| !scripted_run) {
            read_pad(pad, &settings, &mut physical, &mut out);
        }
        let allowed = |button: ActionBinding| !scripted_run || timed.is_down(button);
        press_binds(&mut out, &binds, &inputs, &physical, tick, allowed);
        if scripted_run {
            mouse.wheel.clear();
        } else {
            turn_wheel(&mut mouse, &binds, &mut out, &mut devices, tick);
        }
        steer(
            &mut mouse,
            &mut scripted,
            &mut physical,
            &mut out,
            &mut devices,
            scripted_run,
        );
    }
    if !modal_captured {
        play_scripted(&mut out, &scripted, tick);
    }
    let any_press = buttons.keys.get_just_pressed().next().is_some()
        || buttons.mouse.get_just_pressed().next().is_some();
    if devices.focused && any_press {
        devices.pad_prompts = false;
        physical.mouse_activity = 0.0;
    }
    out.last_input_pad = last_input_pad(devices.pad_prompts);
    let prompts = (devices.pad_prompts, devices.style, frame::bo2_game());
    if physical.shown_prompts != Some(prompts) || binds.is_changed() {
        physical.shown_prompts = Some(prompts);
        fill_labels(&mut overlay, &binds, prompts.0, prompts.1, prompts.2);
    }
}

/// The grab mode the pointer should switch to now, if it needs to change.
///
/// Nothing guarantees the grab stays ours: a compositor drops the constraint
/// when the window loses focus or the pointer leaves the surface, and winit
/// ignores a request for the mode already held. So a window that has just come
/// back releases the grab for one frame and takes it again on the next.
/// Without that bounce, a window returning from an alt-tab or an overview
/// keeps a `Locked` label over a pointer nothing holds, and the view stops
/// turning while every key, and the console, still work.
fn grab_change(
    current: CursorGrabMode,
    want: CursorGrabMode,
    returned: bool,
) -> Option<CursorGrabMode> {
    let bounce = returned && want != CursorGrabMode::None && current == want;
    if bounce {
        Some(CursorGrabMode::None)
    } else {
        (current != want).then_some(want)
    }
}

/// Keeps the pointer grabbed while gameplay owns it, and re-grabs it whenever
/// the window gets it back.
pub(super) fn sync_cursor_grab(
    console: Res<PromptPanelStatus>,
    script_menus: Option<Res<hud::ScriptMenus>>,
    shown_page: Option<Res<ShellScreen>>,
    mut focused: MessageReader<WindowFocused>,
    mut entered: MessageReader<CursorEntered>,
    mut primary_cursor: Query<&mut CursorOptions, With<PrimaryWindow>>,
) {
    let focus_regained = focused.read().any(|event| event.focused);
    let returned = focus_regained || entered.read().count() > 0;
    let menu_open = script_menus.is_some_and(|menus| menus.captures_input());
    let in_game = matches!(shown_page.as_deref(), Some(ShellScreen::InGame));
    let wants_grab = in_game && !console.shown && !menu_open && !crate::startup::scripted_run();
    let Ok(mut options) = primary_cursor.single_mut() else {
        return;
    };
    let want = match wants_grab {
        true => CursorGrabMode::Locked,
        false => CursorGrabMode::None,
    };
    let Some(mode) = grab_change(options.grab_mode, want, returned) else {
        return;
    };
    options.grab_mode = mode;
    // The cursor stays hidden across the one frame the grab is let go.
    if mode == want {
        options.visible = mode == CursorGrabMode::None;
    }
}

#[cfg(test)]
mod tests {
    use bevy::input::InputPlugin;

    use crate::app::gameplay::*;

    fn tick() -> Tick {
        Tick {
            now: 1000,
            frame: 10,
        }
    }

    fn bound(script: &str) -> BindingTable {
        let mut map = BindingTable::default();
        map.forget_all();
        map.run_control_script(script);
        map
    }

    fn digit_for_key(code: KeyCode) -> usize {
        host_key_code(ActionBinding::Board(code))
    }

    #[test]
    fn mouse_travel_within_a_window_switches_the_prompts_back() {
        let mut physical = Physical::default();
        let mut devices = InputDevices {
            pad_prompts: true,
            ..Default::default()
        };
        physical.noticed_mouse(1.0, &mut devices);
        assert!(devices.pad_prompts);
        physical.noticed_mouse(1.5, &mut devices);
        assert!(!devices.pad_prompts);
        assert_eq!(physical.mouse_activity, 0.0);
    }

    #[test]
    fn the_travel_window_restarts_after_a_third_of_a_second() {
        let mut physical = Physical {
            mouse_activity: 1.5,
            ..Physical::default()
        };
        physical.start_window(0.2);
        assert_eq!(physical.mouse_activity, 1.5);
        physical.start_window(0.4);
        assert_eq!(physical.mouse_activity, 0.0);
    }

    #[test]
    fn a_run_may_force_the_device_it_reports() {
        assert!(forced_device(Some("mouse")) == Some(false));
        assert_eq!(forced_device(Some("pad")), Some(true));
        assert_eq!(forced_device(Some("both")), None);
        assert_eq!(forced_device(None), None);
    }

    #[test]
    fn a_frame_starts_clean_and_takes_settings_from_the_game() {
        let mut out = PlayerControlSnapshot::default();
        out.pointer_delta_horizontal = 4.0;
        out.pad_move = [1.0, 1.0];
        out.yaw_sensitivity = 0.0;
        let mut time = Time::<()>::default();
        time.advance_by(std::time::Duration::from_millis(16));
        let settings = PlayerPrefsBundle {
            flip_look: true,
            ..PlayerPrefsBundle::default()
        };
        let tick = begin_frame(&mut out, &time, &settings);
        assert_eq!((out.pointer_delta_horizontal, out.pad_move), (0.0, [0.0; 2]));
        assert_eq!((out.yaw_sensitivity, out.pitch_per_count), (MOUSE_DEGREES, -MOUSE_DEGREES));
        assert_eq!((tick.now, tick.frame), (out.present_tick_ms, out.tick_span_ms));
        assert!(tick.frame > 0);
    }

    #[test]
    fn a_new_pad_starts_fresh() {
        let mut physical = Physical::default();
        physical.blocked.insert(ActionBinding::Board(KeyCode::KeyW));
        let mut out = PlayerControlSnapshot::default();
        out.pad_lockon = Some(7);
        let mut devices = InputDevices {
            aiming_with_pad: true,
            ..Default::default()
        };
        let pad = Some(Entity::PLACEHOLDER);
        assert!(follow_pad_owner(&mut physical, &mut out, &mut devices, pad));
        assert!(physical.movement_ready && physical.look_ready);
        assert_eq!(out.pad_lockon, None);
        assert!(!devices.aiming_with_pad);
        assert!(physical.blocked.contains(&ActionBinding::Board(KeyCode::KeyW)));
        assert!(!follow_pad_owner(
            &mut physical,
            &mut out,
            &mut devices,
            pad
        ));
    }

    #[test]
    fn the_key_table_follows_the_binds_and_lets_go_of_a_key_that_was_rebound() {
        let mut out = PlayerControlSnapshot::default();
        let mut physical = Physical::default();
        let mut keys = ButtonInput::<KeyCode>::default();
        let mouse = ButtonInput::<MouseButton>::default();
        let w = digit_for_key(KeyCode::KeyW);

        let forward = bound("bind w +forward");
        follow_binds(
            &mut out,
            &forward,
            &BindingTableFeed::new(&keys, &mouse),
            &mut physical,
            tick(),
            false,
            false,
        );
        let id = out.client.keys[w].binding;
        assert_ne!(id, 0);

        keys.press(KeyCode::KeyW);
        press_binds(
            &mut out,
            &forward,
            &BindingTableFeed::new(&keys, &mouse),
            &physical,
            tick(),
            |_| true,
        );
        assert_ne!(out.client.keys[w].down, 0);

        keys.clear_just_pressed(KeyCode::KeyW);
        let back = bound("bind w +back");
        follow_binds(
            &mut out,
            &back,
            &BindingTableFeed::new(&keys, &mouse),
            &mut physical,
            tick(),
            false,
            false,
        );
        assert_eq!(out.client.keys[w].down, 0, "the old bind's press is over");
        assert_ne!(out.client.keys[w].binding, id);
        assert!(physical.blocked.contains(&ActionBinding::Board(KeyCode::KeyW)));
    }

    #[test]
    fn a_key_held_when_input_is_taken_away_comes_up_and_stays_blocked() {
        let mut out = PlayerControlSnapshot::default();
        let mut physical = Physical::default();
        let mut keys = ButtonInput::<KeyCode>::default();
        let mouse = ButtonInput::<MouseButton>::default();
        let binds = bound("bind w +forward");
        follow_binds(
            &mut out,
            &binds,
            &BindingTableFeed::new(&keys, &mouse),
            &mut physical,
            tick(),
            false,
            false,
        );
        keys.press(KeyCode::KeyW);
        press_binds(
            &mut out,
            &binds,
            &BindingTableFeed::new(&keys, &mouse),
            &physical,
            tick(),
            |_| true,
        );
        keys.clear_just_pressed(KeyCode::KeyW);

        follow_binds(
            &mut out,
            &binds,
            &BindingTableFeed::new(&keys, &mouse),
            &mut physical,
            tick(),
            true,
            false,
        );
        assert_eq!(out.client.keys[digit_for_key(KeyCode::KeyW)].down, 0);
        assert!(physical.blocked.contains(&ActionBinding::Board(KeyCode::KeyW)));
    }

    #[test]
    fn a_press_a_run_does_not_hold_is_left_out() {
        let mut out = PlayerControlSnapshot::default();
        let mut physical = Physical::default();
        let mut keys = ButtonInput::<KeyCode>::default();
        let mouse = ButtonInput::<MouseButton>::default();
        let binds = bound("bind w +forward");
        follow_binds(
            &mut out,
            &binds,
            &BindingTableFeed::new(&keys, &mouse),
            &mut physical,
            tick(),
            false,
            false,
        );
        keys.press(KeyCode::KeyW);
        press_binds(
            &mut out,
            &binds,
            &BindingTableFeed::new(&keys, &mouse),
            &physical,
            tick(),
            |_| false,
        );
        assert_eq!(out.client.keys[digit_for_key(KeyCode::KeyW)].down, 0);
    }

    #[test]
    fn a_held_console_command_is_pressed_once_and_released_once() {
        let mut out = PlayerControlSnapshot::default();
        let mut scripted = ScriptedControlsStatus::default();
        scripted.keep_down("+forward");
        play_scripted(&mut out, &scripted, tick());
        assert_eq!(out.scenario_identifiers.len(), 1);
        let after_press = format!("{:?}", out.client.pressed_key_latch);
        play_scripted(&mut out, &scripted, tick());
        let expected = after_press;
        assert_eq!(format!("{:?}", out.client.pressed_key_latch), expected, "no second press");
        scripted.let_go("+forward");
        play_scripted(&mut out, &scripted, tick());
        assert!(out.scenario_identifiers.is_empty());
    }

    #[test]
    fn the_hud_names_the_key_for_each_command() {
        let mut hud = OverlayControlSnapshot::default();
        let note = |hud: &mut OverlayControlSnapshot, [order, key]: [&str; 2]| note_command(hud, order, key);
        note(&mut hud, ["+usereload", "F"]);
        assert_eq!(hud.act_key.as_deref(), Some("F"));
        note(&mut hud, ["+activate", "E"]);
        assert_eq!(hud.act_key.as_deref(), Some("F"), "the first key stays");
        note(&mut hud, ["weapnext", "V"]);
        let expected = Some("V");
        assert_eq!(hud.binding_keys.get("weapnext").map(String::as_str), expected);
        let inventory = hud.binding_keys.get("+weapnext_inventory").map(String::as_str);
        assert_eq!(inventory, expected);
        note(&mut hud, ["+forward", "W"]);
        note(&mut hud, ["+forward", "UP"]);
        assert_eq!(hud.binding_keys_all["+forward"], ["W", "UP"]);
        assert_eq!(hud.binding_keys["+forward"], "W");
    }

    #[test]
    fn the_default_binds_label_the_keyboard_and_not_the_pad() {
        let mut binds = BindingTable::default();
        binds.restore_stock_controls();
        let mut hud = OverlayControlSnapshot::default();
        fill_labels(&mut hud, &binds, false, PromptStyle::Generic, false);
        assert!(hud.act_key.is_some());
        assert!(hud.binding_keys.values().any(|label| label == "W"));
        fill_labels(&mut hud, &binds, true, PromptStyle::Generic, false);
        assert!(hud.binding_keys.values().all(|label| label != "W"));
    }

    #[test]
    fn a_returning_window_lets_go_of_the_grab_for_one_frame() {
        let (held, loose) = (CursorGrabMode::Locked, CursorGrabMode::None);
        assert_eq!(grab_change(held, held, true), Some(loose));
        assert_eq!(grab_change(loose, held, false), Some(held));
        assert_eq!(grab_change(held, held, false), None);
        assert_eq!(grab_change(held, loose, false), Some(loose));
        assert_eq!(grab_change(loose, loose, true), None);
    }

    fn app() -> App {
        let systems = (isolate_gameplay_input, publish_client_action_input)
            .chain()
            .after(bevy::input::InputSystems);
        let mut app = App::default();
        app.add_plugins((MinimalPlugins, InputPlugin))
            .init_resource::<BindingTable>()
            .init_resource::<ScriptedControlsStatus>()
            .init_resource::<crate::ScriptedKeys>()
            .init_resource::<super::super::scripted::ScriptedMouse>()
            .init_resource::<PromptPanelStatus>()
            .init_resource::<OverlayControlSnapshot>()
            .insert_resource(PlayerPrefsBundle::default())
            .init_resource::<PlayerControlSnapshot>()
            .init_resource::<ActivePad>()
            .insert_resource(InputDevices {
                focused: true,
                ..Default::default()
            })
            .add_systems(bevy::app::PreUpdate, systems);
        let world = app.world_mut();
        world.resource_mut::<BindingTable>().forget_all();
        world
            .resource_mut::<BindingTable>()
            .run_control_script("bind w +forward");
        app
    }

    /// The key event a keyboard sends. The input systems clear a press that
    /// is set on the resource directly, so a test goes through the event.
    fn press_w(app: &mut App) {
        let world = app.world_mut();
        let window = world.spawn_empty().id();
        let repeat = false;
        world.write_message(bevy::input::keyboard::KeyboardInput {
            key_code: KeyCode::KeyW,
            logical_key: bevy::input::keyboard::Key::Character("w".into()),
            state: bevy::input::ButtonState::Pressed,
            text: None,
            repeat,
            window,
        });
    }

    fn forward_down(app: &App) -> bool {
        let out = app.world().resource::<PlayerControlSnapshot>();
        out.client.keys[digit_for_key(KeyCode::KeyW)].down != 0
    }

    #[test]
    fn a_pressed_key_reaches_the_client_and_the_console_takes_it_away() {
        let mut app = app();
        app.update();
        press_w(&mut app);
        app.update();
        assert!(forward_down(&app), "w was pressed");

        app.world_mut().resource_mut::<PromptPanelStatus>().shown = true;
        app.update();
        assert!(!forward_down(&app), "the console has the keyboard");
        assert!(app.world().resource::<OverlayControlSnapshot>().console_open);
    }

    fn key_down(app: &App, code: KeyCode) -> bool {
        let out = app.world().resource::<PlayerControlSnapshot>();
        out.client.keys[digit_for_key(code)].down != 0
    }

    /// A script's `keydown shift` and `keydown ctrl` come out of the client's
    /// action input as the shift and ctrl keys, as the keyboard's would, and
    /// stay down when the window loses focus.
    #[test]
    fn scripted_shift_and_ctrl_reach_the_client_as_keys() {
        let mut app = app();
        let scripted_keys = super::super::scripted::drive_scripted_keys
            .after(isolate_gameplay_input)
            .before(publish_client_action_input);
        app.add_systems(PreUpdate, scripted_keys);
        let world = app.world_mut();
        world
            .resource_mut::<BindingTable>()
            .run_control_script("bind shift +breath_sprint\nbind ctrl +stance");
        app.update();
        assert!(!key_down(&app, KeyCode::ShiftLeft) && !key_down(&app, KeyCode::ControlLeft));

        let held = |app: &mut App, code: KeyCode| {
            let world = app.world_mut();
            world
                .resource_mut::<crate::ScriptedKeys>()
                .0
                .push((code, f32::INFINITY, false));
        };
        held(&mut app, KeyCode::ShiftLeft);
        app.update();
        app.update();
        assert!(key_down(&app, KeyCode::ShiftLeft), "shift is down");
        assert!(!key_down(&app, KeyCode::ControlLeft), "ctrl is not");
        held(&mut app, KeyCode::ControlLeft);
        app.update();
        assert!(key_down(&app, KeyCode::ShiftLeft) && key_down(&app, KeyCode::ControlLeft));

        let world = app.world_mut();
        world.write_message(bevy::input::keyboard::KeyboardFocusLost);
        app.update();
        app.update();
        assert!(
            key_down(&app, KeyCode::ShiftLeft) && key_down(&app, KeyCode::ControlLeft),
            "a window that lost focus does not let go of a scripted key"
        );

        for (_, left, _) in &mut app.world_mut().resource_mut::<crate::ScriptedKeys>().0 {
            *left = 0.0;
        }
        app.update();
        app.update();
        assert!(!key_down(&app, KeyCode::ShiftLeft) && !key_down(&app, KeyCode::ControlLeft));
    }

    #[test]
    fn an_unfocused_window_sends_nothing() {
        let mut app = app();
        app.world_mut().resource_mut::<InputDevices>().focused = false;
        app.update();
        press_w(&mut app);
        app.update();
        assert!(!forward_down(&app));
    }
}
