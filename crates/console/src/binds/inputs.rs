//! Reading the player's buttons for the binds: held and just-pressed on the
//! keyboard, mouse and pad, and the mouse wheel, which has no held state and
//! reaches the game as a press that lets go at once.

use bevy::input::ButtonInput;
use bevy::input::gamepad::Gamepad;
use bevy::input::keyboard::KeyCode;
use bevy::input::mouse::{MouseButton, MouseScrollUnit};
use input_t6::{ClientInput, key_event};

use super::{BindButton, KeyBinds, host_keynum};

/// Wheel lines in a notch; a trackpad reports pixels, a hundred to the notch.
const PIXELS_PER_NOTCH: f32 = 100.0;
/// The most notches a wheel's carried-over turn can hold, either way.
const MOST_CARRIED: f32 = 32.0;

/// The devices a frame's presses are read from.
pub struct BindInputs<'a> {
    keys: &'a ButtonInput<KeyCode>,
    mouse: &'a ButtonInput<MouseButton>,
    pad: Option<&'a Gamepad>,
}

impl<'a> BindInputs<'a> {
    pub fn new(keys: &'a ButtonInput<KeyCode>, mouse: &'a ButtonInput<MouseButton>) -> Self {
        Self {
            keys,
            mouse,
            pad: None,
        }
    }

    /// Adds the pad the player holds, if there is one.
    pub fn with_pad(mut self, pad: Option<&'a Gamepad>) -> Self {
        self.pad = pad;
        self
    }

    /// Whether `button` is down. The wheel never is.
    pub fn pressed(&self, button: BindButton) -> bool {
        self.read(button, true)
    }

    /// Whether `button` went down this frame. The wheel never does.
    pub fn just_pressed(&self, button: BindButton) -> bool {
        self.read(button, false)
    }

    fn read(&self, button: BindButton, held: bool) -> bool {
        match (button, held) {
            (BindButton::Key(key), true) => self.keys.pressed(key),
            (BindButton::Key(key), false) => self.keys.just_pressed(key),
            (BindButton::Mouse(mouse), true) => self.mouse.pressed(mouse),
            (BindButton::Mouse(mouse), false) => self.mouse.just_pressed(mouse),
            (BindButton::Pad(pad), true) => self
                .pad
                .is_some_and(|held| held.pressed(pad.gamepad_button())),
            (BindButton::Pad(pad), false) => self
                .pad
                .is_some_and(|held| held.just_pressed(pad.gamepad_button())),
            (BindButton::WheelUp | BindButton::WheelDown, _) => false,
        }
    }
}

/// The wheel button a turn of `y` presses; none when the wheel did not move.
pub(crate) fn wheel_button(y: f32) -> Option<BindButton> {
    if y > 0.0 {
        Some(BindButton::WheelUp)
    } else if y < 0.0 {
        Some(BindButton::WheelDown)
    } else {
        None
    }
}

/// The whole notches a turn of `y` adds up to, counting the part of a notch
/// left over from the turns before it. What is left after is kept in `carry`.
pub(crate) fn wheel_detents(unit: MouseScrollUnit, y: f32, carry: &mut f32) -> i32 {
    if !y.is_finite() {
        return 0;
    }
    let notches = match unit {
        MouseScrollUnit::Line => y,
        MouseScrollUnit::Pixel => y / PIXELS_PER_NOTCH,
    };
    *carry = (*carry + notches).clamp(-MOST_CARRIED, MOST_CARRIED);
    let whole = carry.trunc();
    *carry -= whole;
    whole as i32
}

/// Presses and lets go of the wheel button `button` for whatever command it
/// is bound to; nothing happens when it is bound to none.
pub(crate) fn pulse_wheel_binding(
    binds: &KeyBinds,
    client: &mut ClientInput,
    button: BindButton,
    now_msec: i32,
    frame_msec: u32,
) {
    let Some(command) = binds.get(button) else {
        return;
    };
    let number = host_keynum(button);
    client.keys[number].binding = command;
    key_event(client, number, true, now_msec, frame_msec);
    key_event(client, number, false, now_msec, frame_msec);
}

#[cfg(test)]
mod tests {
    use input_t6::command_id_lookup;

    use super::*;

    fn turn(unit: MouseScrollUnit, y: f32, carry: &mut f32) -> i32 {
        wheel_detents(unit, y, carry)
    }

    #[test]
    fn a_turn_up_presses_wheel_up_and_a_turn_down_wheel_down() {
        assert_eq!(wheel_button(1.0), Some(BindButton::WheelUp));
        assert_eq!(wheel_button(0.01), Some(BindButton::WheelUp));
        assert_eq!(wheel_button(-3.0), Some(BindButton::WheelDown));
    }

    #[test]
    fn no_turn_and_no_number_press_nothing() {
        assert_eq!(wheel_button(0.0), None);
        assert_eq!(wheel_button(-0.0), None);
        assert_eq!(wheel_button(f32::NAN), None);
    }

    #[test]
    fn a_line_is_a_notch() {
        let mut carry = 0.0;
        assert_eq!(turn(MouseScrollUnit::Line, 1.0, &mut carry), 1);
        assert_eq!(turn(MouseScrollUnit::Line, -2.0, &mut carry), -2);
        assert_eq!(carry, 0.0);
    }

