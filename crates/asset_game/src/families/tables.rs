//! Reading the authored tables: which attachments exist, which pairs clash,
//! and which families the stats table lists.

use std::collections::btree_map::Entry;
use std::collections::{BTreeMap, HashSet};

use super::{AttachmentChoice, FamilyContent, FamilyKey, FamilySlot, WeaponFamilies, WeaponFamily};
use crate::cac::stats_col;
use crate::{AssetCategoryScope, LoadoutPlacedGroup, LoadoutThrowableGroup, SnapshotStringTable};

/// Columns of `attachmentTable.csv`.
mod attach_col {
    pub const POINT: i32 = 2;
    pub const CAPTION: i32 = 3;
    pub const NAME: i32 = 4;
    pub const ICON: i32 = 6;
    pub const DESC: i32 = 7;
}

/// The stats table lists a weapon's attachment names in these columns.
const OFFERED_COLS: std::ops::RangeInclusive<i32> = 11..=21;

/// Everything the attachment tables say about one game.
#[derive(Clone, Debug, Default)]
pub(super) struct AttachmentRules {
    pub(super) known: BTreeMap<String, AttachmentChoice>,
    /// Unordered pairs that cannot share a weapon, stored smaller name first.
    clashes: HashSet<(String, String)>,
}

impl AttachmentRules {
    pub(super) fn allows_together(&self, first: &str, second: &str) -> bool {
        !self.clashes.contains(&pair_of(first, second))
    }

    fn read_attachments(&mut self, table: &SnapshotStringTable) {
        // A "none" attachment point marks a bookkeeping row, not an attachment.
        let authored = (1..table.height as i32).filter_map(|row| {
            let point = table.cell(row, attach_col::POINT);
            let name = table.cell(row, attach_col::NAME).to_ascii_lowercase();
            (point != "none" && !name.is_empty()).then(|| AttachmentChoice {
                name,
                point: point.to_owned(),
                caption_key: table.cell(row, attach_col::CAPTION).to_owned(),
                icon: table.cell(row, attach_col::ICON).to_owned(),
                description_lookup: table.cell(row, attach_col::DESC).to_owned(),
            })
        });
        for choice in authored {
            if let Entry::Vacant(spot) = self.known.entry(choice.name.clone()) {
                spot.insert(choice);
            }
        }
    }

    /// The combo table is a square grid; a "no" cell bans the row's name with
    /// the name that heads the cell's column.
    fn read_clashes(&mut self, table: &SnapshotStringTable) {
        let heading = |at: i32| table.cell(at, 0).to_ascii_lowercase();
        let banned = (1..table.height as i32).flat_map(|row| {
            (1..table.col_count as i32)
                .filter(move |&col| table.cell(row, col).eq_ignore_ascii_case("no"))
                .map(move |col| (row, col))
        });
        let pairs = banned
            .map(|(row, col)| (heading(row), heading(col)))
            .filter(|(left, right)| !left.is_empty() && !right.is_empty() && left != right)
            .map(|(left, right)| pair_of(&left, &right));
        self.clashes.extend(pairs);
    }
}

fn pair_of(a: &str, b: &str) -> (String, String) {
    let mut pair = [a.to_owned(), b.to_owned()];
    pair.sort();
    let [first, second] = pair;
    (first, second)
}

fn table_is(table: &SnapshotStringTable, designer_label: &str) -> bool {
    table.name.eq_ignore_ascii_case(designer_label)
}

fn slot_of(category: Option<LoadoutPlacedGroup>, offhand: Option<LoadoutThrowableGroup>) -> FamilySlot {
    match (category, offhand) {
        (Some(LoadoutPlacedGroup::Pistol | LoadoutPlacedGroup::AutoSidearm | LoadoutPlacedGroup::Thrown | LoadoutPlacedGroup::Special), _) => FamilySlot::Secondary,
        (Some(_), _) => FamilySlot::Primary,
        (None, Some(LoadoutThrowableGroup::Lethal)) => FamilySlot::Lethal,
        (None, Some(LoadoutThrowableGroup::Tactical)) => FamilySlot::Tactical,
        (None, None) => FamilySlot::Other,
    }
}

/// The weapon row's item group and base name, unless the row is not a weapon.
fn weapon_row(table: &SnapshotStringTable, row: i32) -> Option<(&str, &str)> {
    let group = table.cell(row, stats_col::GROUP);
    let base = table.cell(row, stats_col::REF);
    let is_weapon = group.starts_with("weapon_") && !matches!(base, "" | "weapon_null");
    is_weapon.then_some((group, base))
}

