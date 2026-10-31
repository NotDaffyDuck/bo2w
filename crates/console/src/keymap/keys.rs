//! The buttons a player can bind and what each one is called: the spellings
//! `bind` accepts, how a listing writes it, and the number the host's input
//! layer knows it by.

use bevy::prelude::*;
use frame::PromptStyle;

use super::ActionBinding;
use super::GamepadKey;

/// The number of a key the table does not list.
const OTHER_KEY: usize = 62;
/// The number of a mouse button the table does not list.
const OTHER_MOUSE: usize = 185;

struct Row {
    button: ActionBinding,
    /// The number the host's input layer gives the button.
    number: usize,
    /// How a listing writes it.
    shown: &'static str,
    /// Every spelling that names it, in lower case. Completion offers the first.
    names: &'static [&'static str],
    /// A second button the same names bind: `shift` is either shift key.
    twin: Option<ActionBinding>,
    offered: bool,
}

const fn row(
    button: ActionBinding,
    number: usize,
    shown: &'static str,
    names: &'static [&'static str],
) -> Row {
    Row {
        button,
        number,
        shown,
        names,
        twin: None,
        offered: true,
    }
}

impl Row {
    const fn and(mut self, twin: ActionBinding) -> Self {
        self.twin = Some(twin);
        self
    }

    const fn unoffered(mut self) -> Self {
        self.offered = false;
        self
    }

    fn buttons(&self) -> Vec<ActionBinding> {
        std::iter::once(self.button).chain(self.twin).collect()
    }
}

const fn key(code: KeyCode) -> ActionBinding {
    ActionBinding::Board(code)
}

const fn mouse(button: MouseButton) -> ActionBinding {
    ActionBinding::Pointer(button)
}

