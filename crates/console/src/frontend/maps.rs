//! The map list of the lobby menu: packs of maps shown ten to a page, the page
//! and row the player is looking at, and the labels the menu draws for them.

use ui::{
    MenuMapList,
    frontend::maps::{map_label, map_preview, pack_maps},
};

use super::{Menu, arg};
use crate::ConsoleCommand;

pub(super) const PAGE_SIZE: usize = 10;
const PACK_SLOTS: usize = 16;

/// Which page of which pack the menu shows, and the row the pointer is on.
#[derive(Default)]
pub(super) struct MapView {
    pack: usize,
    page: usize,
    hover: usize,
}

/// One button of the strip above the list: a page of a pack.
struct Slot {
    pack: usize,
    page: usize,
    label: String,
}

/// A pack with more than a page of maps gets a button per page, numbered.
fn slots(maps: &MenuMapList) -> Vec<Slot> {
    let mut slots = Vec::new();
    for (pack, contents) in maps.0.iter().enumerate() {
        let pages = contents.maps.len().div_ceil(PAGE_SIZE);
        for page in 0..pages {
            let label = if pages > 1 {
                format!("{} {}", contents.label, page + 1)
            } else {
                contents.label.clone()
            };
            slots.push(Slot { pack, page, label });
        }
    }
    slots
}

impl MapView {
    /// Opens on the page that holds `map`, pointing at it. A map in no pack
    /// opens the first page.
    fn open_on(&mut self, maps: &MenuMapList, map: &str) {
        self.pack = maps.pack_of(map).unwrap_or(0);
        let row = pack_maps(maps, self.pack)
            .iter()
            .position(|installed| installed == map)
            .unwrap_or(0);
        self.page = row / PAGE_SIZE;
        self.hover = row % PAGE_SIZE;
    }

    fn pick_slot(&mut self, maps: &MenuMapList, slot: usize) -> Result<(), String> {
        let slot = slots(maps)
            .into_iter()
            .nth(slot)
            .ok_or("Invalid map page")?;
        *self = Self {
            pack: slot.pack,
            page: slot.page,
            hover: 0,
        };
        Ok(())
    }

    fn point_at(&mut self, row: usize) -> Result<(), String> {
        if row >= PAGE_SIZE {
            return Err("Invalid map row".into());
        }
        self.hover = row;
        Ok(())
    }

    /// The map on a row of the page shown.
    fn map_at<'a>(&self, maps: &'a MenuMapList, row: usize) -> Option<&'a str> {
        pack_maps(maps, self.pack)
            .get(self.page * PAGE_SIZE + row)
            .map(String::as_str)
    }

    /// The map a click on `row` chooses.
    fn pick<'a>(&self, maps: &'a MenuMapList, row: usize) -> Result<&'a str, String> {
        if row >= PAGE_SIZE {
            return Err("Invalid map row".into());
        }
        self.map_at(maps, row)
            .ok_or_else(|| "Map is unavailable".into())
    }

    /// Every dvar the list draws from: a name per row, the preview of the row
    /// the pointer is on, and the pack strip with the current page lit.
    fn dvars(&self, maps: &MenuMapList) -> Vec<(String, String)> {
        let mut out = Vec::new();
        let pointed = self.map_at(maps, self.hover);
        out.push((
            "ui_map_preview_title".to_owned(),
            pointed.map_or_else(String::new, map_label),
        ));
        out.push((
            "ui_map_preview".to_owned(),
            pointed.map_or_else(String::new, map_preview),
        ));
        for row in 0..PAGE_SIZE {
            out.push((
                format!("ui_map_{row}"),
                self.map_at(maps, row).map_or_else(String::new, map_label),
            ));
        }
        let slots = slots(maps);
        for slot in 0..PACK_SLOTS {
            let shown = slots.get(slot);
            out.push((
                format!("ui_map_pack_{slot}"),
                shown.map_or_else(String::new, |slot| slot.label.clone()),
            ));
            let lit = shown.is_some_and(|slot| slot.pack == self.pack && slot.page == self.page);
            out.push((
                format!("ui_map_pack_tint_{slot}"),
                String::from(if lit { "1" } else { "0.45" }),
            ));
        }
        out
    }
}

