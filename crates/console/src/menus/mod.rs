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

use asset_game::TranslationStringIndex;
use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use frame::PlayerPrefsBundle;
use frame::ReturnedToMenu;
use frame::UiMenuDvars;
use frame::UiPartyState;
use sim::HostChosenMode;

use crate::OrderBlueprint;
use crate::TypedOrder;
use crate::OrderCatalog;
use crate::tool_cmds::PromptReplyWriter;
use std::str::FromStr;

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
pub(crate) fn enlist(catalog: &mut OrderCatalog) {
    for name in COMMANDS {
        if catalog.look_up(name).is_none() {
            catalog.enlist(OrderBlueprint::new(name));
        }
    }
}

/// What the menu remembers between frames.
#[derive(Default)]
pub(crate) struct MenuMemory {
    /// The lobby is listed with the master, not only on this machine.
    listed: bool,
    maps: maps::MapView,
    servers: browser::ServerList,
}

/// Everything a menu command may change.
#[derive(SystemParam)]
pub(crate) struct Menu<'w, 's> {
    commands: Commands<'w, 's>,
    dvars: ResMut<'w, UiMenuDvars>,
    party: ResMut<'w, UiPartyState>,
    swaps: ResMut<'w, session::SessionHandoverOrder>,
    arenas: Res<'w, ui::ScreenArenaRoster>,
    master: lobby::Master<'w>,
    echo: PromptReplyWriter<'w>,
    state: Local<'s, MenuMemory>,
}

