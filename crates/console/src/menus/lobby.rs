//! The party the menu shows: a private lobby on this machine, or a public one
//! the master hosts, and the host's buttons for it.

use asset_game::TranslationStringIndex;
use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use frame::UiPartyState;
use net::{
    DirectoryLink, DirectoryLinkStatus, DirectoryBrowser, DirectoryBrowserView, DirectoryLaunchRequest,
    DirectoryMenuChoice,
};
use sim::HostChosenMode;
use ui::menu_flow::{
    maps::{map_label, map_preview},
    rules::{host_rules, selected_game},
};

use super::Menu;
use super::flag;
use crate::TypedOrder;
use net::QueuedMenuChoice;
use std::string::String;

const SETUP_REFUSAL: &str = "Only the lobby host can change game setup";
const LOBBY_ROWS: usize = 18;

/// The master's side of the menu: where requests go and what it answers.
#[derive(SystemParam)]
pub(crate) struct Master<'w> {
    launch_ask: Option<Res<'w, DirectoryLaunchRequest>>,
    dir_view: Option<Res<'w, DirectoryBrowser>>,
    dir_link: Option<Res<'w, DirectoryLink>>,
    action: Option<ResMut<'w, QueuedMenuChoice>>,
}

/// Why a request to the master cannot be queued now, if it cannot. `pending` is
/// whether a request is waiting, or `None` when nothing takes requests.
fn check(configured: bool, pending: Option<bool>) -> Result<(), String> {
    if !configured {
        return Err("Public lobbies require a configured master".into());
    }
    match &pending {
        None => Err("Lobby service is unavailable".into()),
        Some(true) => Err("A lobby request is already pending".into()),
        Some(false) => Ok(()),
    }
}

impl Master<'_> {
    pub(super) fn submit(&mut self, action: DirectoryMenuChoice) -> Result<(), String> {
        let configured = self
            .launch_ask
            .as_ref()
            .is_some_and(|intent| intent.configured());
        let pending = self.action.as_ref().map(|pending| pending.0.is_some());
        check(configured, pending)?;
        if let Some(pending) = self.action.as_mut() {
            pending.0 = Some(action);
        }
        Ok(())
    }

    fn bridge_state(&self) -> Option<DirectoryLinkStatus> {
        self.dir_link.as_ref().map(|bridge| bridge.state())
    }

    pub(super) fn browser(&self) -> Option<DirectoryBrowserView> {
        self.dir_view.as_ref().map(|browser| browser.snapshot())
    }
}

/// What the master says about the public lobby we are in.
enum Standing {
    /// No answer yet, or none to give.
    Alone,
    Members {
        names: Vec<String>,
        max: u8,
        map: String,
        game_mode: String,
    },
    Ended(String),
}

/// What to tell the player when the master's lobby failed or was closed.
fn ended_text(link: &DirectoryLinkStatus) -> Option<String> {
    match link {
        DirectoryLinkStatus::Failed { error, .. } => Some(format!("{error:?}")),
        DirectoryLinkStatus::Closed { reason, .. } => Some(format!("Lobby closed: {reason:?}")),
        _ => None,
    }
}

fn standing(link: &DirectoryLinkStatus) -> Standing {
    match link {
        DirectoryLinkStatus::Hosting {
            map,
            mode,
            members,
            member_names,
            seat_capacity,
            ..
        }
        | DirectoryLinkStatus::Joined {
            map,
            mode,
            members,
            member_names,
            seat_capacity,
            ..
        } => Standing::Members {
            names: members
                .iter()
                .map(|id| {
                    member_names
                        .get(id)
                        .cloned()
                        .unwrap_or_else(|| "Player".into())
                })
                .collect::<Vec<String>>(),
            max: *seat_capacity,
            map: String::clone(map),
            game_mode: String::clone(mode),
        },
        other => ended_text(other).map_or(Standing::Alone, Standing::Ended),
    }
}

fn players_text(count: usize, max: impl std::fmt::Display) -> String {
    format!("PLAYERS: {count} / {max}")
}

/// A name and a "shown" flag for each seat of the lobby list.
fn member_rows(names: &[String]) -> Vec<(String, String)> {
    let mut cells = Vec::with_capacity(LOBBY_ROWS * 2);
    for row in 0..LOBBY_ROWS {
        let name = names.get(row);
        cells.push((
            format!("ui_lobby_member_{row}"),
            name.map_or_else(String::new, Clone::clone),
        ));
        cells.push((
            format!("ui_lobby_member_{row}_visible"),
            flag(name.is_some()).to_owned(),
        ));
    }
    cells
}

