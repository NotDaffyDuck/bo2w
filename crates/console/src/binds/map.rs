//! Which command each button runs, and the three lines a config or the console
//! can say about it: `bind`, `unbind` and `unbindall`.

use std::collections::HashMap;

use bevy::prelude::Resource;
use input_t6::{command_id_lookup, command_name};

use super::{BindButton, DEFAULT_CONTROLS, display_button, pad_layout, parse_button_name};
use crate::ConsoleCommand;

/// The command (by its number in the input layer) on each button.
#[derive(Resource, Debug, Clone, Default)]
pub struct KeyBinds {
    map: HashMap<BindButton, u32>,
}

impl KeyBinds {
    /// Puts back what a new player has: the default keyboard and mouse, and the
    /// stock pad layout.
    pub fn apply_defaults(&mut self) {
        self.map.clear();
        self.apply_config_script(DEFAULT_CONTROLS);
        self.apply_pad_layout(0);
    }

    /// Runs the lines of a console script that change binds, and says what each
    /// did and what it refused.
    pub fn apply_script(&mut self, script: &str) -> Vec<String> {
        self.run_script(script, true)
    }

    /// Runs a saved config's lines, saying only what it refused.
    pub(crate) fn apply_config_script(&mut self, script: &str) -> Vec<String> {
        self.run_script(script, false)
    }

    fn run_script(&mut self, script: &str, report: bool) -> Vec<String> {
        let mut said = Vec::new();
        for line in script.split([';', '\n']).map(str::trim) {
            if line.is_empty() || line.starts_with("//") {
                continue;
            }
            let Some(command) = ConsoleCommand::parse(line) else {
                continue;
            };
            let outcome = match command.name.as_str() {
                "bind" => self.bind(&command.args),
                "unbind" => self.unbind(&command.args),
                "unbindall" => {
                    self.map.clear();
                    Ok(Some("unbindall".to_owned()))
                }
                other => Err(format!("unknown bind-script command `{other}`")),
            };
            match outcome {
                Ok(Some(text)) if report => said.push(text),
                Ok(_) => {}
                Err(text) => said.push(text),
            }
        }
        said
    }

    /// `bind` alone says nothing, `bind <key>` says what is on it, and
    /// `bind <key> <command>` puts the command on every button the name covers.
    fn bind(&mut self, args: &[String]) -> Result<Option<String>, String> {
        let (key, command) = match args {
            [] => return Ok(None),
            [key] => (key, None),
            [key, words @ ..] => (key, Some(words.join(" "))),
        };
        let Some(command) = command else {
            let buttons = buttons_named(key)?;
            let on_it = buttons.iter().find_map(|button| self.binding_name(*button));
            return Ok(Some(match on_it {
                Some(name) => format!("bind {key} {name}"),
                None => format!("`{key}` is unbound"),
            }));
        };
        let id =
            command_id_lookup(&command).ok_or_else(|| format!("unknown command `{command}`"))?;
        for button in buttons_named(key)? {
            self.set(button, id);
        }
        let name = command_name(id).unwrap_or(command.as_str());
        Ok(Some(format!("bind {key} {name}")))
    }

    fn unbind(&mut self, args: &[String]) -> Result<Option<String>, String> {
        let [key] = args else {
            return Err("usage: unbind <key>".to_owned());
        };
        let mut freed = false;
        for button in buttons_named(key)? {
            freed |= self.clear_button(button);
        }
        Ok(Some(if freed {
            format!("unbind {key}")
        } else {
            format!("`{key}` is unbound")
        }))
    }

    pub fn set(&mut self, button: BindButton, id: u32) {
        self.map.insert(button, id);
    }

    pub fn get(&self, button: BindButton) -> Option<u32> {
        self.map.get(&button).copied()
    }

