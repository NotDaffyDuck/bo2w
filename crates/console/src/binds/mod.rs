//! Binds: which command each key, mouse button, wheel turn and pad button runs.
//!
//! - `keys` names the keyboard and mouse buttons and numbers them for the host.
//! - `pad` is the gamepad's buttons and the stock layouts for them.
//! - `map` holds the binds and reads `bind`, `unbind` and `unbindall`.
//! - `inputs` reads what the player is pressing, wheel included.

use bevy::input::keyboard::KeyCode;
use bevy::input::mouse::MouseButton;

mod inputs;
mod keys;
mod map;
mod pad;

pub use inputs::BindInputs;
pub(crate) use inputs::{pulse_wheel_binding, wheel_button, wheel_detents};
pub use keys::{bindable_keys, display_button, host_keynum, parse_button_name};
pub use map::KeyBinds;
pub use pad::{PadButton, pad_layout};

/// The binds a new player starts with, as the lines `bind` reads.
pub const DEFAULT_CONTROLS: &str = include_str!("../../assets/default_controls.cfg");

/// Something a player can press that a command can be bound to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BindButton {
    Key(KeyCode),
    Mouse(MouseButton),
    WheelUp,
    WheelDown,
    Pad(PadButton),
}

impl BindButton {
    pub fn is_pad(self) -> bool {
        matches!(self, Self::Pad(_))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_pad_button_is_a_pad_button() {
        assert!(BindButton::Pad(PadButton::South).is_pad());
        assert!(!BindButton::Key(KeyCode::KeyA).is_pad());
        assert!(!BindButton::Mouse(MouseButton::Left).is_pad());
        assert!(!BindButton::WheelUp.is_pad());
        assert!(!BindButton::WheelDown.is_pad());
    }

    #[test]
    fn the_default_controls_are_bind_lines_and_comments_and_nothing_else() {
        for line in DEFAULT_CONTROLS.lines() {
            let line = line.trim();
            assert!(
                line.is_empty() || line.starts_with("//") || line.starts_with("bind "),
                "{line:?}"
            );
            assert!(!line.contains(';'), "{line:?}");
        }
    }
}