impl Menu<'_, '_> {
    pub(super) fn open_maps(&mut self) {
        let map = self.dvars.get("ui_mapname").unwrap_or("");
        self.state.maps.open_on(&self.maps, map);
    }

    pub(super) fn pick_map_page(&mut self, command: &ConsoleCommand) -> Result<(), String> {
        let slot = arg(command).ok_or("Invalid map page")?;
        self.state.maps.pick_slot(&self.maps, slot)
    }

    pub(super) fn point_at_map(&mut self, command: &ConsoleCommand) -> Result<(), String> {
        let row = arg(command).ok_or("Invalid map row")?;
        self.state.maps.point_at(row)
    }

    /// The map on a row of the list, named by the command's first word.
    pub(super) fn map_on_row(&self, command: &ConsoleCommand) -> Result<String, String> {
        let row = arg(command).ok_or("Invalid map row")?;
        self.state.maps.pick(&self.maps, row).map(str::to_owned)
    }

    pub(super) fn publish_maps(&mut self) {
        for (name, value) in self.state.maps.dvars(&self.maps) {
            self.dvars.set(&name, value);
        }
    }
}

#[cfg(test)]
mod tests {
    use asset_transport::MapPack;

    use super::*;
    use crate::frontend::testing::{dvar, menu_app, run};

    fn pack(label: &str, count: usize) -> MapPack {
        MapPack {
            label: label.to_owned(),
            maps: (0..count).map(|n| format!("t6:mp_{label}_{n}")).collect(),
        }
    }

    fn list() -> MenuMapList {
        MenuMapList(vec![pack("base", 3), pack("big", 25), pack("none", 0)])
    }

    fn value(dvars: &[(String, String)], name: &str) -> String {
        dvars
            .iter()
            .find(|(dvar, _)| dvar == name)
            .map(|(_, value)| value.clone())
            .unwrap_or_else(|| panic!("{name} is not published"))
    }

    #[test]
    fn a_pack_of_more_than_a_page_gets_a_numbered_button_per_page() {
        let labels: Vec<String> = slots(&list()).into_iter().map(|slot| slot.label).collect();
        assert_eq!(labels, ["base", "big 1", "big 2", "big 3"]);
    }

    #[test]
    fn a_pack_with_no_maps_has_no_button() {
        assert!(slots(&MenuMapList(vec![pack("none", 0)])).is_empty());
    }

    #[test]
    fn opening_on_a_map_lands_on_its_page_and_row() {
        let maps = list();
        let mut view = MapView::default();
        view.open_on(&maps, "t6:mp_big_13");
        assert_eq!((view.pack, view.page, view.hover), (1, 1, 3));
        view.open_on(&maps, "t6:mp_base_2");
        assert_eq!((view.pack, view.page, view.hover), (0, 0, 2));
    }

    #[test]
    fn a_map_in_no_pack_opens_the_first_page() {
        let mut view = MapView::default();
        view.open_on(&list(), "t6:mp_gone");
        assert_eq!((view.pack, view.page, view.hover), (0, 0, 0));
    }

    #[test]
    fn picking_a_button_shows_its_page_with_the_pointer_on_the_first_row() {
        let maps = list();
        let mut view = MapView {
            hover: 7,
            ..MapView::default()
        };
        view.pick_slot(&maps, 3).expect("a button");
        assert_eq!((view.pack, view.page, view.hover), (1, 2, 0));
        assert_eq!(view.map_at(&maps, 4), Some("t6:mp_big_24"));
        assert_eq!(view.map_at(&maps, 5), None);
    }

    #[test]
    fn a_button_that_is_not_there_is_refused() {
        let mut view = MapView::default();
        assert_eq!(view.pick_slot(&list(), 4), Err("Invalid map page".into()));
    }

