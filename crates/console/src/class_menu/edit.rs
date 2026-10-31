//! What the class menu does to the selected class: take an item for a row,
//! put the class back to its default, give it a name.

use ui::classes::display::localized;
use ui::{LoadoutEditorLine, LoadoutRoster, LoadoutSlotStatus, MatchLoadoutVault};

/// The longest a class name can be, in characters.
const NAME_LIMIT: usize = 20;

/// Only the two gun rows have attachments.
pub(super) fn takes_attachments(line: LoadoutEditorLine) -> bool {
    matches!(line, LoadoutEditorLine::Primary | LoadoutEditorLine::Secondary)
}

/// What is on the gun in `row`; nothing for a row that has no attachments.
pub(super) fn attachments_of(slot: &LoadoutSlotStatus, row: LoadoutEditorLine) -> &[String] {
    match row {
        LoadoutEditorLine::Primary => &slot.primary_attachments,
        LoadoutEditorLine::Secondary => &slot.secondary_attachments,
        _ => &[],
    }
}

fn attachments_mut(slot: &mut LoadoutSlotStatus, row: LoadoutEditorLine) -> Option<&mut Vec<String>> {
    match row {
        LoadoutEditorLine::Primary => Some(&mut slot.primary_attachments),
        LoadoutEditorLine::Secondary => Some(&mut slot.secondary_attachments),
        _ => None,
    }
}

/// The attachments after choosing `picked`: none for "none", the others
/// without it when it is already on, otherwise the others with it added — or
/// on its own when the gun takes only one.
fn toggled_list(on_gun: &[String], picked: &str, max: usize) -> Vec<String> {
    if picked.is_empty() {
        return Vec::new();
    }
    if on_gun.iter().any(|name| name == picked) {
        return on_gun
            .iter()
            .filter(|name| *name != picked)
            .cloned()
            .collect::<Vec<String>>();
    }
    let mut list = if max == 1 {
        Vec::new()
    } else {
        on_gun.to_vec()
    };
    list.push(picked.to_owned());
    list
}

/// `list` with its last items dropped until what is left `fits`.
fn fitted(mut list: Vec<String>, fits: impl Fn(&[String]) -> bool) -> Vec<String> {
    while !list.is_empty() && !fits(&list) {
        list.pop();
    }
    list
}

/// `slot` with the item chosen for `row`: the row's item itself, or one of
/// the attachments on its gun. Refuses what the catalog does not allow, and
/// refuses to lock a class that was usable.
pub(super) fn pick(
    catalog: &LoadoutRoster,
    slot: &LoadoutSlotStatus,
    row: LoadoutEditorLine,
    choice: &str,
    attachments: bool,
) -> Result<LoadoutSlotStatus, String> {
    let mut candidate = slot.clone();
    if attachments {
        let list =
            attachments_mut(&mut candidate, row).ok_or("This slot does not accept attachments")?;
        let rules = slot.loadout_rules();
        let chosen = toggled_list(list.as_slice(), choice, rules.max_attachments);
        catalog
            .check_attachments(slot.value_in(row), &chosen, rules)
            .map_err(|refusal| format!("{refusal:?}"))?;
        *list = chosen;
    } else {
        candidate.put_in(row, choice.to_owned());
        keep_what_fits(catalog, &mut candidate);
    }
    catalog.validate_edit(&candidate, row)?;
    candidate.frozen_because = catalog.validate_class(&candidate).err();
    match (&slot.frozen_because, &candidate.frozen_because) {
        (None, Some(reason)) => Err(reason.clone()),
        _ => Ok(candidate),
    }
}

/// A new perk can change how many attachments a gun may carry: drop the
/// ones that no longer fit.
fn keep_what_fits(catalog: &LoadoutRoster, slot: &mut LoadoutSlotStatus) {
    let rules = slot.loadout_rules();
    let fits =
        |weapon: &str, list: &[String]| catalog.check_attachments(weapon, list, rules).is_ok();
    let primary = std::mem::take(&mut slot.primary_attachments);
    slot.primary_attachments = fitted(primary, |list| fits(&slot.primary, list));
    let secondary = std::mem::take(&mut slot.secondary_attachments);
    slot.secondary_attachments = fitted(secondary, |list| fits(&slot.secondary, list));
}

/// The class the slot goes back to: its own preset when it is named for one,
/// else the showcase class in its place, under the slot's name.
fn default_for(
    catalog: &LoadoutRoster,
    slot: &LoadoutSlotStatus,
    at: usize,
) -> Result<LoadoutSlotStatus, String> {
    if let Some(preset) = ui::showcase_classes()
        .iter()
        .find(|preset| preset.name == slot.name)
    {
        return Ok(LoadoutSlotStatus::from_preset(preset));
    }
    let book = catalog
        .resolver
        .0
        .as_deref()
        .ok_or("Weapon catalog is not ready")?;
    let mut class = MatchLoadoutVault::from_showcase(0, book)
        .slots
        .into_iter()
        .nth(at)
        .ok_or("No available default class")?;
    class.name.clone_from(&slot.name);
    Ok(class)
}

