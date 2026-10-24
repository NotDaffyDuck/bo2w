//! The player's devices become the client's action input each frame: keys,
//! mouse and pad go through the binds, and the pointer stays held while play
//! owns it.

use std::collections::{BTreeSet, HashSet};

use bevy::{
    ecs::system::SystemParam,
    input::{
        gamepad::{Gamepad, GamepadButton},
        mouse::{MouseMotion, MouseWheel},
    },
    prelude::*,
    window::{CursorEntered, CursorGrabMode, CursorOptions, PrimaryWindow, WindowFocused},
};
use frame::{ActivePad, AppScreen, GameSettings, HudInputView, InputDevices, PromptStyle};
use input_t6::{ClientInput, KEY_COUNT, SCRIPT_KEYNUM, input_cmd, key_event, key_up_command_id};
use net::{ClientActionInput, frame_time_msec, key_frame_msec};

use super::{scripted::Timed, state::ConsoleState};
use crate::{
    BindButton, ConsoleInputState, KeyBinds,
    binds::{BindInputs, display_button, host_keynum, pulse_wheel_binding, wheel_detents},
};

/// Degrees of turn for one count of mouse travel, in BO2's defaults.
const MOUSE_DEGREES: f32 = 0.022;
/// Mouse travel, within one window of time, that says the player has picked
/// the mouse up.
const MOUSE_NOTICED: f32 = 2.0;
const MOUSE_WINDOW_SECONDS: f32 = 0.3;

/// With the console open the game sees no keys and no mouse.
pub(super) fn isolate_gameplay_input(
    console: Res<ConsoleState>,
    mut keys: ResMut<ButtonInput<KeyCode>>,
    mut mouse: MessageReader<MouseMotion>,
) {
    if !console.open {
        return;
    }
    keys.reset_all();
    mouse.clear();
}

/// When this frame happens, in the units the client's commands use.
#[derive(Clone, Copy)]
struct Tick {
    now: i32,
    frame: u32,
}

/// What the player has in their hands.
#[derive(SystemParam)]
pub(super) struct Buttons<'w, 's> {
    keys: Res<'w, ButtonInput<KeyCode>>,
    mouse: Res<'w, ButtonInput<MouseButton>>,
    gamepads: Query<'w, 's, &'static Gamepad>,
    active: Res<'w, ActivePad>,
}

/// What the mouse did since the last frame.
#[derive(SystemParam)]
pub(super) struct MouseIn<'w, 's> {
    motion: MessageReader<'w, 's, MouseMotion>,
    wheel: MessageReader<'w, 's, MouseWheel>,
    /// The part of a wheel notch not yet turned into a whole step.
    carry: Local<'s, f32>,
}

/// Whether something other than play has the input.
#[derive(SystemParam)]
pub(super) struct Modal<'w> {
    console: Res<'w, ConsoleState>,
    menus: Option<Res<'w, hud::ScriptMenus>>,
}

impl Modal<'_> {
    fn grabs_input(&self) -> bool {
        self.console.open
            || self
                .menus
                .as_ref()
                .is_some_and(|menus| menus.captures_input())
    }
}

/// What the input system remembers from one frame to the next.
#[derive(Default)]
pub(super) struct Physical {
    active_pad: Option<Entity>,
    /// Buttons held down since before they could count: a press that began
    /// while the console had the input, or under another bind.
    blocked: HashSet<BindButton>,
    movement_ready: bool,
    look_ready: bool,
    mouse_activity: f32,
    mouse_activity_start: f32,
    prompts: Option<(bool, PromptStyle)>,
}

impl Physical {
    fn start_window(&mut self, elapsed: f32) {
        if elapsed - self.mouse_activity_start > MOUSE_WINDOW_SECONDS {
            self.mouse_activity = 0.0;
            self.mouse_activity_start = elapsed;
        }
    }

    /// Counts mouse travel; enough of it within a window switches the prompts
    /// back to the keyboard's.
    fn noticed_mouse(&mut self, travel: f32, devices: &mut InputDevices) {
        self.mouse_activity += travel;
        if self.mouse_activity >= MOUSE_NOTICED {
            devices.pad_prompts = false;
            self.mouse_activity = 0.0;
        }
    }
}

