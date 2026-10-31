//! The table from buttons to commands, plus the three console lines that edit
//! it: `bind`, `unbind` and `unbindall`.

use std::collections::HashMap;

use bevy::ecs::resource::Resource;
use input_t6::{action_number_for, action_label};

use super::ActionBinding;
use super::STOCK_BINDINGS;
use super::button_label;
use super::pad_layout;
use super::button_from_name;
use crate::TypedOrder;

/// Each button's command, stored as its number in the input layer.
#[derive(Resource, Debug, Clone, Default)]
pub struct BindingTable {
    table: HashMap<ActionBinding, u32>,
}

impl BindingTable {
    /// Restores a new player's setup: default keyboard and mouse binds with the
    /// stock pad layout.
    pub fn restore_stock_controls(&mut self) {
        self.table.clear();
        self.run_prefs_script(STOCK_BINDINGS);
        self.apply_pad_layout(0);
    }

    /// Executes the bind-changing lines of a console script, returning a reply
    /// for every line, whether it worked or was refused.
    pub fn run_control_script(&mut self, text: &str) -> Vec<String> {
        self.execute(text, true)
    }

    /// Executes a saved config's lines and reports only the refusals.
    pub(crate) fn run_prefs_script(&mut self, text: &str) -> Vec<String> {
        self.execute(text, false)
    }

    fn execute(&mut self, text: &str, verbose: bool) -> Vec<String> {
        let mut replies = Vec::new();
        let lines = text.split([';', '\n']).map(str::trim);
        for line in lines.filter(|l| !l.is_empty() && !l.starts_with("//")) {
            let Some(parsed) = TypedOrder::parse(line) else {
                continue;
            };
            let result = match parsed.name.as_str() {
                "bind" => self.bind(&parsed.words),
                "unbind" => self.unbind(&parsed.words),
                "unbindall" => self.unbind_all(),
                unknown => Err(format!("unknown bind-script command `{unknown}`")),
            };
            match result {
                Ok(Some(reply)) if verbose => replies.push(reply),
                Ok(_) => {}
                Err(refusal) => replies.push(refusal),
            }
        }
        replies
    }

    /// With no argument it is silent; with a key it reports what that key runs;
    /// with a key and a command it binds the command to each button the key
    /// name stands for.
    fn bind(&mut self, words: &[String]) -> Result<Option<String>, String> {
        let (key, wanted) = match words {
            [] => return Ok(None),
            [key] => (key, None),
            [key, rest @ ..] => (key, Some(rest.join(" "))),
        };
        let Some(wanted) = wanted else {
            let found = buttons_named(key)?
                .into_iter()
                .find_map(|control| self.command_on(control));
            let reply = match found {
                Some(name) => format!("bind {key} {name}"),
                None => format!("`{key}` is unbound"),
            };
            return Ok(Some(reply));
        };
        let Some(id) = action_number_for(&wanted) else {
            return Err(format!("unknown command `{wanted}`"));
        };
        for control in buttons_named(key)? {
            self.assign(control, id);
        }
        let shown = action_label(id).unwrap_or(wanted.as_str());
        Ok(Some(format!("bind {key} {shown}")))
    }

    fn unbind(&mut self, words: &[String]) -> Result<Option<String>, String> {
        let [key] = words else {
            return Err("usage: unbind <key>".to_owned());
        };
        let mut any_freed = false;
        for control in buttons_named(key)? {
            any_freed |= self.free_control(control);
        }
        let reply = if any_freed {
            format!("unbind {key}")
        } else {
            format!("`{key}` is unbound")
        };
        Ok(Some(reply))
    }

    /// `unbindall`: drops every bind and says so.
    fn unbind_all(&mut self) -> Result<Option<String>, String> {
        self.forget_all();
        Ok(Some(String::from("unbindall")))
    }

    /// Puts command `id` on `control`, replacing what was there.
    pub fn assign(&mut self, control: ActionBinding, id: u32) {
        self.table.insert(control, id);
    }

    /// The command number on `control`, if any.
    pub fn bound_to(&self, control: ActionBinding) -> Option<u32> {
        self.table.get(&control).copied()
    }

