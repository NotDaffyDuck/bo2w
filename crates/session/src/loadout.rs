//! Turning a class as the player built it (names from the menu) into the
//! class the match runs (ids), and saying why a class can't be used.

use asset_game::{FamilyKey, LoadoutRules, WeaponRegistry, WeaponSelection};
use sim::match_state::{
    CLASS_CATALOG_DEATHSTREAKS, CLASS_CATALOG_PERKS, perk_slot_from_class_catalog,
};
use weapon_t6::combat_facts::WeaponCombatFacts;

/// A class as the menu holds it: weapon names, the attachments on the first
/// two, three perks and a deathstreak.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ClassRow {
    /// Primary, secondary, lethal, tactical.
    pub weapons: [String; 4],
    /// Attachments on the primary and the secondary.
    pub attachments: [Vec<String>; 2],
    pub perks: [String; 3],
    pub deathstreak: String,
}

impl From<&frame::HostClassSlot> for ClassRow {
    fn from(slot: &frame::HostClassSlot) -> Self {
        Self {
            weapons: [
                slot.primary.clone(),
                slot.secondary.clone(),
                slot.lethal.clone(),
                slot.tactical.clone(),
            ],
            attachments: [
                slot.primary_attachments.clone(),
                slot.secondary_attachments.clone(),
            ],
            perks: slot.perks.clone(),
            deathstreak: slot.deathstreak.clone(),
        }
    }
}

/// The class the match will run, and the reasons (if any) it is locked.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AuthoritativeClassProjection {
    pub def: sim::ClassDef,
    pub lock_reason: Option<String>,
}

/// "item:code", the form every refusal in this module takes.
fn refusal(item: &str, code: &str) -> String {
    format!("{item}:{code}")
}

/// The weapon id for a named weapon with the attachments the class puts on
/// it. An empty name is an empty slot (id 0).
pub fn resolve_class_weapon(
    weapons: &WeaponRegistry,
    name: &str,
    attachments: &[String],
    rules: LoadoutRules,
) -> Result<u32, String> {
    if name.is_empty() {
        return match attachments.is_empty() {
            true => Ok(0),
            false => Err("weapon.unknown_family".to_owned()),
        };
    }
    let configure = |selection: WeaponSelection| {
        weapons
            .resolve_configuration(&selection, rules)
            .map(|resolved| resolved.id)
            .map_err(|why| refusal(name, why.code()))
    };
    if let Some(family) = FamilyKey::parse(name)
        && weapons.weapon_families().family(&family).is_some()
    {
        return configure(WeaponSelection::with(family, attachments));
    }
    let Ok(Some(id)) = weapons.resolve_index(name) else {
        return Err(refusal(name, "catalog.unknown"));
    };
    match weapons.describe_configuration(id) {
        Some(described) if described.family.is_some() => {
            let mut selection = described.clone();
            for extra in attachments {
                if !selection.attachments.contains(extra) {
                    selection.attachments.push(extra.clone());
                }
            }
            configure(selection)
        }
        _ if attachments.is_empty() => weapons
            .configuration_admission(id)
            .map(|()| id)
            .map_err(|why| refusal(name, why.code())),
        _ => Err(refusal(name, "weapon.unknown_family")),
    }
}

/// Whether an equipment slot (0 lethal, 1 tactical) holds the right kind of
/// offhand item.
fn offhand_fits(slot: usize, offhand_class: i32) -> bool {
    match slot {
        0 => matches!(offhand_class, 1 | 4 | 5),
        1 => matches!(offhand_class, 2 | 3),
        _ => false,
    }
}

/// Why a class's weapons can't be used: a gun with no validated combat
/// profile, or an offhand item of the wrong kind or without runtime facts.
/// Several reasons are joined with "; ".
pub fn authoritative_class_lock_reason(
    names: [&str; 4],
    ids: [u32; 4],
    combat: &[WeaponCombatFacts],
    equipment: &[sim::EquipmentRuntimeFacts],
) -> Option<String> {
    let mut reasons = Vec::new();
    for slot in 0..2 {
        let has_profile = combat
            .get(ids[slot] as usize)
            .copied()
            .is_some_and(WeaponCombatFacts::is_usable);
        if ids[slot] != 0 && !has_profile {
            reasons.push(refusal(names[slot], "validated.missing_profile"));
        }
    }
    for slot in 0..2 {
        let id = ids[2 + slot];
        let fits = equipment
            .get(id as usize)
            .copied()
            .is_some_and(|facts| facts.is_offhand() && offhand_fits(slot, facts.offhand_class));
        if id != 0 && !fits {
            reasons.push(refusal(names[2 + slot], "offhand.missing_runtime_facts"));
        }
    }
    (!reasons.is_empty()).then(|| reasons.join("; "))
}