    #[test]
    fn a_click_chooses_the_map_on_its_row() {
        let maps = list();
        let mut view = MapView::default();
        view.pick_slot(&maps, 3).expect("a button");
        assert_eq!(view.pick(&maps, 0), Ok("t6:mp_big_20"));
        assert_eq!(view.pick(&maps, 5), Err("Map is unavailable".into()));
        assert_eq!(view.pick(&maps, 10), Err("Invalid map row".into()));
    }

    #[test]
    fn the_pointer_stays_on_the_page() {
        let mut view = MapView::default();
        assert_eq!(view.point_at(9), Ok(()));
        assert_eq!(view.point_at(10), Err("Invalid map row".into()));
        assert_eq!(view.hover, 9);
    }

    #[test]
    fn rows_name_the_maps_of_the_page_and_the_rest_are_blank() {
        let maps = list();
        let mut view = MapView::default();
        view.pick_slot(&maps, 3).expect("a button");
        let dvars = view.dvars(&maps);
        assert_eq!(value(&dvars, "ui_map_0"), map_label("t6:mp_big_20"));
        assert_eq!(value(&dvars, "ui_map_4"), map_label("t6:mp_big_24"));
        assert_eq!(value(&dvars, "ui_map_5"), "");
    }

    #[test]
    fn the_preview_follows_the_pointer() {
        let maps = list();
        let mut view = MapView::default();
        view.point_at(1).expect("a row");
        let dvars = view.dvars(&maps);
        assert_eq!(
            value(&dvars, "ui_map_preview_title"),
            map_label("t6:mp_base_1")
        );
        assert_eq!(value(&dvars, "ui_map_preview"), map_preview("t6:mp_base_1"));
        view.point_at(8).expect("a row");
        assert_eq!(value(&view.dvars(&maps), "ui_map_preview"), "");
    }

    #[test]
    fn only_the_page_shown_is_lit_in_the_strip() {
        let maps = list();
        let mut view = MapView::default();
        view.pick_slot(&maps, 2).expect("a button");
        let dvars = view.dvars(&maps);
        let tints: Vec<String> = (0..5)
            .map(|slot| value(&dvars, &format!("ui_map_pack_tint_{slot}")))
            .collect();
        assert_eq!(tints, ["0.45", "0.45", "1", "0.45", "0.45"]);
        assert_eq!(value(&dvars, "ui_map_pack_2"), "big 2");
        assert_eq!(value(&dvars, "ui_map_pack_4"), "");
        assert_eq!(value(&dvars, "ui_map_pack_15"), "");
    }

    #[test]
    fn the_commands_move_the_list_and_the_frame_publishes_it() {
        let mut app = menu_app(&[("base", 3), ("big", 25)]);
        run(&mut app, "ui_map_pack 2");
        run(&mut app, "ui_map_hover 3");
        assert_eq!(dvar(&app, "ui_map_pack_tint_2"), "1");
        assert_eq!(dvar(&app, "ui_map_0"), map_label("t6:mp_big_10"));
        assert_eq!(
            dvar(&app, "ui_map_preview_title"),
            map_label("t6:mp_big_13")
        );
    }

    #[test]
    fn opening_the_list_starts_on_the_chosen_map() {
        let mut app = menu_app(&[("base", 3), ("big", 25)]);
        run(&mut app, "set ui_mapname t6:mp_big_21");
        run(&mut app, "ui_maps");
        assert_eq!(dvar(&app, "ui_map_0"), map_label("t6:mp_big_20"));
        assert_eq!(
            dvar(&app, "ui_map_preview_title"),
            map_label("t6:mp_big_21")
        );
    }

    #[test]
    fn bad_words_are_reported_not_acted_on() {
        let mut app = menu_app(&[("base", 3)]);
        run(&mut app, "ui_map_pack nine");
        assert_eq!(dvar(&app, "ui_frontend_status"), "Invalid map page");
        run(&mut app, "ui_map_hover 10");
        assert_eq!(dvar(&app, "ui_frontend_status"), "Invalid map row");
        run(&mut app, "ui_map_hover");
        assert_eq!(dvar(&app, "ui_frontend_status"), "Invalid map row");
    }
}