    /// The command name on `control`, if any.
    pub fn command_on(&self, control: ActionBinding) -> Option<&'static str> {
        self.bound_to(control).and_then(action_label)
    }

    /// Removes the bind on `control`; true if there was one.
    pub fn free_control(&mut self, control: ActionBinding) -> bool {
        self.table.remove(&control).is_some()
    }

    /// Drops every bind, keyboard, mouse and pad alike.
    pub fn forget_all(&mut self) {
        self.table.clear();
    }

    /// Removes command `id` from the pad's buttons when `pad` is set, otherwise
    /// from the keyboard and mouse; true if anything was removed.
    pub fn clear_command_on(&mut self, id: u32, pad: bool) -> bool {
        let count = self.table.len();
        self.table
            .retain(|button, bound| *bound != id || button.is_pad() != pad);
        self.table.len() < count
    }

    /// Replaces the pad's binds with numbered layout `layout`; keyboard and
    /// mouse binds stay. Commands the input layer does not know are skipped.
    pub fn apply_pad_layout(&mut self, layout: usize) {
        self.table.retain(|button, _| !button.is_pad());
        for bind in pad_layout(layout) {
            let Some(id) = action_number_for(bind.command) else {
                continue;
            };
            self.assign(ActionBinding::Pad(bind.button), id);
        }
    }

    pub fn has_pad_binds(&self) -> bool {
        self.table.keys().any(|button| button.is_pad())
    }

    pub fn iter(&self) -> impl Iterator<Item = (ActionBinding, u32)> + '_ {
        self.table.iter().map(|(&button, &id)| (button, id))
    }

    /// A sorted `bind <button> <command>` line for each bind. The two sides of
    /// a modifier key display alike and collapse into one line.
    pub fn control_listing(&self) -> Vec<String> {
        let mut out = Vec::new();
        for (control, id) in self.iter() {
            if let Some(name) = action_label(id) {
                out.push(format!("bind {} {name}", button_label(control)));
            }
        }
        out.sort();
        out.dedup();
        out
    }
}

fn buttons_named(key: &str) -> Result<Vec<ActionBinding>, String> {
    match button_from_name(key) {
        Some(buttons) => Ok(buttons),
        None => Err(format!("unknown key `{key}`")),
    }
}

#[cfg(test)]
mod tests {
    use bevy::prelude::*;

    use crate::keymap::map::*;
    use crate::GamepadKey;

    fn id(command: &str) -> u32 {
        action_number_for(command).unwrap_or_else(|| panic!("no command {command}"))
    }

    fn key(code: KeyCode) -> ActionBinding {
        ActionBinding::Board(code)
    }

    fn run(text: &str) -> (Vec<String>, BindingTable) {
        let mut binds = BindingTable::default();
        let said = binds.run_control_script(text);
        (said, binds)
    }

    #[test]
    fn a_bind_line_puts_the_command_on_the_key_and_says_so() {
        let (said, binds) = run("bind w +forward");
        assert_eq!(said, ["bind w +forward"]);
        assert_eq!(binds.bound_to(key(KeyCode::KeyW)), Some(id("+forward")));
        assert_eq!(binds.command_on(key(KeyCode::KeyW)), Some("+forward"));
    }

    #[test]
    fn a_command_of_several_words_is_the_rest_of_the_line() {
        let (said, binds) = run("bind 3 +actionslot 4");
        assert_eq!(said.len(), 1);
        assert!(said[0].starts_with("bind 3 "), "{said:?}");
        assert_eq!(binds.bound_to(key(KeyCode::Digit3)), Some(id("+actionslot 4")));
    }

    #[test]
    fn a_name_that_covers_both_sides_binds_both() {
        let (_, binds) = run("bind shift +breath_sprint");
        let sprint = Some(id("+breath_sprint"));
        assert_eq!(binds.bound_to(key(KeyCode::ShiftLeft)), sprint);
        assert_eq!(binds.bound_to(key(KeyCode::ShiftRight)), sprint);
        let (_, binds) = run("bind rshift +breath_sprint");
        assert_eq!(binds.bound_to(key(KeyCode::ShiftLeft)), None);
        assert_eq!(binds.bound_to(key(KeyCode::ShiftRight)), sprint);
    }

    #[test]
    fn binding_a_key_again_replaces_what_was_on_it() {
        let (_, binds) = run("bind w +forward; bind w +back");
        assert_eq!(binds.bound_to(key(KeyCode::KeyW)), Some(id("+back")));
        assert_eq!(binds.iter().count(), 1);
    }

    #[test]
    fn mouse_wheel_and_pad_buttons_bind_by_name() {
        let (_, binds) =
            run("bind mouse1 +attack\nbind mwheeldown weapprev\nbind BUTTON_A +gostand");
        let actual = binds.bound_to(ActionBinding::Pointer(MouseButton::Left));
        assert_eq!(actual, Some(id("+attack")));
        assert_eq!(binds.bound_to(ActionBinding::WheelDown), Some(id("weapprev")));
        let actual = binds.bound_to(ActionBinding::Pad(GamepadKey::South));
        assert_eq!(actual, Some(id("+gostand")));
    }

