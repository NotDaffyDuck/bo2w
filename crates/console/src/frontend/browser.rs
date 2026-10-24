//! The list of public lobbies the master knows of, shown ten to a page, and the
//! ways into one of them.

use net::{MasterAdvert, MasterMenuAction};

use super::{Menu, arg, maps::PAGE_SIZE};
use crate::ConsoleCommand;

/// The page of lobbies shown, and the lobbies on it.
#[derive(Default)]
pub(super) struct ServerList {
    page: usize,
    shown: Vec<MasterAdvert>,
}

/// The last page that has anything on it; an empty list still has a first page.
fn last_page(total: usize) -> usize {
    total.saturating_sub(1) / PAGE_SIZE
}

/// A lobby is worth a click when it has room, is not locked or mid-match, and
/// everything it needs is installed here.
fn joinable(advert: &MasterAdvert) -> bool {
    !(advert.locked
        || advert.in_match
        || advert.players >= advert.max_players
        || !advert.missing.is_empty())
}

fn label(advert: &MasterAdvert) -> String {
    format!(
        "{}  {}  {}/{}",
        advert.name, advert.map, advert.players, advert.max_players
    )
}

/// What the status line says about the list. `None` leaves the line as it is,
/// which is right while a refresh is on its way.
fn status(error: Option<&str>, loading: bool, any: bool) -> Option<String> {
    match (error, loading, any) {
        (Some(error), ..) => Some(format!("Could not refresh lobbies: {error}")),
        (None, true, _) => None,
        (None, false, true) => Some(String::new()),
        (None, false, false) => Some("No lobbies found".into()),
    }
}

impl ServerList {
    fn rewind(&mut self) {
        self.page = 0;
    }

    /// Moves by `delta` pages, staying among the pages that exist for `total`
    /// lobbies.
    fn turn(&mut self, delta: isize, total: usize) {
        self.page = self.page.saturating_add_signed(delta).min(last_page(total));
    }

    /// Takes the page of `adverts` the list is on, or the last one if the list
    /// has shrunk past it.
    fn show(&mut self, adverts: Vec<MasterAdvert>) {
        self.page = self.page.min(last_page(adverts.len()));
        self.shown = adverts
            .into_iter()
            .skip(self.page * PAGE_SIZE)
            .take(PAGE_SIZE)
            .collect();
    }

    /// The lobby on a row of the page shown.
    fn row(&self, row: usize) -> Result<&MasterAdvert, String> {
        self.shown
            .get(row)
            .ok_or_else(|| "Lobby is no longer available".into())
    }
}

impl Menu<'_, '_> {
    pub(super) fn refresh_browser(&mut self) -> Result<(), String> {
        self.master.submit(MasterMenuAction::Refresh)?;
        self.state.servers.rewind();
        self.dvars.set("ui_browser_status", "Refreshing lobbies...");
        Ok(())
    }

    pub(super) fn turn_browser_page(&mut self, command: &ConsoleCommand) {
        let total = self
            .master
            .browser()
            .map_or(0, |snapshot| snapshot.adverts.len());
        self.state.servers.turn(arg(command).unwrap_or(0), total);
    }

    pub(super) fn join_row(&mut self, command: &ConsoleCommand) -> Result<(), String> {
        let row = arg(command).ok_or("Invalid lobby row")?;
        let advert = self.state.servers.row(row)?;
        if !joinable(advert) {
            return Err("Lobby is unavailable or requires missing content".into());
        }
        let join = MasterMenuAction::Join {
            advert_id: advert.id,
            map: advert.map.clone(),
            mode: advert.mode.clone(),
        };
        self.enter(join)
    }

