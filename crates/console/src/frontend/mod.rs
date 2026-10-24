//! The main menu's side of the console: the `ui_*` commands the menu scripts
//! send, and the dvars it draws from, refreshed every frame.
//!
//! * `lobby`: the lobby, local or public, and who is in it.
//! * `maps`: the pages of maps and the row the pointer is on.
//! * `browser`: the public lobbies the master lists, and joining one.
//!
//! Failures are not fatal: the reason is shown on a status line of the menu
//! and said on the console.

mod browser;
mod lobby;
mod maps;

use std::str::FromStr;

use asset_game::LocalizeCatalog;
use bevy::{ecs::system::SystemParam, prelude::*};
use frame::{GameSettings, ReturnedToMenu, UiMenuDvars, UiPartyState};
use sim::HostGameModeSelection;

use crate::{CommandSpec, ConsoleCommand, ConsoleRegistry, feature_dispatch::ConsoleEcho};

const COMMANDS: [&str; 17] = [
    "set",
    "seta",
    "setfromdvar",
    "ui_create_lobby",
    "ui_leave_lobby",
    "ui_start_match",
    "ui_lobby_privacy",
    "ui_vote_skip",
    "ui_maps",
    "ui_map_pack",
    "ui_map_hover",
    "ui_select_map",
    "ui_select_mode",
    "ui_browser_refresh",
    "ui_browser_page",
    "ui_join_lobby",
    "ui_join_lobby_id",
];

/// Adds the menu's commands to the registry, leaving any that is already there.
pub(crate) fn register(registry: &mut ConsoleRegistry) {
    for name in COMMANDS {
        if registry.resolve(name).is_none() {
            registry.register(CommandSpec::new(name));
        }
    }
}

/// What the menu remembers between frames.
#[derive(Default)]
pub(crate) struct State {
    /// The lobby is listed with the master, not only on this machine.
    public: bool,
    maps: maps::MapView,
    servers: browser::ServerList,
}

/// Everything a menu command may change.
#[derive(SystemParam)]
pub(crate) struct Menu<'w, 's> {
    commands: Commands<'w, 's>,
    dvars: ResMut<'w, UiMenuDvars>,
    party: ResMut<'w, UiPartyState>,
    swaps: ResMut<'w, session::SessionSwapRequest>,
    maps: Res<'w, ui::MenuMapList>,
    master: lobby::Master<'w>,
    echo: ConsoleEcho<'w>,
    state: Local<'s, State>,
}

/// The first word of a command, read as a `T`.
fn arg<T: FromStr>(command: &ConsoleCommand) -> Option<T> {
    command.args.first()?.parse().ok()
}

/// How a switch is written in a dvar.
fn flag(on: bool) -> &'static str {
    if on { "1" } else { "0" }
}

/// The status line a failed command is reported on: the lobby browser has its
/// own, everything else shares one.
fn status_dvar(command: &str) -> &'static str {
    if command.starts_with("ui_browser") || command == "ui_join_lobby" {
        "ui_browser_status"
    } else {
        "ui_frontend_status"
    }
}

impl Menu<'_, '_> {
    /// Returning to the menu from a match clears the match; returning while a
    /// public lobby was open means the lobby is over.
    fn back_from(&mut self, from_world: bool, in_menu: bool) {
        if from_world {
            self.commands.remove_resource::<frame::HostMatchRules>();
            self.commands.remove_resource::<HostGameModeSelection>();
            *self.party = UiPartyState::default();
            self.state.public = false;
        } else if in_menu && self.state.public {
            let reason = self.lobby_ended_text();
            self.state.public = false;
            if !self.party.is_host {
                *self.party = UiPartyState::default();
            }
            self.dvars.set("ui_frontend_status", reason);
        }
    }

    /// The map and mode the host starts from, until one is chosen.
    fn seed_defaults(&mut self) {
        if self.dvars.get("ui_mapname").is_none()
            && let Some(map) = self.maps.maps().find(|map| map.starts_with("t6:"))
        {
            self.dvars.set("ui_mapname", map.clone());
        }
        if self.dvars.get("ui_gametype").is_none() {
            self.dvars
                .set("ui_gametype", HostGameModeSelection::from_env().token());
        }
    }

