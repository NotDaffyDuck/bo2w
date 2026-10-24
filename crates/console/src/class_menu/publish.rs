//! What the class menu's screen reads: dvars for the ten classes, the open
//! class's loadout, the picker and the preview of the item under the pointer.

use asset_game::{CacStatBar, CacWeaponPreview, LocalizeCatalog};
use frame::UiMenuDvars;
use ui::classes::display::{category_label, label, localized, preview_image};
use ui::classes::setup::attachment_preview_key;
use ui::{ClassEditRow, ClassLoadoutCatalog, ClassSlotState, SessionClassStore};

use super::browse::{Browse, PAGE_SIZE};

/// Classes the menu lists.
const SLOTS: usize = 10;

/// What the screen is written from, and in.
pub(super) struct Words<'a> {
    pub(super) catalog: &'a ClassLoadoutCatalog,
    pub(super) loc: &'a LocalizeCatalog,
}

fn flag(on: bool) -> &'static str {
    if on { "1" } else { "0" }
}

fn upper(game: asset_core::AssetNamespace) -> String {
    game.as_str().to_uppercase()
}

/// Shows the classes, the open class and the picker. A page past the end of
/// the choices is first brought back to the last one.
pub(super) fn publish(
    dvars: &mut UiMenuDvars,
    store: &SessionClassStore,
    browse: &mut Browse,
    words: &Words,
) {
    for at in 0..SLOTS {
        dvars.set(
            &format!("ui_class_slot_{at}"),
            store.slots.get(at).map_or("", |slot| slot.name.as_str()),
        );
    }
    let slot = store.slots.get(store.selected);
    if let Some(slot) = slot {
        class(dvars, store.selected, slot, words);
    }
    let choices = browse.choices(words.catalog, slot);
    browse.page = browse.page.min(Browse::last_page(choices.len()));
    picker(dvars, browse, &choices, slot, words);
    preview(dvars, browse, &choices, slot, words);
}

fn described(key: &str, words: &Words) -> String {
    words
        .catalog
        .previews
        .get(key)
        .and_then(|preview| words.loc.text(preview.desc_key.trim_start_matches('@')))
        .unwrap_or("")
        .to_owned()
}

/// The open class: its name, its eight items and its guns' attachments. A
/// class the catalog cannot use says so, unless something else already did.
fn class(dvars: &mut UiMenuDvars, selected: usize, slot: &ClassSlotState, words: &Words) {
    let Words { catalog, loc } = *words;
    if dvars.get("ui_class_status").is_none_or(str::is_empty)
        && let Some(reason) = &slot.lock_reason
    {
        dvars.set("ui_class_status", format!("Class unavailable: {reason}"));
    }
    dvars.set("ui_class_title", &slot.name);
    dvars.set("ui_class_saved_name", &slot.name);
    dvars.set("ui_class_index", selected.to_string());
    for row in ClassEditRow::ALL {
        let item = slot.row_value(row);
        let n = row.as_u8();
        dvars.set(&format!("ui_class_value_{n}"), label(item, catalog, loc));
        dvars.set(&format!("ui_class_image_{n}"), preview_image(item, catalog));
        dvars.set(&format!("ui_class_desc_{n}"), described(item, words));
    }
    for (name, weapon, attached) in [
        ("primary", &slot.primary, &slot.primary_attachments),
        ("secondary", &slot.secondary, &slot.secondary_attachments),
    ] {
        let keys: Vec<String> = attached
            .iter()
            .map(|attachment| attachment_preview_key(weapon, attachment))
            .collect();
        for at in 0..2 {
            let key = keys.get(at).map_or("", String::as_str);
            dvars.set(
                &format!("ui_class_{name}_attachment_image_{at}"),
                preview_image(key, catalog),
            );
            dvars.set(
                &format!("ui_class_{name}_attachment_name_{at}"),
                if key.is_empty() {
                    String::new()
                } else {
                    label(key, catalog, loc)
                },
            );
        }
        let named = if keys.is_empty() {
            label("", catalog, loc)
        } else {
            keys.iter()
                .map(|key| label(key, catalog, loc))
                .collect::<Vec<_>>()
                .join(" + ")
        };
        dvars.set(&format!("ui_class_{name}_attachments"), named);
    }
}