    #[test]
    fn half_notches_add_up_to_a_whole_one() {
        let mut carry = 0.0;
        assert_eq!(turn(MouseScrollUnit::Line, 0.5, &mut carry), 0);
        assert_eq!(carry, 0.5);
        assert_eq!(turn(MouseScrollUnit::Line, 0.5, &mut carry), 1);
        assert_eq!(carry, 0.0);
    }

    #[test]
    fn a_turn_down_keeps_its_leftover_below_zero() {
        let mut carry = 0.0;
        assert_eq!(turn(MouseScrollUnit::Line, -1.5, &mut carry), -1);
        assert_eq!(carry, -0.5);
        assert_eq!(turn(MouseScrollUnit::Line, -0.5, &mut carry), -1);
    }

    #[test]
    fn a_hundred_pixels_is_a_notch() {
        let mut carry = 0.0;
        assert_eq!(turn(MouseScrollUnit::Pixel, 100.0, &mut carry), 1);
        let steps: Vec<i32> = (0..3)
            .map(|_| turn(MouseScrollUnit::Pixel, 40.0, &mut carry))
            .collect();
        assert_eq!(steps, [0, 0, 1]);
        assert!((carry - 0.2).abs() < 1e-6, "{carry}");
    }

    #[test]
    fn a_turn_that_is_not_a_number_changes_nothing() {
        let mut carry = 0.25;
        for y in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            assert_eq!(turn(MouseScrollUnit::Line, y, &mut carry), 0);
        }
        assert_eq!(carry, 0.25);
    }

    #[test]
    fn a_huge_turn_is_held_to_a_few_dozen_notches() {
        let mut carry = 0.0;
        assert_eq!(turn(MouseScrollUnit::Line, 5000.0, &mut carry), 32);
        assert_eq!(turn(MouseScrollUnit::Pixel, -1.0e9, &mut carry), -32);
    }

    #[test]
    fn a_pressed_key_is_held_and_new_until_the_frame_ends() {
        let mut keys = ButtonInput::<KeyCode>::default();
        let mouse = ButtonInput::<MouseButton>::default();
        let w = BindButton::Key(KeyCode::KeyW);
        keys.press(KeyCode::KeyW);
        let inputs = BindInputs::new(&keys, &mouse);
        assert!(inputs.pressed(w));
        assert!(inputs.just_pressed(w));
        assert!(!inputs.pressed(BindButton::Key(KeyCode::KeyS)));

        keys.clear();
        let inputs = BindInputs::new(&keys, &mouse);
        assert!(inputs.pressed(w));
        assert!(!inputs.just_pressed(w));
    }

    #[test]
    fn the_mouse_is_read_the_same_way() {
        let keys = ButtonInput::<KeyCode>::default();
        let mut mouse = ButtonInput::<MouseButton>::default();
        mouse.press(MouseButton::Right);
        let inputs = BindInputs::new(&keys, &mouse);
        let right = BindButton::Mouse(MouseButton::Right);
        assert!(inputs.pressed(right));
        assert!(inputs.just_pressed(right));
        assert!(!inputs.pressed(BindButton::Mouse(MouseButton::Left)));
    }

    #[test]
    fn the_wheel_and_a_pad_nobody_holds_are_never_down() {
        let mut keys = ButtonInput::<KeyCode>::default();
        let mut mouse = ButtonInput::<MouseButton>::default();
        keys.press(KeyCode::Space);
        mouse.press(MouseButton::Left);
        let inputs = BindInputs::new(&keys, &mouse).with_pad(None);
        for button in [
            BindButton::WheelUp,
            BindButton::WheelDown,
            BindButton::Pad(crate::PadButton::South),
        ] {
            assert!(!inputs.pressed(button), "{button:?}");
            assert!(!inputs.just_pressed(button), "{button:?}");
        }
    }

    #[test]
    fn a_wheel_pulse_presses_and_releases_the_command_bound_to_it() {
        let mut binds = KeyBinds::default();
        let command = command_id_lookup("weapprev").expect("a command the game has");
        binds.set(BindButton::WheelDown, command);
        let mut client = ClientInput::default();
        pulse_wheel_binding(&binds, &mut client, BindButton::WheelDown, 1000, 8);
        let key = client.keys[host_keynum(BindButton::WheelDown)];
        assert_eq!(key.binding, command);
        assert_eq!(key.down, 0);
    }

    #[test]
    fn a_wheel_turn_with_nothing_bound_to_it_touches_nothing() {
        let mut binds = KeyBinds::default();
        let command = command_id_lookup("weapprev").expect("a command the game has");
        binds.set(BindButton::WheelDown, command);
        let mut client = ClientInput::default();
        pulse_wheel_binding(&binds, &mut client, BindButton::WheelUp, 1000, 8);
        assert_eq!(client.keys[host_keynum(BindButton::WheelUp)].binding, 0);
        assert_eq!(client.keys[host_keynum(BindButton::WheelDown)].binding, 0);
    }
}