/// The slot at `at` put back to its default, if the default is usable.
pub(super) fn reset(
    catalog: &LoadoutRoster,
    slot: &LoadoutSlotStatus,
    at: usize,
) -> Result<LoadoutSlotStatus, String> {
    let class = default_for(catalog, slot, at)?;
    catalog.validate_class(&class)?;
    Ok(class)
}

/// The name typed in the menu, trimmed and cut to the limit; an empty one is
/// refused in the menu's own words.
pub(super) fn named(typed: &str, loc: &asset_game::TranslationStringIndex) -> Result<String, String> {
    let name: String = typed.trim().chars().take(NAME_LIMIT).collect();
    let name = name.trim_end();
    if name.is_empty() {
        return Err(localized(
            loc,
            "MENU_IWNET_CREATE_BADNAME",
            "Enter a class name",
        ));
    }
    Ok(name.to_owned())
}

#[cfg(test)]
mod tests {
    use crate::class_menu::edit::*;
    use crate::class_menu::fixture::{AK47, catalog, slot};

    fn names(list: &[&str]) -> Vec<String> {
        list.iter().map(|name| (*name).to_owned()).collect()
    }

    #[test]
    fn choosing_none_clears_and_choosing_what_is_on_takes_it_off() {
        let on = names(&["reflex", "grip"]);
        assert_eq!(toggled_list(&on, "", 2), Vec::<String>::new());
        assert_eq!(toggled_list(&on, "reflex", 2), names(&["grip"]));
        assert_eq!(toggled_list(&on, "grip", 1), names(&["reflex"]));
    }

    #[test]
    fn a_new_attachment_joins_the_others_or_replaces_the_one_a_gun_may_take() {
        assert_eq!(toggled_list(&[], "reflex", 2), names(&["reflex"]));
        let actual = toggled_list(&names(&["reflex"]), "grip", 2);
        assert_eq!(actual, names(&["reflex", "grip"]));
        let actual = toggled_list(&names(&["reflex"]), "grip", 1);
        assert_eq!(actual, names(&["grip"]));
        let third = "silencer";
        let actual = toggled_list(&names(&["reflex", "grip"]), third, 2);
        assert_eq!(actual, names(&["reflex", "grip", "silencer"]), "the catalog refuses the third, not this");
    }

    #[test]
    fn a_list_loses_its_last_items_until_it_fits() {
        let list = names(&["a", "b", "c"]);
        let actual = fitted(list.clone(), |list| list.len() <= 2);
        assert_eq!(actual, names(&["a", "b"]));
        assert_eq!(fitted(list.clone(), |_| true), list);
        assert_eq!(fitted(list, |list| list.is_empty()), Vec::<String>::new());
        assert_eq!(fitted(Vec::new(), |_| false), Vec::<String>::new());
    }

    #[test]
    fn only_the_gun_rows_have_attachments() {
        let mut class = slot("one");
        class.primary_attachments = names(&["reflex"]);
        class.secondary_attachments = names(&["extclip"]);
        for row in LoadoutEditorLine::ALL {
            let expected: &[&str] = match row {
                LoadoutEditorLine::Primary => &["reflex"],
                LoadoutEditorLine::Secondary => &["extclip"],
                _ => &[],
            };
            assert_eq!(attachments_of(&class, row), expected, "{row:?}");
            assert_eq!(takes_attachments(row), !expected.is_empty(), "{row:?}");
            let actual = attachments_mut(&mut class, row).is_some();
            assert_eq!(actual, takes_attachments(row), "{row:?}");
        }
    }

    /// A class that is already unusable, so that a pick may leave it so.
    fn locked(name: &str) -> LoadoutSlotStatus {
        LoadoutSlotStatus {
            frozen_because: Some("no such gun".into()),
            ..slot(name)
        }
    }

    fn other_perk(catalog: &LoadoutRoster, class: &LoadoutSlotStatus) -> String {
        catalog
            .options(LoadoutEditorLine::Perk2)
            .into_iter()
            .find(|perk| perk != &class.second_specialty)
            .expect("the roster has more than one perk")
    }

    #[test]
    fn a_perk_is_taken_by_a_class_that_was_already_locked() {
        let catalog = catalog();
        let class = locked("one");
        let perk = other_perk(&catalog, &class);
        let picked = pick(&catalog, &class, LoadoutEditorLine::Perk2, &perk, false).unwrap();
        assert_eq!(picked.second_specialty, perk);
        assert!(
            picked.frozen_because.is_some(),
            "an empty catalog resolves nothing"
        );
        assert_eq!(picked.primary, class.primary, "nothing else moves");
    }

    #[test]
    fn a_pick_may_not_lock_a_class_that_was_usable() {
        let catalog = catalog();
        let class = slot("one");
        let perk = other_perk(&catalog, &class);
        let mut expected = class.clone();
        expected.put_in(LoadoutEditorLine::Perk2, perk.clone());
        let reason = catalog.validate_class(&expected).unwrap_err();
        let want = Err(reason);
        assert_eq!(pick(&catalog, &class, LoadoutEditorLine::Perk2, &perk, false), want);
    }