fn privacy_label(public: bool, localize: Option<&TranslationStringIndex>) -> &str {
    let (key, plain) = if public {
        ("MPUI_LOBBY", "PUBLIC LOBBY")
    } else {
        ("MPUI_PRIVATE_MATCH_LOBBY", "PRIVATE LOBBY")
    };
    localize
        .and_then(|catalog| catalog.text(key))
        .unwrap_or(plain)
}

impl Menu<'_, '_> {
    fn host_only(&self, refusal: &str) -> Result<(), String> {
        if self.party.in_lobby && self.party.is_host {
            Ok(())
        } else {
            Err(refusal.into())
        }
    }

    pub(super) fn selected(&self) -> Result<(String, HostChosenMode), String> {
        selected_game(&self.dvars, &self.arenas)
    }

    pub(super) fn create_lobby(&mut self) -> Result<(), String> {
        self.selected()?;
        *self.party = UiPartyState {
            in_party: true,
            in_lobby: true,
            is_host: true,
        };
        self.state.listed = false;
        self.dvars.set("ui_frontend_status", "");
        Ok(())
    }

    pub(super) fn leave_lobby(&mut self) -> Result<(), String> {
        if self.state.listed {
            self.master.submit(DirectoryMenuChoice::ExitRoom)?;
        }
        *self.party = UiPartyState::default();
        self.state.listed = false;
        Ok(())
    }

    /// Joining a public lobby: the master has been asked, the answer will come.
    pub(super) fn enter(&mut self, join: DirectoryMenuChoice) -> Result<(), String> {
        self.master.submit(join)?;
        self.state.listed = true;
        *self.party = UiPartyState {
            in_party: true,
            in_lobby: true,
            is_host: false,
        };
        self.dvars.set("ui_frontend_status", "Joining lobby...");
        Ok(())
    }

    pub(super) fn toggle_privacy(&mut self) -> Result<(), String> {
        self.host_only("Only the host can change lobby privacy")?;
        if self.state.listed {
            self.master.submit(DirectoryMenuChoice::ExitRoom)?;
        } else {
            let (map, mode) = self.selected()?;
            self.master.submit(DirectoryMenuChoice::Host {
                map,
                mode: mode.token().into(),
            })?;
        }
        self.state.listed = !self.state.listed;
        Ok(())
    }

    pub(super) fn start_match(&mut self) -> Result<(), String> {
        self.host_only("Only the lobby host can start a match")?;
        let (map, mode) = self.selected()?;
        self.commands.insert_resource(host_rules(&self.dvars));
        if self.state.listed {
            return self.master.submit(DirectoryMenuChoice::BeginGame {
                map,
                mode: mode.token().into(),
            });
        }
        let id = self.swaps.ask_for_region(map.clone())?;
        self.commands.insert_resource(mode);
        self.echo.write(format!(
            "menu: starting {map} {} (swap #{id})",
            mode.token()
        ));
        Ok(())
    }

    pub(super) fn vote_skip(&mut self) -> Result<(), String> {
        self.master.submit(DirectoryMenuChoice::BallotForSkip)
    }

    pub(super) fn select_map(&mut self, command: &TypedOrder) -> Result<(), String> {
        self.host_only(SETUP_REFUSAL)?;
        let (_, mode) = self.selected()?;
        let map = self.map_on_row(command)?;
        self.choose(map, mode)
    }

    pub(super) fn select_mode(&mut self, command: &TypedOrder) -> Result<(), String> {
        self.host_only(SETUP_REFUSAL)?;
        let (map, _) = self.selected()?;
        let mode = command
            .words
            .first()
            .and_then(|token| HostChosenMode::from_short_code(token))
            .ok_or("Unsupported game mode")?;
        self.choose(map, mode)
    }

    fn choose(&mut self, arena: String, mode: HostChosenMode) -> Result<(), String> {
        if self.state.listed {
            self.master.submit(DirectoryMenuChoice::RefreshRoom {
                map: arena.to_string(),
                mode: mode.token().into(),
            })?;
        }
        self.dvars.set("ui_mapname", arena);
        self.dvars.set("ui_gametype", mode.token());
        Ok(())
    }

    /// What the menu says when a public lobby ends under it.
    pub(super) fn lobby_ended_text(&self) -> String {
        self.master
            .bridge_state()
            .as_ref()
            .and_then(ended_text)
            .unwrap_or_else(|| "Public lobby ended".into())
    }

