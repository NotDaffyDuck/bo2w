//! Binds: which command each key, mouse button, wheel turn and pad button runs.
//!
//! - `keys` names the keyboard and mouse buttons and numbers them for the host.
//! - `pad` is the gamepad's buttons and the stock layouts for them.
//! - `map` holds the binds and reads `bind`, `unbind` and `unbindall`.
//! - `inputs` reads what the player is pressing, wheel included.

mod inputs;
mod keys;
mod map;
mod pad;

pub use inputs::BindingTableFeed;
use bevy::input::keyboard::KeyCode;
use bevy::input::mouse::MouseButton;
pub(crate) use inputs::{pulse_wheel_binding, wheel_button, wheel_detents};
pub use keys::key_name_list;
pub use keys::button_label;
pub use keys::host_key_code;
pub use keys::button_from_name;
pub use map::BindingTable;
pub use pad::{GamepadKey, pad_layout};

/// The binds a new player starts with, as the lines `bind` reads.
pub const STOCK_BINDINGS: &str = include_str!("../../assets/default_controls.cfg");

/// Something a player can press that a command can be bound to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ActionBinding {
    Board(KeyCode),
    Pointer(MouseButton),
    WheelUp,
    WheelDown,
    Pad(GamepadKey),
}

impl ActionBinding {
    pub fn is_pad(self) -> bool {
        let control = self;
        matches!(control, Self::Pad(_))
    }
}

#[cfg(test)]
mod tests {

    use crate::keymap::*;

    #[test]
    fn only_a_pad_button_is_a_pad_button() {
        assert!(ActionBinding::Pad(GamepadKey::South).is_pad());
        assert!(!ActionBinding::Board(KeyCode::KeyA).is_pad());
        assert!(!ActionBinding::Pointer(MouseButton::Left).is_pad());
        assert!(!ActionBinding::WheelUp.is_pad());
        assert!(!ActionBinding::WheelDown.is_pad());
    }

    #[test]
    fn the_default_controls_are_bind_lines_and_comments_and_nothing_else() {
        for line in STOCK_BINDINGS.lines() {
            let line = str::trim(line);
            assert!(
                line.is_empty() || line.starts_with("//") || line.starts_with("bind "),
                "{line:?}"
            );
            assert!(!line.contains(';'), "{line:?}");
        }
    }
}