/// The catalog number of a perk (1-based), by its reference name.
pub fn perk_catalog_id(reference: &str) -> Option<u32> {
    let index = CLASS_CATALOG_PERKS
        .iter()
        .position(|perk| perk.eq_ignore_ascii_case(reference))?;
    u32::try_from(index + 1).ok()
}

/// A slot that names nothing.
fn is_unset(name: &str) -> bool {
    name.is_empty() || name == "specialty_null"
}

fn resolve_perk(slot: usize, name: &str) -> Result<u32, String> {
    let id = perk_catalog_id(name).ok_or_else(|| refusal(name, "perk.unknown_catalog_entry"))?;
    match perk_slot_from_class_catalog(id) == Some(slot) {
        true => Ok(id),
        false => Err(refusal(name, "perk.invalid_slot")),
    }
}

fn resolve_deathstreak(name: &str) -> Result<u8, String> {
    CLASS_CATALOG_DEATHSTREAKS
        .iter()
        .position(|candidate| *candidate == name)
        .and_then(|index| u8::try_from(index + 1).ok())
        .ok_or_else(|| refusal(name, "deathstreak.unknown_catalog_entry"))
}

/// The ids behind a class's names, or the first thing that won't resolve.
pub fn resolve_personal_class(
    row: &ClassRow,
    registry: &WeaponRegistry,
) -> Result<sim::PersonalClass, String> {
    let mut class = sim::PersonalClass::default();
    let rules = LoadoutRules::for_class(&row.perks[0]);
    for (slot, id) in class.weapons.iter_mut().enumerate() {
        let attachments = row.attachments.get(slot).map_or(&[][..], Vec::as_slice);
        *id = resolve_class_weapon(registry, &row.weapons[slot], attachments, rules)?;
    }
    for (slot, name) in row.perks.iter().enumerate() {
        if !is_unset(name) {
            class.perks[slot] = resolve_perk(slot, name)?;
        }
    }
    if !is_unset(&row.deathstreak) {
        class.deathstreak = resolve_deathstreak(&row.deathstreak)?;
    }
    Ok(class)
}