/// The device the command reports as last used. A scripted run plays as a pad
/// unless `BO2MP_LAST_INPUT=mouse`, so a test can check the mouse's
/// dual-wield triggers.
fn last_input_pad(pad_prompts: bool) -> bool {
    static FORCED: std::sync::OnceLock<Option<bool>> = std::sync::OnceLock::new();
    let forced =
        *FORCED.get_or_init(|| forced_device(std::env::var("BO2MP_LAST_INPUT").ok().as_deref()));
    forced.unwrap_or(pad_prompts || crate::startup::scripted_run())
}

fn forced_device(value: Option<&str>) -> Option<bool> {
    match value {
        Some("mouse") => Some(false),
        Some("pad") => Some(true),
        _ => None,
    }
}

/// Clears what the last frame left, and restates what comes from settings.
fn begin_frame(out: &mut ClientActionInput, time: &Time, settings: &GameSettings) -> Tick {
    out.pad_aim_assist = 0;
    out.pad_move = [0.0; 2];
    out.pad_look = [0.0; 2];
    out.pad_look_delta = [0.0; 2];
    out.pad_deflection = 0.0;
    out.mouse_x = 0.0;
    out.mouse_y = 0.0;
    out.frame_msec = key_frame_msec(time.delta_secs());
    out.now_msec = frame_time_msec(time.elapsed_secs());
    out.sensitivity = settings.sensitivity;
    if out.m_yaw == 0.0 {
        out.m_yaw = MOUSE_DEGREES;
    }
    out.m_pitch = if settings.invert_mouse {
        -MOUSE_DEGREES
    } else {
        MOUSE_DEGREES
    };
    if out.fov_scale == 0.0 {
        out.fov_scale = 1.0;
    }
    Tick {
        now: out.now_msec,
        frame: out.frame_msec,
    }
}

/// Forgets the pad's turning and aim help.
fn release_pad(out: &mut ClientActionInput, devices: &mut InputDevices) {
    out.pad_turn_rate = [0.0; 2];
    out.pad_lockon = None;
    out.pad_autoaim = None;
    devices.aiming_with_pad = false;
}

/// Notes which pad is in use. A new one starts fresh: its buttons held on
/// arrival do not count, and its sticks must pass through rest first.
fn follow_pad_owner(
    physical: &mut Physical,
    out: &mut ClientActionInput,
    devices: &mut InputDevices,
    owner: Option<Entity>,
) -> bool {
    let changed = physical.active_pad != owner;
    physical.active_pad = owner;
    if changed {
        physical.blocked.retain(|button| !button.is_pad());
        physical.movement_ready = true;
        physical.look_ready = true;
        release_pad(out, devices);
    }
    changed
}

fn lift(out: &mut ClientActionInput, key_num: usize, tick: Tick) {
    if out.client.keys[key_num].down != 0 {
        key_event(&mut out.client, key_num, false, tick.now, tick.frame);
    }
}

/// Makes the client's key table the binds' table. A key whose bind changed is
/// let go, and stays blocked until it comes up.
fn follow_binds(
    out: &mut ClientActionInput,
    binds: &KeyBinds,
    inputs: &BindInputs,
    physical: &mut Physical,
    tick: Tick,
    captured: bool,
    owner_changed: bool,
) {
    let mut current = [0u32; KEY_COUNT];
    for (button, id) in binds.iter() {
        let key_num = host_keynum(button);
        if key_num >= KEY_COUNT {
            continue;
        }
        current[key_num] = id;
        let remapped = out.client.keys[key_num].binding != id;
        if !(remapped || captured || (owner_changed && button.is_pad())) {
            continue;
        }
        lift(out, key_num, tick);
        if inputs.pressed(button) && (captured || remapped || !inputs.just_pressed(button)) {
            physical.blocked.insert(button);
        }
    }
    for (key_num, id) in current.into_iter().enumerate() {
        if out.client.keys[key_num].binding != id {
            lift(out, key_num, tick);
            out.client.keys[key_num].binding = id;
        }
    }
}