    #[test]
    fn a_key_alone_says_what_is_on_it_or_that_it_is_free() {
        let mut binds = BindingTable::default();
        assert_eq!(binds.run_control_script("bind w"), ["`w` is unbound"]);
        binds.run_control_script("bind w +forward");
        assert_eq!(binds.run_control_script("bind w"), ["bind w +forward"]);
        assert_eq!(binds.run_control_script("bind W"), ["bind W +forward"]);
    }

    #[test]
    fn a_modifier_alone_says_what_is_on_either_side() {
        let mut binds = BindingTable::default();
        binds.run_control_script("bind rshift +breath_sprint");
        let expected = ["bind shift +breath_sprint"];
        assert_eq!(binds.run_control_script("bind shift"), expected);
    }

    #[test]
    fn bind_with_nothing_after_it_says_nothing() {
        assert_eq!(run("bind").0, Vec::<String>::new());
    }

    #[test]
    fn a_key_or_command_that_does_not_exist_is_refused_by_name() {
        let (said, binds) = run("bind f13 +forward\nbind w +nosuchcommand\nbind f13");
        let expected = [
            "unknown key `f13`",
            "unknown command `+nosuchcommand`",
            "unknown key `f13`"
        ];
        assert_eq!(said, expected);
        assert_eq!(binds.iter().count(), 0);
    }

    #[test]
    fn when_both_are_wrong_the_command_is_the_one_named() {
        let expected = ["unknown command `+nosuchcommand`"];
        assert_eq!(run("bind f13 +nosuchcommand").0, expected);
    }

    #[test]
    fn unbind_frees_the_key_and_says_so_once() {
        let mut binds = BindingTable::default();
        binds.run_control_script("bind w +forward");
        assert_eq!(binds.run_control_script("unbind w"), ["unbind w"]);
        assert_eq!(binds.bound_to(key(KeyCode::KeyW)), None);
        assert_eq!(binds.run_control_script("unbind w"), ["`w` is unbound"]);
    }

    #[test]
    fn unbinding_a_modifier_frees_both_sides() {
        let mut binds = BindingTable::default();
        binds.run_control_script("bind shift +breath_sprint");
        assert_eq!(binds.run_control_script("unbind shift"), ["unbind shift"]);
        assert!(binds.iter().next().is_none());
    }

    #[test]
    fn unbind_takes_exactly_one_key() {
        for line in ["unbind", "unbind w s"] {
            assert_eq!(run(line).0, ["usage: unbind <key>"], "{line}");
        }
        assert_eq!(run("unbind f13").0, ["unknown key `f13`"]);
    }

    #[test]
    fn unbindall_frees_everything() {
        let mut binds = BindingTable::default();
        binds.run_control_script("bind w +forward; bind s +back");
        assert_eq!(binds.run_control_script("unbindall"), ["unbindall"]);
        assert_eq!(binds.iter().count(), 0);
    }

    #[test]
    fn lines_are_split_on_semicolons_and_newlines_and_comments_are_skipped() {
        let (said, binds) =
            run("// a comment\n\n  bind w +forward ; bind s +back\n// bind d +moveright\n");
        assert_eq!(said, ["bind w +forward", "bind s +back"]);
        assert_eq!(binds.iter().count(), 2);
    }

    #[test]
    fn a_line_that_is_not_a_bind_line_is_named() {
        assert_eq!(run("say hello").0, ["unknown bind-script command `say`"]);
    }

    #[test]
    fn a_saved_config_is_quiet_about_what_it_did_and_loud_about_what_it_refused() {
        let mut binds = BindingTable::default();
        let said = binds.run_prefs_script(
            "bind w +forward\nunbind s\nbind f13 +forward\nunbindall\nbind a +moveleft",
        );
        assert_eq!(said, ["unknown key `f13`"]);
        assert_eq!(binds.iter().count(), 1);
    }

    #[test]
    fn the_listing_has_one_sorted_line_per_bind() {
        let (_, binds) =
            run("bind s +back\nbind shift +breath_sprint\nbind mouse1 +attack\nbind a +moveleft");
        let expected = [
            "bind MOUSE1 +attack",
            "bind SHIFT +breath_sprint",
            "bind a +moveleft",
            "bind s +back",
        ];
        assert_eq!(binds.control_listing(), expected);
    }

    #[test]
    fn what_the_listing_says_can_be_read_back_into_the_same_binds() {
        let mut binds = BindingTable::default();
        binds.restore_stock_controls();
        let lines = binds.control_listing();
        let mut again = BindingTable::default();
        let refused = again.run_prefs_script(&lines.join("\n"));
        assert_eq!(refused, Vec::<String>::new());
        assert_eq!(again.control_listing(), lines);
    }

