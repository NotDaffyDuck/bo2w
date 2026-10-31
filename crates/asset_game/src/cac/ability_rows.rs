use super::{loc_ref, stats_col};
use crate::SnapshotStringTable;

/// Columns of the stats table only the perks use.
mod col {
    pub const INDEX: i32 = 0;
    /// The default classes the item is in: `class_smg class_custom_cqb`, each
    /// possibly followed by other words.
    pub const CLASSES: i32 = 11;
    /// The class slot: `specialty1`..`specialty3` for a perk.
    pub const SLOT: i32 = 13;
}

/// The class slots perks go in. BO2 has three and no deathstreak.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LoadoutAbilitySlot {
    Perk1,
    Second,
    Perk3,
}

impl LoadoutAbilitySlot {
    pub const ALL_KINDS: [Self; 3] = [Self::Perk1, Self::Second, Self::Perk3];

    fn from_cell(cell: &str) -> Option<Self> {
        match cell {
            "specialty1" => Some(Self::Perk1),
            "specialty2" => Some(Self::Second),
            "specialty3" => Some(Self::Perk3),
            _ => None,
        }
    }

    /// 0, 1 or 2: the position in a class's perk list.
    #[must_use]
    pub fn position(self) -> usize {
        self as usize
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LoadoutPerkEntry {
    /// The stats table's item number; what a class stores for the perk.
    pub index: u32,
    /// The specialties the perk gives, joined by `|`.
    pub reference: String,
    pub title_token: String,
    pub image: String,
    pub description_lookup: String,
    pub bay: LoadoutAbilitySlot,
    /// The default classes the perk is in (`class_smg`, `class_custom_lmg`).
    pub default_in: Vec<String>,
}

/// The perks a class can pick, in table order: the stats table's rows that
/// sit in a `specialty` slot. Rows with no reference are not offered.
#[must_use]
pub fn perk_rows(table: &SnapshotStringTable) -> Vec<LoadoutPerkEntry> {
    (0..table.height as i32).filter_map(|row| perk_at(table, row)).collect()
}

/// The perk a stats row describes, if it sits in a specialty slot and has
/// both an item number and a reference.
fn perk_at(table: &SnapshotStringTable, line_no: i32) -> Option<LoadoutPerkEntry> {
    let slot = LoadoutAbilitySlot::from_cell(table.cell(line_no, col::SLOT))?;
    let index = table.cell(line_no, col::INDEX).trim().parse().ok()?;
    let reference = Some(table.cell(line_no, stats_col::REF)).filter(|text| !text.is_empty())?;
    Some(LoadoutPerkEntry {
        index,
        bay: slot,
        reference: reference.into(),
        title_token: loc_ref(table.cell(line_no, stats_col::NAME)),
        image: table.cell(line_no, stats_col::IMAGE).into(),
        description_lookup: loc_ref(table.cell(line_no, stats_col::DESC)),
        default_in: default_classes(table.cell(line_no, col::CLASSES)),
    })
}

/// The default classes a classes cell names; other words are ignored.
fn default_classes(cell: &str) -> Vec<String> {
    cell.split_whitespace()
        .filter_map(|word| word.starts_with("class_").then(|| word.to_owned()))
        .collect()
}

/// The perks the stats table puts in the default class `class`, one per
/// slot (`None` where it names none).
#[must_use]
pub fn default_class_perks<'a>(stat_lines: &'a [LoadoutPerkEntry], class: &str) -> [Option<&'a LoadoutPerkEntry>; 3] {
    LoadoutAbilitySlot::ALL_KINDS.map(|slot| {
        stat_lines.iter()
            .find(|row| row.bay == slot && row.default_in.iter().any(|name| name == class))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn table(rows: &[[&str; 14]]) -> SnapshotStringTable {
        let cells = rows.concat().into_iter().map(String::from).collect();
        SnapshotStringTable { name: "mp/statstable.csv".into(), col_count: 14, height: rows.len(), grid: cells }
    }

    const LIGHT: [&str; 14] = [
        "150",
        "",
        "perk",
        "PERK_LIGHTWEIGHT",
        "specialty_movefaster|specialty_fallheight",
        "",
        "img_light",
        "PERK_LIGHTWEIGHT_DESC",
        "",
        "1",
        "0",
        "class_smg class_custom_cqb",
        "0",
        "specialty1",
    ];
    const FLAK: [&str; 14] = [
        "151",
        "",
        "perk",
        "PERK_FLAK",
        "specialty_flakjacket",
        "",
        "img_flak",
        "PERK_FLAK_DESC",
        "",
        "2",
        "0",
        "",
        "0",
        "specialty2",
    ];
    const GUN: [&str; 14] = [
        "10",
        "",
        "weapon_smg",
        "WEAPON_MP7",
        "mp7",
        "",
        "img",
        "d",
        "",
        "1",
        "0",
        "class_custom_smg",
        "0",
        "primary",
    ];

    #[test]
    fn only_rows_in_a_specialty_slot_are_perks() {
        let rows = perk_rows(&table(&[GUN, LIGHT, FLAK]));
        let refs: Vec<&str> = rows.iter().map(|row| row.reference.as_str()).collect();
        assert_eq!(refs, ["specialty_movefaster|specialty_fallheight", "specialty_flakjacket"]);
        assert_eq!(rows[0].index, 150);
        assert_eq!(rows[0].bay, LoadoutAbilitySlot::Perk1);
        assert_eq!(rows[1].bay, LoadoutAbilitySlot::Second);
        assert_eq!(rows[0].title_token, "@PERK_LIGHTWEIGHT");
        assert_eq!(rows[0].default_in, ["class_smg", "class_custom_cqb"]);
    }

    #[test]
    fn a_perk_row_needs_an_item_number_and_a_reference() {
        let mut no_reference = LIGHT;
        no_reference[4] = "";
        let mut spaced_number = FLAK;
        spaced_number[0] = " 151 ";
        let mut not_a_number = FLAK;
        not_a_number[0] = "x";
        let mut other_words = LIGHT;
        other_words[11] = "classy class_lmg  bonus class_";
        let rows = perk_rows(&table(&[no_reference, spaced_number, not_a_number, other_words]));
        assert_eq!(rows.iter().map(|row| row.index).collect::<Vec<_>>(), [151, 150]);
        assert_eq!(rows[1].default_in, ["class_lmg", "class_"]);
        assert!(rows[0].default_in.is_empty());
        assert_eq!((rows[0].image.as_str(), rows[0].description_lookup.as_str()), ("img_flak", "@PERK_FLAK_DESC"));
    }

    #[test]
    fn a_default_class_takes_the_perk_each_slot_lists_for_it() {
        let rows = perk_rows(&table(&[LIGHT, FLAK]));
        let smg = default_class_perks(&rows, "class_smg");
        assert_eq!(smg[0].map(|row| row.index), Some(150));
        assert!(smg[1].is_none() && smg[2].is_none());
        assert!(default_class_perks(&rows, "class_lmg").iter().all(Option::is_none));
    }
}