/// Nothing reaches the game: every key comes up, and the mouse is drained.
fn capture(
    mouse: &mut MouseIn<'_, '_>,
    scripted: &mut ConsoleInputState,
    physical: &mut Physical,
    out: &mut ClientActionInput,
    devices: &mut InputDevices,
    tick: Tick,
) {
    let travel: f32 = mouse
        .motion
        .read()
        .map(|event| event.delta.x.abs() + event.delta.y.abs())
        .sum();
    if devices.focused {
        physical.noticed_mouse(travel, devices);
    }
    mouse.wheel.clear();
    scripted.take_mouse();
    *mouse.carry = 0.0;
    for key_num in 0..KEY_COUNT {
        lift(out, key_num, tick);
    }
    let keys = out.client.keys;
    out.scripted_ids.clear();
    out.client = ClientInput {
        keys,
        ..Default::default()
    };
    release_pad(out, devices);
    out.pad_was_ads = false;
    physical.movement_ready = false;
    physical.look_ready = false;
}

/// The sticks. A stick that was held while the pad changed hands, or while the
/// console had the input, does nothing until it has rested.
fn read_pad(
    pad: &Gamepad,
    settings: &GameSettings,
    physical: &mut Physical,
    out: &mut ClientActionInput,
) {
    let sticks = crate::gamepad::sticks(pad, settings);
    physical.movement_ready |= sticks.movement == Vec2::ZERO;
    physical.look_ready |= sticks.look == Vec2::ZERO;
    if physical.movement_ready {
        out.pad_move = [sticks.movement.x, sticks.movement.y];
    }
    if physical.look_ready {
        let look = crate::gamepad::shaped_look(sticks.look, settings);
        out.pad_look = [look.x, look.y];
        out.pad_deflection = sticks.look.length();
    }
}

/// Sends the bound buttons' presses and releases to the client. `allowed` says
/// which presses count: all of them, except in a scripted run, which takes
/// only the ones its own commands hold.
fn press_binds(
    out: &mut ClientActionInput,
    binds: &KeyBinds,
    inputs: &BindInputs,
    physical: &Physical,
    tick: Tick,
    allowed: impl Fn(BindButton) -> bool,
) {
    for (button, _) in binds.iter() {
        let key_num = host_keynum(button);
        if key_num >= KEY_COUNT || physical.blocked.contains(&button) {
            continue;
        }
        if inputs.just_pressed(button) && allowed(button) {
            key_event(&mut out.client, key_num, true, tick.now, tick.frame);
        }
        if !inputs.pressed(button) {
            lift(out, key_num, tick);
        }
    }
}

/// A wheel notch is a press of the wheel's bind, once for each step.
fn turn_wheel(
    mouse: &mut MouseIn<'_, '_>,
    binds: &KeyBinds,
    out: &mut ClientActionInput,
    devices: &mut InputDevices,
    tick: Tick,
) {
    for event in mouse.wheel.read() {
        let steps = wheel_detents(event.unit, event.y, &mut mouse.carry);
        let button = if steps > 0 {
            BindButton::WheelUp
        } else {
            BindButton::WheelDown
        };
        for _ in 0..steps.unsigned_abs() {
            pulse_wheel_binding(binds, &mut out.client, button, tick.now, tick.frame);
        }
        if steps != 0 {
            devices.pad_prompts = false;
        }
    }
}

/// Adds the frame's mouse travel, real and scripted, and settles whether the
/// view is steered by the mouse or the pad.
fn steer(
    mouse: &mut MouseIn<'_, '_>,
    scripted: &mut ConsoleInputState,
    physical: &mut Physical,
    out: &mut ClientActionInput,
    devices: &mut InputDevices,
    scripted_run: bool,
) {
    let (queued_x, queued_y) = scripted.take_mouse();
    let (rate_x, rate_y) = scripted.mouse_rate().unwrap_or((0.0, 0.0));
    out.mouse_x += queued_x + rate_x;
    out.mouse_y += queued_y + rate_y;
    for event in mouse.motion.read() {
        if !scripted_run {
            out.mouse_x += event.delta.x;
            out.mouse_y += event.delta.y;
        }
    }
    if out.mouse_x != 0.0 || out.mouse_y != 0.0 {
        devices.aiming_with_pad = false;
        out.pad_look = [0.0; 2];
        out.pad_deflection = 0.0;
        out.pad_turn_rate = [0.0; 2];
        physical.noticed_mouse(out.mouse_x.abs() + out.mouse_y.abs(), devices);
    } else if out.pad_deflection > 0.0 {
        devices.aiming_with_pad = true;
    }
}

