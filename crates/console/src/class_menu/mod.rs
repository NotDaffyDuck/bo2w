//! The class menu's commands. The screen sends one for each thing the player
//! does in it (open a class, open a row, turn a page, pick an item, rename);
//! after each frame's commands the menu's dvars are written afresh from the
//! classes and where the player is in the pickers.

mod browse;
mod edit;
#[cfg(test)]
mod fixture;
mod publish;

use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use frame::UiMenuDvars;
use ui::{LoadoutEditorLine, LoadoutRoster, LoadoutSlotStatus, MatchLoadoutVault};

use crate::tool_cmds::PromptReplyWriter;
use crate::OrderBlueprint;
use crate::TypedOrder;
use crate::OrderCatalog;
use browse::{Browse, PAGE_SIZE};
use publish::{Words, publish};

const NO_CLASS: &str = "Class is unavailable";

const COMMANDS: [&str; 11] = [
    "ui_class_slot",
    "ui_class_preview_slot",
    "ui_class_edit",
    "ui_class_game",
    "ui_class_category",
    "ui_class_pick",
    "ui_class_page",
    "ui_class_attachments",
    "ui_class_reset",
    "ui_class_rename",
    "ui_class_hover",
];

pub(crate) fn enlist(book: &mut OrderCatalog) {
    for order in COMMANDS {
        if book.look_up(order).is_none() {
            book.enlist(OrderBlueprint::new(order));
        }
    }
}

/// The number a command names: a class, a row, a game, a choice.
fn index(command: &TypedOrder) -> Result<usize, String> {
    command
        .words
        .first()
        .and_then(|arg| arg.parse().ok())
        .ok_or_else(|| "Invalid class selection".to_owned())
}

/// What the commands change and read.
#[derive(SystemParam)]
pub(crate) struct Menu<'w> {
    store: ResMut<'w, MatchLoadoutVault>,
    roster: Res<'w, LoadoutRoster>,
    loc: Res<'w, asset_game::TranslationStringIndex>,
    dvars: ResMut<'w, UiMenuDvars>,
}

impl Menu<'_> {
    fn slot_at(&self, at: usize) -> Result<&LoadoutSlotStatus, String> {
        self.store.slots.get(at).ok_or_else(|| NO_CLASS.to_owned())
    }

    fn run(
        &mut self,
        browse: &mut Browse,
        order: &TypedOrder,
        echo: &mut PromptReplyWriter,
    ) -> Result<(), String> {
        match order.name.as_str() {
            "ui_class_hover" => browse.hover = index(order)?.min(PAGE_SIZE - 1),
            "ui_class_preview_slot" => {
                let at = index(order)?;
                self.slot_at(at)?;
                self.store.selected = at;
            }
            "ui_class_slot" => {
                let at = index(order)?;
                let name = self.slot_at(at)?.name.clone();
                self.dvars.set("ui_class_name", name);
                self.store.selected = at;
                *browse = Browse::default();
            }
            "ui_class_edit" | "ui_class_attachments" => {
                let row = LoadoutEditorLine::ALL
                    .get(index(order)?)
                    .copied()
                    .ok_or("Unknown loadout slot")?;
                let attachments = order.name == "ui_class_attachments";
                if attachments && !edit::takes_attachments(row) {
                    return Err("This slot does not accept attachments".into());
                }
                browse.open_row(row, attachments);
            }
            "ui_class_game" => browse.open_game(&self.roster, index(order)?)?,
            "ui_class_category" => browse.open_folder(&self.roster, index(order)?)?,
            "ui_class_page" => {
                let delta = order
                    .words
                    .first()
                    .and_then(|arg| arg.parse::<isize>().ok())
                    .ok_or("Invalid page")?;
                let slot = self.store.slots.get(self.store.selected);
                let choices = browse.choices(&self.roster, slot).len();
                browse.turn(delta, choices);
            }
            "ui_class_pick" => self.pick(browse, order, echo)?,
            "ui_class_reset" => {
                let at = self.store.selected;
                let reset = edit::reset(&self.roster, self.slot_at(at)?, at)?;
                self.dvars.set("ui_class_name", &reset.name);
                self.store.slots[at] = reset;
            }
            "ui_class_rename" => {
                let name = edit::named(self.dvars.get("ui_class_name").unwrap_or(""), &self.loc)?;
                let at = self.store.selected;
                self.store.slots.get_mut(at).ok_or(NO_CLASS)?.name = name;
            }
            _ => {}
        }
        Ok(())
    }

    /// Takes the choice at row `at` of the page for the open row, if the class
    /// can have it.
    fn pick(
        &mut self,
        browse: &mut Browse,
        command: &TypedOrder,
        echo: &mut PromptReplyWriter,
    ) -> Result<(), String> {
        let row = browse.row.ok_or("No loadout slot selected")?;
        let at = index(command)?;
        if at >= PAGE_SIZE {
            return Err("Invalid item row".into());
        }
        let selected = self.store.selected;
        let slot = self.store.slots.get(selected);
        let value = browse
            .choices(&self.roster, slot)
            .into_iter()
            .nth(browse.page * PAGE_SIZE + at)
            .ok_or("Item is unavailable")?;
        let picked = edit::pick(
            &self.roster,
            slot.ok_or(NO_CLASS)?,
            row,
            &value,
            browse.attachments,
        )?;
        echo.write(format!(
            "menu: class {} {} = {}",
            selected + 1,
            row.label(),
            picked.value_in(row)
        ));
        self.store.slots[selected] = picked;
        browse.after_pick(row);
        Ok(())
    }
}

