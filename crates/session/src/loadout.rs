//! Turning a class as the player built it (names from the menu) into the
//! class the match runs (ids), and saying why a class can't be used.

use asset_game::{
    LoadoutPerkEntry, FamilyKey, LoadoutRules, WeaponLedger, WeaponSelection, default_class_perks,
};
use weapon_t6::combat_facts::GunBattleProfile;

/// A class as the menu holds it: weapon names, the attachments on the first
/// two and three perks (each a perk's reference, or empty for none).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ClassRow {
    /// Primary, secondary, lethal, tactical.
    pub weapons: [String; 4],
    /// Attachments on the primary and the secondary.
    pub mods: [Vec<String>; 2],
    pub specialties: [String; 3],
}

impl From<&frame::ServerKitSlot> for ClassRow {
    fn from(source: &frame::ServerKitSlot) -> Self {
        let weapons = [
            source.primary.clone(),
            source.secondary.clone(),
            source.lethal.clone(),
            source.tactical.clone(),
        ];
        let mods = [
            source.main_gun_parts.clone(),
            source.side_gun_parts.clone(),
        ];
        Self {
            weapons,
            mods,
            specialties: source.perks.clone(),
        }
    }
}

/// The class the match will run, and the reasons (if any) it is locked.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HostLoadoutMirror {
    pub def: sim::LoadoutClassSpec,
    pub frozen_because: Option<String>,
}

/// "item:code", the form every refusal in this module takes.
fn refusal(subject: &str, reason: &str) -> String {
    [subject, reason].join(":")
}

/// The weapon id for a named weapon with the attachments the class puts on
/// it. An empty name is an empty slot (id 0).
pub fn resolve_class_weapon(
    registry: &WeaponLedger,
    gun: &str,
    extras: &[String],
    rules: LoadoutRules,
) -> Result<u32, String> {
    if gun.is_empty() {
        if extras.is_empty() {
            return Ok(0u32);
        }
        return Err(String::from("weapon.unknown_family"));
    }
    let configure = |wanted: WeaponSelection| {
        let resolved = registry.resolve_configuration(&wanted, rules);
        resolved
            .map(|found| found.id)
            .map_err(|cause| refusal(gun, cause.code()))
    };
    if let Some(family) = FamilyKey::parse(gun)
        && registry.weapon_families().family(&family).is_some()
    {
        return configure(WeaponSelection::with(family, extras));
    }
    let Ok(Some(index)) = registry.follow_slot_link(gun) else {
        return Err(refusal(gun, "catalog.unknown"));
    };
    let base = registry.describe_configuration(index);
    if let Some(base) = base.filter(|described| described.family.is_some()) {
        let mut wanted = base.clone();
        for attachment in extras {
            if !wanted.attachments.contains(attachment) {
                wanted.attachments.push(attachment.clone());
            }
        }
        return configure(wanted);
    }
    if extras.len() > 0 {
        return Err(refusal(gun, "weapon.unknown_family"));
    }
    match registry.configuration_admission(index) {
        Ok(()) => Ok(index),
        Err(cause) => Err(refusal(gun, cause.code())),
    }
}

/// Whether an equipment slot (0 lethal, 1 tactical) holds the right kind of
/// offhand item.
fn offhand_fits(position: usize, kind: i32) -> bool {
    let accepted: &[i32] = match position {
        0 => &[1, 4, 5],
        1 => &[2, 3],
        _ => &[],
    };
    accepted.contains(&kind)
}

/// Why a class's weapons can't be used: a gun with no validated combat
/// profile, or an offhand item of the wrong kind or without runtime facts.
/// Several reasons are joined with "; ".
pub fn host_loadout_lock_cause(
    labels: [&str; 4],
    ids: [u32; 4],
    combat: &[GunBattleProfile],
    gear: &[sim::GadgetLiveStats],
) -> Option<String> {
    let mut problems: Vec<String> = Vec::new();
    for gun in 0..2 {
        let usable = combat
            .get(ids[gun] as usize)
            .copied()
            .is_some_and(GunBattleProfile::ready_for_use);
        if ids[gun] != 0 && !usable {
            problems.push(refusal(labels[gun], "validated.missing_profile"));
        }
    }
    for offhand in 0..2 {
        let gun_catalog_no = ids[offhand + 2];
        let fits = gear
            .get(gun_catalog_no as usize)
            .copied()
            .is_some_and(|known| {
                known.is_secondary_gear() && offhand_fits(offhand, known.secondary_hand_category)
            });
        if gun_catalog_no != 0 && !fits {
            problems.push(refusal(labels[offhand + 2], "offhand.missing_runtime_facts"));
        }
    }
    if problems.is_empty() {
        None
    } else {
        Some(problems.join("; "))
    }
}

