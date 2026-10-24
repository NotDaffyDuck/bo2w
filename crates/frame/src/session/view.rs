//! Whose view the screen shows, which keys the HUD names, and the flags the
//! loading screen shares with the rest of the app.

use std::collections::BTreeMap;

use bevy::prelude::{Component, Resource};

/// Whose eyes the camera looks through.
#[derive(Resource, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ViewSubject {
    #[default]
    None,
    /// The local player's own client.
    Own { client: u32 },
    /// Watching someone else: `viewer` is the local client, `focus` the one
    /// being watched (if any yet).
    Seat { viewer: u32, focus: Option<u32> },
}

impl ViewSubject {
    /// Watching a replay of a life rather than playing.
    pub fn in_killcam(self) -> bool {
        matches!(self, Self::Seat { .. })
    }
}

/// The keys the HUD prints on its prompts.
#[derive(Resource, Clone, Debug, Default)]
pub struct HudInputView {
    /// The key for "use".
    pub use_key: Option<String>,
    /// The key a command is bound to, by command.
    pub binding_keys: BTreeMap<String, String>,
    /// Every key a command is bound to, by command, in key order.
    pub binding_keys_all: BTreeMap<String, Vec<String>>,
    pub console_open: bool,
    pub script_menu_open: bool,
    /// The keys for the four action slots.
    pub action_slot_keys: [Option<String>; 4],
}

/// Set while the loading screen is still up and the authority's navigation
/// data is being prepared; bots and the loading effects read it.
#[derive(Resource, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct BotNavigationReady(pub bool);

/// Marks the camera that draws the UI.
#[derive(Component)]
pub struct UiCamera;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_view_subject_by_default() {
        assert_eq!(ViewSubject::default(), ViewSubject::None);
    }

    #[test]
    fn only_a_seat_is_a_killcam() {
        assert!(!ViewSubject::None.in_killcam());
        assert!(!ViewSubject::Own { client: 1 }.in_killcam());
        assert!(
            ViewSubject::Seat {
                viewer: 1,
                focus: Some(2)
            }
            .in_killcam()
        );
        assert!(
            ViewSubject::Seat {
                viewer: 1,
                focus: None
            }
            .in_killcam()
        );
    }

    #[test]
    fn a_fresh_hud_view_names_no_keys_and_no_menus_are_open() {
        let view = HudInputView::default();
        assert!(view.use_key.is_none());
        assert!(view.binding_keys.is_empty() && view.binding_keys_all.is_empty());
        assert!(!view.console_open && !view.script_menu_open);
        assert!(view.action_slot_keys.iter().all(Option::is_none));
    }

    #[test]
    fn binding_keys_list_in_command_order() {
        let mut view = HudInputView::default();
        view.binding_keys_all
            .insert("zoom".into(), vec!["MOUSE2".into(), "Z".into()]);
        view.binding_keys_all
            .insert("fire".into(), vec!["MOUSE1".into()]);
        let commands: Vec<&str> = view.binding_keys_all.keys().map(String::as_str).collect();
        assert_eq!(commands, ["fire", "zoom"]);
    }

    #[test]
    fn bot_navigation_starts_unprepared() {
        assert_eq!(BotNavigationReady::default(), BotNavigationReady(false));
    }
}