    /// The name of the command on `button`.
    pub fn binding_name(&self, button: BindButton) -> Option<&'static str> {
        self.get(button).and_then(command_name)
    }

    /// Frees `button`, and says whether it had a command.
    pub fn clear_button(&mut self, button: BindButton) -> bool {
        self.map.remove(&button).is_some()
    }

    pub fn clear_all(&mut self) {
        self.map.clear();
    }

    /// Takes command `id` off the pad's buttons (`pad`) or off the keyboard and
    /// mouse (`!pad`), and says whether it was on any.
    pub fn clear_command_on(&mut self, id: u32, pad: bool) -> bool {
        let before = self.map.len();
        self.map
            .retain(|button, bound| *bound != id || button.is_pad() != pad);
        self.map.len() != before
    }

    /// Swaps the pad's binds for numbered layout `layout`, leaving the keyboard
    /// and mouse alone. A command the input layer lacks is left off.
    pub fn apply_pad_layout(&mut self, layout: usize) {
        self.map.retain(|button, _| !button.is_pad());
        for (pad, command) in pad_layout(layout) {
            if let Some(id) = command_id_lookup(command) {
                self.set(BindButton::Pad(pad), id);
            }
        }
    }

    pub fn has_pad_binds(&self) -> bool {
        self.map.keys().any(|button| button.is_pad())
    }

    pub fn iter(&self) -> impl Iterator<Item = (BindButton, u32)> + '_ {
        self.map.iter().map(|(button, id)| (*button, *id))
    }

    /// One `bind <button> <command>` line per bind, in order. Both sides of a
    /// modifier key read the same, so they make one line.
    pub fn list_lines(&self) -> Vec<String> {
        let mut lines: Vec<String> = self
            .iter()
            .filter_map(|(button, id)| {
                command_name(id).map(|name| format!("bind {} {name}", display_button(button)))
            })
            .collect();
        lines.sort();
        lines.dedup();
        lines
    }
}

fn buttons_named(key: &str) -> Result<Vec<BindButton>, String> {
    parse_button_name(key).ok_or_else(|| format!("unknown key `{key}`"))
}

#[cfg(test)]
mod tests {
    use bevy::input::keyboard::KeyCode;
    use bevy::input::mouse::MouseButton;

    use super::*;
    use crate::PadButton;

    fn id(command: &str) -> u32 {
        command_id_lookup(command).unwrap_or_else(|| panic!("no command {command}"))
    }

    fn key(code: KeyCode) -> BindButton {
        BindButton::Key(code)
    }

    fn run(script: &str) -> (KeyBinds, Vec<String>) {
        let mut binds = KeyBinds::default();
        let said = binds.apply_script(script);
        (binds, said)
    }

    #[test]
    fn a_bind_line_puts_the_command_on_the_key_and_says_so() {
        let (binds, said) = run("bind w +forward");
        assert_eq!(said, ["bind w +forward"]);
        assert_eq!(binds.get(key(KeyCode::KeyW)), Some(id("+forward")));
        assert_eq!(binds.binding_name(key(KeyCode::KeyW)), Some("+forward"));
    }

    #[test]
    fn a_command_of_several_words_is_the_rest_of_the_line() {
        let (binds, said) = run("bind 3 +actionslot 4");
        assert_eq!(said.len(), 1);
        assert!(said[0].starts_with("bind 3 "), "{said:?}");
        assert_eq!(binds.get(key(KeyCode::Digit3)), Some(id("+actionslot 4")));
    }

    #[test]
    fn a_name_that_covers_both_sides_binds_both() {
        let (binds, _) = run("bind shift +breath_sprint");
        let sprint = Some(id("+breath_sprint"));
        assert_eq!(binds.get(key(KeyCode::ShiftLeft)), sprint);
        assert_eq!(binds.get(key(KeyCode::ShiftRight)), sprint);
        let (binds, _) = run("bind rshift +breath_sprint");
        assert_eq!(binds.get(key(KeyCode::ShiftLeft)), None);
        assert_eq!(binds.get(key(KeyCode::ShiftRight)), sprint);
    }

    #[test]
    fn binding_a_key_again_replaces_what_was_on_it() {
        let (binds, _) = run("bind w +forward; bind w +back");
        assert_eq!(binds.get(key(KeyCode::KeyW)), Some(id("+back")));
        assert_eq!(binds.iter().count(), 1);
    }

    #[test]
    fn mouse_wheel_and_pad_buttons_bind_by_name() {
        let (binds, _) =
            run("bind mouse1 +attack\nbind mwheeldown weapprev\nbind BUTTON_A +gostand");
        assert_eq!(
            binds.get(BindButton::Mouse(MouseButton::Left)),
            Some(id("+attack"))
        );
        assert_eq!(binds.get(BindButton::WheelDown), Some(id("weapprev")));
        assert_eq!(
            binds.get(BindButton::Pad(PadButton::South)),
            Some(id("+gostand"))
        );
    }