/// Runs the menu's commands, then shows the menu as it stands. A command that
/// works clears the menu's status line (looking around does not); one that does
/// not leaves its reason there and in the console.
pub(crate) fn route_class_menu(
    mut commands: MessageReader<TypedOrder>,
    mut browse: Local<Browse>,
    mut menu: Menu,
    mut echo: PromptReplyWriter,
) {
    for order in commands
        .read()
        .filter(|order| order.name.starts_with("ui_class_"))
    {
        match menu.run(&mut browse, order, &mut echo) {
            Ok(())
                if matches!(
                    order.name.as_str(),
                    "ui_class_hover" | "ui_class_preview_slot"
                ) => {}
            Ok(()) => menu.dvars.set("ui_class_status", ""),
            Err(error) => {
                menu.dvars.set("ui_class_status", &error);
                echo.write(format!("menu: {error}"));
            }
        }
    }
    let words = Words {
        catalog: &menu.roster,
        loc: &menu.loc,
    };
    publish(&mut menu.dvars, &menu.store, &mut browse, &words);
}

#[cfg(test)]
mod tests {
    use crate::class_menu::*;
    use crate::class_menu::fixture::{AK47, MP7, catalog, menu_app, slot};
    use crate::tool_cmds::testing::{feed, said};
    use ui::classes::setup::CatalogResolver;

    fn dvar(app: &App, key: &str) -> String {
        app.world()
            .resource::<UiMenuDvars>()
            .get(key)
            .unwrap_or("<unset>")
            .to_owned()
    }

    fn set_dvar(app: &mut App, key: &str, to: &str) {
        app.world_mut()
            .resource_mut::<UiMenuDvars>()
            .set(key, to);
    }

    fn classes(app: &App) -> &MatchLoadoutVault {
        app.world().resource::<MatchLoadoutVault>()
    }