/// Sends the commands the console holds down: a press for each one new since
/// the last frame, a release for each one gone.
fn play_scripted(out: &mut ClientActionInput, scripted: &ConsoleInputState, tick: Tick) {
    let now_held: BTreeSet<u32> = scripted.ids().collect();
    let pressed: Vec<u32> = now_held.difference(&out.scripted_ids).copied().collect();
    let released: Vec<u32> = out.scripted_ids.difference(&now_held).copied().collect();
    for id in pressed {
        let since = tick.now.wrapping_sub(tick.frame as i32);
        let since = if since == 0 {
            -(tick.frame as i32)
        } else {
            since
        };
        input_cmd(&mut out.client, id, SCRIPT_KEYNUM, since, tick.frame);
    }
    for id in released {
        if let Some(up) = key_up_command_id(id) {
            input_cmd(&mut out.client, up, SCRIPT_KEYNUM, tick.now, tick.frame);
        }
    }
    out.scripted_ids = now_held;
}

/// Files `label` as the key for `command` in the HUD's prompts.
fn note_command(hud: &mut HudInputView, command: &str, label: &str) {
    // BO2's HUD asks for the switch-weapon key by its own name
    // (`[{+weapnext_inventory}]`, the VTOL's FLIR prompt); a key bound to
    // plain `weapnext` answers it too.
    let names: &[&str] = if matches!(command, "weapnext" | "+weapnext_inventory") {
        &["weapnext", "+weapnext_inventory"]
    } else {
        &[command]
    };
    for name in names {
        hud.binding_keys
            .entry((*name).to_owned())
            .or_insert_with(|| label.to_owned());
        hud.binding_keys_all
            .entry((*name).to_owned())
            .or_default()
            .push(label.to_owned());
    }
    if matches!(command, "+activate" | "+usereload") && hud.use_key.is_none() {
        hud.use_key = Some(label.to_owned());
    }
}