    #[test]
    fn a_key_alone_says_what_is_on_it_or_that_it_is_free() {
        let mut binds = KeyBinds::default();
        assert_eq!(binds.apply_script("bind w"), ["`w` is unbound"]);
        binds.apply_script("bind w +forward");
        assert_eq!(binds.apply_script("bind w"), ["bind w +forward"]);
        assert_eq!(binds.apply_script("bind W"), ["bind W +forward"]);
    }

    #[test]
    fn a_modifier_alone_says_what_is_on_either_side() {
        let mut binds = KeyBinds::default();
        binds.apply_script("bind rshift +breath_sprint");
        assert_eq!(
            binds.apply_script("bind shift"),
            ["bind shift +breath_sprint"]
        );
    }

    #[test]
    fn bind_with_nothing_after_it_says_nothing() {
        assert_eq!(run("bind").1, Vec::<String>::new());
    }

    #[test]
    fn a_key_or_command_that_does_not_exist_is_refused_by_name() {
        let (binds, said) = run("bind f13 +forward\nbind w +nosuchcommand\nbind f13");
        assert_eq!(
            said,
            [
                "unknown key `f13`",
                "unknown command `+nosuchcommand`",
                "unknown key `f13`"
            ]
        );
        assert_eq!(binds.iter().count(), 0);
    }

    #[test]
    fn when_both_are_wrong_the_command_is_the_one_named() {
        assert_eq!(
            run("bind f13 +nosuchcommand").1,
            ["unknown command `+nosuchcommand`"]
        );
    }

    #[test]
    fn unbind_frees_the_key_and_says_so_once() {
        let mut binds = KeyBinds::default();
        binds.apply_script("bind w +forward");
        assert_eq!(binds.apply_script("unbind w"), ["unbind w"]);
        assert_eq!(binds.get(key(KeyCode::KeyW)), None);
        assert_eq!(binds.apply_script("unbind w"), ["`w` is unbound"]);
    }

    #[test]
    fn unbinding_a_modifier_frees_both_sides() {
        let mut binds = KeyBinds::default();
        binds.apply_script("bind shift +breath_sprint");
        assert_eq!(binds.apply_script("unbind shift"), ["unbind shift"]);
        assert!(binds.iter().next().is_none());
    }

    #[test]
    fn unbind_takes_exactly_one_key() {
        for line in ["unbind", "unbind w s"] {
            assert_eq!(run(line).1, ["usage: unbind <key>"], "{line}");
        }
        assert_eq!(run("unbind f13").1, ["unknown key `f13`"]);
    }

    #[test]
    fn unbindall_frees_everything() {
        let mut binds = KeyBinds::default();
        binds.apply_script("bind w +forward; bind s +back");
        assert_eq!(binds.apply_script("unbindall"), ["unbindall"]);
        assert_eq!(binds.iter().count(), 0);
    }

    #[test]
    fn lines_are_split_on_semicolons_and_newlines_and_comments_are_skipped() {
        let (binds, said) =
            run("// a comment\n\n  bind w +forward ; bind s +back\n// bind d +moveright\n");
        assert_eq!(said, ["bind w +forward", "bind s +back"]);
        assert_eq!(binds.iter().count(), 2);
    }

    #[test]
    fn a_line_that_is_not_a_bind_line_is_named() {
        assert_eq!(run("say hello").1, ["unknown bind-script command `say`"]);
    }

    #[test]
    fn a_saved_config_is_quiet_about_what_it_did_and_loud_about_what_it_refused() {
        let mut binds = KeyBinds::default();
        let said = binds.apply_config_script(
            "bind w +forward\nunbind s\nbind f13 +forward\nunbindall\nbind a +moveleft",
        );
        assert_eq!(said, ["unknown key `f13`"]);
        assert_eq!(binds.iter().count(), 1);
    }

    #[test]
    fn the_listing_has_one_sorted_line_per_bind() {
        let (binds, _) =
            run("bind s +back\nbind shift +breath_sprint\nbind mouse1 +attack\nbind a +moveleft");
        assert_eq!(
            binds.list_lines(),
            [
                "bind MOUSE1 +attack",
                "bind SHIFT +breath_sprint",
                "bind a +moveleft",
                "bind s +back",
            ]
        );
    }

    #[test]
    fn what_the_listing_says_can_be_read_back_into_the_same_binds() {
        let mut binds = KeyBinds::default();
        binds.apply_defaults();
        let lines = binds.list_lines();
        let mut again = KeyBinds::default();
        let refused = again.apply_config_script(&lines.join("\n"));
        assert_eq!(refused, Vec::<String>::new());
        assert_eq!(again.list_lines(), lines);
    }

