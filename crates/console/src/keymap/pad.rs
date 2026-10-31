//! The gamepad's buttons: what the pad calls them, what each prompt shows, and
//! the commands the stock button layouts put on them.

use bevy::input::gamepad::GamepadButton;
use frame::PromptStyle;

/// A button on a pad, by where it sits rather than by what a maker prints on it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum GamepadKey {
    South = 0,
    East = 1,
    West = 2,
    North = 3,
    LeftBumper = 4,
    RightBumper = 5,
    LeftTrigger = 6,
    RightTrigger = 7,
    LeftStick = 8,
    RightStick = 9,
    DpadUp = 10,
    DpadDown = 11,
    DpadLeft = 12,
    DpadRight = 13,
    Select = 14,
    Start = 15,
}

/// The number of the first pad button; the rest follow in `ALL`'s order.
const FIRST_PAD_KEYNUM: usize = 190;

impl GamepadKey {
    pub const ALL: [GamepadKey; 16] = [
        Self::South,
        Self::East,
        Self::West,
        Self::North,
        Self::LeftBumper,
        Self::RightBumper,
        Self::LeftTrigger,
        Self::RightTrigger,
        Self::LeftStick,
        Self::RightStick,
        Self::DpadUp,
        Self::DpadDown,
        Self::DpadLeft,
        Self::DpadRight,
        Self::Select,
        Self::Start,
    ];

    /// The engine's button for this one.
    pub const fn gamepad_button(self) -> GamepadButton {
        match self {
            Self::South => GamepadButton::South,
            Self::East => GamepadButton::East,
            Self::West => GamepadButton::West,
            Self::North => GamepadButton::North,
            Self::LeftBumper => GamepadButton::LeftTrigger,
            Self::RightBumper => GamepadButton::RightTrigger,
            Self::LeftTrigger => GamepadButton::LeftTrigger2,
            Self::RightTrigger => GamepadButton::RightTrigger2,
            Self::LeftStick => GamepadButton::LeftThumb,
            Self::RightStick => GamepadButton::RightThumb,
            Self::DpadUp => GamepadButton::DPadUp,
            Self::DpadDown => GamepadButton::DPadDown,
            Self::DpadLeft => GamepadButton::DPadLeft,
            Self::DpadRight => GamepadButton::DPadRight,
            Self::Select => GamepadButton::Select,
            Self::Start => GamepadButton::Start,
        }
    }

    /// The button the engine's `button` is, if the game uses it.
    pub fn from_gamepad_button(button: GamepadButton) -> Option<Self> {
        Self::ALL
            .iter()
            .copied()
            .find(|pad| pad.gamepad_button() == button)
    }