    /// `set name` says the value; `set name words...` stores them.
    fn set_dvar(&mut self, command: &ConsoleCommand) {
        match command.args.as_slice() {
            [] => {}
            [name] => {
                let line = format!("{name} = {}", self.dvars.get(name).unwrap_or(""));
                self.echo.write(line);
            }
            [name, values @ ..] => self.dvars.set(name, values.join(" ")),
        }
    }

    fn copy_dvar(&mut self, command: &ConsoleCommand) {
        if let [name, source] = command.args.as_slice()
            && let Some(value) = self.dvars.get(source).map(str::to_owned)
        {
            self.dvars.set(name, value);
        }
    }

    fn run(&mut self, command: &ConsoleCommand) -> Result<(), String> {
        match command.name.as_str() {
            "set" | "seta" => self.set_dvar(command),
            "setfromdvar" => self.copy_dvar(command),
            "ui_create_lobby" => self.create_lobby()?,
            "ui_leave_lobby" => self.leave_lobby()?,
            "ui_lobby_privacy" => self.toggle_privacy()?,
            "ui_start_match" => self.start_match()?,
            "ui_vote_skip" => self.vote_skip()?,
            "ui_maps" => self.open_maps(),
            "ui_map_pack" => self.pick_map_page(command)?,
            "ui_map_hover" => self.point_at_map(command)?,
            "ui_select_map" => self.select_map(command)?,
            "ui_select_mode" => self.select_mode(command)?,
            "ui_browser_refresh" => self.refresh_browser()?,
            "ui_browser_page" => self.turn_browser_page(command),
            "ui_join_lobby" => self.join_row(command)?,
            "ui_join_lobby_id" => self.join_id(command)?,
            _ => {}
        }
        Ok(())
    }

    fn report(&mut self, command: &ConsoleCommand, error: &str) {
        self.dvars.set(status_dvar(&command.name), error);
        self.echo.write(format!("menu: {error}"));
    }

    fn publish(&mut self, own_name: &str, localize: Option<&LocalizeCatalog>) {
        self.publish_lobby(own_name, localize);
        self.publish_maps();
        self.publish_browser();
    }
}

pub(crate) fn route(
    mut events: MessageReader<ConsoleCommand>,
    mut returned: MessageReader<ReturnedToMenu>,
    settings: Res<GameSettings>,
    localize: Option<Res<LocalizeCatalog>>,
    mut menu: Menu,
) {
    let (mut from_world, mut in_menu) = (false, false);
    for fact in returned.read() {
        from_world |= fact.had_world;
        in_menu |= !fact.had_world;
    }
    menu.back_from(from_world, in_menu);
    menu.seed_defaults();
    for command in events.read() {
        if let Err(error) = menu.run(command) {
            menu.report(command, &error);
        }
    }
    menu.publish(&settings.player_name, localize.as_deref());
}

/// What the tests of this module and its parts share: an app with the menu's
/// system and resources, a way to type a line, and a way to read a dvar.
#[cfg(test)]
pub(crate) mod testing {
    use asset_transport::MapPack;

    use super::*;
    use crate::feature_dispatch::testing::{console_app, feed};

    pub(crate) use crate::feature_dispatch::testing::said;

    /// An app whose menu has one pack per `(label, map count)`.
    pub(crate) fn menu_app(packs: &[(&str, usize)]) -> App {
        let packs = packs
            .iter()
            .map(|&(label, count)| MapPack {
                label: label.to_owned(),
                maps: (0..count).map(|n| format!("t6:mp_{label}_{n}")).collect(),
            })
            .collect();
        let mut app = console_app();
        app.init_resource::<UiMenuDvars>()
            .init_resource::<UiPartyState>()
            .init_resource::<session::SessionSwapRequest>()
            .init_resource::<GameSettings>()
            .insert_resource(ui::MenuMapList(packs))
            .add_message::<ReturnedToMenu>()
            .add_systems(Update, route);
        app
    }