/// The class `class_id` as the match will run it. A class that doesn't
/// resolve becomes an empty class with the reason it is locked.
pub fn project_class(
    class_id: u32,
    row: &ClassRow,
    weapons: &WeaponRegistry,
    combat: &[WeaponCombatFacts],
    equipment: &[sim::EquipmentRuntimeFacts],
) -> AuthoritativeClassProjection {
    let id = sim::ClassId(class_id);
    let (mut def, mut reasons) = match resolve_personal_class(row, weapons).and_then(|class| {
        class
            .definition(id, 1)
            .ok_or_else(|| "class.invalid_definition".to_owned())
    }) {
        Ok(def) => (def, Vec::new()),
        Err(reason) => (sim::ClassDef::primary_secondary(id, 1, 0, 0), vec![reason]),
    };
    let names = row.weapons.each_ref().map(String::as_str);
    reasons.extend(authoritative_class_lock_reason(
        names,
        def.weapon_slot_ids(),
        combat,
        equipment,
    ));
    def.locked = !reasons.is_empty();
    AuthoritativeClassProjection {
        def,
        lock_reason: (!reasons.is_empty()).then(|| reasons.join("; ")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names<'a>(a: &'a str, b: &'a str, c: &'a str, d: &'a str) -> [&'a str; 4] {
        [a, b, c, d]
    }

    #[test]
    fn perks_are_numbered_from_one_ignoring_case() {
        let first = CLASS_CATALOG_PERKS[0];
        assert_eq!(perk_catalog_id(first), Some(1));
        assert_eq!(perk_catalog_id(&first.to_ascii_uppercase()), Some(1));
        assert_eq!(perk_catalog_id("no_such_perk"), None);
    }

    #[test]
    fn deathstreaks_are_numbered_from_one() {
        assert_eq!(resolve_deathstreak(CLASS_CATALOG_DEATHSTREAKS[0]), Ok(1));
        assert_eq!(
            resolve_deathstreak("nope"),
            Err("nope:deathstreak.unknown_catalog_entry".to_owned())
        );
    }

    #[test]
    fn a_perk_must_sit_in_its_own_slot() {
        let id = perk_catalog_id(CLASS_CATALOG_PERKS[0]).unwrap();
        let home = perk_slot_from_class_catalog(id).unwrap();
        let elsewhere = (home + 1) % 3;
        assert_eq!(resolve_perk(home, CLASS_CATALOG_PERKS[0]), Ok(id));
        assert_eq!(
            resolve_perk(elsewhere, CLASS_CATALOG_PERKS[0]),
            Err(format!("{}:perk.invalid_slot", CLASS_CATALOG_PERKS[0]))
        );
    }

    #[test]
    fn an_unknown_perk_is_refused_by_name() {
        assert_eq!(
            resolve_perk(0, "nonsense"),
            Err("nonsense:perk.unknown_catalog_entry".to_owned())
        );
    }

    #[test]
    fn an_empty_weapon_slot_is_id_zero_but_not_with_attachments() {
        let registry = WeaponRegistry::default();
        let rules = LoadoutRules::for_class("");
        assert_eq!(resolve_class_weapon(&registry, "", &[], rules), Ok(0));
        assert_eq!(
            resolve_class_weapon(&registry, "", &["reflex".to_owned()], rules),
            Err("weapon.unknown_family".to_owned())
        );
    }

    #[test]
    fn a_weapon_the_catalog_lacks_is_refused() {
        let registry = WeaponRegistry::default();
        let rules = LoadoutRules::for_class("");
        assert_eq!(
            resolve_class_weapon(&registry, "ghost_mp", &[], rules),
            Err("ghost_mp:catalog.unknown".to_owned())
        );
    }

    #[test]
    fn offhand_slots_take_only_their_own_kind() {
        for class in [1, 4, 5] {
            assert!(offhand_fits(0, class));
            assert!(!offhand_fits(1, class));
        }
        for class in [2, 3] {
            assert!(offhand_fits(1, class));
            assert!(!offhand_fits(0, class));
        }
        assert!(!offhand_fits(0, 0));
        assert!(!offhand_fits(2, 1));
    }

    #[test]
    fn empty_slots_are_never_a_lock_reason() {
        assert_eq!(
            authoritative_class_lock_reason(names("", "", "", ""), [0; 4], &[], &[]),
            None
        );
    }

    #[test]
    fn a_gun_without_a_profile_locks_the_class() {
        assert_eq!(
            authoritative_class_lock_reason(names("gun_a", "", "", ""), [5, 0, 0, 0], &[], &[]),
            Some("gun_a:validated.missing_profile".to_owned())
        );
    }

    #[test]
    fn an_offhand_without_facts_locks_the_class() {
        assert_eq!(
            authoritative_class_lock_reason(names("", "", "nade", ""), [0, 0, 7, 0], &[], &[]),
            Some("nade:offhand.missing_runtime_facts".to_owned())
        );
    }

    #[test]
    fn several_reasons_join_in_slot_order() {
        assert_eq!(
            authoritative_class_lock_reason(names("a", "b", "c", "d"), [1, 2, 3, 4], &[], &[]),
            Some(
                "a:validated.missing_profile; b:validated.missing_profile; \
                 c:offhand.missing_runtime_facts; d:offhand.missing_runtime_facts"
                    .to_owned()
            )
        );
    }

    #[test]
    fn a_class_that_does_not_resolve_is_locked_with_the_reason() {
        let row = ClassRow {
            weapons: [
                "ghost_mp".to_owned(),
                String::new(),
                String::new(),
                String::new(),
            ],
            ..ClassRow::default()
        };
        let projection = project_class(3, &row, &WeaponRegistry::default(), &[], &[]);
        assert!(projection.def.locked);
        assert_eq!(
            projection.lock_reason.as_deref(),
            Some("ghost_mp:catalog.unknown")
        );
    }

    #[test]
    fn an_empty_class_resolves_to_empty_ids() {
        let class =
            resolve_personal_class(&ClassRow::default(), &WeaponRegistry::default()).unwrap();
        assert_eq!(class.weapons, sim::PersonalClass::default().weapons);
        assert_eq!(class.deathstreak, 0);
    }
}