    /// The name `bind` takes for it and a listing writes.
    pub const fn console_name(self) -> &'static str {
        match self {
            Self::South => "BUTTON_A",
            Self::East => "BUTTON_B",
            Self::West => "BUTTON_X",
            Self::North => "BUTTON_Y",
            Self::LeftBumper => "BUTTON_LSHLDR",
            Self::RightBumper => "BUTTON_RSHLDR",
            Self::LeftTrigger => "BUTTON_LTRIG",
            Self::RightTrigger => "BUTTON_RTRIG",
            Self::LeftStick => "BUTTON_LSTICK",
            Self::RightStick => "BUTTON_RSTICK",
            Self::DpadUp => "DPAD_UP",
            Self::DpadDown => "DPAD_DOWN",
            Self::DpadLeft => "DPAD_LEFT",
            Self::DpadRight => "DPAD_RIGHT",
            Self::Select => "BUTTON_BACK",
            Self::Start => "BUTTON_START",
        }
    }

    /// The short name an Xbox pad prints on it.
    pub const fn label(self: GamepadKey) -> &'static str {
        match self {
            Self::South => "A",
            Self::East => "B",
            Self::West => "X",
            Self::North => "Y",
            Self::LeftBumper => "LB",
            Self::RightBumper => "RB",
            Self::LeftTrigger => "LT",
            Self::RightTrigger => "RT",
            Self::LeftStick => "LS",
            Self::RightStick => "RS",
            Self::DpadUp => "D-UP",
            Self::DpadDown => "D-DOWN",
            Self::DpadLeft => "D-LEFT",
            Self::DpadRight => "D-RIGHT",
            Self::Select => "BACK",
            Self::Start => "START",
        }
    }

    /// bo2zm: the name `frame::bo2_pad_glyph` knows this button by.
    pub const fn bo2_name(self) -> &'static str {
        match self {
            Self::South => "south",
            Self::East => "east",
            Self::West => "west",
            Self::North => "north",
            Self::LeftBumper => "lb",
            Self::RightBumper => "rb",
            Self::LeftTrigger => "lt",
            Self::RightTrigger => "rt",
            Self::LeftStick => "ls",
            Self::RightStick => "rs",
            Self::DpadUp => "dpad_up",
            Self::DpadDown => "dpad_down",
            Self::DpadLeft => "dpad_left",
            Self::DpadRight => "dpad_right",
            Self::Select => "select",
            Self::Start => "start",
        }
    }

    /// What a prompt shows for it on the kind of pad the player holds. The
    /// d-pad and Start read the same on every pad but the PlayStation's Start.
    pub const fn prompt(self, style: PromptStyle) -> &'static str {
        match (style, self) {
            (PromptStyle::Xbox, _) => self.label(),
            (PromptStyle::PlayStation, Self::South) => "×",
            (PromptStyle::PlayStation, Self::East) => "○",
            (PromptStyle::PlayStation, Self::West) => "□",
            (PromptStyle::PlayStation, Self::North) => "△",
            (PromptStyle::PlayStation, Self::LeftBumper) => "L1",
            (PromptStyle::PlayStation, Self::RightBumper) => "R1",
            (PromptStyle::PlayStation, Self::LeftTrigger) => "L2",
            (PromptStyle::PlayStation, Self::RightTrigger) => "R2",
            (PromptStyle::PlayStation, Self::LeftStick) => "L3",
            (PromptStyle::PlayStation, Self::RightStick) => "R3",
            (PromptStyle::PlayStation, Self::Select) => "SHARE",
            (PromptStyle::PlayStation, Self::Start) => "OPTIONS",
            (PromptStyle::Generic, Self::South) => "SOUTH",
            (PromptStyle::Generic, Self::East) => "EAST",
            (PromptStyle::Generic, Self::West) => "WEST",
            (PromptStyle::Generic, Self::North) => "NORTH",
            (PromptStyle::Generic, Self::LeftBumper) => "L-BUMPER",
            (PromptStyle::Generic, Self::RightBumper) => "R-BUMPER",
            (PromptStyle::Generic, Self::LeftTrigger) => "L-TRIGGER",
            (PromptStyle::Generic, Self::RightTrigger) => "R-TRIGGER",
            (PromptStyle::Generic, Self::LeftStick) => "L-STICK",
            (PromptStyle::Generic, Self::RightStick) => "R-STICK",
            (PromptStyle::Generic, Self::Select) => "SELECT",
            _ => self.label(),
        }
    }

    /// The number the host's input layer gives it.
    pub(super) const fn keynum(self) -> usize {
        FIRST_PAD_KEYNUM + self as usize
    }
}

/// One button of a pad layout and the command it runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PadBind {
    pub button: GamepadKey,
    pub command: &'static str,
}

const fn bind(button: GamepadKey, command: &'static str) -> PadBind {
    PadBind { button, command }
}

/// What the stock layout does before a numbered layout changes anything.
const STOCK: &[PadBind] = &[
    bind(GamepadKey::RightTrigger, "+attack"),
    bind(GamepadKey::LeftTrigger, "+speed_throw"),
    bind(GamepadKey::RightBumper, "+frag"),
    bind(GamepadKey::LeftBumper, "+smoke"),
    bind(GamepadKey::South, "+gostand"),
    bind(GamepadKey::East, "+stance"),
    bind(GamepadKey::West, "+usereload"),
    bind(GamepadKey::North, "+weapnext_inventory"),
    bind(GamepadKey::LeftStick, "+breath_sprint"),
    bind(GamepadKey::RightStick, "+melee"),
    bind(GamepadKey::DpadUp, "+actionslot 1"),
    bind(GamepadKey::DpadDown, "+actionslot 2"),
    bind(GamepadKey::DpadLeft, "+actionslot 3"),
    bind(GamepadKey::DpadRight, "+actionslot 4"),
    bind(GamepadKey::Select, "+scores"),
];