    #[test]
    fn a_perk_the_catalog_does_not_list_is_refused() {
        let catalog = catalog();
        let want = Err("Perk is unavailable".into());
        assert_eq!(pick(
            &catalog,
            &locked("one"),
            LoadoutEditorLine::Perk1,
            "specialty_none",
            false
        ), want);
    }

    #[test]
    fn a_weapon_the_catalog_cannot_resolve_is_refused() {
        let catalog = catalog();
        let class = locked("one");
        let refused = pick(
            &catalog,
            &class,
            LoadoutEditorLine::Primary,
            "t6:weapon/mp7_mp",
            false,
        );
        assert!(refused.is_err(), "{refused:?}");
    }

    #[test]
    fn nothing_is_picked_without_a_weapon_catalog() {
        let mut catalog = catalog();
        catalog.resolver = ui::classes::setup::CatalogResolver::default();
        let class = locked("one");
        let perk = other_perk(&catalog, &class);
        let want = Err("Weapon catalog is not ready".into());
        assert_eq!(pick(&catalog, &class, LoadoutEditorLine::Perk2, &perk, false), want);
    }

    #[test]
    fn attachments_are_checked_by_the_catalog_as_the_class_would_have_them() {
        let catalog = catalog();
        let class = locked("one");
        let refusal = catalog
            .check_attachments(AK47, &names(&["reflex"]), class.loadout_rules())
            .map_err(|refusal| format!("{refusal:?}"))
            .unwrap_err();
        let want = Err(refusal);
        assert_eq!(pick(&catalog, &class, LoadoutEditorLine::Primary, "reflex", true), want, "an empty catalog has no ak47");
    }

    #[test]
    fn a_row_pick_drops_the_attachments_the_catalog_no_longer_accepts() {
        let catalog = catalog();
        let class = LoadoutSlotStatus {
            primary_attachments: names(&["reflex"]),
            secondary_attachments: names(&["extclip"]),
            ..locked("one")
        };
        let perk = other_perk(&catalog, &class);
        let picked = pick(&catalog, &class, LoadoutEditorLine::Perk2, &perk, false).unwrap();
        assert_eq!(picked.primary_attachments, Vec::<String>::new());
        assert_eq!(picked.secondary_attachments, Vec::<String>::new());
    }

    #[test]
    fn attachments_for_a_row_that_has_none_are_refused() {
        let catalog = catalog();
        let class = locked("one");
        let refused = pick(&catalog, &class, LoadoutEditorLine::Perk2, "x", true);
        assert!(refused.is_err(), "{refused:?}");
    }

    #[test]
    fn a_class_goes_back_to_the_preset_it_is_named_for() {
        weapon_t6::class_presets::register();
        let catalog = catalog();
        let preset = &ui::showcase_classes()[0];
        let class = slot(preset.name);
        let want = Ok(LoadoutSlotStatus::from_preset(preset));
        assert_eq!(default_for(&catalog, &class, 3), want);
        let actual = reset(&catalog, &class, 3);
        assert_eq!(actual, Err(catalog
            .validate_class(&LoadoutSlotStatus::from_preset(preset))
            .unwrap_err()), "and a default the catalog cannot resolve is not taken");
    }

    #[test]
    fn a_class_with_a_name_of_its_own_needs_a_catalog_with_something_to_offer() {
        let mut catalog = catalog();
        let class = slot("mine");
        let want = Err("No available default class".into());
        assert_eq!(reset(&catalog, &class, 0), want);
        catalog.resolver = ui::classes::setup::CatalogResolver::default();
        let want = Err("Weapon catalog is not ready".into());
        assert_eq!(reset(&catalog, &class, 0), want);
    }

    #[test]
    fn a_name_is_trimmed_and_cut_to_twenty_characters() {
        let loc = asset_game::TranslationStringIndex::default();
        assert_eq!(named("  Rusher  ", &loc), Ok("Rusher".into()));
        let want = Ok("abcdefghijklmnopqrst".into());
        assert_eq!(named("abcdefghijklmnopqrstuvwxyz", &loc), want);
        let want = Ok("abcdefghijklmnopqrs".into());
        assert_eq!(named("abcdefghijklmnopqrs tuv", &loc), want, "no space is left hanging at the cut");
        let want = 20;
        assert_eq!(named("ééééééééééééééééééééé", &loc)
            .unwrap()
            .chars()
            .count(), want);
    }

    #[test]
    fn an_empty_name_is_refused_in_the_menus_words() {
        let plain = asset_game::TranslationStringIndex::default();
        assert_eq!(named(" \t ", &plain), Err("Enter a class name".into()));
        let loc =
            asset_game::TranslationStringIndex::from_rows([("MENU_IWNET_CREATE_BADNAME", "Name it")]);
        assert_eq!(named("", &loc), Err("Name it".into()));
    }
}