    /// Joins a lobby by the id its host reads out. The id is hex, and only
    /// ASCII can be.
    pub(super) fn join_id(&mut self, command: &ConsoleCommand) -> Result<(), String> {
        let text = command
            .args
            .first()
            .filter(|id| id.is_ascii())
            .ok_or("usage: ui_join_lobby_id <hex ID>")?;
        let advert_id = text.parse().map_err(|_| "Invalid lobby ID")?;
        let (map, mode) = self.selected()?;
        self.enter(MasterMenuAction::Join {
            advert_id,
            map,
            mode: mode.token().into(),
        })
    }

    pub(super) fn publish_browser(&mut self) {
        if let Some(snapshot) = self.master.browser() {
            let (error, loading) = (snapshot.error, snapshot.loading);
            self.state.servers.show(snapshot.adverts);
            let any = !self.state.servers.shown.is_empty();
            if let Some(text) = status(error.as_deref(), loading, any) {
                self.dvars.set("ui_browser_status", text);
            }
        }
        for row in 0..PAGE_SIZE {
            let text = self
                .state
                .servers
                .shown
                .get(row)
                .map_or_else(String::new, label);
            self.dvars.set(&format!("ui_browser_{row}"), text);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frontend::testing::{dvar, menu_app, run};

    fn advert(name: &str, players: u8, max_players: u8) -> MasterAdvert {
        MasterAdvert {
            id: Default::default(),
            name: name.to_owned(),
            map: "t6:mp_base_1".to_owned(),
            mode: "tdm".to_owned(),
            players,
            max_players,
            locked: false,
            in_match: false,
            requires: Default::default(),
            missing: Default::default(),
        }
    }

    fn adverts(count: usize) -> Vec<MasterAdvert> {
        (0..count)
            .map(|n| advert(&format!("room {n}"), 1, 8))
            .collect()
    }

    #[test]
    fn an_empty_list_still_has_a_first_page() {
        let pages: Vec<usize> = [0, 1, 10, 11, 20, 21, 25]
            .into_iter()
            .map(last_page)
            .collect();
        assert_eq!(pages, [0, 0, 0, 1, 1, 2, 2]);
    }

    #[test]
    fn a_lobby_with_room_and_nothing_missing_can_be_joined() {
        assert!(joinable(&advert("a", 3, 8)));
    }

    #[test]
    fn a_full_locked_or_started_lobby_cannot_be_joined() {
        assert!(!joinable(&advert("full", 8, 8)));
        let mut locked = advert("locked", 1, 8);
        locked.locked = true;
        assert!(!joinable(&locked));
        let mut started = advert("started", 1, 8);
        started.in_match = true;
        assert!(!joinable(&started));
    }

    #[test]
    fn a_lobby_that_needs_content_not_installed_cannot_be_joined() {
        let mut needs = advert("needs", 1, 8);
        needs.missing = net::content_required_by_map("t6:mp_base_1").expect("known content");
        assert!(!joinable(&needs));
    }

    #[test]
    fn a_row_reads_name_map_and_seats() {
        assert_eq!(label(&advert("room", 3, 12)), "room  t6:mp_base_1  3/12");
    }

    #[test]
    fn the_status_line_follows_the_master() {
        assert_eq!(
            status(Some("timed out"), true, true),
            Some("Could not refresh lobbies: timed out".into())
        );
        assert_eq!(status(None, true, false), None);
        assert_eq!(status(None, false, true), Some(String::new()));
        assert_eq!(status(None, false, false), Some("No lobbies found".into()));
    }

    #[test]
    fn turning_stays_among_the_pages_that_exist() {
        let mut list = ServerList::default();
        list.turn(1, 25);
        assert_eq!(list.page, 1);
        list.turn(5, 25);
        assert_eq!(list.page, 2);
        list.turn(-1, 25);
        assert_eq!(list.page, 1);
        list.turn(-9, 25);
        assert_eq!(list.page, 0);
        list.turn(1, 0);
        assert_eq!(list.page, 0);
    }

    #[test]
    fn a_refresh_goes_back_to_the_first_page() {
        let mut list = ServerList::default();
        list.turn(2, 25);
        list.rewind();
        assert_eq!(list.page, 0);
    }

    #[test]
    fn the_list_shows_ten_lobbies_of_the_page() {
        let mut list = ServerList::default();
        list.turn(2, 25);
        list.show(adverts(25));
        let names: Vec<&str> = list
            .shown
            .iter()
            .map(|advert| advert.name.as_str())
            .collect();
        assert_eq!(
            names,
            ["room 20", "room 21", "room 22", "room 23", "room 24"]
        );
        list.turn(-2, 25);
        list.show(adverts(25));
        assert_eq!(list.shown.len(), PAGE_SIZE);
        assert_eq!(list.shown[0].name, "room 0");
    }

    #[test]
    fn a_list_that_shrinks_shows_its_last_page() {
        let mut list = ServerList::default();
        list.turn(2, 25);
        list.show(adverts(25));
        list.show(adverts(12));
        assert_eq!(list.page, 1);
        assert_eq!(list.shown.len(), 2);
        list.show(Vec::new());
        assert_eq!(list.page, 0);
        assert!(list.shown.is_empty());
    }

    #[test]
    fn a_row_that_is_not_shown_is_gone() {
        let mut list = ServerList::default();
        list.show(adverts(2));
        assert_eq!(list.row(1).map(|advert| advert.name.as_str()), Ok("room 1"));
        assert_eq!(
            list.row(2).map(|_| ()),
            Err("Lobby is no longer available".into())
        );
    }

    #[test]
    fn without_a_master_a_refresh_is_refused_on_the_browser_line() {
        let mut app = menu_app(&[("base", 3)]);
        run(&mut app, "ui_browser_refresh");
        assert_eq!(
            dvar(&app, "ui_browser_status"),
            "Public lobbies require a configured master"
        );
    }

    #[test]
    fn turning_the_page_with_no_master_is_harmless() {
        let mut app = menu_app(&[("base", 3)]);
        run(&mut app, "ui_browser_page 1");
        run(&mut app, "ui_browser_page nonsense");
        assert_eq!(dvar(&app, "ui_browser_0"), "");
        assert_eq!(dvar(&app, "ui_browser_9"), "");
    }

    #[test]
    fn a_row_with_no_lobby_on_it_is_refused() {
        let mut app = menu_app(&[("base", 3)]);
        run(&mut app, "ui_join_lobby 0");
        assert_eq!(
            dvar(&app, "ui_browser_status"),
            "Lobby is no longer available"
        );
        run(&mut app, "ui_join_lobby x");
        assert_eq!(dvar(&app, "ui_browser_status"), "Invalid lobby row");
        run(&mut app, "ui_join_lobby");
        assert_eq!(dvar(&app, "ui_browser_status"), "Invalid lobby row");
    }

    #[test]
    fn joining_by_id_wants_a_hex_id() {
        let mut app = menu_app(&[("base", 3)]);
        run(&mut app, "ui_join_lobby_id");
        assert_eq!(
            dvar(&app, "ui_frontend_status"),
            "usage: ui_join_lobby_id <hex ID>"
        );
        run(&mut app, &format!("ui_join_lobby_id {}", "é".repeat(16)));
        assert_eq!(
            dvar(&app, "ui_frontend_status"),
            "usage: ui_join_lobby_id <hex ID>"
        );
        run(&mut app, "ui_join_lobby_id abc");
        assert_eq!(dvar(&app, "ui_frontend_status"), "Invalid lobby ID");
        run(&mut app, &format!("ui_join_lobby_id {}", "zz".repeat(16)));
        assert_eq!(dvar(&app, "ui_frontend_status"), "Invalid lobby ID");
    }

    #[test]
    fn a_good_id_goes_as_far_as_the_master() {
        let mut app = menu_app(&[("base", 3)]);
        run(&mut app, &format!("ui_join_lobby_id {}", "ab".repeat(16)));
        assert_eq!(
            dvar(&app, "ui_frontend_status"),
            "Public lobbies require a configured master"
        );
    }
}