    fn classes_mut(app: &mut App) -> Mut<'_, MatchLoadoutVault> {
        app.world_mut().resource_mut::<MatchLoadoutVault>()
    }

    #[test]
    fn the_eleven_commands_are_registered_once_and_stay_out_of_the_way() {
        let mut book = OrderCatalog::default();
        enlist(&mut book);
        enlist(&mut book);
        assert_eq!(book.usage_listing().len(), 11);
        for name in COMMANDS {
            assert!(book.look_up(name).is_some(), "{name}");
        }
    }

    #[test]
    fn the_menu_is_shown_without_a_command_being_sent() {
        let mut app = menu_app();
        app.update();
        assert_eq!(dvar(&app, "ui_class_slot_0"), "Class 1");
        assert_eq!(dvar(&app, "ui_class_title"), "Class 1");
        assert!(said(&app).is_empty());
    }

    #[test]
    fn commands_for_other_things_are_left_alone() {
        let mut app = menu_app();
        set_dvar(&mut app, "ui_class_status", "kept");
        feed(&mut app, "showpos; ui_classy");
        assert_eq!(dvar(&app, "ui_class_status"), "kept");
        assert!(said(&app).is_empty());
    }

    #[test]
    fn opening_a_class_selects_it_names_it_and_closes_whatever_was_open() {
        let mut app = menu_app();
        feed(&mut app, "ui_class_edit 0; ui_class_slot 3");
        assert_eq!(classes(&app).selected, 3);
        assert_eq!(dvar(&app, "ui_class_name"), "Class 4");
        assert_eq!(dvar(&app, "ui_class_title"), "Class 4");
        assert_eq!(dvar(&app, "ui_class_index"), "3");
        let expected = "Classes";
        assert_eq!(dvar(&app, "ui_class_caption"), expected, "the row was closed");
        assert!(said(&app).is_empty());
    }

    #[test]
    fn looking_at_a_class_selects_it_and_changes_nothing_else() {
        let mut app = menu_app();
        feed(&mut app, "ui_class_edit 0");
        set_dvar(&mut app, "ui_class_status", "kept");
        feed(&mut app, "ui_class_preview_slot 2");
        assert_eq!(classes(&app).selected, 2);
        assert_eq!(dvar(&app, "ui_class_title"), "Class 3");
        let expected = "PRIMARY";
        assert_eq!(dvar(&app, "ui_class_caption"), expected, "the row stayed open");
        assert_eq!(dvar(&app, "ui_class_name"), "<unset>");
        assert_eq!(dvar(&app, "ui_class_status"), "kept");
    }

    #[test]
    fn a_class_that_is_not_there_or_a_number_that_is_not_one_is_said_and_shown() {
        for (line, why) in [
            ("ui_class_slot 10", "Class is unavailable"),
            ("ui_class_preview_slot 99", "Class is unavailable"),
            ("ui_class_slot x", "Invalid class selection"),
            ("ui_class_slot", "Invalid class selection"),
            ("ui_class_edit -1", "Invalid class selection"),
        ] {
            let mut app = menu_app();
            feed(&mut app, line);
            assert_eq!(said(&app), [format!("menu: {why}")], "{line}");
            assert_eq!(dvar(&app, "ui_class_status"), why, "{line}");
            assert_eq!(classes(&app).selected, 0, "{line}");
        }
    }

    #[test]
    fn a_command_that_works_clears_the_status_and_looking_around_does_not() {
        let mut app = menu_app();
        set_dvar(&mut app, "ui_class_status", "old");
        feed(&mut app, "ui_class_hover 1; ui_class_preview_slot 1");
        assert_eq!(dvar(&app, "ui_class_status"), "old");
        feed(&mut app, "ui_class_edit 0");
        assert_eq!(dvar(&app, "ui_class_status"), "");
    }

    #[test]
    fn a_row_opens_its_picker_and_an_unknown_row_does_not() {
        let mut app = menu_app();
        feed(&mut app, "ui_class_edit 0");
        assert_eq!(dvar(&app, "ui_class_caption"), "PRIMARY");
        assert_eq!(dvar(&app, "ui_class_game_count"), "2");
        feed(&mut app, "ui_class_edit 8");
        assert_eq!(said(&app), ["menu: Unknown loadout slot"]);
        let expected = "PRIMARY";
        assert_eq!(dvar(&app, "ui_class_caption"), expected, "still on the row");
    }

    #[test]
    fn attachments_open_for_a_gun_and_are_refused_for_anything_else_without_a_trace() {
        let mut app = menu_app();
        feed(&mut app, "ui_class_attachments 0");
        assert_eq!(dvar(&app, "ui_class_caption"), "PRIMARY / ATTACHMENTS");
        assert_eq!(dvar(&app, "ui_class_choice_1"), "Reflex Sight");

        feed(&mut app, "ui_class_attachments 2");
        assert_eq!(said(&app), ["menu: This slot does not accept attachments"]);
        let expected = "PRIMARY / ATTACHMENTS";
        assert_eq!(dvar(&app, "ui_class_caption"), expected, "what was open stays open");
        feed(&mut app, "ui_class_attachments 1");
        assert_eq!(dvar(&app, "ui_class_caption"), "Secondary / ATTACHMENTS");
    }

    #[test]
    fn a_game_then_a_category_narrows_the_picker() {
        let mut app = menu_app();
        feed(&mut app, "ui_class_edit 0; ui_class_game 1");
        assert_eq!(dvar(&app, "ui_class_game_label"), "T6");
        assert_eq!(dvar(&app, "ui_class_category_count"), "2");
        feed(&mut app, "ui_class_category 1");
        assert_eq!(dvar(&app, "ui_class_picker_title"), "T6 / SUB MACHINE GUNS");
        assert_eq!(dvar(&app, "ui_class_choice_count"), "1");
        assert_eq!(dvar(&app, "ui_class_choice_0"), "MP7");
        feed(&mut app, "ui_class_game 0");
        assert_eq!(dvar(&app, "ui_class_game_label"), "T5");
        let expected = "";
        assert_eq!(dvar(&app, "ui_class_category_label"), expected, "a new game has no category yet");
        assert_eq!(dvar(&app, "ui_class_category_count"), "1");
        assert_eq!(dvar(&app, "ui_class_category_0"), "ASSAULT RIFLES");
        assert_eq!(dvar(&app, "ui_class_picker_title"), "PRIMARY");
    }

    #[test]
    fn a_game_or_category_that_is_not_listed_is_refused() {
        let mut app = menu_app();
        feed(
            &mut app,
            "ui_class_edit 0; ui_class_game 5; ui_class_category 0",
        );
        let expected = ["menu: Game is unavailable", "menu: Category is unavailable",];
        assert_eq!(said(&app), expected, "no game is chosen so no category is listed");
        assert_eq!(dvar(&app, "ui_class_game_label"), "");
    }

    fn many_app() -> App {
        let mut app = menu_app();
        let mut many = catalog();
        many.lethal = (0..23)
            .map(|n| format!("t6:weapon/nade{n}_mp").as_str().into())
            .collect::<Vec<frame::CustomClassGunChoice>>();
        app.insert_resource(many);
        feed(&mut app, "ui_class_edit 2");
        app
    }

    #[test]
    fn pages_turn_either_way_and_stop_at_the_first_and_the_last() {
        let mut app = many_app();
        assert_eq!(dvar(&app, "ui_class_page"), "1 / 3");
        for (line, shown, first) in [
            ("ui_class_page 1", "2 / 3", "NADE10"),
            ("ui_class_page 1", "3 / 3", "NADE20"),
            ("ui_class_page 1", "3 / 3", "NADE20"),
            ("ui_class_page -1", "2 / 3", "NADE10"),
            ("ui_class_page -50", "1 / 3", "NADE0"),
            ("ui_class_page 50", "3 / 3", "NADE20"),
        ] {
            feed(&mut app, line);
            assert_eq!(dvar(&app, "ui_class_page"), shown, "{line}");
            assert_eq!(dvar(&app, "ui_class_choice_0"), first, "{line}");
        }
        feed(&mut app, "ui_class_page up");
        assert_eq!(said(&app), ["menu: Invalid page"]);
    }

    #[test]
    fn the_pointer_picks_the_preview_and_stays_on_the_page() {
        let mut app = menu_app();
        feed(&mut app, "ui_class_edit 0");
        assert_eq!(dvar(&app, "ui_class_preview_title"), "AK-47");
        feed(&mut app, "ui_class_hover 1");
        assert_eq!(dvar(&app, "ui_class_preview_title"), "MP7");
        feed(&mut app, "ui_class_hover 99");
        let expected = "";
        assert_eq!(dvar(&app, "ui_class_preview"), expected, "the last row, and no error");
        assert!(said(&app).is_empty());
        feed(&mut app, "ui_class_hover 1; ui_class_game 1");
        let expected = "AK-47";
        assert_eq!(dvar(&app, "ui_class_preview_title"), expected, "a new list starts at its first choice");
    }

    #[test]
    fn a_pick_before_a_row_is_open_or_for_a_row_that_is_not_there_is_refused_in_order() {
        for (line, why) in [
            ("ui_class_pick 0", "No loadout slot selected"),
            ("ui_class_pick x", "No loadout slot selected"),
            (
                "ui_class_edit 0; ui_class_pick x",
                "Invalid class selection",
            ),
            ("ui_class_edit 0; ui_class_pick 10", "Invalid item row"),
            ("ui_class_edit 0; ui_class_pick 3", "Item is unavailable"),
        ] {
            let mut app = menu_app();
            feed(&mut app, line);
            let expected = format!("menu: {why}");
            assert_eq!(said(&app).last().map(String::as_str), Some(expected.as_str()), "{line}");
            assert_eq!(dvar(&app, "ui_class_status"), why, "{line}");
        }
        let mut app = menu_app();
        classes_mut(&mut app).selected = 40;
        feed(&mut app, "ui_class_edit 0; ui_class_pick 0");
        assert_eq!(said(&app), ["menu: Class is unavailable"]);
    }

    #[test]
    fn a_pick_the_catalog_refuses_says_why_and_changes_nothing() {
        let refusal = edit::pick(
            &catalog(),
            &slot("Class 1"),
            LoadoutEditorLine::Primary,
            MP7,
            false,
        )
        .unwrap_err();
        let mut app = menu_app();
        feed(&mut app, "ui_class_edit 0; ui_class_pick 1");
        assert_eq!(said(&app), [format!("menu: {refusal}")]);
        assert_eq!(dvar(&app, "ui_class_status"), refusal);
        assert_eq!(classes(&app).slots[0].primary, AK47);
        let expected = "PRIMARY";
        assert_eq!(dvar(&app, "ui_class_caption"), expected, "the picker stays up");
    }

    #[test]
    fn a_pick_that_works_changes_the_class_and_says_so() {
        let mut app = menu_app();
        classes_mut(&mut app).slots[0].frozen_because = Some("a gun the catalog cannot place".into());
        let other = catalog().options(LoadoutEditorLine::Perk1)[1].clone();
        feed(&mut app, "ui_class_edit 4");
        set_dvar(&mut app, "ui_class_status", "old");
        feed(&mut app, "ui_class_pick 1");
        assert_eq!(said(&app), [format!("menu: class 1 Perk 1 = {other}")]);
        assert_eq!(classes(&app).slots[0].first_specialty, other);
        assert!(
            dvar(&app, "ui_class_status").starts_with("Class unavailable"),
            "the old line is gone; the guns still cannot be placed, and the menu says so"
        );
        assert_eq!(dvar(&app, "ui_class_value_4"), ui_value(&other));
        let expected = "Perk 1";
        assert_eq!(dvar(&app, "ui_class_caption"), expected, "the row stays open");
    }

    fn ui_value(key: &str) -> String {
        ui::classes::display::label(key, &catalog(), &crate::class_menu::fixture::words())
    }

    #[test]
    fn a_pick_goes_to_the_selected_class() {
        let mut app = menu_app();
        classes_mut(&mut app).slots[4].frozen_because = Some("locked".into());
        let other = catalog().options(LoadoutEditorLine::Perk2)[1].clone();
        feed(
            &mut app,
            "ui_class_slot 4; ui_class_edit 5; ui_class_pick 1",
        );
        assert_eq!(said(&app), [format!("menu: class 5 Perk 2 = {other}")]);
        assert_eq!(classes(&app).slots[4].second_specialty, other);
        assert_eq!(classes(&app).slots[0].second_specialty, "specialty_bulletdamage");
    }

    #[test]
    fn resetting_a_class_that_cannot_be_rebuilt_says_why() {
        let mut app = menu_app();
        let mut catalog = catalog();
        catalog.resolver = CatalogResolver(None);
        app.insert_resource(catalog);
        classes_mut(&mut app).slots[0].name = "Mine".into();
        feed(&mut app, "ui_class_reset");
        assert_eq!(said(&app), ["menu: Weapon catalog is not ready"]);
        assert_eq!(classes(&app).slots[0].name, "Mine");

        classes_mut(&mut app).selected = 40;
        feed(&mut app, "ui_class_reset");
        let expected = Some("menu: Class is unavailable");
        assert_eq!(said(&app).last().map(String::as_str), expected);
    }

    #[test]
    fn a_reset_the_catalog_refuses_leaves_the_class_as_it_was() {
        let mut app = menu_app();
        classes_mut(&mut app).slots[0].primary = MP7.into();
        let refusal = {
            let catalog = catalog();
            edit::reset(&catalog, &slot("Class 1"), 0).unwrap_err()
        };
        feed(&mut app, "ui_class_reset");
        assert_eq!(said(&app), [format!("menu: {refusal}")]);
        assert_eq!(classes(&app).slots[0].primary, MP7);
    }

    #[test]
    fn renaming_takes_the_typed_name_trimmed_and_cut_to_twenty() {
        let mut app = menu_app();
        set_dvar(
            &mut app,
            "ui_class_name",
            "  A very long class name that is cut  ",
        );
        feed(&mut app, "ui_class_rename");
        assert_eq!(classes(&app).slots[0].name, "A very long class na");
        assert_eq!(dvar(&app, "ui_class_title"), "A very long class na");
        assert_eq!(dvar(&app, "ui_class_slot_0"), "A very long class na");
        assert_eq!(dvar(&app, "ui_class_status"), "");
        assert!(said(&app).is_empty());
    }

    #[test]
    fn a_blank_name_is_refused_in_the_menus_own_words() {
        let mut app = menu_app();
        set_dvar(&mut app, "ui_class_name", "   ");
        feed(&mut app, "ui_class_rename");
        assert_eq!(said(&app), ["menu: Enter a class name"]);
        assert_eq!(classes(&app).slots[0].name, "Class 1");
        feed(&mut app, "ui_class_slot 1");
        set_dvar(&mut app, "ui_class_name", "Sniper");
        feed(&mut app, "ui_class_rename");
        assert_eq!(classes(&app).slots[1].name, "Sniper");
        assert_eq!(classes(&app).slots[0].name, "Class 1");
    }
}