/// The row's name, or `otherwise` (a key and its fallback) before any row is
/// open.
fn row_name(row: Option<ClassEditRow>, otherwise: (&str, &str), loc: &LocalizeCatalog) -> String {
    row.map_or_else(
        || localized(loc, otherwise.0, otherwise.1),
        |row| localized(loc, row.loc_key(), row.label()),
    )
}

/// The picker: its captions, its games, categories and the page of choices
/// it shows.
fn picker(
    dvars: &mut UiMenuDvars,
    browse: &Browse,
    choices: &[String],
    slot: Option<&ClassSlotState>,
    words: &Words,
) {
    let Words { catalog, loc } = *words;
    let games = browse.games(catalog);
    let folders = browse.folders(catalog);
    let first = browse.page * PAGE_SIZE;
    dvars.set("ui_class_game_count", games.len().to_string());
    dvars.set("ui_class_category_count", folders.len().to_string());
    dvars.set(
        "ui_class_choice_count",
        choices
            .len()
            .saturating_sub(first)
            .min(PAGE_SIZE)
            .to_string(),
    );
    dvars.set("ui_class_more_pages", flag(choices.len() > PAGE_SIZE));
    dvars.set(
        "ui_class_nested",
        flag(!browse.attachments && browse.row.is_some_and(ClassLoadoutCatalog::uses_categories)),
    );
    let attachments = || localized(loc, "MENU_ATTACHMENTS_CAPS", "Attachments");
    let mut caption = row_name(browse.row, ("MENU_CLASSES", "Classes"), loc);
    if browse.attachments {
        caption.push_str(" / ");
        caption.push_str(&attachments());
    }
    dvars.set("ui_class_caption", caption);
    dvars.set(
        "ui_class_game_label",
        browse.game.map_or_else(String::new, upper),
    );
    dvars.set(
        "ui_class_category_label",
        browse
            .folder
            .map_or_else(String::new, |folder| category_label(folder, loc)),
    );
    for at in 0..PAGE_SIZE {
        dvars.set(
            &format!("ui_class_game_{at}"),
            games.get(at).map_or_else(String::new, |game| upper(*game)),
        );
        dvars.set(
            &format!("ui_class_category_{at}"),
            folders
                .get(at)
                .map_or_else(String::new, |folder| category_label(*folder, loc)),
        );
        dvars.set(
            &format!("ui_class_choice_{at}"),
            choices.get(first + at).map_or_else(String::new, |choice| {
                let current = slot.is_some_and(|slot| browse.is_current(slot, choice));
                let name = label(&browse.key_of(slot, choice), catalog, loc);
                if current { format!("* {name}") } else { name }
            }),
        );
    }
    let title = if browse.attachments {
        attachments()
    } else if let Some(folder) = browse.folder {
        format!(
            "{} / {}",
            upper(folder.namespace),
            category_label(folder, loc)
        )
    } else {
        row_name(browse.row, ("MENU_WEAPON_CLASSES_CAPS", "Classes"), loc)
    };
    dvars.set("ui_class_picker_title", title);
    dvars.set(
        "ui_class_page",
        format!(
            "{} / {}",
            browse.page + 1,
            Browse::page_count(choices.len())
        ),
    );
}

/// The choice under the pointer: its key and what the catalog knows of it.
fn hovered<'a>(
    browse: &Browse,
    choices: &[String],
    slot: Option<&ClassSlotState>,
    catalog: &'a ClassLoadoutCatalog,
) -> Option<(String, &'a CacWeaponPreview)> {
    let choice = choices.get(browse.page * PAGE_SIZE + browse.hover)?;
    let key = browse.key_of(slot, choice);
    let preview = catalog.previews.get(&key)?;
    Some((key, preview))
}