    #[test]
    fn the_default_controls_are_all_good_lines() {
        let wanted = DEFAULT_CONTROLS
            .lines()
            .filter(|line| line.starts_with("bind "))
            .count();
        let mut binds = KeyBinds::default();
        let said = binds.apply_script(DEFAULT_CONTROLS);
        assert_eq!(said.len(), wanted);
        assert!(
            said.iter().all(|line| line.starts_with("bind ")),
            "{said:?}"
        );
        assert!(!binds.has_pad_binds());
    }

    #[test]
    fn the_defaults_move_with_wasd_and_shoot_with_the_left_button() {
        let mut binds = KeyBinds::default();
        binds.apply_defaults();
        for (code, command) in [
            (KeyCode::KeyW, "+forward"),
            (KeyCode::KeyS, "+back"),
            (KeyCode::KeyA, "+moveleft"),
            (KeyCode::KeyD, "+moveright"),
            (KeyCode::Space, "+gostand"),
            (KeyCode::KeyR, "+reload"),
        ] {
            assert_eq!(binds.get(key(code)), Some(id(command)), "{code:?}");
        }
        assert_eq!(
            binds.get(BindButton::Mouse(MouseButton::Left)),
            Some(id("+attack"))
        );
        assert_eq!(
            binds.get(BindButton::Mouse(MouseButton::Right)),
            Some(id("+speed_throw"))
        );
        assert_eq!(
            binds.get(BindButton::WheelUp),
            Some(id("+weapnext_inventory"))
        );
    }

    #[test]
    fn the_defaults_put_back_the_pad_too_and_drop_what_was_there() {
        let mut binds = KeyBinds::default();
        binds.apply_script("bind j +activate\nbind BUTTON_A +melee");
        binds.apply_defaults();
        assert_eq!(binds.get(key(KeyCode::KeyJ)), None);
        assert_eq!(
            binds.get(BindButton::Pad(PadButton::South)),
            Some(id("+gostand"))
        );
        assert!(binds.has_pad_binds());
    }

    #[test]
    fn a_pad_layout_swaps_the_pad_and_leaves_the_keyboard() {
        let mut binds = KeyBinds::default();
        binds.apply_defaults();
        let keyboard: Vec<String> = binds
            .list_lines()
            .into_iter()
            .filter(|line| !line.contains("BUTTON_") && !line.contains("DPAD_"))
            .collect();
        binds.apply_pad_layout(2);
        assert_eq!(
            binds.get(BindButton::Pad(PadButton::LeftTrigger)),
            Some(id("+attack"))
        );
        assert_eq!(
            binds.get(BindButton::Pad(PadButton::RightTrigger)),
            Some(id("+speed_throw"))
        );
        let after: Vec<String> = binds
            .list_lines()
            .into_iter()
            .filter(|line| !line.contains("BUTTON_") && !line.contains("DPAD_"))
            .collect();
        assert_eq!(after, keyboard);
    }

    #[test]
    fn a_pad_layout_drops_pad_binds_the_player_made() {
        let mut binds = KeyBinds::default();
        binds.apply_script("bind BUTTON_START +activate");
        binds.apply_pad_layout(0);
        assert_eq!(binds.get(BindButton::Pad(PadButton::Start)), None);
    }

    #[test]
    fn a_command_comes_off_the_pad_or_off_the_keyboard_not_both() {
        let mut binds = KeyBinds::default();
        binds.apply_defaults();
        let attack = id("+attack");
        assert!(binds.clear_command_on(attack, true));
        assert_eq!(binds.get(BindButton::Pad(PadButton::RightTrigger)), None);
        assert_eq!(
            binds.get(BindButton::Mouse(MouseButton::Left)),
            Some(attack)
        );
        assert!(!binds.clear_command_on(attack, true));
        assert!(binds.clear_command_on(attack, false));
        assert_eq!(binds.get(BindButton::Mouse(MouseButton::Left)), None);
    }

    #[test]
    fn clearing_one_button_says_whether_it_had_a_command() {
        let (mut binds, _) = run("bind w +forward");
        assert!(binds.clear_button(key(KeyCode::KeyW)));
        assert!(!binds.clear_button(key(KeyCode::KeyW)));
        binds.set(key(KeyCode::KeyQ), id("+back"));
        binds.clear_all();
        assert_eq!(binds.iter().count(), 0);
    }
}
