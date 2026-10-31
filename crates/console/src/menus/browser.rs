//! The list of public lobbies the master knows of, shown ten to a page, and the
//! ways into one of them.

use net::{DirectoryListing, DirectoryMenuChoice};

use super::Menu;
use super::arg;
use super::maps::PAGE_SIZE;
use crate::TypedOrder;

/// The page of lobbies shown, and the lobbies on it.
#[derive(Default)]
pub(super) struct ServerList {
    page: usize,
    shown: Vec<DirectoryListing>,
}

/// The last page that has anything on it; an empty list still has a first page.
fn last_page(total: usize) -> usize {
    total.saturating_sub(1) / PAGE_SIZE
}

/// A lobby is worth a click when it has room, is not locked or mid-match, and
/// everything it needs is installed here.
fn open_to_join(advert: &DirectoryListing) -> bool {
    !(advert.locked
        || advert.underway
        || advert.players >= advert.seat_capacity
        || !advert.missing.is_empty())
}

fn label(listing: &DirectoryListing) -> String {
    format!(
        "{}  {}  {}/{}",
        listing.name, listing.map, listing.players, listing.seat_capacity
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
    fn show(&mut self, adverts: Vec<DirectoryListing>) {
        self.page = self.page.min(last_page(adverts.len()));
        self.shown = IntoIterator::into_iter(adverts)
            .skip(self.page * PAGE_SIZE)
            .take(PAGE_SIZE)
            .collect::<Vec<DirectoryListing>>();
    }

    /// The lobby on a row of the page shown.
    fn entry_at(&self, line: usize) -> Result<&DirectoryListing, String> {
        self.shown
            .get(line)
            .ok_or_else(|| "Lobby is no longer available".into())
    }
}

impl Menu<'_, '_> {
    pub(super) fn refresh_browser(&mut self) -> Result<(), String> {
        self.master.submit(DirectoryMenuChoice::Refresh)?;
        self.state.servers.rewind();
        self.dvars.set("ui_browser_status", "Refreshing lobbies...");
        Ok(())
    }

    pub(super) fn turn_browser_page(&mut self, command: &TypedOrder) {
        let total = self
            .master
            .browser()
            .map_or(0, |snapshot| snapshot.adverts.len());
        self.state.servers.turn(arg(command).unwrap_or(0), total);
    }

    pub(super) fn join_row(&mut self, command: &TypedOrder) -> Result<(), String> {
        let row = arg(command).ok_or("Invalid lobby row")?;
        let advert = self.state.servers.entry_at(row)?;
        if !open_to_join(advert) {
            return Err("Lobby is unavailable or requires missing content".into());
        }
        let join = DirectoryMenuChoice::Join {
            listing_token: advert.id,
            map: String::clone(&advert.map),
            mode: String::clone(&advert.mode),
        };
        self.enter(join)
    }

    /// Joins a lobby by the id its host reads out. The id is hex, and only
    /// ASCII can be.
    pub(super) fn join_id(&mut self, command: &TypedOrder) -> Result<(), String> {
        let text = command
            .words
            .first()
            .filter(|id| id.is_ascii())
            .ok_or("usage: ui_join_lobby_id <hex ID>")?;
        let listing_token = text.parse().map_err(|_| "Invalid lobby ID")?;
        let (map, mode) = self.selected()?;
        self.enter(DirectoryMenuChoice::Join {
            listing_token,
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
    use crate::menus::browser::*;
    use crate::menus::testing::{dvar, menu_app, drive};

    fn advert(name: &str, players: u8, seat_capacity: u8) -> DirectoryListing {
        let (locked, in_match) = (false, false);
        DirectoryListing {
            id: Default::default(),
            name: String::from(name),
            map: "t6:mp_base_1".to_owned(),
            mode: "tdm".to_owned(),
            players,
            seat_capacity,
            locked,
            underway: in_match,
            requires: Default::default(),
            missing: Default::default(),
        }
    }

    fn adverts(rooms: usize) -> Vec<DirectoryListing> {
        (0..rooms)
            .map(|n| advert(&format!("room {n}"), 1, 8))
            .collect::<Vec<DirectoryListing>>()
    }

    #[test]
    fn an_empty_list_still_has_a_first_page() {
        let pages: Vec<usize> = [0, 1, 10, 11, 20, 21, 25]
            .iter()
            .copied()
            .map(last_page)
            .collect::<Vec<usize>>();
        assert_eq!(pages, [0, 0, 0, 1, 1, 2, 2]);
    }

    #[test]
    fn a_lobby_with_room_and_nothing_missing_can_be_joined() {
        assert!(open_to_join(&advert("a", 3, 8)));
    }

    #[test]
    fn a_full_locked_or_started_lobby_cannot_be_joined() {
        assert!(!open_to_join(&advert("full", 8, 8)));
        let mut locked = advert("locked", 1, 8);
        locked.locked = true;
        assert!(!open_to_join(&locked));
        let mut started = advert("started", 1, 8);
        started.underway = true;
        assert!(!open_to_join(&started));
    }

    #[test]
    fn a_lobby_that_needs_content_not_installed_cannot_be_joined() {
        let mut needs = advert("needs", 1, 8);
        let packs = vec![net::ContentPack { bit: 1 << 3, prefix: "t6", label: "t6" }];
        needs.missing = net::ContentCatalog::new(packs, None)
            .mask_for_map("t6:mp_base_1")
            .expect("known content");
        assert!(!open_to_join(&needs));
    }

    #[test]
    fn a_row_reads_name_map_and_seats() {
        assert_eq!(label(&advert("room", 3, 12)), "room  t6:mp_base_1  3/12");
    }

    #[test]
    fn the_status_line_follows_the_master() {
        let expected = Some("Could not refresh lobbies: timed out".into());
        assert_eq!(status(Some("timed out"), true, true), expected);
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
            .collect::<Vec<&str>>();
        let expected = ["room 20", "room 21", "room 22", "room 23", "room 24"];
        assert_eq!(names, expected);
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
        assert_eq!(list.entry_at(1).map(|advert| advert.name.as_str()), Ok("room 1"));
        let expected = Err("Lobby is no longer available".into());
        assert_eq!(list.entry_at(2).map(|_| ()), expected);
    }

    #[test]
    fn without_a_master_a_refresh_is_refused_on_the_browser_line() {
        let mut app = menu_app(&[("base", 3)]);
        drive(&mut app, "ui_browser_refresh");
        let expected = "Public lobbies require a configured master";
        assert_eq!(dvar(&app, "ui_browser_status"), expected);
    }

    #[test]
    fn turning_the_page_with_no_master_is_harmless() {
        let mut app = menu_app(&[("base", 3)]);
        drive(&mut app, "ui_browser_page 1");
        drive(&mut app, "ui_browser_page nonsense");
        assert_eq!(dvar(&app, "ui_browser_0"), "");
        assert_eq!(dvar(&app, "ui_browser_9"), "");
    }

    #[test]
    fn a_row_with_no_lobby_on_it_is_refused() {
        let mut app = menu_app(&[("base", 3)]);
        drive(&mut app, "ui_join_lobby 0");
        let expected = "Lobby is no longer available";
        assert_eq!(dvar(&app, "ui_browser_status"), expected);
        drive(&mut app, "ui_join_lobby x");
        assert_eq!(dvar(&app, "ui_browser_status"), "Invalid lobby row");
        drive(&mut app, "ui_join_lobby");
        assert_eq!(dvar(&app, "ui_browser_status"), "Invalid lobby row");
    }

    #[test]
    fn joining_by_id_wants_a_hex_id() {
        let mut app = menu_app(&[("base", 3)]);
        drive(&mut app, "ui_join_lobby_id");
        let expected = "usage: ui_join_lobby_id <hex ID>";
        assert_eq!(dvar(&app, "ui_frontend_status"), expected);
        drive(&mut app, &format!("ui_join_lobby_id {}", "é".repeat(16)));
        let expected = "usage: ui_join_lobby_id <hex ID>";
        assert_eq!(dvar(&app, "ui_frontend_status"), expected);
        drive(&mut app, "ui_join_lobby_id abc");
        assert_eq!(dvar(&app, "ui_frontend_status"), "Invalid lobby ID");
        drive(&mut app, &format!("ui_join_lobby_id {}", "zz".repeat(16)));
        assert_eq!(dvar(&app, "ui_frontend_status"), "Invalid lobby ID");
    }

    #[test]
    fn a_good_id_goes_as_far_as_the_master() {
        let mut app = menu_app(&[("base", 3)]);
        drive(&mut app, &format!("ui_join_lobby_id {}", "ab".repeat(16)));
        let expected = "Public lobbies require a configured master";
        assert_eq!(dvar(&app, "ui_frontend_status"), expected);
    }
}