static ROWS: &[Row] = &[
    row(key(KeyCode::KeyA), 1, "a", &["a"]),
    row(key(KeyCode::KeyB), 2, "b", &["b"]),
    row(key(KeyCode::KeyC), 3, "c", &["c"]),
    row(key(KeyCode::KeyD), 4, "d", &["d"]),
    row(key(KeyCode::KeyE), 5, "e", &["e"]),
    row(key(KeyCode::KeyF), 6, "f", &["f"]),
    row(key(KeyCode::KeyG), 7, "g", &["g"]),
    row(key(KeyCode::KeyH), 8, "h", &["h"]),
    row(key(KeyCode::KeyI), 9, "i", &["i"]),
    row(key(KeyCode::KeyJ), 10, "j", &["j"]),
    row(key(KeyCode::KeyK), 11, "k", &["k"]),
    row(key(KeyCode::KeyL), 12, "l", &["l"]),
    row(key(KeyCode::KeyM), 13, "m", &["m"]),
    row(key(KeyCode::KeyN), 14, "n", &["n"]),
    row(key(KeyCode::KeyO), 15, "o", &["o"]),
    row(key(KeyCode::KeyP), 16, "p", &["p"]),
    row(key(KeyCode::KeyQ), 17, "q", &["q"]),
    row(key(KeyCode::KeyR), 18, "r", &["r"]),
    row(key(KeyCode::KeyS), 19, "s", &["s"]),
    row(key(KeyCode::KeyT), 20, "t", &["t"]),
    row(key(KeyCode::KeyU), 21, "u", &["u"]),
    row(key(KeyCode::KeyV), 22, "v", &["v"]),
    row(key(KeyCode::KeyW), 23, "w", &["w"]),
    row(key(KeyCode::KeyX), 24, "x", &["x"]),
    row(key(KeyCode::KeyY), 25, "y", &["y"]),
    row(key(KeyCode::KeyZ), 26, "z", &["z"]),
    row(key(KeyCode::Digit0), 27, "0", &["0"]),
    row(key(KeyCode::Digit1), 28, "1", &["1"]),
    row(key(KeyCode::Digit2), 29, "2", &["2"]),
    row(key(KeyCode::Digit3), 30, "3", &["3"]),
    row(key(KeyCode::Digit4), 31, "4", &["4"]),
    row(key(KeyCode::Digit5), 32, "5", &["5"]),
    row(key(KeyCode::Digit6), 33, "6", &["6"]),
    row(key(KeyCode::Digit7), 34, "7", &["7"]),
    row(key(KeyCode::Digit8), 35, "8", &["8"]),
    row(key(KeyCode::Digit9), 36, "9", &["9"]),
    row(key(KeyCode::Space), 37, "SPACE", &["space"]),
    row(key(KeyCode::Tab), 38, "TAB", &["tab"]),
    row(
        key(KeyCode::ShiftLeft),
        39,
        "SHIFT",
        &["shift", "shiftleft", "lshift"],
    )
    .and(key(KeyCode::ShiftRight)),
    row(
        key(KeyCode::ShiftRight),
        40,
        "SHIFT",
        &["shiftright", "rshift"],
    )
    .unoffered(),
    row(
        key(KeyCode::ControlLeft),
        41,
        "CTRL",
        &["ctrl", "control", "ctrlleft", "lctrl"],
    )
    .and(key(KeyCode::ControlRight)),
    row(
        key(KeyCode::ControlRight),
        42,
        "CTRL",
        &["ctrlright", "rctrl"],
    )
    .unoffered(),
    row(
        key(KeyCode::AltLeft),
        43,
        "ALT",
        &["alt", "altleft", "lalt"],
    )
    .and(key(KeyCode::AltRight)),
    row(key(KeyCode::AltRight), 44, "ALT", &["altright", "ralt"]).unoffered(),
    row(key(KeyCode::Enter), 45, "ENTER", &["enter", "return"]),
    row(key(KeyCode::Backspace), 46, "BACKSPACE", &["backspace"]),
    row(key(KeyCode::Escape), 47, "ESCAPE", &["escape", "esc"]),
    row(key(KeyCode::ArrowUp), 48, "UPARROW", &["uparrow", "up"]),
    row(
        key(KeyCode::ArrowDown),
        49,
        "DOWNARROW",
        &["downarrow", "down"],
    ),
    row(
        key(KeyCode::ArrowLeft),
        50,
        "LEFTARROW",
        &["leftarrow", "left"],
    ),
    row(
        key(KeyCode::ArrowRight),
        51,
        "RIGHTARROW",
        &["rightarrow", "right"],
    ),
    row(key(KeyCode::Semicolon), 52, "SEMICOLON", &["semicolon"]),
    row(key(KeyCode::Quote), 53, "QUOTE", &["quote"]),
    row(key(KeyCode::Comma), 54, "COMMA", &["comma"]),
    row(key(KeyCode::Period), 55, "PERIOD", &["period"]),
    row(key(KeyCode::Slash), 56, "SLASH", &["slash"]),
    row(key(KeyCode::Minus), 57, "MINUS", &["minus"]),
    row(key(KeyCode::Equal), 58, "EQUAL", &["equal", "equals"]),
    row(
        key(KeyCode::BracketLeft),
        59,
        "BRACKETLEFT",
        &["bracketleft", "["],
    ),
    row(
        key(KeyCode::BracketRight),
        60,
        "BRACKETRIGHT",
        &["bracketright", "]"],
    ),
    row(key(KeyCode::Backslash), 61, "BACKSLASH", &["backslash"]),
    row(
        mouse(MouseButton::Left),
        180,
        "MOUSE1",
        &["mouse1", "mouseleft", "lmb"],
    ),
    row(
        mouse(MouseButton::Right),
        181,
        "MOUSE2",
        &["mouse2", "mouseright", "rmb"],
    ),
    row(
        mouse(MouseButton::Middle),
        182,
        "MOUSE3",
        &["mouse3", "mousemiddle", "mmb"],
    ),
    row(mouse(MouseButton::Back), 183, "MOUSE4", &["mouse4"]),
    row(mouse(MouseButton::Forward), 184, "MOUSE5", &["mouse5"]),
    row(
        ActionBinding::WheelUp,
        186,
        "MWHEELUP",
        &["mwheelup", "wheelup"],
    ),
    row(
        ActionBinding::WheelDown,
        187,
        "MWHEELDOWN",
        &["mwheeldown", "wheeldown"],
    ),
];

fn row_of(button: ActionBinding) -> Option<&'static Row> {
    ROWS.iter().find(|row| row.button == button)
}

/// The number the host's input layer gives `button`.
pub fn host_key_code(button: ActionBinding) -> usize {
    match (button, row_of(button)) {
        (ActionBinding::Pad(pad), _) => pad.keynum(),
        (_, Some(row)) => row.number,
        (ActionBinding::Pointer(_), None) => OTHER_MOUSE,
        (_, None) => OTHER_KEY,
    }
}

/// How a listing writes `button`.
pub fn button_label(button: ActionBinding) -> String {
    if let Some(row) = row_of(button) {
        return row.shown.to_owned();
    }
    match &button {
        ActionBinding::Board(code) => format!("{code:?}"),
        ActionBinding::Pointer(other) => format!("{other:?}"),
        ActionBinding::Pad(pad) => pad.console_name().to_owned(),
        ActionBinding::WheelUp | ActionBinding::WheelDown => format!("{button:?}"),
    }
}