/// The first word of a command, read as a `T`.
fn arg<T: FromStr>(command: &TypedOrder) -> Option<T> {
    command.words.first()?.parse().ok()
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
            self.commands.remove_resource::<HostChosenMode>();
            *self.party = UiPartyState::default();
            self.state.listed = false;
        } else if in_menu && self.state.listed {
            let reason = self.lobby_ended_text();
            self.state.listed = false;
            if !self.party.is_host {
                *self.party = UiPartyState::default();
            }
            self.dvars.set("ui_frontend_status", reason);
        }
    }

    /// The map and mode the host starts from, until one is chosen.
    fn seed_defaults(&mut self) {
        if self.dvars.get("ui_mapname").is_none()
            && let Some(map) = self.arenas.maps().find(|map| map.starts_with("t6:"))
        {
            self.dvars.set("ui_mapname", map.clone());
        }
        if self.dvars.get("ui_gametype").is_none() {
            self.dvars
                .set("ui_gametype", HostChosenMode::read_env().token());
        }
    }

    /// `set name` says the value; `set name words...` stores them.
    fn set_dvar(&mut self, command: &TypedOrder) {
        match command.words.as_slice() {
            [] => {}
            [name] => {
                let line = format!("{name} = {}", self.dvars.get(name).unwrap_or(""));
                self.echo.write(line);
            }
            [name, values @ ..] => self.dvars.set(name, values.join(" ")),
        }
    }

    fn copy_dvar(&mut self, command: &TypedOrder) {
        if let [name, source] = command.words.as_slice()
            && let Some(value) = self.dvars.get(source).map(str::to_owned)
        {
            self.dvars.set(name, value);
        }
    }

    fn perform(&mut self, command: &TypedOrder) -> Result<(), String> {
        let verb = command.name.as_str();
        match verb {
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

    fn report(&mut self, command: &TypedOrder, error: &str) {
        self.dvars.set(status_dvar(&command.name), error);
        self.echo.write(format!("menu: {error}"));
    }

    fn publish(&mut self, own_name: &str, localize: Option<&TranslationStringIndex>) {
        self.publish_lobby(own_name, localize);
        self.publish_maps();
        self.publish_browser();
    }
}

pub(crate) fn route_menu_orders(
    mut events: MessageReader<TypedOrder>,
    mut returned: MessageReader<ReturnedToMenu>,
    settings: Res<PlayerPrefsBundle>,
    localize: Option<Res<TranslationStringIndex>>,
    mut menu: Menu,
) {
    let (mut from_world, mut in_menu) = (false, false);
    for fact in returned.read() {
        from_world |= fact.had_world;
        in_menu |= !fact.had_world;
    }
    menu.back_from(from_world, in_menu);
    menu.seed_defaults();
    for order in events.read() {
        if let Err(error) = menu.perform(order) {
            menu.report(order, &error);
        }
    }
    menu.publish(&settings.shown_name(), localize.as_deref());
}

/// What the tests of this module and its parts share: an app with the menu's
/// system and resources, a way to type a line, and a way to read a dvar.
#[cfg(test)]
pub(crate) mod testing {
    use asset_transport::MapPack;

    use crate::menus::*;
    use crate::tool_cmds::testing::{console_app, feed};

    pub(crate) use crate::tool_cmds::testing::said;

    /// An app whose menu has one pack per `(label, map count)`.
    pub(crate) fn menu_app(packs: &[(&str, usize)]) -> App {
        let packs = packs
            .iter()
            .map(|&(label, count)| MapPack {
                label: String::from(label),
                maps: (0..count).map(|n| format!("t6:mp_{label}_{n}")).collect(),
            })
            .collect::<Vec<MapPack>>();
        let mut app = console_app();
        app.init_resource::<UiMenuDvars>()
            .init_resource::<UiPartyState>()
            .init_resource::<session::SessionHandoverOrder>()
            .init_resource::<PlayerPrefsBundle>()
            .insert_resource(ui::ScreenArenaRoster(packs))
            .add_message::<ReturnedToMenu>()
            .add_systems(bevy::app::Update, route_menu_orders);
        app
    }

    /// Types one line of script and runs a frame.
    pub(crate) fn drive(app: &mut App, typed: &str) {
        feed(app, typed);
    }

    /// A dvar's value, empty while it has none.
    pub(crate) fn dvar(app: &App, key: &str) -> String {
        app.world()
            .resource::<UiMenuDvars>()
            .get(key)
            .map_or(String::new(), str::to_owned)
    }
}

#[cfg(test)]
mod tests {
    use crate::menus::*;
    use testing::{dvar, menu_app, drive, said};

    #[test]
    fn every_command_is_registered_once() {
        let mut book = OrderCatalog::default();
        enlist(&mut book);
        enlist(&mut book);
        let mut names = book.all_names();
        assert_eq!(names.len(), COMMANDS.len());
        names.sort_by(|left, right| left.cmp(right));
        let mut expected = COMMANDS.to_vec();
        expected.sort();
        assert_eq!(names, expected);
    }

    #[test]
    fn a_command_registered_first_is_left_alone() {
        let mut book = OrderCatalog::default();
        book.enlist(OrderBlueprint::new("set").help_line("set <name> <value>"));
        enlist(&mut book);
        let want = Some("set <name> <value>");
        assert_eq!(book.look_up("set").map(|spec| spec.help_line.as_str()), want);
        let want = 1;
        assert_eq!(book
            .all_names()
            .iter()
            .filter(|name| *name == "set")
            .count(), want);
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
        let command = TypedOrder::parse("x 12 -3").expect("a command line");
        assert_eq!(arg::<usize>(&command), Some(12));
        assert_eq!(arg::<isize>(&command), Some(12));
        let negative = TypedOrder::parse("x -3").expect("a command line");
        assert_eq!(arg::<usize>(&negative), None);
        assert_eq!(arg::<isize>(&negative), Some(-3));
        let bare = TypedOrder::parse("x").expect("a command line");
        assert_eq!(arg::<usize>(&bare), None);
    }

    #[test]
    fn set_stores_the_words_it_is_given() {
        let mut app = menu_app(&[("base", 3)]);
        drive(&mut app, "set ui_note one two three");
        assert_eq!(dvar(&app, "ui_note"), "one two three");
        drive(&mut app, "seta ui_note four");
        assert_eq!(dvar(&app, "ui_note"), "four");
    }

    #[test]
    fn set_with_only_a_name_says_the_value() {
        let mut app = menu_app(&[("base", 3)]);
        drive(&mut app, "set ui_note hello");
        drive(&mut app, "set ui_note");
        drive(&mut app, "set ui_never_set");
        assert_eq!(said(&app), ["ui_note = hello", "ui_never_set = "]);
    }

    #[test]
    fn set_with_nothing_does_nothing() {
        let mut app = menu_app(&[("base", 3)]);
        drive(&mut app, "set");
        assert!(said(&app).is_empty());
    }

    #[test]
    fn setfromdvar_copies_a_dvar_that_exists() {
        let mut app = menu_app(&[("base", 3)]);
        drive(&mut app, "set ui_from copied");
        drive(&mut app, "setfromdvar ui_to ui_from");
        assert_eq!(dvar(&app, "ui_to"), "copied");
        drive(&mut app, "setfromdvar ui_other ui_missing");
        assert_eq!(dvar(&app, "ui_other"), "");
        assert!(said(&app).is_empty());
    }

    #[test]
    fn the_menu_starts_on_the_first_map_and_the_default_mode() {
        let mut app = menu_app(&[("base", 3)]);
        app.update();
        assert_eq!(dvar(&app, "ui_mapname"), "t6:mp_base_0");
        assert!(HostChosenMode::from_short_code(&dvar(&app, "ui_gametype")).is_some());
    }

    #[test]
    fn a_map_or_mode_already_chosen_is_not_replaced() {
        let mut app = menu_app(&[("base", 3)]);
        drive(&mut app, "set ui_mapname t6:mp_base_2");
        drive(&mut app, "set ui_gametype tdm");
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
        drive(&mut app, "ui_start_match");
        let want = "Only the lobby host can start a match";
        assert_eq!(dvar(&app, "ui_frontend_status"), want);
        assert_eq!(said(&app), ["menu: Only the lobby host can start a match"]);
    }

    #[test]
    fn a_command_of_another_part_of_the_console_is_ignored() {
        let mut app = menu_app(&[("base", 3)]);
        drive(&mut app, "echo hello");
        assert!(said(&app).is_empty());
        assert_eq!(dvar(&app, "ui_frontend_status"), "");
    }

    #[test]
    fn every_dvar_the_menu_draws_from_is_there_after_one_frame() {
        let mut app = menu_app(&[("base", 3)]);
        app.update();
        for key in [
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
                app.world().resource::<UiMenuDvars>().get(key).is_some(),
                "{key} is not published"
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
        drive(&mut app, "ui_create_lobby");
        drive(&mut app, "ui_start_match");
        assert!(app.world().contains_resource::<frame::HostMatchRules>());
        returned(&mut app, true);
        assert!(!app.world().contains_resource::<frame::HostMatchRules>());
        assert!(!app.world().contains_resource::<HostChosenMode>());
        let party = app.world().resource::<UiPartyState>();
        assert!(!party.in_party && !party.in_lobby && !party.is_host);
        assert_eq!(dvar(&app, "ui_lobby_host"), "0");
    }

    #[test]
    fn coming_back_to_the_menu_keeps_a_local_lobby() {
        let mut app = menu_app(&[("base", 3)]);
        drive(&mut app, "ui_create_lobby");
        returned(&mut app, false);
        let party = app.world().resource::<UiPartyState>();
        assert!(party.in_lobby && party.is_host);
        assert_eq!(dvar(&app, "ui_lobby_host"), "1");
    }
}