/// A slot that names nothing.
fn is_unset(perk: &str) -> bool {
    matches!(perk, "" | "specialty_null")
}

/// The perk a stats table lists under `reference`, ignoring case.
fn find_perk<'a>(registry: &'a WeaponLedger, reference: &str) -> Option<&'a LoadoutPerkEntry> {
    let table = registry.weapon_families().perks();
    table
        .iter()
        .find(|entry| entry.reference.eq_ignore_ascii_case(reference))
}

/// The catalog number of a perk: its stats table item number.
pub fn perk_item_number(registry: &WeaponLedger, reference: &str) -> Option<u32> {
    let row = find_perk(registry, reference)?;
    Some(row.index)
}

fn resolve_perk(registry: &WeaponLedger, position: usize, reference: &str) -> Result<u32, String> {
    let Some(row) = find_perk(registry, reference) else {
        return Err(refusal(reference, "perk.unknown_catalog_entry"));
    };
    if row.bay.position() != position {
        return Err(refusal(reference, "perk.invalid_slot"));
    }
    Ok(row.index)
}

/// Which class slot (0, 1, 2) each perk of the stats table sits in, by its
/// item number: what the match checks a class's perks against.
pub fn class_perk_slots(registry: &WeaponLedger) -> Vec<(u32, u8)> {
    let mut pairs: Vec<(u32, u8)> = Vec::new();
    for row in registry.weapon_families().perks() {
        pairs.push((row.index, row.bay.position() as u8));
    }
    pairs
}

/// Gives a stock class the perks the stats table lists for it under its
/// name (`class_smg`...), when it names none of its own.
pub fn fill_default_perks(slot: &mut frame::ServerKitSlot, registry: &WeaponLedger) {
    if slot.perks.iter().any(|chosen| !is_unset(chosen)) {
        return;
    }
    let defaults = default_class_perks(registry.weapon_families().perks(), &slot.name);
    for (current, stock) in slot.perks.iter_mut().zip(defaults) {
        let Some(stock) = stock else {
            continue;
        };
        *current = stock.reference.clone();
    }
}

/// The ids behind a class's names, or the first thing that won't resolve.
pub fn resolve_personal_class(
    row: &ClassRow,
    registry: &WeaponLedger,
) -> Result<sim::PersonalClass, String> {
    let mut resolved = sim::PersonalClass::default();
    let rules = LoadoutRules::default();
    for (position, target) in resolved.weapons.iter_mut().enumerate() {
        let mods = row.mods.get(position).map_or(&[][..], |list| list.as_slice());
        *target = resolve_class_weapon(registry, &row.weapons[position], mods, rules)?;
    }
    for (position, reference) in row.specialties.iter().enumerate() {
        if is_unset(reference) {
            continue;
        }
        resolved.perks[position] = resolve_perk(registry, position, reference)?;
    }
    Ok(resolved)
}