/// The buttons a typed name stands for, whatever its case; none when it names
/// nothing. A pad button answers to its console name, its label and its
/// generic prompt.
pub fn button_from_name(name: &str) -> Option<Vec<ActionBinding>> {
    let name = name.trim();
    let lowered = name.to_ascii_lowercase();
    if let Some(row) = ROWS.iter().find(|row| row.names.contains(&lowered.as_str())) {
        return Some(row.buttons());
    }
    let name = lowered.to_ascii_uppercase();
    GamepadKey::ALL
        .iter()
        .copied()
        .find(|pad| {
            pad.console_name() == name
                || pad.label() == name
                || name == format!("BUTTON_{}", pad.prompt(PromptStyle::Generic))
        })
        .map(|pad| vec![ActionBinding::Pad(pad)])
}

/// The names the prompt completes after `bind`: one per button, the pad's
/// Start left out since it opens the menu.
pub fn key_name_list() -> Vec<&'static str> {
    let pads = GamepadKey::ALL
        .iter()
        .copied()
        .filter(|pad| *pad != GamepadKey::Start)
        .map(GamepadKey::console_name);
    ROWS.iter()
        .filter(|row| row.offered)
        .map(|row| row.names[0])
        .chain(pads)
        .collect::<Vec<&str>>()
}

#[cfg(test)]
mod tests {
    use crate::keymap::keys::*;

    fn one(name: &str) -> Option<ActionBinding> {
        match button_from_name(name)?.as_slice() {
            [only] => Some(*only),
            _ => None,
        }
    }

    #[test]
    fn no_two_rows_share_a_number_a_button_or_a_name() {
        let mut numbers: Vec<usize> = ROWS.iter().map(|row| row.number).collect();
        numbers.sort_unstable();
        numbers.dedup();
        assert_eq!(numbers.len(), ROWS.len());

        let names: Vec<&str> = ROWS
            .iter()
            .flat_map(|row| row.names.iter().copied())
            .collect::<Vec<&str>>();
        let spellings = names.len();
        let distinct: std::collections::BTreeSet<&str> = names.iter().copied().collect();
        assert_eq!(distinct.len(), spellings);

        for (at, row) in ROWS.iter().enumerate() {
            assert!(ROWS[..at].iter().all(|before| before.button != row.button));
        }
    }

    #[test]
    fn the_numbers_keep_clear_of_the_ones_kept_for_buttons_off_the_table() {
        for row in ROWS {
            assert_ne!(row.number, OTHER_KEY);
            assert_ne!(row.number, OTHER_MOUSE);
            assert!(
                row.number < 190,
                "{} runs into the pad's numbers",
                row.shown
            );
        }
    }

    #[test]
    fn a_name_is_read_in_any_case_with_spaces_around_it() {
        assert_eq!(one("W"), Some(key(KeyCode::KeyW)));
        assert_eq!(one("  MOUSE1 "), Some(mouse(MouseButton::Left)));
        assert_eq!(one("Space"), Some(key(KeyCode::Space)));
        assert_eq!(one("return"), Some(key(KeyCode::Enter)));
        assert_eq!(one("["), Some(key(KeyCode::BracketLeft)));
    }

    #[test]
    fn the_plain_modifier_names_reach_both_sides_and_the_sided_ones_one() {
        let actual = button_from_name("shift");
        assert_eq!(actual, Some(vec![key(KeyCode::ShiftLeft), key(KeyCode::ShiftRight)]));
        let actual = button_from_name("CTRL");
        assert_eq!(actual, Some(vec![key(KeyCode::ControlLeft), key(KeyCode::ControlRight)]));
        let actual = button_from_name("alt");
        assert_eq!(actual, Some(vec![key(KeyCode::AltLeft), key(KeyCode::AltRight)]));
        assert_eq!(one("rshift"), Some(key(KeyCode::ShiftRight)));
        assert_eq!(one("ctrlright"), Some(key(KeyCode::ControlRight)));
        assert_eq!(one("ralt"), Some(key(KeyCode::AltRight)));
    }

    #[test]
    fn the_wheel_has_two_names_a_direction() {
        assert_eq!(one("mwheelup"), Some(ActionBinding::WheelUp));
        assert_eq!(one("WheelUp"), Some(ActionBinding::WheelUp));
        assert_eq!(one("mwheeldown"), Some(ActionBinding::WheelDown));
        assert_eq!(one("wheeldown"), Some(ActionBinding::WheelDown));
    }

