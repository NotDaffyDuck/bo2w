//! Where the player is in the class menu's pickers: which loadout row is
//! open, which game and category are being browsed, and which page of choices
//! is up. The commands move it; the publisher shows it.

use asset_core::AssetCategoryScope;
use ui::{LoadoutEditorLine, LoadoutRoster, ClassPickerFolder, LoadoutSlotStatus};

use super::edit::{attachments_of, takes_attachments};

/// Choices on a page of a picker.
pub(super) const PAGE_SIZE: usize = 10;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Browse {
    pub(super) row: Option<LoadoutEditorLine>,
    pub(super) game: Option<AssetCategoryScope>,
    pub(super) folder: Option<ClassPickerFolder>,
    pub(super) page: usize,
    /// Picking attachments for the row's weapon rather than the row itself.
    pub(super) attachments: bool,
    /// The row of the page the pointer is over, for the preview.
    pub(super) hover: usize,
}

impl Browse {
    /// The pages a list of `len` choices takes, at least one.
    pub(super) fn page_count(len: usize) -> usize {
        len.max(1).div_ceil(PAGE_SIZE)
    }

    pub(super) fn last_page(len: usize) -> usize {
        Self::page_count(len) - 1
    }

    /// The games the open row has anything from, each once.
    pub(super) fn games(&self, catalog: &LoadoutRoster) -> Vec<AssetCategoryScope> {
        let mut games = Vec::new();
        for folder in self
            .row
            .map(|row| catalog.groupings_of(row))
            .unwrap_or_default()
        {
            if !games.contains(&folder.namespace) {
                games.push(folder.namespace);
            }
        }
        games
    }

    /// The categories of the chosen game; none until a game is chosen.
    pub(super) fn folders(&self, catalog: &LoadoutRoster) -> Vec<ClassPickerFolder> {
        let Some(row) = self.row else {
            return Vec::new();
        };
        catalog
            .groupings_of(row)
            .into_iter()
            .filter(|folder| Some(folder.namespace) == self.game)
            .collect::<Vec<ClassPickerFolder>>()
    }

    /// Everything the picker lists, over all its pages: the attachments the
    /// slot's weapon takes (after "none"), the chosen category's items, or the
    /// row's whole list.
    pub(super) fn choices(
        &self,
        catalog: &LoadoutRoster,
        slot: Option<&LoadoutSlotStatus>,
    ) -> Vec<String> {
        let Some(line) = self.row else {
            return Vec::new();
        };
        if self.attachments {
            let Some(chosen) = slot else {
                return Vec::new();
            };
            let offered = catalog.attachments(line, chosen.value_in(line));
            [String::new()]
                .into_iter()
                .chain(offered.iter().cloned())
                .collect::<Vec<String>>()
        } else if let Some(folder) = self.folder {
            catalog.keys_in_category(line, folder)
        } else {
            catalog.options(line)
        }
    }

    /// The key a listed choice goes by in the catalog's previews and in
    /// names: an attachment belongs to the weapon it is on.
    pub(super) fn key_of(&self, slot: Option<&LoadoutSlotStatus>, choice: &str) -> String {
        match (self.attachments, choice.is_empty(), slot.zip(self.row)) {
            (true, false, Some((held, row))) => {
                ui::classes::setup::attachment_preview_key(held.value_in(row), choice)
            }
            (true, false, None) => String::new(),
            _ => choice.to_owned(),
        }
    }

    /// Whether the slot has this choice now. In attachment mode "none" is
    /// current while nothing is attached.
    pub(super) fn is_current(&self, slot: &LoadoutSlotStatus, choice: &str) -> bool {
        let Some(row) = self.row else {
            return false;
        };
        if !self.attachments {
            return slot.value_in(row) == choice;
        }
        let attached = attachments_of(slot, row);
        if choice.is_empty() {
            attached.is_empty()
        } else {
            attached.iter().any(|name| name == choice)
        }
    }

    /// Opens a row's picker from its first page.
    pub(super) fn open_row(&mut self, row: LoadoutEditorLine, picks_attachments: bool) {
        *self = Self {
            row: Some(row),
            attachments: picks_attachments,
            ..Self::default()
        };
    }

    pub(super) fn open_game(
        &mut self,
        catalog: &LoadoutRoster,
        at: usize,
    ) -> Result<(), String> {
        let game = *self.games(catalog).get(at).ok_or("Game is unavailable")?;
        *self = Self {
            game: Some(game),
            folder: None,
            page: 0,
            hover: 0,
            ..*self
        };
        Ok(())
    }