/// Layout 1: melee on B and stance on the right stick.
const TACTICAL: &[PadBind] = &[
    bind(GamepadKey::East, "+melee"),
    bind(GamepadKey::RightStick, "+stance"),
];

/// Layout 2: aim and fire swap triggers, and the two bumpers swap their throws.
const LEFTY: &[PadBind] = &[
    bind(GamepadKey::LeftTrigger, "+attack"),
    bind(GamepadKey::RightTrigger, "+speed_throw"),
    bind(GamepadKey::LeftBumper, "+frag"),
    bind(GamepadKey::RightBumper, "+smoke"),
];

/// Layout 3: jump on the left bumper, the smoke grenade on A.
const BUMPER_JUMPER: &[PadBind] = &[
    bind(GamepadKey::LeftBumper, "+gostand"),
    bind(GamepadKey::South, "+smoke"),
];

/// Layout 4: layout 3, then layout 1 over it.
const BUMPER_JUMPER_TACTICAL: &[PadBind] = &[
    bind(GamepadKey::LeftBumper, "+gostand"),
    bind(GamepadKey::South, "+smoke"),
    bind(GamepadKey::East, "+melee"),
    bind(GamepadKey::RightStick, "+stance"),
];

/// The command on each button under numbered layout `layout`; a number the
/// game has no layout for gets the stock one. A layout's own changes come last.
pub fn pad_layout(layout: usize) -> Vec<PadBind> {
    let changes: &[PadBind] = match layout {
        1 => TACTICAL,
        2 => LEFTY,
        3 => BUMPER_JUMPER,
        4 => BUMPER_JUMPER_TACTICAL,
        _ => &[],
    };
    let mut binds = STOCK.to_vec();
    for &change in changes {
        binds.retain(|held| held.button != change.button);
        binds.push(change);
    }
    binds
}

#[cfg(test)]
mod tests {
    use input_t6::action_number_for;

    use crate::keymap::pad::*;

