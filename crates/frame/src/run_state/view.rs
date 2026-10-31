//! Whose view the screen shows, which keys the HUD names, and the flags the
//! loading screen shares with the rest of the app.

use std::collections::BTreeMap;

use bevy::prelude::{Component, Resource};

/// Whose eyes the camera looks through.
#[derive(Resource, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum EyeSubject {
    #[default]
    None,
    /// The local player's own client.
    Own { client: u32 },
    /// Watching someone else: `viewer` is the local client, `focus` the one
    /// being watched (if any yet).
    ChairSlot { viewer: u32, focus: Option<u32> },
}

impl EyeSubject {
    /// Watching a replay of a life rather than playing.
    pub fn replaying_final_death(self) -> bool {
        let subject = self;
        matches!(subject, Self::ChairSlot { .. })
    }
}

/// The keys the HUD prints on its prompts.
#[derive(Resource, Clone, Debug, Default)]
pub struct OverlayControlSnapshot {
    /// The key for "use".
    pub act_key: Option<String>,
    /// The key a command is bound to, by command.
    pub binding_keys: BTreeMap<String, String>,
    /// Every key a command is bound to, by command, in key order.
    pub binding_keys_all: BTreeMap<String, Vec<String>>,
    pub console_open: bool,
    pub script_menu_open: bool,
    /// The keys for the four action slots.
    pub bound_action_buttons: [Option<String>; 4],
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
        assert_eq!(EyeSubject::default(), EyeSubject::None);
    }

    #[test]
    fn only_a_seat_is_a_killcam() {
        assert!(!EyeSubject::None.replaying_final_death());
        assert!(!EyeSubject::Own { client: 1 }.replaying_final_death());
        assert!(
            EyeSubject::ChairSlot {
                viewer: 1,
                focus: Some(2)
            }
            .replaying_final_death()
        );
        let unwatched = None;
        assert!(
            EyeSubject::ChairSlot {
                viewer: 1,
                focus: unwatched
            }
            .replaying_final_death()
        );
    }

    #[test]
    fn a_fresh_hud_view_names_no_keys_and_no_menus_are_open() {
        let view = OverlayControlSnapshot::default();
        assert!(view.act_key.is_none());
        assert!(view.binding_keys.is_empty() && view.binding_keys_all.is_empty());
        assert!(!view.console_open && !view.script_menu_open);
        assert!(view.bound_action_buttons.iter().all(Option::is_none));
    }

    #[test]
    fn binding_keys_list_in_command_order() {
        let mut view = OverlayControlSnapshot::default();
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
