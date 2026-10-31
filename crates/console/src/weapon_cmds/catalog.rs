//! What the weapon commands ask of the weapon catalog: the configuration a name
//! and its attachments make, what the weapon in hand offers to toggle, and the
//! names the prompt completes.

use asset_game::{
    AttachmentOption, ConfigurationRefusal, FamilyKey, FamilySlot, LoadoutRules, WeaponLedger,
    WeaponSelection,
};
use assets::StagedWeapons;

/// A refusal as the console words it: the reason, then its stable code.
fn refused(refusal: &ConfigurationRefusal) -> String {
    format!("{refusal} ({})", refusal.code())
}

/// The id of the configuration `raw` names with `attachments` on it: a weapon
/// family (`t6:an94`) or a single weapon (`an94`, `an94_mp`).
pub(crate) fn resolve_give_id(
    book: &WeaponLedger,
    raw: &str,
    attachments: &[String],
) -> Result<u32, String> {
    let family = FamilyKey::parse(raw).filter(|key| book.weapon_families().find(key).is_some());
    if let Some(family) = family {
        return configure(book, raw, &WeaponSelection::with(family, attachments));
    }
    let id = weapon_named(book, raw)?;
    if attachments.is_empty() {
        return book
            .configuration_admission(id)
            .map(|()| id)
            .map_err(|why| format!("`{raw}`: {}", refused(&why)));
    }
    let mut selection = book
        .describe_configuration(id)
        .cloned()
        .ok_or_else(|| format!("`{raw}` belongs to no weapon family"))?;
    selection.attachments.extend_from_slice(attachments);
    configure(book, raw, &selection)
}

fn configure(
    book: &WeaponLedger,
    raw: &str,
    selection: &WeaponSelection,
) -> Result<u32, String> {
    book
        .resolve_configuration(selection, LoadoutRules::default())
        .map(|resolved| resolved.id)
        .map_err(|why| format!("`{raw}`: {}", refused(&why)))
}

/// The weapon a name stands for, written with or without the `_mp` its row
/// carries.
fn weapon_named(book: &WeaponLedger, spelled: &str) -> Result<u32, String> {
    match book.follow_slot_link(spelled) {
        Ok(Some(id)) => return Ok(id),
        Ok(None) => return Err("empty weapon name".into()),
        Err(_) => {}
    }
    let suffixed = if spelled.ends_with("_mp") {
        spelled.to_owned()
    } else {
        format!("{spelled}_mp")
    };
    match book.follow_slot_link(&suffixed) {
        Ok(Some(found)) => Ok(found),
        _ => Err(format!("unknown weapon `{spelled}`")),
    }
}

/// What `weapon` is made of: its family and the attachments on it.
fn selection_of(book: &WeaponLedger, weapon: u32) -> Result<WeaponSelection, String> {
    book
        .describe_configuration(weapon)
        .filter(|selection| selection.family.is_some())
        .cloned()
        .ok_or_else(|| format!("`{}` belongs to no weapon family", book.label_for(weapon)))
}

fn choices_of(
    book: &WeaponLedger,
    selection: &WeaponSelection,
) -> Result<Vec<AttachmentOption>, String> {
    book
        .list_attachment_choices(selection, LoadoutRules::default())
        .map_err(|why| refused(&why))
}

/// One line on the weapon in hand: what is on it and what each attachment
/// would do if toggled.
pub(super) fn attachment_hints(book: &WeaponLedger, weapon: u32) -> Result<String, String> {
    let selection = selection_of(book, weapon)?;
    let options = choices_of(book, &selection)?;
    Ok(hint_line(
        &book.configuration_label(weapon),
        &selection.attachments,
        &options,
    ))
}

fn hint_line(label: &str, selected: &[String], options: &[AttachmentOption]) -> String {
    let selected = if selected.is_empty() {
        String::from("none")
    } else {
        selected.join(", ")
    };
    let choices = if options.is_empty() {
        "no authored attachment choices".to_owned()
    } else {
        let texts: Vec<String> = options.iter().map(choice_text).collect();
        texts.join(", ")
    };
    format!("{label}; selected: {selected}; choices: {choices}")
}

fn choice_text(option: &AttachmentOption) -> String {
    let name = &option.choice.name;
    let action = if option.selected { "remove" } else { "add" };
    match &option.toggle {
        Ok(_) => format!("{name} ({action})"),
        Err(why) => format!("{name} ({action} unavailable: {why}; {})", why.code()),
    }
}

/// The id of the weapon in hand with the attachment `name` added, or taken off
/// if it is on.
pub(super) fn toggled_id(
    book: &WeaponLedger,
    in_hand: u32,
    name: &str,
) -> Result<u32, String> {
    let selection = selection_of(book, in_hand)?;
    pick_toggle(&choices_of(book, &selection)?, name)
}

fn pick_toggle(options: &[AttachmentOption], name: &str) -> Result<u32, String> {
    let wanted = name.to_ascii_lowercase();
    let option = options
        .iter()
        .find(|option| option.choice.name == wanted)
        .ok_or_else(|| format!("attachment `{name}` is not offered on this weapon"))?;
    option.toggle.clone().map_err(|why| refused(&why))
}

/// The weapons `give` completes: every primary and secondary the catalog
/// offers that has a gun model to show.
pub fn weapon_completions(weapons: &StagedWeapons) -> Vec<String> {
    let book = &weapons.0;
    let offered = book
        .weapon_families()
        .offered()
        .filter(|family| matches!(family.bay, FamilySlot::Primary | FamilySlot::Secondary))
        .filter(|family| {
            family
                .base
                .is_some_and(|base| book.view_mesh_for(base).is_some())
        })
        .map(|family| family.key.short());
    sorted_unique(offered)
}