/// The attachments a stats row offers: alphabetical, each once.
fn offered_attachments(rules: &AttachmentRules, table: &SnapshotStringTable, row: i32) -> Vec<AttachmentChoice> {
    OFFERED_COLS
        .filter_map(|col| rules.known.get(&table.cell(row, col).to_ascii_lowercase()))
        .map(|choice| (choice.name.as_str(), choice))
        .collect::<BTreeMap<_, _>>()
        .into_values()
        .cloned()
        .collect()
}

/// Why a family stays off the class menu, if it does.
fn left_out_reason(
    key: &FamilyKey,
    group_label: &str,
    bay: FamilySlot,
    base_id: Option<u32>,
    content: &impl FamilyContent,
) -> Option<String> {
    let Some(id) = base_id else {
        return Some(format!("authored base `{}` is not in the loaded content", key.base));
    };
    content
        .admission(id)
        .err()
        .map(|refusal| refusal.to_string())
        .or_else(|| (bay == FamilySlot::Other).then(|| format!("`{group_label}` is not a class slot")))
}

impl WeaponFamilies {
    /// The first attachment table and the first combo table of each game win.
    pub(super) fn read_attachment_tables(&mut self, tables: &[(AssetCategoryScope, SnapshotStringTable)]) {
        let named = |authored: &'static str| tables.iter().filter(move |(_, table)| table_is(table, authored));
        for (tbl_scope, table) in named("mp/attachmentTable.csv") {
            if let std::collections::hash_map::Entry::Vacant(spot) = self.rules.entry(*tbl_scope) {
                spot.insert(AttachmentRules::default()).read_attachments(table);
            }
        }
        for (combo_scope, table) in named("mp/attachmentCombos.csv") {
            let rules = self.rules.entry(*combo_scope).or_default();
            if rules.clashes.is_empty() {
                rules.read_clashes(table);
            }
        }
    }

    /// One family per weapon row of a stats table; a base seen before is kept
    /// as first read.
    pub(super) fn read_family_rows(
        &mut self,
        namespace: AssetCategoryScope,
        table: &SnapshotStringTable,
        content: &impl FamilyContent,
    ) {
        let rules = self.rules.remove(&namespace).unwrap_or_default();
        let weapon_rows = (1..table.height as i32).filter_map(|row| Some((row, weapon_row(table, row)?)));
        for (stat_ix, (group, base)) in weapon_rows {
            let key = FamilyKey::new(namespace, base);
            if rules.known.contains_key(&key.base) || self.index.contains_key(&key) {
                continue;
            }
            let base_id = content.lookup(namespace, &key.base);
            let offhand = base_id
                .map(|id| content.secondary_hand_category(id))
                .and_then(crate::cac_offhand_bucket);
            let slot = slot_of(crate::loadout_category_of_group(group), offhand);
            let reason = left_out_reason(&key, group, slot, base_id, content);

            self.add_family(
                WeaponFamily {
                    loot_bundle: group.to_owned(),
                    bay: slot,
                    display_key: table.cell(stat_ix, stats_col::NAME).to_owned(),
                    image: table.cell(stat_ix, stats_col::IMAGE).to_owned(),
                    base: base_id,
                    attachments: offered_attachments(&rules, table, stat_ix),
                    key,
                },
                reason,
            );
        }
        self.rules.insert(namespace, rules);
    }