    fn command_on(layout: usize, button: GamepadKey) -> Option<&'static str> {
        pad_layout(layout)
            .iter()
            .find(|bind| bind.button == button)
            .map(|bind| bind.command)
    }

    #[test]
    fn all_lists_each_button_once_in_the_order_that_numbers_them() {
        for (at, pad) in GamepadKey::ALL.into_iter().enumerate() {
            assert_eq!(pad as usize, at);
            assert_eq!(pad.keynum(), 190 + at);
        }
    }

    #[test]
    fn the_engine_button_comes_back_as_the_same_pad_button() {
        for pad in GamepadKey::ALL {
            let expected = Some(pad);
            assert_eq!(GamepadKey::from_gamepad_button(pad.gamepad_button()), expected);
        }
    }

    #[test]
    fn a_button_the_game_has_no_use_for_is_not_a_pad_button() {
        for button in [GamepadButton::C, GamepadButton::Z, GamepadButton::Mode] {
            assert_eq!(GamepadKey::from_gamepad_button(button), None);
        }
    }

    #[test]
    fn every_button_has_a_name_and_a_label_of_its_own() {
        for names in [
            GamepadKey::ALL.map(GamepadKey::console_name),
            GamepadKey::ALL.map(GamepadKey::label),
        ] {
            let mut sorted = names.to_vec();
            sorted.sort_unstable();
            sorted.dedup();
            assert_eq!(sorted.len(), GamepadKey::ALL.len());
        }
    }

    #[test]
    fn an_xbox_prompt_is_the_label() {
        for pad in GamepadKey::ALL {
            assert_eq!(pad.prompt(PromptStyle::Xbox), pad.label());
        }
    }

    #[test]
    fn a_playstation_prompt_uses_the_symbols_and_numbered_shoulders() {
        let shown = |pad: GamepadKey| pad.prompt(PromptStyle::PlayStation);
        assert_eq!(shown(GamepadKey::South), "×");
        assert_eq!(shown(GamepadKey::North), "△");
        assert_eq!(shown(GamepadKey::LeftBumper), "L1");
        assert_eq!(shown(GamepadKey::RightTrigger), "R2");
        assert_eq!(shown(GamepadKey::Select), "SHARE");
        let start = shown(GamepadKey::Start);
        assert!(start == "OPTIONS");
        assert_eq!(shown(GamepadKey::DpadUp), "D-UP");
    }

    #[test]
    fn a_generic_prompt_says_where_the_button_is() {
        let shown = |pad: GamepadKey| pad.prompt(PromptStyle::Generic);
        assert_eq!(shown(GamepadKey::West), "WEST");
        assert_eq!(shown(GamepadKey::LeftBumper), "L-BUMPER");
        assert_eq!(shown(GamepadKey::RightStick), "R-STICK");
        assert_eq!(shown(GamepadKey::Select), "SELECT");
        assert_eq!(shown(GamepadKey::Start), "START");
        assert_eq!(shown(GamepadKey::DpadRight), "D-RIGHT");
    }

    #[test]
    fn the_stock_layout_binds_fifteen_buttons_and_leaves_start_for_the_menu() {
        let stock = pad_layout(0);
        assert_eq!(stock.len(), 15);
        assert_eq!(command_on(0, GamepadKey::Start), None);
        assert_eq!(command_on(0, GamepadKey::RightTrigger), Some("+attack"));
        assert_eq!(command_on(0, GamepadKey::LeftTrigger), Some("+speed_throw"));
        assert_eq!(command_on(0, GamepadKey::DpadLeft), Some("+actionslot 3"));
    }

    #[test]
    fn a_number_with_no_layout_gets_the_stock_one() {
        assert_eq!(pad_layout(5), pad_layout(0));
        assert_eq!(pad_layout(usize::MAX), pad_layout(0));
    }

    #[test]
    fn tactical_moves_melee_to_b_and_stance_to_the_right_stick() {
        assert_eq!(command_on(1, GamepadKey::East), Some("+melee"));
        assert_eq!(command_on(1, GamepadKey::RightStick), Some("+stance"));
        assert_eq!(command_on(1, GamepadKey::South), Some("+gostand"));
    }

    #[test]
    fn lefty_swaps_the_triggers_and_the_throws() {
        assert_eq!(command_on(2, GamepadKey::LeftTrigger), Some("+attack"));
        assert_eq!(command_on(2, GamepadKey::RightTrigger), Some("+speed_throw"));
        assert_eq!(command_on(2, GamepadKey::LeftBumper), Some("+frag"));
        assert_eq!(command_on(2, GamepadKey::RightBumper), Some("+smoke"));
    }

    #[test]
    fn bumper_jumper_jumps_on_the_bumper_and_tactical_adds_to_it() {
        assert_eq!(command_on(3, GamepadKey::LeftBumper), Some("+gostand"));
        assert_eq!(command_on(3, GamepadKey::South), Some("+smoke"));
        assert_eq!(command_on(3, GamepadKey::East), Some("+stance"));
        for PadBind { button, command } in pad_layout(1) {
            if matches!(button, GamepadKey::East | GamepadKey::RightStick) {
                assert_eq!(command_on(4, button), Some(command));
            }
        }
        assert_eq!(command_on(4, GamepadKey::LeftBumper), Some("+gostand"));
        assert_eq!(command_on(4, GamepadKey::South), Some("+smoke"));
    }

    #[test]
    fn no_layout_puts_two_commands_on_one_button_or_drops_a_button() {
        for layout in 0..=4 {
            let binds = pad_layout(layout);
            assert_eq!(binds.len(), 15, "layout {layout}");
            let mut buttons: Vec<GamepadKey> = binds.iter().map(|bind| bind.button).collect();
            buttons.sort_unstable();
            buttons.dedup();
            assert_eq!(buttons.len(), 15, "layout {layout}");
        }
    }

    #[test]
    fn every_command_a_layout_uses_is_one_the_input_layer_knows() {
        for layout in 0..=4 {
            for PadBind { button, command } in pad_layout(layout) {
                assert!(
                    action_number_for(command).is_some(),
                    "layout {layout}: {} has the unknown command {command}",
                    button.console_name()
                );
            }
        }
    }
}