    pub(super) fn open_folder(
        &mut self,
        catalog: &LoadoutRoster,
        at: usize,
    ) -> Result<(), String> {
        let folder = *self
            .folders(catalog)
            .get(at)
            .ok_or("Category is unavailable")?;
        *self = Self {
            folder: Some(folder),
            page: 0,
            hover: 0,
            ..*self
        };
        Ok(())
    }

    /// Turns the page by `delta`, stopping at the first and the last.
    pub(super) fn turn(&mut self, delta: isize, choices: usize) {
        self.hover = 0;
        self.page = self
            .page
            .saturating_add_signed(delta)
            .min(Self::last_page(choices));
    }

    /// What follows taking `row`'s item: a new weapon goes straight on to its
    /// attachments.
    pub(super) fn after_pick(&mut self, row: LoadoutEditorLine) {
        if !self.attachments && takes_attachments(row) {
            self.attachments = true;
            self.page = 0;
            self.hover = 0;
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::class_menu::browse::*;
    use crate::class_menu::fixture::{catalog, slot};

    const T5: AssetCategoryScope = AssetCategoryScope::T5;
    const T6: AssetCategoryScope = AssetCategoryScope::T6;
    const AK47: &str = "t6:weapon/ak47_mp";
    const MP7: &str = "t6:weapon/mp7_mp";
    const M16: &str = "t5:weapon/m16_mp";

    fn on(row: LoadoutEditorLine) -> Browse {
        let mut browse = Browse::default();
        browse.open_row(row, false);
        browse
    }

    #[test]
    fn a_page_holds_ten_and_a_list_always_has_a_first_page() {
        let pages = [(0, 1), (1, 1), (10, 1), (11, 2), (20, 2), (21, 3)];
        for (len, count) in pages {
            assert_eq!(Browse::page_count(len), count, "{len} choices");
            assert_eq!(Browse::last_page(len), count - 1, "{len} choices");
        }
    }

    #[test]
    fn the_games_of_a_row_are_listed_once_each_older_game_first() {
        let catalog = catalog();
        assert_eq!(on(LoadoutEditorLine::Primary).games(&catalog), [T5, T6]);
        assert_eq!(on(LoadoutEditorLine::Lethal).games(&catalog), [T6]);
        assert!(Browse::default().games(&catalog).is_empty());
    }

    #[test]
    fn categories_wait_for_a_game_and_then_are_that_games_alone() {
        let catalog = catalog();
        let mut browse = on(LoadoutEditorLine::Primary);
        assert!(browse.folders(&catalog).is_empty());
        browse.open_game(&catalog, 1).unwrap();
        let folders = browse.folders(&catalog);
        assert_eq!(folders.len(), 2, "assault and smg");
        assert!(folders.iter().all(|folder| folder.namespace == T6));
        browse.open_game(&catalog, 0).unwrap();
        assert_eq!(browse.folders(&catalog).len(), 1);
    }

    #[test]
    fn the_choices_are_the_row_then_a_category_then_the_weapons_attachments() {
        let catalog = catalog();
        let class = slot("one");
        let mut browse = on(LoadoutEditorLine::Primary);
        assert_eq!(browse.choices(&catalog, Some(&class)), [AK47, MP7, M16]);

        browse.open_game(&catalog, 0).unwrap();
        browse.open_folder(&catalog, 0).unwrap();
        assert_eq!(browse.choices(&catalog, Some(&class)), [M16]);

        browse.open_row(LoadoutEditorLine::Primary, true);
        let expected = ["", "reflex", "grip"];
        assert_eq!(browse.choices(&catalog, Some(&class)), expected, "none first, then what the ak47 takes");
        assert!(browse.choices(&catalog, None).is_empty());
        assert!(Browse::default().choices(&catalog, Some(&class)).is_empty());
    }

    #[test]
    fn opening_a_game_or_a_category_that_is_not_listed_changes_nothing() {
        let catalog = catalog();
        let mut browse = on(LoadoutEditorLine::Primary);
        let expected = Err("Game is unavailable".into());
        assert_eq!(browse.open_game(&catalog, 2), expected);
        browse.open_game(&catalog, 1).unwrap();
        browse.page = 3;
        let before = browse;
        let expected = Err("Category is unavailable".into());
        assert_eq!(browse.open_folder(&catalog, 2), expected);
        assert_eq!(browse, before);
    }

    #[test]
    fn a_new_game_or_category_starts_again_from_the_first_page() {
        let catalog = catalog();
        let mut browse = on(LoadoutEditorLine::Primary);
        browse.open_game(&catalog, 1).unwrap();
        browse.open_folder(&catalog, 0).unwrap();
        browse.page = 4;
        browse.hover = 6;
        browse.open_game(&catalog, 0).unwrap();
        let expected = (Some(T5), None, 0, 0);
        assert_eq!((browse.game, browse.folder, browse.page, browse.hover), expected);
        browse.page = 4;
        browse.open_folder(&catalog, 0).unwrap();
        assert_eq!((browse.page, browse.hover), (0, 0));
        assert_eq!(browse.row, Some(LoadoutEditorLine::Primary));
    }

    #[test]
    fn opening_a_row_drops_everything_but_the_row() {
        let mut browse = Browse {
            row: Some(LoadoutEditorLine::Perk1),
            game: Some(T6),
            folder: None,
            page: 2,
            attachments: false,
            hover: 5,
        };
        browse.open_row(LoadoutEditorLine::Secondary, true);
        let expected = Browse {
            row: Some(LoadoutEditorLine::Secondary),
            attachments: true,
            ..Browse::default()
        };
        assert_eq!(browse, expected);
    }

    #[test]
    fn turning_stops_at_the_first_page_and_the_last() {
        let mut browse = on(LoadoutEditorLine::Lethal);
        browse.hover = 4;
        browse.turn(1, 25);
        assert_eq!((browse.page, browse.hover), (1, 0));
        browse.turn(5, 25);
        assert_eq!(browse.page, 2, "25 choices make 3 pages");
        browse.turn(-1, 25);
        assert_eq!(browse.page, 1);
        browse.turn(-50, 25);
        assert_eq!(browse.page, 0);
        browse.turn(1, 0);
        assert_eq!(browse.page, 0, "an empty list has the one page");
    }

    #[test]
    fn a_choice_goes_by_its_weapons_name_when_it_is_an_attachment() {
        let class = slot("one");
        let mut browse = on(LoadoutEditorLine::Primary);
        assert_eq!(browse.key_of(Some(&class), MP7), MP7);
        browse.open_row(LoadoutEditorLine::Primary, true);
        let expected = "t6:weapon/ak47_mp+reflex";
        assert_eq!(browse.key_of(Some(&class), "reflex"), expected);
        assert_eq!(browse.key_of(Some(&class), ""), "");
        assert_eq!(browse.key_of(None, "reflex"), "");
    }

    #[test]
    fn the_current_choice_is_what_the_slot_holds_or_has_attached() {
        let mut class = slot("one");
        let mut browse = on(LoadoutEditorLine::Primary);
        assert!(browse.is_current(&class, AK47));
        assert!(!browse.is_current(&class, MP7));
        assert!(!Browse::default().is_current(&class, AK47));

        browse.open_row(LoadoutEditorLine::Primary, true);
        assert!(browse.is_current(&class, ""), "nothing attached is none");
        assert!(!browse.is_current(&class, "reflex"));
        class.primary_attachments = vec!["reflex".into()];
        assert!(!browse.is_current(&class, ""));
        assert!(browse.is_current(&class, "reflex"));
        assert!(!browse.is_current(&class, "grip"));
    }

    #[test]
    fn a_weapon_pick_goes_on_to_attachments_and_nothing_else_does() {
        for row in [LoadoutEditorLine::Primary, LoadoutEditorLine::Secondary] {
            let mut browse = on(row);
            browse.page = 2;
            browse.hover = 3;
            browse.after_pick(row);
            let expected = (true, 0, 0);
            assert_eq!((browse.attachments, browse.page, browse.hover), expected);
        }
        for row in [LoadoutEditorLine::Lethal, LoadoutEditorLine::Perk2] {
            let mut browse = on(row);
            browse.page = 2;
            browse.after_pick(row);
            assert_eq!((browse.attachments, browse.page), (false, 2));
        }
        let mut browse = on(LoadoutEditorLine::Primary);
        browse.open_row(LoadoutEditorLine::Primary, true);
        browse.page = 1;
        browse.after_pick(LoadoutEditorLine::Primary);
        assert_eq!(browse.page, 1, "already on attachments");
    }
}