    /// Lists a family, and notes why it is left out of the class menu.
    fn add_family(&mut self, family: WeaponFamily, left_out: Option<String>) {
        if let Some(reason) = left_out {
            self.excluded.push((family.key.asset_key(), reason));
        }
        self.index.insert(family.key.clone(), self.families.len());
        self.families.push(family);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const T6: AssetCategoryScope = AssetCategoryScope::T6;
    const T5: AssetCategoryScope = AssetCategoryScope::T5;

    /// A table from rows of text; short rows are padded with empty cells.
    fn sheet(sheet_name: &str, rows: &[&[&str]]) -> SnapshotStringTable {
        let columns = rows.iter().map(|row| row.len()).max().unwrap_or(0);
        let cells = rows
            .iter()
            .flat_map(|row| (0..columns).map(|at| row.get(at).copied().unwrap_or("").to_owned()))
            .collect();
        SnapshotStringTable { name: sheet_name.into(), col_count: columns, height: rows.len(), grid: cells }
    }

    #[test]
    fn the_slot_follows_the_category_and_only_without_one_the_offhand() {
        let lethal = Some(LoadoutThrowableGroup::Lethal);
        let tactical = Some(LoadoutThrowableGroup::Tactical);
        let cases = [
            (Some(LoadoutPlacedGroup::Pistol), None, FamilySlot::Secondary),
            (Some(LoadoutPlacedGroup::AutoSidearm), tactical, FamilySlot::Secondary),
            (Some(LoadoutPlacedGroup::Thrown), None, FamilySlot::Secondary),
            (Some(LoadoutPlacedGroup::Special), lethal, FamilySlot::Secondary),
            (Some(LoadoutPlacedGroup::Assault), lethal, FamilySlot::Primary),
            (Some(LoadoutPlacedGroup::Cqb), None, FamilySlot::Primary),
            (None, lethal, FamilySlot::Lethal),
            (None, tactical, FamilySlot::Tactical),
            (None, None, FamilySlot::Other),
        ];
        for (category, offhand, expected) in cases {
            assert_eq!(slot_of(category, offhand), expected, "{category:?} {offhand:?}");
        }
    }

    #[test]
    fn a_pair_is_stored_the_same_whichever_way_round() {
        assert_eq!(pair_of("scope", "brake"), ("brake".to_owned(), "scope".to_owned()));
        assert_eq!(pair_of("brake", "scope"), pair_of("scope", "brake"));
        assert_eq!(pair_of("a", "a"), ("a".to_owned(), "a".to_owned()));
    }

    #[test]
    fn the_first_attachment_and_combo_tables_of_a_game_win() {
        let attachments = sheet(
            "mp/attachmentTable.csv",
            &[
                &["", "", "point", "caption", "name", "", "icon", "desc"],
                &["", "", "optic", "cap_scope", "Scope", "", "icon_scope", "desc_scope"],
                &["", "", "none", "", "filler", "", "", ""],
                &["", "", "muzzle", "", "", "", "", ""],
                &["", "", "muzzle", "cap_brake", "Brake", "", "icon_brake", "desc_brake"],
                &["", "", "underbarrel", "", "scope", "", "other", "other"],
            ],
        );
        let later = sheet(
            "MP/ATTACHMENTTABLE.CSV",
            &[&["", "", "point", "", "name"], &["", "", "optic", "", "extra"]],
        );
        let combos = sheet(
            "mp/attachmentCombos.csv",
            &[&["", "", "", ""], &["scope", "", "yes", "NO"], &["brake", "no", "", ""], &["Grip", "", "", "no"]],
        );
        let later_combos = sheet(
            "mp/attachmentCombos.csv",
            &[&["", "", "", ""], &["brake", "", "", "no"], &["x", "", "", ""], &["grip", "", "", ""]],
        );
        let mut families = WeaponFamilies::default();
        families.read_attachment_tables(&[
            (T6, attachments),
            (T6, later.clone()),
            (T5, later),
            (T6, combos),
            (T6, later_combos),
        ]);

        let t6 = &families.rules[&T6];
        assert_eq!(t6.known.keys().collect::<Vec<_>>(), ["brake", "scope"]);
        assert_eq!(t6.known["scope"].icon, "icon_scope");
        assert_eq!(t6.known["brake"].point, "muzzle");
        assert_eq!(families.rules[&T5].known.keys().collect::<Vec<_>>(), ["extra"]);

        assert!(!t6.allows_together("scope", "grip"));
        assert!(!t6.allows_together("grip", "scope"));
        assert!(!t6.allows_together("brake", "scope"));
        assert!(t6.allows_together("brake", "grip"));
        assert!(t6.allows_together("grip", "grip"));
        assert!(families.rules[&T5].allows_together("scope", "grip"));
    }

    #[test]
    fn a_stats_row_offers_its_known_attachments_once_in_alphabetical_order() {
        let mut rules = AttachmentRules::default();
        for name in ["acog", "grip", "mag"] {
            let choice = AttachmentChoice {
                name: name.into(),
                point: "p".into(),
                caption_key: String::new(),
                icon: String::new(),
                description_lookup: String::new(),
            };
            rules.known.insert(name.into(), choice);
        }
        let mut row = vec![""; 22];
        row[11..16].copy_from_slice(&["mag", "ACOG", "bogus", "acog", ""]);
        let table = sheet("mp/statsTable.csv", &[&["header"][..], &row[..]]);
        let offered: Vec<_> = offered_attachments(&rules, &table, 1).into_iter().map(|c| c.name).collect();
        assert_eq!(offered, ["acog", "mag"]);
        assert!(offered_attachments(&rules, &table, 0).is_empty());
    }

    #[test]
    fn only_weapon_rows_with_a_real_base_are_read() {
        let table = sheet(
            "mp/statsTable.csv",
            &[
                &["", "", "weapon_smg", "", "first_row"],
                &["", "", "weapon_smg", "", "pdw"],
                &["", "", "weapon_smg", "", ""],
                &["", "", "weapon_smg", "", "weapon_null"],
                &["", "", "perk_x", "", "other"],
            ],
        );
        let rows: Vec<_> = (0..5).map(|row| weapon_row(&table, row)).collect();
        assert_eq!(
            rows,
            [Some(("weapon_smg", "first_row")), Some(("weapon_smg", "pdw")), None, None, None]
        );
    }
}