    #[test]
    fn a_pad_button_answers_to_its_console_name_its_label_and_its_generic_prompt() {
        let south = Some(ActionBinding::Pad(GamepadKey::South));
        assert_eq!(one("BUTTON_A"), south);
        assert_eq!(one("button_a"), south);
        assert_eq!(one("button_south"), south);
        assert_eq!(one("lb"), Some(ActionBinding::Pad(GamepadKey::LeftBumper)));
        assert_eq!(one("dpad_up"), Some(ActionBinding::Pad(GamepadKey::DpadUp)));
        assert_eq!(one("back"), Some(ActionBinding::Pad(GamepadKey::Select)));
        assert_eq!(one("button_start"), Some(ActionBinding::Pad(GamepadKey::Start)));
        let expected = Some(ActionBinding::Pad(GamepadKey::Select));
        assert_eq!(one("button_select"), expected);
    }

    #[test]
    fn a_single_letter_is_the_key_not_the_pad_button_of_that_label() {
        assert_eq!(one("a"), Some(key(KeyCode::KeyA)));
        assert_eq!(one("x"), Some(key(KeyCode::KeyX)));
        assert_eq!(one("y"), Some(key(KeyCode::KeyY)));
    }

    #[test]
    fn what_names_nothing_parses_to_nothing() {
        for name in ["", "  ", "f13", "mouse6", "button_", "shift left"] {
            assert_eq!(button_from_name(name), None, "{name:?}");
        }
    }

    #[test]
    fn a_listing_writes_letters_small_and_the_rest_large() {
        assert_eq!(button_label(key(KeyCode::KeyA)), "a");
        assert_eq!(button_label(key(KeyCode::Digit7)), "7");
        assert_eq!(button_label(key(KeyCode::Space)), "SPACE");
        assert_eq!(button_label(key(KeyCode::ArrowLeft)), "LEFTARROW");
        assert_eq!(button_label(key(KeyCode::ShiftRight)), "SHIFT");
        assert_eq!(button_label(mouse(MouseButton::Forward)), "MOUSE5");
        assert_eq!(button_label(ActionBinding::WheelDown), "MWHEELDOWN");
        let expected = "BUTTON_BACK";
        assert_eq!(button_label(ActionBinding::Pad(GamepadKey::Select)), expected);
    }

    #[test]
    fn a_button_off_the_table_is_written_as_the_engine_names_it() {
        assert_eq!(button_label(key(KeyCode::F1)), "F1");
        assert_eq!(button_label(mouse(MouseButton::Other(7))), "Other(7)");
    }

    #[test]
    fn every_button_has_the_number_the_host_gives_it() {
        assert_eq!(host_key_code(key(KeyCode::KeyA)), 1);
        assert_eq!(host_key_code(key(KeyCode::Backslash)), 61);
        assert_eq!(host_key_code(key(KeyCode::ShiftRight)), 40);
        assert_eq!(host_key_code(mouse(MouseButton::Left)), 180);
        assert_eq!(host_key_code(mouse(MouseButton::Forward)), 184);
        assert_eq!(host_key_code(ActionBinding::WheelUp), 186);
        assert_eq!(host_key_code(ActionBinding::WheelDown), 187);
        assert_eq!(host_key_code(ActionBinding::Pad(GamepadKey::South)), 190);
        assert_eq!(host_key_code(ActionBinding::Pad(GamepadKey::Start)), 205);
    }

    #[test]
    fn a_button_off_the_table_shares_the_number_kept_for_its_kind() {
        assert_eq!(host_key_code(key(KeyCode::F5)), 62);
        assert_eq!(host_key_code(mouse(MouseButton::Other(9))), 185);
    }

    #[test]
    fn completion_offers_each_button_once_and_leaves_out_the_second_sides_and_start() {
        let offered = key_name_list();
        assert_eq!(offered.first(), Some(&"a"));
        assert_eq!(offered.last(), Some(&"BUTTON_BACK"));
        for name in ["shift", "mouse1", "mwheelup", "BUTTON_A", "DPAD_RIGHT"] {
            assert!(offered.contains(&name), "{name}");
        }
        for name in ["rshift", "ctrlright", "altright", "BUTTON_START", "up"] {
            assert!(!offered.contains(&name), "{name}");
        }
        let mut sorted = offered.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), offered.len());
    }

    #[test]
    fn everything_completion_offers_can_be_bound() {
        for name in key_name_list() {
            assert!(button_from_name(name).is_some(), "{name}");
        }
    }

    #[test]
    fn a_name_found_by_parsing_is_written_back_as_the_same_button() {
        for row in ROWS.iter().filter(|row| row.twin.is_none()) {
            for name in row.names {
                assert_eq!(one(name), Some(row.button), "{name}");
            }
        }
    }
}