/// The item under the pointer: picture, name, stat bars and description.
fn preview(
    dvars: &mut UiMenuDvars,
    browse: &Browse,
    choices: &[String],
    slot: Option<&ClassSlotState>,
    words: &Words,
) {
    let Words { catalog, loc } = *words;
    let shown = hovered(browse, choices, slot, catalog);
    dvars.set(
        "ui_class_preview",
        shown
            .as_ref()
            .map_or_else(String::new, |(key, _)| preview_image(key, catalog)),
    );
    let square = browse.attachments || browse.row.is_some_and(|row| row.perk_slot().is_some());
    dvars.set("ui_class_preview_width", if square { "100" } else { "200" });
    dvars.set(
        "ui_class_preview_title",
        shown.as_ref().map_or_else(String::new, |(_, preview)| {
            localized(loc, &preview.name_key, &preview.reference)
        }),
    );
    for (at, bar) in CacStatBar::ALL.into_iter().enumerate() {
        let value = shown.as_ref().and_then(|(_, preview)| {
            preview
                .bars
                .iter()
                .find(|(kind, _)| *kind == bar)
                .map(|(_, value)| *value)
        });
        dvars.set(
            &format!("ui_class_stat_{at}"),
            value.unwrap_or(0).clamp(0, 100).to_string(),
        );
        dvars.set(
            &format!("ui_class_stat_visible_{at}"),
            flag(value.is_some()),
        );
    }
    dvars.set(
        "ui_class_description",
        shown
            .as_ref()
            .and_then(|(_, preview)| loc.text(preview.desc_key.trim_start_matches('@')))
            .unwrap_or(""),
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::class_menu::fixture::{MP7, catalog, store, words};

    fn open(row: ClassEditRow, attachments: bool) -> Browse {
        let mut browse = Browse::default();
        browse.open_row(row, attachments);
        browse
    }

    fn shown(store: &SessionClassStore, browse: &mut Browse) -> UiMenuDvars {
        let mut dvars = UiMenuDvars::default();
        show(&mut dvars, store, browse);
        dvars
    }

    fn show(dvars: &mut UiMenuDvars, store: &SessionClassStore, browse: &mut Browse) {
        let (catalog, loc) = (catalog(), words());
        publish(
            dvars,
            store,
            browse,
            &Words {
                catalog: &catalog,
                loc: &loc,
            },
        );
    }

    fn at<'a>(dvars: &'a UiMenuDvars, name: &str) -> &'a str {
        dvars
            .get(name)
            .unwrap_or_else(|| panic!("{name} is not set"))
    }

    #[test]
    fn the_ten_classes_are_listed_by_name_and_the_unused_ones_are_blank() {
        let mut few = store();
        few.slots.truncate(3);
        let dvars = shown(&few, &mut Browse::default());
        for (n, name) in [(0, "Class 1"), (2, "Class 3"), (3, ""), (9, "")] {
            assert_eq!(at(&dvars, &format!("ui_class_slot_{n}")), name, "{n}");
        }
    }

    #[test]
    fn the_open_class_is_shown_by_name_index_and_what_it_holds() {
        let mut classes = store();
        classes.selected = 2;
        classes.slots[2].primary = MP7.into();
        let dvars = shown(&classes, &mut Browse::default());
        assert_eq!(at(&dvars, "ui_class_title"), "Class 3");
        assert_eq!(at(&dvars, "ui_class_saved_name"), "Class 3");
        assert_eq!(at(&dvars, "ui_class_index"), "2");
        assert_eq!(at(&dvars, "ui_class_value_0"), "MP7");
        assert_eq!(at(&dvars, "ui_class_image_0"), "t6:material/ui_mp7");
        assert_eq!(
            at(&dvars, "ui_class_value_4"),
            "Marathon",
            "perk 1 is row 4"
        );
        assert_eq!(
            at(&dvars, "ui_class_value_5"),
            "SPECIALTY BULLETDAMAGE",
            "no words for it"
        );
        assert_eq!(
            at(&dvars, "ui_class_desc_0"),
            "",
            "the mp7 has no description"
        );
        let dvars = shown(&store(), &mut Browse::default());
        assert_eq!(at(&dvars, "ui_class_value_0"), "AK-47");
        assert_eq!(at(&dvars, "ui_class_image_0"), "t6:material/ui_ak47");
        assert_eq!(at(&dvars, "ui_class_desc_0"), "Reliable and strong.");
    }

    #[test]
    fn a_gun_shows_each_attachment_and_all_of_them_together() {
        let mut classes = store();
        classes.slots[0].primary_attachments = vec!["reflex".into(), "grip".into()];
        let dvars = shown(&classes, &mut Browse::default());
        assert_eq!(
            at(&dvars, "ui_class_primary_attachment_name_0"),
            "Reflex Sight"
        );
        assert_eq!(
            at(&dvars, "ui_class_primary_attachment_image_0"),
            "t6:material/ui_reflex"
        );
        assert_eq!(at(&dvars, "ui_class_primary_attachment_name_1"), "GRIP");
        assert_eq!(at(&dvars, "ui_class_primary_attachment_image_1"), "");
        assert_eq!(
            at(&dvars, "ui_class_primary_attachments"),
            "Reflex Sight + GRIP"
        );
        assert_eq!(at(&dvars, "ui_class_secondary_attachment_name_0"), "");
        assert_eq!(at(&dvars, "ui_class_secondary_attachments"), "None");
    }

    #[test]
    fn an_unusable_class_says_why_unless_something_else_was_said() {
        let mut classes = store();
        classes.slots[0].lock_reason = Some("no such gun".into());
        let mut dvars = shown(&classes, &mut Browse::default());
        assert_eq!(
            at(&dvars, "ui_class_status"),
            "Class unavailable: no such gun"
        );
        dvars.set("ui_class_status", "Item is unavailable");
        show(&mut dvars, &classes, &mut Browse::default());
        assert_eq!(at(&dvars, "ui_class_status"), "Item is unavailable");
        dvars.set("ui_class_status", "");
        show(&mut dvars, &classes, &mut Browse::default());
        assert_eq!(
            at(&dvars, "ui_class_status"),
            "Class unavailable: no such gun"
        );
        let fine = shown(&store(), &mut Browse::default());
        assert_eq!(fine.get("ui_class_status"), None);
    }

    #[test]
    fn no_class_open_shows_the_ten_slots_and_no_class() {
        let empty = SessionClassStore::default();
        let dvars = shown(&empty, &mut Browse::default());
        assert_eq!(at(&dvars, "ui_class_slot_0"), "");
        assert_eq!(dvars.get("ui_class_title"), None);
        assert_eq!(at(&dvars, "ui_class_choice_count"), "0");
    }

    #[test]
    fn with_nothing_open_the_picker_is_empty_and_captioned_classes() {
        let dvars = shown(&store(), &mut Browse::default());
        assert_eq!(at(&dvars, "ui_class_caption"), "Classes");
        assert_eq!(at(&dvars, "ui_class_picker_title"), "Classes");
        assert_eq!(at(&dvars, "ui_class_game_count"), "0");
        assert_eq!(at(&dvars, "ui_class_category_count"), "0");
        assert_eq!(at(&dvars, "ui_class_choice_count"), "0");
        assert_eq!(at(&dvars, "ui_class_more_pages"), "0");
        assert_eq!(at(&dvars, "ui_class_nested"), "0");
        assert_eq!(at(&dvars, "ui_class_page"), "1 / 1");
        assert_eq!(at(&dvars, "ui_class_preview"), "");
        assert_eq!(at(&dvars, "ui_class_game_label"), "");
        assert_eq!(at(&dvars, "ui_class_category_label"), "");
    }

    #[test]
    fn an_open_weapon_row_lists_its_games_and_marks_the_one_the_class_holds() {
        let mut browse = open(ClassEditRow::Primary, false);
        let dvars = shown(&store(), &mut browse);
        assert_eq!(at(&dvars, "ui_class_caption"), "PRIMARY");
        assert_eq!(at(&dvars, "ui_class_picker_title"), "PRIMARY");
        assert_eq!(at(&dvars, "ui_class_nested"), "1");
        assert_eq!(at(&dvars, "ui_class_game_count"), "2");
        assert_eq!(at(&dvars, "ui_class_game_0"), "T5");
        assert_eq!(at(&dvars, "ui_class_game_1"), "T6");
        assert_eq!(at(&dvars, "ui_class_game_2"), "");
        assert_eq!(at(&dvars, "ui_class_category_count"), "0", "no game chosen");
        assert_eq!(at(&dvars, "ui_class_choice_count"), "3");
        assert_eq!(at(&dvars, "ui_class_choice_0"), "* AK-47");
        assert_eq!(at(&dvars, "ui_class_choice_1"), "MP7");
        assert_eq!(at(&dvars, "ui_class_choice_2"), "M16");
        assert_eq!(at(&dvars, "ui_class_choice_3"), "");
        assert_eq!(at(&dvars, "ui_class_page"), "1 / 1");
    }

    #[test]
    fn a_chosen_game_and_category_are_named_in_the_title() {
        let catalog = catalog();
        let mut browse = open(ClassEditRow::Primary, false);
        browse.open_game(&catalog, 1).unwrap();
        let dvars = shown(&store(), &mut browse);
        assert_eq!(at(&dvars, "ui_class_game_label"), "T6");
        assert_eq!(at(&dvars, "ui_class_category_count"), "2");
        assert_eq!(at(&dvars, "ui_class_category_0"), "ASSAULT RIFLES");
        assert_eq!(at(&dvars, "ui_class_category_1"), "SUB MACHINE GUNS");

        browse.open_folder(&catalog, 1).unwrap();
        let dvars = shown(&store(), &mut browse);
        assert_eq!(at(&dvars, "ui_class_category_label"), "SUB MACHINE GUNS");
        assert_eq!(at(&dvars, "ui_class_picker_title"), "T6 / SUB MACHINE GUNS");
        assert_eq!(at(&dvars, "ui_class_choice_count"), "1");
        assert_eq!(at(&dvars, "ui_class_choice_0"), "MP7");
    }

    #[test]
    fn attachments_are_listed_after_none_with_what_is_on_marked() {
        let mut classes = store();
        let mut browse = open(ClassEditRow::Primary, true);
        let dvars = shown(&classes, &mut browse);
        assert_eq!(at(&dvars, "ui_class_caption"), "PRIMARY / ATTACHMENTS");
        assert_eq!(at(&dvars, "ui_class_picker_title"), "ATTACHMENTS");
        assert_eq!(at(&dvars, "ui_class_nested"), "0");
        assert_eq!(at(&dvars, "ui_class_choice_count"), "3");
        assert_eq!(at(&dvars, "ui_class_choice_0"), "* None");
        assert_eq!(at(&dvars, "ui_class_choice_1"), "Reflex Sight");
        assert_eq!(at(&dvars, "ui_class_choice_2"), "GRIP");

        classes.slots[0].primary_attachments = vec!["grip".into()];
        let dvars = shown(&classes, &mut browse);
        assert_eq!(at(&dvars, "ui_class_choice_0"), "None");
        assert_eq!(at(&dvars, "ui_class_choice_2"), "* GRIP");
    }

    #[test]
    fn more_than_a_page_of_choices_shows_a_page_at_a_time() {
        let mut catalog_of_many = catalog();
        catalog_of_many.lethal = (0..23)
            .map(|n| format!("t6:weapon/nade{n}_mp").as_str().into())
            .collect();
        let loc = words();
        let classes = store();
        let mut dvars = UiMenuDvars::default();
        let mut browse = open(ClassEditRow::Lethal, false);
        let words = Words {
            catalog: &catalog_of_many,
            loc: &loc,
        };
        publish(&mut dvars, &classes, &mut browse, &words);
        assert_eq!(at(&dvars, "ui_class_more_pages"), "1");
        assert_eq!(at(&dvars, "ui_class_choice_count"), "10");
        assert_eq!(at(&dvars, "ui_class_page"), "1 / 3");
        assert_eq!(at(&dvars, "ui_class_choice_0"), "NADE0");

        browse.page = 2;
        publish(&mut dvars, &classes, &mut browse, &words);
        assert_eq!(at(&dvars, "ui_class_choice_count"), "3");
        assert_eq!(at(&dvars, "ui_class_page"), "3 / 3");
        assert_eq!(at(&dvars, "ui_class_choice_0"), "NADE20");
        assert_eq!(at(&dvars, "ui_class_choice_3"), "");
    }

    #[test]
    fn a_page_left_past_the_end_is_brought_back_before_anything_counts_it() {
        let mut browse = open(ClassEditRow::Primary, false);
        browse.page = 7;
        let dvars = shown(&store(), &mut browse);
        assert_eq!(browse.page, 0);
        assert_eq!(at(&dvars, "ui_class_choice_count"), "3");
        assert_eq!(at(&dvars, "ui_class_choice_0"), "* AK-47");
        assert_eq!(at(&dvars, "ui_class_page"), "1 / 1");
    }

    #[test]
    fn the_item_under_the_pointer_is_previewed_with_its_bars_and_words() {
        let mut browse = open(ClassEditRow::Primary, false);
        let dvars = shown(&store(), &mut browse);
        assert_eq!(at(&dvars, "ui_class_preview"), "t6:material/ui_ak47");
        assert_eq!(at(&dvars, "ui_class_preview_width"), "200");
        assert_eq!(at(&dvars, "ui_class_preview_title"), "AK-47");
        assert_eq!(at(&dvars, "ui_class_description"), "Reliable and strong.");
        for (bar, value, visible) in [
            (0, "40", "1"),
            (1, "100", "1"),
            (2, "0", "0"),
            (4, "0", "1"),
        ] {
            assert_eq!(
                at(&dvars, &format!("ui_class_stat_{bar}")),
                value,
                "bar {bar}"
            );
            assert_eq!(
                at(&dvars, &format!("ui_class_stat_visible_{bar}")),
                visible,
                "bar {bar}"
            );
        }

        browse.hover = 1;
        let dvars = shown(&store(), &mut browse);
        assert_eq!(at(&dvars, "ui_class_preview"), "t6:material/ui_mp7");
        assert_eq!(at(&dvars, "ui_class_preview_title"), "MP7");
        assert_eq!(at(&dvars, "ui_class_stat_visible_0"), "0");

        browse.hover = 2;
        let dvars = shown(&store(), &mut browse);
        assert_eq!(at(&dvars, "ui_class_preview"), "", "the m16 has no preview");
        assert_eq!(at(&dvars, "ui_class_preview_title"), "");
        assert_eq!(at(&dvars, "ui_class_description"), "");
        browse.hover = 9;
        let dvars = shown(&store(), &mut browse);
        assert_eq!(at(&dvars, "ui_class_preview"), "", "past the last choice");
    }

    #[test]
    fn an_attachment_is_previewed_as_the_weapons_and_a_square_picture() {
        let mut browse = open(ClassEditRow::Primary, true);
        browse.hover = 1;
        let dvars = shown(&store(), &mut browse);
        assert_eq!(at(&dvars, "ui_class_preview"), "t6:material/ui_reflex");
        assert_eq!(at(&dvars, "ui_class_preview_width"), "100");
        assert_eq!(at(&dvars, "ui_class_preview_title"), "Reflex Sight");
        assert_eq!(at(&dvars, "ui_class_description"), "A red dot.");
        browse.hover = 0;
        let dvars = shown(&store(), &mut browse);
        assert_eq!(at(&dvars, "ui_class_preview"), "", "none has no picture");
    }

    #[test]
    fn perk_pictures_are_square_too() {
        for row in [
            ClassEditRow::Perk1,
            ClassEditRow::Perk2,
            ClassEditRow::Perk3,
            ClassEditRow::Deathstreak,
        ] {
            let dvars = shown(&store(), &mut open(row, false));
            assert_eq!(at(&dvars, "ui_class_preview_width"), "100", "{row:?}");
        }
        for row in [ClassEditRow::Lethal, ClassEditRow::Tactical] {
            let dvars = shown(&store(), &mut open(row, false));
            assert_eq!(at(&dvars, "ui_class_preview_width"), "200", "{row:?}");
        }
    }

    #[test]
    fn nothing_a_class_has_is_published_for_a_class_that_is_not_there() {
        let mut classes = store();
        classes.selected = 40;
        let mut browse = open(ClassEditRow::Primary, true);
        let dvars = shown(&classes, &mut browse);
        assert_eq!(dvars.get("ui_class_index"), None);
        assert_eq!(
            at(&dvars, "ui_class_choice_count"),
            "0",
            "no weapon, no attachments"
        );
    }
}