/// Names the keys, or the pad's buttons, that the HUD's prompts show.
fn fill_labels(hud: &mut HudInputView, binds: &KeyBinds, pad_prompts: bool, style: PromptStyle) {
    let mut shown: Vec<_> = binds
        .iter()
        .filter(|(button, _)| button.is_pad() == pad_prompts)
        .collect();
    shown.sort_by_key(|(button, _)| host_keynum(*button));
    hud.binding_keys.clear();
    hud.binding_keys_all.clear();
    hud.use_key = None;
    hud.action_slot_keys = [None, None, None, None];
    for (button, id) in shown {
        let label = match button {
            BindButton::Pad(pad) => pad.prompt(style).to_owned(),
            _ => display_button(button).to_uppercase(),
        };
        if let Some(command) = binds.binding_name(button) {
            note_command(hud, command, &label);
        }
        for (index, slot) in hud.action_slot_keys.iter_mut().enumerate() {
            if id == 15 + index as u32 * 2 && slot.is_none() {
                *slot = Some(label.clone());
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) fn publish_client_action_input(
    time: Res<Time>,
    buttons: Buttons<'_, '_>,
    mut mouse: MouseIn<'_, '_>,
    mut physical: Local<Physical>,
    binds: Res<KeyBinds>,
    mut scripted: ResMut<ConsoleInputState>,
    timed: Timed<'_>,
    modal: Modal<'_>,
    mut hud_input: ResMut<HudInputView>,
    settings: Res<GameSettings>,
    mut out: ResMut<ClientActionInput>,
    mut devices: ResMut<InputDevices>,
) {
    physical.start_window(time.elapsed_secs());
    let pad = buttons
        .active
        .0
        .and_then(|entity| buttons.gamepads.get(entity).ok());
    let inputs = BindInputs::new(&buttons.keys, &buttons.mouse).with_pad(pad);
    let owner_changed = follow_pad_owner(&mut physical, &mut out, &mut devices, buttons.active.0);
    let tick = begin_frame(&mut out, &time, &settings);
    if binds.is_changed() {
        *mouse.carry = 0.0;
    }
    let scripted_run = crate::startup::scripted_run();
    let modal_captured = modal.grabs_input()
        || buttons.keys.just_pressed(KeyCode::Escape)
        || pad.is_some_and(|pad| pad.just_pressed(GamepadButton::Start));
    let captured = !devices.focused || modal_captured;
    hud_input.console_open = modal.console.open;
    physical.blocked.retain(|button| inputs.pressed(*button));

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
        out.pad_sensitivity = settings.pad_sensitivity / GameSettings::PAD_SENSITIVITY_DEFAULT;
        out.pad_ads_sensitivity = settings.pad_ads_sensitivity;
        if let Some(pad) = pad.filter(|_| !scripted_run) {
            read_pad(pad, &settings, &mut physical, &mut out);
        }
        press_binds(&mut out, &binds, &inputs, &physical, tick, |button| {
            !scripted_run || timed.holds(button)
        });
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
    if devices.focused
        && (buttons.keys.get_just_pressed().next().is_some()
            || buttons.mouse.get_just_pressed().next().is_some())
    {
        devices.pad_prompts = false;
        physical.mouse_activity = 0.0;
    }
    out.last_input_pad = last_input_pad(devices.pad_prompts);
    let prompts = (devices.pad_prompts, devices.style);
    if physical.prompts != Some(prompts) || binds.is_changed() {
        physical.prompts = Some(prompts);
        fill_labels(&mut hud_input, &binds, prompts.0, prompts.1);
    }
}

/// What the pointer's grab should become, if it should change now.
///
/// The grab is not ours alone to keep: a compositor drops the constraint when
/// the window loses focus or the pointer leaves the surface. Asking for the
/// mode already held does nothing inside winit, so a window that has just come
/// back lets go for one frame and takes the grab again on the next. Without
/// that bounce a window back from an alt-tab or an overview keeps a `Locked`
/// label over a pointer nothing holds, and the view stops turning while every
/// key, and the console, still work.
fn grab_change(
    current: CursorGrabMode,
    want: CursorGrabMode,
    returned: bool,
) -> Option<CursorGrabMode> {
    if returned && want != CursorGrabMode::None && current == want {
        return Some(CursorGrabMode::None);
    }
    (current != want).then_some(want)
}

/// Holds the pointer for as long as gameplay owns it, and takes it back
/// whenever the window does.
pub(super) fn sync_cursor_grab(
    console: Res<ConsoleState>,
    script_menus: Option<Res<hud::ScriptMenus>>,
    screen: Option<Res<AppScreen>>,
    mut focused: MessageReader<WindowFocused>,
    mut entered: MessageReader<CursorEntered>,
    mut windows: Query<&mut CursorOptions, With<PrimaryWindow>>,
) {
    let regained = focused.read().any(|event| event.focused);
    let returned = entered.read().count() > 0 || regained;
    let menu_open = script_menus.is_some_and(|menus| menus.captures_input());
    let in_game = screen.is_some_and(|screen| matches!(*screen, AppScreen::InGame));
    let grab = in_game && !console.open && !menu_open && !crate::startup::scripted_run();
    let Ok(mut cursor) = windows.single_mut() else {
        return;
    };
    let want = if grab {
        CursorGrabMode::Locked
    } else {
        CursorGrabMode::None
    };
    if let Some(mode) = grab_change(cursor.grab_mode, want, returned) {
        cursor.grab_mode = mode;
        // The cursor stays hidden across the one frame the grab is let go.
        if mode == want {
            cursor.visible = mode == CursorGrabMode::None;
        }
    }
}

#[cfg(test)]
mod tests {
    use bevy::input::InputPlugin;

    use super::*;

    fn tick() -> Tick {
        Tick {
            now: 1000,
            frame: 10,
        }
    }

    fn bound(script: &str) -> KeyBinds {
        let mut binds = KeyBinds::default();
        binds.clear_all();
        binds.apply_script(script);
        binds
    }

    fn key_num(code: KeyCode) -> usize {
        host_keynum(BindButton::Key(code))
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
        assert_eq!(forced_device(Some("mouse")), Some(false));
        assert_eq!(forced_device(Some("pad")), Some(true));
        assert_eq!(forced_device(Some("both")), None);
        assert_eq!(forced_device(None), None);
    }

    #[test]
    fn a_frame_starts_clean_and_takes_settings_from_the_game() {
        let mut out = ClientActionInput::default();
        out.mouse_x = 4.0;
        out.pad_move = [1.0, 1.0];
        out.m_yaw = 0.0;
        let mut time = Time::<()>::default();
        time.advance_by(std::time::Duration::from_millis(16));
        let settings = GameSettings {
            invert_mouse: true,
            ..GameSettings::default()
        };
        let tick = begin_frame(&mut out, &time, &settings);
        assert_eq!((out.mouse_x, out.pad_move), (0.0, [0.0; 2]));
        assert_eq!((out.m_yaw, out.m_pitch), (MOUSE_DEGREES, -MOUSE_DEGREES));
        assert_eq!((tick.now, tick.frame), (out.now_msec, out.frame_msec));
        assert!(tick.frame > 0);
    }

    #[test]
    fn a_new_pad_starts_fresh() {
        let mut physical = Physical::default();
        physical.blocked.insert(BindButton::Key(KeyCode::KeyW));
        let mut out = ClientActionInput::default();
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
        assert!(physical.blocked.contains(&BindButton::Key(KeyCode::KeyW)));
        assert!(!follow_pad_owner(
            &mut physical,
            &mut out,
            &mut devices,
            pad
        ));
    }

    #[test]
    fn the_key_table_follows_the_binds_and_lets_go_of_a_key_that_was_rebound() {
        let mut out = ClientActionInput::default();
        let mut physical = Physical::default();
        let mut keys = ButtonInput::<KeyCode>::default();
        let mouse = ButtonInput::<MouseButton>::default();
        let w = key_num(KeyCode::KeyW);

        let forward = bound("bind w +forward");
        follow_binds(
            &mut out,
            &forward,
            &BindInputs::new(&keys, &mouse),
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
            &BindInputs::new(&keys, &mouse),
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
            &BindInputs::new(&keys, &mouse),
            &mut physical,
            tick(),
            false,
            false,
        );
        assert_eq!(out.client.keys[w].down, 0, "the old bind's press is over");
        assert_ne!(out.client.keys[w].binding, id);
        assert!(physical.blocked.contains(&BindButton::Key(KeyCode::KeyW)));
    }

    #[test]
    fn a_key_held_when_input_is_taken_away_comes_up_and_stays_blocked() {
        let mut out = ClientActionInput::default();
        let mut physical = Physical::default();
        let mut keys = ButtonInput::<KeyCode>::default();
        let mouse = ButtonInput::<MouseButton>::default();
        let binds = bound("bind w +forward");
        follow_binds(
            &mut out,
            &binds,
            &BindInputs::new(&keys, &mouse),
            &mut physical,
            tick(),
            false,
            false,
        );
        keys.press(KeyCode::KeyW);
        press_binds(
            &mut out,
            &binds,
            &BindInputs::new(&keys, &mouse),
            &physical,
            tick(),
            |_| true,
        );
        keys.clear_just_pressed(KeyCode::KeyW);

        follow_binds(
            &mut out,
            &binds,
            &BindInputs::new(&keys, &mouse),
            &mut physical,
            tick(),
            true,
            false,
        );
        assert_eq!(out.client.keys[key_num(KeyCode::KeyW)].down, 0);
        assert!(physical.blocked.contains(&BindButton::Key(KeyCode::KeyW)));
    }

    #[test]
    fn a_press_a_run_does_not_hold_is_left_out() {
        let mut out = ClientActionInput::default();
        let mut physical = Physical::default();
        let mut keys = ButtonInput::<KeyCode>::default();
        let mouse = ButtonInput::<MouseButton>::default();
        let binds = bound("bind w +forward");
        follow_binds(
            &mut out,
            &binds,
            &BindInputs::new(&keys, &mouse),
            &mut physical,
            tick(),
            false,
            false,
        );
        keys.press(KeyCode::KeyW);
        press_binds(
            &mut out,
            &binds,
            &BindInputs::new(&keys, &mouse),
            &physical,
            tick(),
            |_| false,
        );
        assert_eq!(out.client.keys[key_num(KeyCode::KeyW)].down, 0);
    }

    #[test]
    fn a_held_console_command_is_pressed_once_and_released_once() {
        let mut out = ClientActionInput::default();
        let mut scripted = ConsoleInputState::default();
        scripted.hold("+forward");
        play_scripted(&mut out, &scripted, tick());
        assert_eq!(out.scripted_ids.len(), 1);
        let after_press = format!("{:?}", out.client.kb);
        play_scripted(&mut out, &scripted, tick());
        assert_eq!(
            format!("{:?}", out.client.kb),
            after_press,
            "no second press"
        );
        scripted.release("+forward");
        play_scripted(&mut out, &scripted, tick());
        assert!(out.scripted_ids.is_empty());
    }

    #[test]
    fn the_hud_names_the_key_for_each_command() {
        let mut hud = HudInputView::default();
        note_command(&mut hud, "+usereload", "F");
        assert_eq!(hud.use_key.as_deref(), Some("F"));
        note_command(&mut hud, "+activate", "E");
        assert_eq!(hud.use_key.as_deref(), Some("F"), "the first key stays");
        note_command(&mut hud, "weapnext", "V");
        assert_eq!(
            hud.binding_keys.get("weapnext").map(String::as_str),
            Some("V")
        );
        assert_eq!(
            hud.binding_keys
                .get("+weapnext_inventory")
                .map(String::as_str),
            Some("V")
        );
        note_command(&mut hud, "+forward", "W");
        note_command(&mut hud, "+forward", "UP");
        assert_eq!(hud.binding_keys_all["+forward"], ["W", "UP"]);
        assert_eq!(hud.binding_keys["+forward"], "W");
    }

    #[test]
    fn the_default_binds_label_the_keyboard_and_not_the_pad() {
        let mut binds = KeyBinds::default();
        binds.apply_defaults();
        let mut hud = HudInputView::default();
        fill_labels(&mut hud, &binds, false, PromptStyle::Generic);
        assert!(hud.use_key.is_some());
        assert!(hud.binding_keys.values().any(|label| label == "W"));
        fill_labels(&mut hud, &binds, true, PromptStyle::Generic);
        assert!(hud.binding_keys.values().all(|label| label != "W"));
    }

    #[test]
    fn a_returning_window_lets_go_of_the_grab_for_one_frame() {
        use CursorGrabMode::{Locked, None as Free};
        assert_eq!(grab_change(Locked, Locked, true), Some(Free));
        assert_eq!(grab_change(Free, Locked, false), Some(Locked));
        assert_eq!(grab_change(Locked, Locked, false), None);
        assert_eq!(grab_change(Locked, Free, false), Some(Free));
        assert_eq!(grab_change(Free, Free, true), None);
    }

    fn app() -> App {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, InputPlugin))
            .init_resource::<KeyBinds>()
            .init_resource::<ConsoleInputState>()
            .init_resource::<crate::ScriptedKeys>()
            .init_resource::<super::super::scripted::ScriptedMouse>()
            .init_resource::<ConsoleState>()
            .init_resource::<HudInputView>()
            .insert_resource(GameSettings::default())
            .init_resource::<ClientActionInput>()
            .init_resource::<ActivePad>()
            .insert_resource(InputDevices {
                focused: true,
                ..Default::default()
            })
            .add_systems(
                PreUpdate,
                (isolate_gameplay_input, publish_client_action_input)
                    .chain()
                    .after(bevy::input::InputSystems),
            );
        app.world_mut().resource_mut::<KeyBinds>().clear_all();
        app.world_mut()
            .resource_mut::<KeyBinds>()
            .apply_script("bind w +forward");
        app
    }

    /// The key event a keyboard sends. The input systems clear a press that
    /// is set on the resource directly, so a test goes through the event.
    fn press_w(app: &mut App) {
        let window = app.world_mut().spawn_empty().id();
        app.world_mut()
            .write_message(bevy::input::keyboard::KeyboardInput {
                key_code: KeyCode::KeyW,
                logical_key: bevy::input::keyboard::Key::Character("w".into()),
                state: bevy::input::ButtonState::Pressed,
                text: None,
                repeat: false,
                window,
            });
    }

    fn forward_down(app: &App) -> bool {
        let out = app.world().resource::<ClientActionInput>();
        out.client.keys[key_num(KeyCode::KeyW)].down != 0
    }

    #[test]
    fn a_pressed_key_reaches_the_client_and_the_console_takes_it_away() {
        let mut app = app();
        app.update();
        press_w(&mut app);
        app.update();
        assert!(forward_down(&app), "w was pressed");

        app.world_mut().resource_mut::<ConsoleState>().open = true;
        app.update();
        assert!(!forward_down(&app), "the console has the keyboard");
        assert!(app.world().resource::<HudInputView>().console_open);
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