    /// Types one line of script and runs a frame.
    pub(crate) fn run(app: &mut App, line: &str) {
        feed(app, line);
    }

    /// A dvar's value, empty while it has none.
    pub(crate) fn dvar(app: &App, name: &str) -> String {
        app.world()
            .resource::<UiMenuDvars>()
            .get(name)
            .unwrap_or("")
            .to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use testing::{dvar, menu_app, run, said};

    #[test]
    fn every_command_is_registered_once() {
        let mut registry = ConsoleRegistry::default();
        register(&mut registry);
        register(&mut registry);
        let mut names = registry.names();
        assert_eq!(names.len(), COMMANDS.len());
        names.sort();
        let mut expected = COMMANDS.to_vec();
        expected.sort();
        assert_eq!(names, expected);
    }

    #[test]
    fn a_command_registered_first_is_left_alone() {
        let mut registry = ConsoleRegistry::default();
        registry.register(CommandSpec::new("set").usage("set <name> <value>"));
        register(&mut registry);
        assert_eq!(
            registry.resolve("set").map(|spec| spec.usage.as_str()),
            Some("set <name> <value>")
        );
        assert_eq!(
            registry
                .names()
                .iter()
                .filter(|name| *name == "set")
                .count(),
            1
        );
    }

    #[test]
    fn a_failure_is_shown_on_the_status_line_of_its_screen() {
        assert_eq!(status_dvar("ui_browser_refresh"), "ui_browser_status");
        assert_eq!(status_dvar("ui_browser_page"), "ui_browser_status");
        assert_eq!(status_dvar("ui_join_lobby"), "ui_browser_status");
        assert_eq!(status_dvar("ui_join_lobby_id"), "ui_frontend_status");
        assert_eq!(status_dvar("ui_start_match"), "ui_frontend_status");
    }

    #[test]
    fn switches_are_written_one_and_zero() {
        assert_eq!((flag(true), flag(false)), ("1", "0"));
    }

    #[test]
    fn a_word_is_read_as_the_type_wanted() {
        let command = ConsoleCommand::parse("x 12 -3").expect("a command line");
        assert_eq!(arg::<usize>(&command), Some(12));
        assert_eq!(arg::<isize>(&command), Some(12));
        let negative = ConsoleCommand::parse("x -3").expect("a command line");
        assert_eq!(arg::<usize>(&negative), None);
        assert_eq!(arg::<isize>(&negative), Some(-3));
        let bare = ConsoleCommand::parse("x").expect("a command line");
        assert_eq!(arg::<usize>(&bare), None);
    }

    #[test]
    fn set_stores_the_words_it_is_given() {
        let mut app = menu_app(&[("base", 3)]);
        run(&mut app, "set ui_note one two three");
        assert_eq!(dvar(&app, "ui_note"), "one two three");
        run(&mut app, "seta ui_note four");
        assert_eq!(dvar(&app, "ui_note"), "four");
    }

    #[test]
    fn set_with_only_a_name_says_the_value() {
        let mut app = menu_app(&[("base", 3)]);
        run(&mut app, "set ui_note hello");
        run(&mut app, "set ui_note");
        run(&mut app, "set ui_never_set");
        assert_eq!(said(&app), ["ui_note = hello", "ui_never_set = "]);
    }

    #[test]
    fn set_with_nothing_does_nothing() {
        let mut app = menu_app(&[("base", 3)]);
        run(&mut app, "set");
        assert!(said(&app).is_empty());
    }

    #[test]
    fn setfromdvar_copies_a_dvar_that_exists() {
        let mut app = menu_app(&[("base", 3)]);
        run(&mut app, "set ui_from copied");
        run(&mut app, "setfromdvar ui_to ui_from");
        assert_eq!(dvar(&app, "ui_to"), "copied");
        run(&mut app, "setfromdvar ui_other ui_missing");
        assert_eq!(dvar(&app, "ui_other"), "");
        assert!(said(&app).is_empty());
    }

    #[test]
    fn the_menu_starts_on_the_first_map_and_the_default_mode() {
        let mut app = menu_app(&[("base", 3)]);
        app.update();
        assert_eq!(dvar(&app, "ui_mapname"), "t6:mp_base_0");
        assert!(HostGameModeSelection::from_token(&dvar(&app, "ui_gametype")).is_some());
    }

    #[test]
    fn a_map_or_mode_already_chosen_is_not_replaced() {
        let mut app = menu_app(&[("base", 3)]);
        run(&mut app, "set ui_mapname t6:mp_base_2");
        run(&mut app, "set ui_gametype tdm");
        app.update();
        assert_eq!(dvar(&app, "ui_mapname"), "t6:mp_base_2");
        assert_eq!(dvar(&app, "ui_gametype"), "tdm");
    }

    #[test]
    fn with_no_maps_installed_no_map_is_chosen() {
        let mut app = menu_app(&[]);
        app.update();
        assert_eq!(dvar(&app, "ui_mapname"), "");
    }

    #[test]
    fn a_failure_is_shown_and_said() {
        let mut app = menu_app(&[("base", 3)]);
        run(&mut app, "ui_start_match");
        assert_eq!(
            dvar(&app, "ui_frontend_status"),
            "Only the lobby host can start a match"
        );
        assert_eq!(said(&app), ["menu: Only the lobby host can start a match"]);
    }

    #[test]
    fn a_command_of_another_part_of_the_console_is_ignored() {
        let mut app = menu_app(&[("base", 3)]);
        run(&mut app, "echo hello");
        assert!(said(&app).is_empty());
        assert_eq!(dvar(&app, "ui_frontend_status"), "");
    }

    #[test]
    fn every_dvar_the_menu_draws_from_is_there_after_one_frame() {
        let mut app = menu_app(&[("base", 3)]);
        app.update();
        for name in [
            "ui_lobby_players",
            "ui_lobby_member_0",
            "ui_lobby_member_17_visible",
            "ui_lobby_host",
            "ui_lobby_public",
            "ui_lobby_privacy",
            "ui_map_label",
            "ui_lobby_preview",
            "ui_mode_label",
            "ui_map_preview_title",
            "ui_map_9",
            "ui_map_pack_15",
            "ui_map_pack_tint_15",
            "ui_browser_9",
        ] {
            assert!(
                app.world().resource::<UiMenuDvars>().get(name).is_some(),
                "{name} is not published"
            );
        }
    }

    fn returned(app: &mut App, had_world: bool) {
        app.world_mut().write_message(ReturnedToMenu {
            swap_id: 1,
            had_world,
        });
        app.update();
    }

    #[test]
    fn coming_back_from_a_match_ends_the_lobby() {
        let mut app = menu_app(&[("base", 3)]);
        run(&mut app, "ui_create_lobby");
        run(&mut app, "ui_start_match");
        assert!(
            app.world()
                .get_resource::<frame::HostMatchRules>()
                .is_some()
        );
        returned(&mut app, true);
        assert!(
            app.world()
                .get_resource::<frame::HostMatchRules>()
                .is_none()
        );
        assert!(
            app.world()
                .get_resource::<HostGameModeSelection>()
                .is_none()
        );
        let party = app.world().resource::<UiPartyState>();
        assert!(!party.active && !party.in_lobby && !party.is_host);
        assert_eq!(dvar(&app, "ui_lobby_host"), "0");
    }

    #[test]
    fn coming_back_to_the_menu_keeps_a_local_lobby() {
        let mut app = menu_app(&[("base", 3)]);
        run(&mut app, "ui_create_lobby");
        returned(&mut app, false);
        let party = app.world().resource::<UiPartyState>();
        assert!(party.in_lobby && party.is_host);
        assert_eq!(dvar(&app, "ui_lobby_host"), "1");
    }
}