/// The class `class_id` as the match will run it. A class that doesn't
/// resolve becomes an empty class with the reason it is locked.
pub fn project_loadout(
    class_number: u32,
    row: &ClassRow,
    weapons: &WeaponLedger,
    combat: &[GunBattleProfile],
    gear: &[sim::GadgetLiveStats],
) -> HostLoadoutMirror {
    let id = sim::ClassId(class_number);
    let built = resolve_personal_class(row, weapons).and_then(|personal| {
        personal
            .definition(id, 1)
            .ok_or_else(|| String::from("class.invalid_definition"))
    });
    let (mut def, mut reasons) = match built {
        Ok(def) => (def, Vec::new()),
        Err(reason) => (sim::LoadoutClassSpec::primary_secondary(id, 1, 0, 0), vec![reason]),
    };
    let weapon_names = row.weapons.each_ref().map(String::as_str);
    let weapon_ids = def.gun_slot_ids();
    let weapon_reasons = host_loadout_lock_cause(weapon_names, weapon_ids, combat, gear);
    reasons.extend(weapon_reasons);
    def.locked = !reasons.is_empty();
    let frozen_because = if reasons.is_empty() {
        None
    } else {
        Some(reasons.join("; "))
    };
    HostLoadoutMirror { def, frozen_because }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names<'a>(first: &'a str, second: &'a str, third: &'a str, fourth: &'a str) -> [&'a str; 4] {
        [first, second, third, fourth]
    }

    #[test]
    fn a_registry_without_a_stats_table_knows_no_perks() {
        let registry = WeaponLedger::default();
        let known = perk_item_number(&registry, "specialty_movefaster");
        assert_eq!(known, None);
        let nonsense = resolve_perk(&registry, 0, "nonsense");
        assert_eq!(nonsense, Err("nonsense:perk.unknown_catalog_entry".to_owned()));
        assert!(class_perk_slots(&registry).is_empty());
    }

    #[test]
    fn nothing_and_the_null_perk_are_unset() {
        assert!(is_unset("") && is_unset("specialty_null"));
        assert!(!is_unset("specialty_movefaster"));
    }

    #[test]
    fn a_stock_class_with_no_stats_table_keeps_its_empty_perks() {
        let mut slot = frame::ServerKitSlot::from(&frame::KitTemplate {
            name: "class_smg",
            primary: "",
            main_gun_parts: &[],
            secondary: "",
            side_gun_parts: &[],
            lethal: "",
            tactical: "",
            perks: ["", "", ""],
        });
        fill_default_perks(&mut slot, &WeaponLedger::default());
        assert!(slot.perks.iter().all(String::is_empty));
    }

    #[test]
    fn an_empty_weapon_slot_is_id_zero_but_not_with_attachments() {
        let registry = WeaponLedger::default();
        let rules = LoadoutRules::default();
        let bare = resolve_class_weapon(&registry, "", &[], rules);
        assert_eq!(bare, Ok(0));
        let optic = ["reflex".to_owned()];
        let with_optic = resolve_class_weapon(&registry, "", &optic, rules);
        assert_eq!(with_optic, Err("weapon.unknown_family".to_owned()));
    }

    #[test]
    fn a_weapon_the_catalog_lacks_is_refused() {
        let registry = WeaponLedger::default();
        let rules = LoadoutRules::default();
        let ghost = resolve_class_weapon(&registry, "ghost_mp", &[], rules);
        assert_eq!(ghost, Err("ghost_mp:catalog.unknown".to_owned()));
    }

    #[test]
    fn offhand_slots_take_only_their_own_kind() {
        for lethal in [1, 4, 5] {
            assert!(offhand_fits(0, lethal));
            assert!(!offhand_fits(1, lethal));
        }
        for tactical in [2, 3] {
            assert!(offhand_fits(1, tactical));
            assert!(!offhand_fits(0, tactical));
        }
        assert!(!offhand_fits(0, 0));
        assert!(!offhand_fits(2, 1));
    }

    #[test]
    fn empty_slots_are_never_a_lock_reason() {
        let reason = host_loadout_lock_cause(names("", "", "", ""), [0; 4], &[], &[]);
        assert_eq!(reason, None);
    }

    #[test]
    fn a_gun_without_a_profile_locks_the_class() {
        let reason =
            host_loadout_lock_cause(names("gun_a", "", "", ""), [5, 0, 0, 0], &[], &[]);
        assert_eq!(reason, Some("gun_a:validated.missing_profile".to_owned()));
    }

    #[test]
    fn an_offhand_without_facts_locks_the_class() {
        let reason =
            host_loadout_lock_cause(names("", "", "nade", ""), [0, 0, 7, 0], &[], &[]);
        assert_eq!(reason, Some("nade:offhand.missing_runtime_facts".to_owned()));
    }

    #[test]
    fn several_reasons_join_in_slot_order() {
        let joined =
            host_loadout_lock_cause(names("a", "b", "c", "d"), [1, 2, 3, 4], &[], &[]);
        let expected = "a:validated.missing_profile; b:validated.missing_profile; \
                        c:offhand.missing_runtime_facts; d:offhand.missing_runtime_facts";
        assert_eq!(joined.as_deref(), Some(expected));
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
        let projection = project_loadout(3, &row, &WeaponLedger::default(), &[], &[]);
        assert!(projection.def.locked);
        assert_eq!(
            projection.frozen_because.as_deref(),
            Some("ghost_mp:catalog.unknown")
        );
    }

    #[test]
    fn an_empty_class_resolves_to_empty_ids() {
        let empty = ClassRow::default();
        let resolved = resolve_personal_class(&empty, &WeaponLedger::default()).unwrap();
        assert_eq!(resolved.weapons, sim::PersonalClass::default().weapons);
        assert_eq!(resolved.perks, [0; 3]);
    }
}