    #[test]
    fn the_default_controls_are_all_good_lines() {
        let wanted = STOCK_BINDINGS
            .lines()
            .filter(|line| line.starts_with("bind "))
            .count();
        let mut binds = BindingTable::default();
        let said = binds.run_control_script(STOCK_BINDINGS);
        assert_eq!(said.len(), wanted);
        assert!(
            said.iter().all(|line| line.starts_with("bind ")),
            "{said:?}"
        );
        assert!(!binds.has_pad_binds());
    }

    #[test]
    fn the_defaults_move_with_wasd_and_shoot_with_the_left_button() {
        let mut binds = BindingTable::default();
        binds.restore_stock_controls();
        for (code, command) in [
            (KeyCode::KeyW, "+forward"),
            (KeyCode::KeyS, "+back"),
            (KeyCode::KeyA, "+moveleft"),
            (KeyCode::KeyD, "+moveright"),
            (KeyCode::Space, "+gostand"),
            (KeyCode::KeyR, "+reload"),
        ] {
            assert_eq!(binds.bound_to(key(code)), Some(id(command)), "{code:?}");
        }
        let actual = binds.bound_to(ActionBinding::Pointer(MouseButton::Left));
        assert_eq!(actual, Some(id("+attack")));
        let actual = binds.bound_to(ActionBinding::Pointer(MouseButton::Right));
        assert_eq!(actual, Some(id("+speed_throw")));
        let actual = binds.bound_to(ActionBinding::WheelUp);
        assert_eq!(actual, Some(id("+weapnext_inventory")));
    }

    #[test]
    fn the_defaults_put_back_the_pad_too_and_drop_what_was_there() {
        let mut binds = BindingTable::default();
        binds.run_control_script("bind j +activate\nbind BUTTON_A +melee");
        binds.restore_stock_controls();
        assert_eq!(binds.bound_to(key(KeyCode::KeyJ)), None);
        let actual = binds.bound_to(ActionBinding::Pad(GamepadKey::South));
        assert_eq!(actual, Some(id("+gostand")));
        assert!(binds.has_pad_binds());
    }

    #[test]
    fn a_pad_layout_swaps_the_pad_and_leaves_the_keyboard() {
        let mut binds = BindingTable::default();
        binds.restore_stock_controls();
        let keyboard: Vec<String> = binds
            .control_listing()
            .into_iter()
            .filter(|line| !line.contains("BUTTON_") && !line.contains("DPAD_"))
            .collect::<Vec<String>>();
        binds.apply_pad_layout(2);
        let actual = binds.bound_to(ActionBinding::Pad(GamepadKey::LeftTrigger));
        assert_eq!(actual, Some(id("+attack")));
        let actual = binds.bound_to(ActionBinding::Pad(GamepadKey::RightTrigger));
        assert_eq!(actual, Some(id("+speed_throw")));
        let after: Vec<String> = binds
            .control_listing()
            .into_iter()
            .filter(|line| !line.contains("BUTTON_") && !line.contains("DPAD_"))
            .collect::<Vec<String>>();
        assert_eq!(after, keyboard);
    }

    #[test]
    fn a_pad_layout_drops_pad_binds_the_player_made() {
        let mut binds = BindingTable::default();
        binds.run_control_script("bind BUTTON_START +activate");
        binds.apply_pad_layout(0);
        assert_eq!(binds.bound_to(ActionBinding::Pad(GamepadKey::Start)), None);
    }

    #[test]
    fn a_command_comes_off_the_pad_or_off_the_keyboard_not_both() {
        let mut binds = BindingTable::default();
        binds.restore_stock_controls();
        let attack = id("+attack");
        assert!(binds.clear_command_on(attack, true));
        assert_eq!(binds.bound_to(ActionBinding::Pad(GamepadKey::RightTrigger)), None);
        let expected = Some(attack);
        assert_eq!(binds.bound_to(ActionBinding::Pointer(MouseButton::Left)), expected);
        assert!(!binds.clear_command_on(attack, true));
        assert!(binds.clear_command_on(attack, false));
        assert_eq!(binds.bound_to(ActionBinding::Pointer(MouseButton::Left)), None);
    }

    #[test]
    fn clearing_one_button_says_whether_it_had_a_command() {
        let (_, mut binds) = run("bind w +forward");
        assert!(binds.free_control(key(KeyCode::KeyW)));
        assert!(!binds.free_control(key(KeyCode::KeyW)));
        binds.assign(key(KeyCode::KeyQ), id("+back"));
        binds.forget_all();
        assert_eq!(binds.iter().count(), 0);
    }
}
