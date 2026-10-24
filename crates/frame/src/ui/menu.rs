//! State the menu UI shares with the rest of the app.

use std::collections::HashMap;

use bevy::prelude::{Message, Resource};

/// The state of the party, for the menu to show.
#[derive(Resource, Debug, Default)]
pub struct UiPartyState {
    pub active: bool,
    pub in_lobby: bool,
    pub is_host: bool,
}

/// The menu's named values. Names are not case sensitive.
#[derive(Resource, Debug, Default)]
pub struct UiMenuDvars {
    values: HashMap<String, String>,
}

impl UiMenuDvars {
    pub fn get(&self, name: &str) -> Option<&str> {
        self.values
            .get(&name.to_ascii_lowercase())
            .map(String::as_str)
    }

    pub fn set(&mut self, name: &str, value: impl Into<String>) {
        self.values.insert(name.to_ascii_lowercase(), value.into());
    }

    /// Every value, by lowercase name, in no particular order.
    pub fn iter(&self) -> impl Iterator<Item = (&str, &str)> {
        self.values.iter().map(|(k, v)| (k.as_str(), v.as_str()))
    }
}

/// The rules the host sets for the match, as name and value pairs.
#[derive(Resource, Clone, Debug, Default)]
pub struct HostMatchRules(pub Vec<(String, String)>);

/// A console command the menu asks to run.
#[derive(Message, Clone, Debug)]
pub struct UiExecCommand {
    pub text: String,
}

/// Which menu-script menus are active and open.
#[derive(Resource, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LuiMenus {
    pub active: bool,
    pub open: bool,
}

/// How strongly the world behind the menu is blurred.
#[derive(Resource, Clone, Copy, Debug, Default, PartialEq)]
pub struct WorldBlur(pub f32);

/// How strongly the world behind the menu is darkened.
#[derive(Resource, Clone, Copy, Debug, Default, PartialEq)]
pub struct WorldDim(pub f32);

/// How many open menus dim the world.
#[derive(Resource, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MenuDims(pub u32);

/// A solo game is paused.
#[derive(Resource, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct GamePaused(pub bool);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dvar_names_ignore_case_on_both_sides() {
        let mut dvars = UiMenuDvars::default();
        dvars.set("Ui_Mode", "lobby");
        assert_eq!(dvars.get("ui_mode"), Some("lobby"));
        assert_eq!(dvars.get("UI_MODE"), Some("lobby"));
    }

    #[test]
    fn an_unset_dvar_is_absent() {
        assert_eq!(UiMenuDvars::default().get("missing"), None);
    }

    #[test]
    fn setting_a_dvar_again_under_another_case_overwrites_it() {
        let mut dvars = UiMenuDvars::default();
        dvars.set("Name", "a");
        dvars.set("NAME", String::from("b"));
        assert_eq!(dvars.get("name"), Some("b"));
        assert_eq!(dvars.iter().count(), 1);
    }

    #[test]
    fn iteration_yields_lowercase_names() {
        let mut dvars = UiMenuDvars::default();
        dvars.set("Alpha", "1");
        dvars.set("Beta", "2");
        let mut all: Vec<(&str, &str)> = dvars.iter().collect();
        all.sort_unstable();
        assert_eq!(all, [("alpha", "1"), ("beta", "2")]);
    }

    #[test]
    fn menus_start_inactive_and_the_world_untouched() {
        assert_eq!(
            LuiMenus::default(),
            LuiMenus {
                active: false,
                open: false
            }
        );
        assert_eq!(WorldBlur::default(), WorldBlur(0.0));
        assert_eq!(WorldDim::default(), WorldDim(0.0));
        assert_eq!(MenuDims::default(), MenuDims(0));
        assert_eq!(GamePaused::default(), GamePaused(false));
    }

    #[test]
    fn no_party_and_no_rules_by_default() {
        let party = UiPartyState::default();
        assert!(!party.active && !party.in_lobby && !party.is_host);
        assert!(HostMatchRules::default().0.is_empty());
    }
}