    pub(super) fn publish_lobby(&mut self, own_name: &str, localize: Option<&TranslationStringIndex>) {
        let standing = self
            .master
            .bridge_state()
            .filter(|_| self.state.listed)
            .map_or(Standing::Alone, |state| standing(&state));
        let (names, players) = match standing {
            Standing::Members {
                names,
                max,
                map,
                game_mode: mode,
            } => {
                if self.dvars.get("ui_frontend_status") == Some("Joining lobby...") {
                    self.dvars.set("ui_frontend_status", "");
                }
                self.dvars.set("ui_mapname", map);
                self.dvars.set("ui_gametype", mode);
                let players = players_text(names.len(), max);
                (names, players)
            }
            Standing::Ended(reason) => {
                self.dvars.set("ui_frontend_status", reason);
                (vec![own_name.to_owned()], players_text(1, LOBBY_ROWS))
            }
            Standing::Alone => (vec![own_name.to_owned()], players_text(1, LOBBY_ROWS)),
        };
        self.dvars.set("ui_lobby_players", players);
        for (name, value) in member_rows(&names) {
            self.dvars.set(&name, value);
        }
        self.dvars.set("ui_lobby_host", flag(self.party.is_host));
        self.dvars.set("ui_lobby_public", flag(self.state.listed));
        self.dvars.set(
            "ui_lobby_privacy",
            privacy_label(self.state.listed, localize),
        );
        let map = self.dvars.get("ui_mapname").unwrap_or("").to_owned();
        self.dvars.set("ui_map_label", map_label(&map));
        self.dvars.set("ui_lobby_preview", map_preview(&map));
        let blank = "";
        let mode = self
            .dvars
            .get("ui_gametype")
            .and_then(HostChosenMode::from_short_code)
            .map_or(blank, |mode| mode.shown_label());
        self.dvars.set("ui_mode_label", mode);
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use frame::HostMatchRules;
    use frame::ReturnedToMenu;
    use net::{LobbySessionLabel, LinkFailureRecord};

    use crate::menus::lobby::*;
    use crate::menus::testing::{dvar, menu_app, drive, said};

    fn identity() -> LobbySessionLabel {
        LobbySessionLabel::unassigned(1)
    }

    /// A lobby this machine hosts, one member per name. An empty name is a
    /// member the master has not named.
    fn hosting(names: &[&str], seat_capacity: u8) -> DirectoryLinkStatus {
        let mut members = Vec::new();
        let mut member_names = HashMap::new();
        for (slot, who) in names.iter().enumerate() {
            let mut id = identity().member_id;
            id.0[0] = u8::try_from(slot).expect("a few members") + 1;
            members.push(id);
            if !who.is_empty() {
                member_names.insert(id, (*who).to_owned());
            }
        }
        DirectoryLinkStatus::Hosting {
            identity: identity(),
            name: "room".into(),
            map: "t6:mp_base_1".into(),
            mode: "tdm".into(),
            members,
            member_names,
            seat_capacity,
            pass_vote_count: 0,
            in_match: bool::default(),
        }
    }

    fn joined(state: DirectoryLinkStatus) -> DirectoryLinkStatus {
        let DirectoryLinkStatus::Hosting {
            identity,
            name,
            map,
            mode,
            members,
            member_names,
            seat_capacity,
            pass_vote_count,
            in_match,
        } = state
        else {
            panic!("a hosted lobby to join");
        };
        DirectoryLinkStatus::Joined {
            identity,
            name,
            map,
            mode,
            members,
            member_names,
            seat_capacity,
            pass_vote_count,
            playing: in_match,
        }
    }

    #[test]
    fn a_request_needs_a_master_a_service_and_a_free_slot() {
        let unconfigured = Err("Public lobbies require a configured master".to_owned());
        let free = Some(false);
        assert_eq!(check(false, free), unconfigured);
        assert_eq!(check(false, None), unconfigured);
        let expected = Err("Lobby service is unavailable".into());
        assert_eq!(check(true, None), expected);
        let expected = Err("A lobby request is already pending".into());
        assert_eq!(check(true, Some(true)), expected);
        assert_eq!(check(true, free), Ok(()));
    }

    #[test]
    fn a_hosted_lobby_lists_its_members_by_name() {
        let Standing::Members {
            names,
            max,
            map,
            game_mode: mode,
        } = standing(&hosting(&["Sam", "", "Ann"], 12))
        else {
            panic!("a hosted lobby has members");
        };
        assert_eq!(names, ["Sam", "Player", "Ann"]);
        let expected = (12, "t6:mp_base_1", "tdm");
        assert_eq!((max, map.as_str(), mode.as_str()), expected);
    }

    #[test]
    fn a_joined_lobby_reads_the_same_as_a_hosted_one() {
        let state = joined(hosting(&["Sam"], 4));
        assert!(matches!(
            standing(&state),
            Standing::Members { names, max: 4, .. } if names == ["Sam"]
        ));
    }

    #[test]
    fn a_lobby_that_has_not_answered_has_no_one_in_it() {
        for state in [
            DirectoryLinkStatus::Connecting {
                identity: identity(),
            },
            DirectoryLinkStatus::Joining {
                identity: identity(),
            },
            DirectoryLinkStatus::Left {
                identity: identity(),
            },
        ] {
            assert!(matches!(standing(&state), Standing::Alone));
            assert_eq!(ended_text(&state), None);
        }
    }

    #[test]
    fn a_failed_lobby_says_why() {
        let failure = LinkFailureRecord::new("gameplay", "local", "lost the host");
        let state = DirectoryLinkStatus::Failed {
            identity: identity(),
            error: failure.clone(),
        };
        let text = ended_text(&state).expect("a reason");
        assert_eq!(text, format!("{failure:?}"));
        assert!(matches!(standing(&state), Standing::Ended(shown) if shown == text));
    }

    #[test]
    fn the_member_list_has_a_row_for_every_seat() {
        let rows = member_rows(&["Sam".to_owned(), "Ann".to_owned()]);
        assert_eq!(rows.len(), LOBBY_ROWS * 2);
        let get = |name: &str| {
            rows.as_slice()
                .iter()
                .find(|(row, _)| row == name)
                .map(|(_, value)| value.as_str())
        };
        assert_eq!(get("ui_lobby_member_1"), Some("Ann"));
        assert_eq!(get("ui_lobby_member_1_visible"), Some("1"));
        assert_eq!(get("ui_lobby_member_2"), Some(""));
        assert_eq!(get("ui_lobby_member_2_visible"), Some("0"));
        assert_eq!(get("ui_lobby_member_17_visible"), Some("0"));
        assert_eq!(get("ui_lobby_member_18"), None);
    }

    #[test]
    fn the_player_count_reads_taken_of_possible() {
        assert_eq!(players_text(3, 12), "PLAYERS: 3 / 12");
        assert_eq!(players_text(1, LOBBY_ROWS), "PLAYERS: 1 / 18");
    }

    #[test]
    fn the_privacy_label_falls_back_to_plain_words() {
        assert_eq!(privacy_label(true, None), "PUBLIC LOBBY");
        assert_eq!(privacy_label(false, None), "PRIVATE LOBBY");
        let without_text = TranslationStringIndex::default();
        assert_eq!(privacy_label(true, Some(&without_text)), "PUBLIC LOBBY");
    }

    fn lobby_app() -> App {
        menu_app(&[("base", 3)])
    }

    /// Whether the party is active, in a lobby, and hosting it.
    fn party(app: &App) -> (bool, bool, bool) {
        let party = app.world().resource::<UiPartyState>();
        (party.in_party, party.in_lobby, party.is_host)
    }

    #[test]
    fn creating_a_lobby_makes_this_player_its_host() {
        let mut app = lobby_app();
        let world = app.world_mut();
        world
            .resource_mut::<frame::PlayerPrefsBundle>()
            .shown_name = "Sam".into();
        drive(&mut app, "ui_create_lobby");
        assert_eq!(party(&app), (true, true, true));
        assert_eq!(dvar(&app, "ui_lobby_host"), "1");
        assert_eq!(dvar(&app, "ui_lobby_public"), "0");
        assert_eq!(dvar(&app, "ui_lobby_member_0"), "Sam");
        assert_eq!(dvar(&app, "ui_lobby_member_0_visible"), "1");
        assert_eq!(dvar(&app, "ui_lobby_member_1_visible"), "0");
        assert_eq!(dvar(&app, "ui_lobby_players"), "PLAYERS: 1 / 18");
        assert_eq!(dvar(&app, "ui_lobby_privacy"), "PRIVATE LOBBY");
    }

    #[test]
    fn a_lobby_needs_a_map_that_is_installed() {
        let mut app = menu_app(&[]);
        drive(&mut app, "ui_create_lobby");
        assert_ne!(dvar(&app, "ui_frontend_status"), "");
        assert!(!party(&app).1);

        let mut app = lobby_app();
        drive(&mut app, "set ui_mapname t6:mp_gone");
        drive(&mut app, "ui_create_lobby");
        assert_ne!(dvar(&app, "ui_frontend_status"), "");
        assert!(!party(&app).1);
    }

    #[test]
    fn creating_a_lobby_clears_the_old_status() {
        let mut app = lobby_app();
        drive(&mut app, "ui_start_match");
        assert_ne!(dvar(&app, "ui_frontend_status"), "");
        drive(&mut app, "ui_create_lobby");
        assert_eq!(dvar(&app, "ui_frontend_status"), "");
    }

    #[test]
    fn only_the_host_can_set_up_or_start() {
        let mut app = lobby_app();
        for (line, refusal) in [
            ("ui_start_match", "Only the lobby host can start a match"),
            (
                "ui_select_map 1",
                "Only the lobby host can change game setup",
            ),
            (
                "ui_select_mode x",
                "Only the lobby host can change game setup",
            ),
            ("ui_lobby_privacy", "Only the host can change lobby privacy"),
        ] {
            drive(&mut app, line);
            assert_eq!(dvar(&app, "ui_frontend_status"), refusal, "{line}");
            assert!(said(&app).contains(&format!("menu: {refusal}")), "{line}");
        }
    }

    #[test]
    fn leaving_empties_the_party() {
        let mut app = lobby_app();
        drive(&mut app, "ui_create_lobby");
        drive(&mut app, "ui_leave_lobby");
        assert_eq!(party(&app), (false, false, false));
        assert_eq!(dvar(&app, "ui_lobby_host"), "0");
    }

    #[test]
    fn the_host_picks_the_map_from_the_page_shown() {
        let mut app = lobby_app();
        drive(&mut app, "ui_create_lobby");
        drive(&mut app, "ui_select_map 2");
        assert_eq!(dvar(&app, "ui_mapname"), "t6:mp_base_2");
        assert_eq!(dvar(&app, "ui_map_label"), map_label("t6:mp_base_2"));
        assert_eq!(dvar(&app, "ui_lobby_preview"), map_preview("t6:mp_base_2"));
        drive(&mut app, "ui_select_map 5");
        assert_eq!(dvar(&app, "ui_frontend_status"), "Map is unavailable");
        drive(&mut app, "ui_select_map 10");
        assert_eq!(dvar(&app, "ui_frontend_status"), "Invalid map row");
        drive(&mut app, "ui_select_map");
        assert_eq!(dvar(&app, "ui_frontend_status"), "Invalid map row");
        assert_eq!(dvar(&app, "ui_mapname"), "t6:mp_base_2");
    }

    #[test]
    fn the_host_picks_the_mode_by_its_token() {
        let mut app = lobby_app();
        let token = HostChosenMode::read_env().token();
        drive(&mut app, "ui_create_lobby");
        drive(&mut app, &format!("ui_select_mode {token}"));
        assert_eq!(dvar(&app, "ui_gametype"), token);
        assert_ne!(dvar(&app, "ui_mode_label"), "");
        drive(&mut app, "ui_select_mode nonsense");
        assert_eq!(dvar(&app, "ui_frontend_status"), "Unsupported game mode");
        assert_eq!(dvar(&app, "ui_gametype"), token);
    }

    #[test]
    fn starting_a_private_match_asks_for_the_map_and_keeps_the_rules() {
        let mut app = lobby_app();
        drive(&mut app, "ui_create_lobby");
        drive(&mut app, "ui_start_match");
        assert!(app.world().get_resource::<HostMatchRules>().is_some());
        assert!(app.world().contains_resource::<HostChosenMode>());
        assert!(
            said(&app)
                .iter()
                .any(|line| line.starts_with("menu: starting t6:mp_base_0 ")),
            "{:?}",
            said(&app)
        );
    }

    #[test]
    fn going_public_without_a_master_is_refused_and_stays_private() {
        let mut app = lobby_app();
        drive(&mut app, "ui_create_lobby");
        drive(&mut app, "ui_lobby_privacy");
        let expected = "Public lobbies require a configured master";
        assert_eq!(dvar(&app, "ui_frontend_status"), expected);
        assert_eq!(dvar(&app, "ui_lobby_public"), "0");
        drive(&mut app, "ui_vote_skip");
        assert!(
            said(&app)
                .iter()
                .any(|line| line == "menu: Public lobbies require a configured master")
        );
    }

    #[test]
    fn coming_back_from_a_match_starts_a_fresh_menu() {
        let mut app = lobby_app();
        drive(&mut app, "ui_create_lobby");
        drive(&mut app, "ui_start_match");
        app.world_mut().write_message(ReturnedToMenu {
            swap_id: 1,
            had_world: true,
        });
        app.update();
        assert!(app.world().get_resource::<HostMatchRules>().is_none());
        assert!(!app.world().contains_resource::<HostChosenMode>());
        assert_eq!(party(&app), (false, false, false));
        assert_eq!(dvar(&app, "ui_lobby_host"), "0");
    }
}