/// The attachments `attach` completes: the ones the weapon in hand offers.
pub fn attach_completions(weapons: &StagedWeapons, weapon: u32) -> Vec<String> {
    let book = &weapons.0;
    selection_of(book, weapon)
        .and_then(|selection| choices_of(book, &selection))
        .map_or_else(
            |_| Vec::new(),
            |options| {
                options
                    .into_iter()
                    .map(|option| option.choice.name)
                    .collect::<Vec<String>>()
            },
        )
}

fn sorted_unique(names: impl IntoIterator<Item = String>) -> Vec<String> {
    let mut sorted: Vec<String> = names.into_iter().collect();
    sorted.sort();
    sorted.dedup();
    sorted
}

#[cfg(test)]
mod tests {
    use asset_game::AttachmentChoice;

    use crate::weapon_cmds::catalog::*;

    fn option(
        name: &str,
        on: bool,
        toggle: Result<u32, ConfigurationRefusal>,
    ) -> AttachmentOption {
        AttachmentOption {
            choice: AttachmentChoice {
                name: name.to_owned(),
                point: String::new(),
                caption_key: String::new(),
                icon: String::new(),
                description_lookup: String::new(),
            },
            selected: on,
            toggle,
        }
    }

    fn clash() -> ConfigurationRefusal {
        ConfigurationRefusal::Incompatible {
            a: "grip".to_owned(),
            b: "reflex".to_owned(),
        }
    }

    fn words(list: &[&str]) -> Vec<String> {
        list.iter().map(|word| (*word).to_owned()).collect()
    }

    #[test]
    fn the_hint_line_says_what_is_on_and_what_each_choice_would_do() {
        let options = [
            option("reflex", true, Ok(10)),
            option("acog", false, Ok(11)),
        ];
        let actual = hint_line("t6:an94 +reflex", &words(&["reflex"]), &options);
        assert_eq!(actual, "t6:an94 +reflex; selected: reflex; choices: reflex (remove), acog (add)");
    }

    #[test]
    fn a_choice_that_cannot_be_toggled_says_why_and_gives_its_code() {
        let options = [option("grip", false, Err(clash()))];
        let actual = hint_line("t6:an94 +reflex", &words(&["reflex", "dualclip"]), &options);
        assert_eq!(actual, "t6:an94 +reflex; selected: reflex, dualclip; choices: grip (add unavailable: \
         `grip` and `reflex` cannot be combined; weapon.incompatible)");
    }

    #[test]
    fn a_bare_weapon_with_nothing_to_choose_says_so() {
        let expected = "t6:an94; selected: none; choices: no authored attachment choices";
        assert_eq!(hint_line("t6:an94", &[], &[]), expected);
    }

    #[test]
    fn a_toggle_goes_by_the_name_in_either_case() {
        let options = [
            option("reflex", true, Ok(10)),
            option("acog", false, Ok(11)),
        ];
        assert_eq!(pick_toggle(&options, "acog"), Ok(11));
        assert_eq!(pick_toggle(&options, "ACOG"), Ok(11));
        assert_eq!(pick_toggle(&options, "Reflex"), Ok(10));
    }

    #[test]
    fn a_toggle_the_weapon_does_not_offer_is_named_as_typed() {
        let options = [option("acog", false, Ok(11))];
        let expected = Err("attachment `Laser` is not offered on this weapon".into());
        assert_eq!(pick_toggle(&options, "Laser"), expected);
    }

    #[test]
    fn a_toggle_that_is_refused_gives_the_refusal_and_its_code() {
        let options = [option("grip", false, Err(clash()))];
        let actual = pick_toggle(&options, "grip");
        assert_eq!(actual, Err("`grip` and `reflex` cannot be combined (weapon.incompatible)".into()));
    }

    #[test]
    fn completions_are_sorted_and_listed_once() {
        let names = words(&["t6:xpr50", "t6:an94", "t6:an94", "mp:ak47"]);
        assert_eq!(sorted_unique(names), ["mp:ak47", "t6:an94", "t6:xpr50"]);
    }

    #[test]
    fn a_name_the_catalog_does_not_know_is_unknown_with_or_without_the_suffix() {
        let none = WeaponLedger::default();
        for raw in ["an94", "an94_mp", "t6:an94"] {
            let expected = Err(format!("unknown weapon `{raw}`"));
            assert_eq!(resolve_give_id(&none, raw, &[]), expected);
        }
        let expected = Err("unknown weapon `an94`".into());
        assert_eq!(resolve_give_id(&none, "an94", &words(&["reflex"])), expected);
    }

    #[test]
    fn no_name_at_all_is_an_empty_weapon_name() {
        let expected = Err("empty weapon name".into());
        assert_eq!(resolve_give_id(&WeaponLedger::default(), "", &[]), expected);
    }

    #[test]
    fn a_weapon_with_no_family_has_no_hints_and_no_toggles() {
        let none = WeaponLedger::default();
        let why = "`` belongs to no weapon family";
        assert_eq!(attachment_hints(&none, 7), Err(why.into()));
        assert_eq!(toggled_id(&none, 7, "reflex"), Err(why.into()));
    }

    #[test]
    fn an_empty_catalog_completes_nothing() {
        let weapons = StagedWeapons::default();
        assert!(weapon_completions(&weapons).is_empty());
        assert!(attach_completions(&weapons, 7).is_empty());
    }
}
