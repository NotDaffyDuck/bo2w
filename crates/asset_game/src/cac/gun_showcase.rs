use super::{loc_ref, stats_col};
use crate::SnapshotStringTable;

/// The five bars the class menu draws under a weapon.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum LoadoutStatMeter {
    Accuracy,
    Damage,
    Range,
    ShotPace,
    Mobility,
}

impl LoadoutStatMeter {
    pub const ALL_KINDS: [Self; 5] = [Self::Accuracy, Self::Damage, Self::Range, Self::ShotPace, Self::Mobility];

    /// Where the stats table keeps this bar's value.
    fn column(self) -> i32 {
        22 + self as i32
    }
}

/// A weapon's caption, picture and bars as the stats table authors them.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LoadoutGunShowcase {
    pub reference: String,
    pub title_token: String,
    pub image: String,
    pub description_lookup: String,
    pub bars: Vec<(LoadoutStatMeter, i32)>,
}

/// The stats-table row whose reference is the weapon's name, either bare or carrying
/// the `_mp` suffix, and the spelling that matched. The name may be an asset key.
fn find_row<'a>(table: &SnapshotStringTable, weapon_name: &'a str) -> Option<(i32, &'a str)> {
    let leaf = weapon_name.rsplit_once(['/', ':']).map_or(weapon_name, |(_, leaf)| leaf);
    std::iter::once(leaf)
        .chain(leaf.strip_suffix("_mp"))
        .find_map(|spelling| Some((table.row_where_cell(stats_col::REF, spelling)?, spelling)))
}

/// The bars a row gives a number for.
fn bars_at(table: &SnapshotStringTable, row: i32) -> Vec<(LoadoutStatMeter, i32)> {
    LoadoutStatMeter::ALL_KINDS
        .into_iter()
        .filter_map(|bar| table.cell(row, bar.column()).parse().ok().map(|value| (bar, value)))
        .collect()
}

/// The stats-table row of a weapon, found by its reference with or without
/// the `_mp` suffix. `weapon_name` may be an asset key.
#[must_use]
pub fn gun_preview(table: &SnapshotStringTable, weapon_name: &str) -> Option<LoadoutGunShowcase> {
    let (line_no, reference) = find_row(table, weapon_name)?;
    Some(LoadoutGunShowcase {
        reference: reference.into(),
        title_token: loc_ref(table.cell(line_no, stats_col::NAME)),
        image: table.cell(line_no, stats_col::IMAGE).into(),
        description_lookup: loc_ref(table.cell(line_no, stats_col::DESC)),
        bars: bars_at(table, line_no),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const WIDTH: usize = 27;

    /// A stats table of the given rows; each row is a list of (column, text).
    fn stats(rows: &[&[(usize, &str)]]) -> SnapshotStringTable {
        let mut cells = vec![String::new(); WIDTH * rows.len()];
        for (at, cell_set) in rows.iter().enumerate() {
            for (col, text) in cell_set.iter() {
                cells[at * WIDTH + col] = (*text).into();
            }
        }
        SnapshotStringTable { name: "mp/statstable.csv".into(), col_count: WIDTH, height: rows.len(), grid: cells }
    }

    fn table() -> SnapshotStringTable {
        stats(&[
            &[(4, "ref")],
            &[(3, "WEAPON_AN94"), (4, "an94"), (6, "img_an94"), (7, "AN94_DESC"), (22, "7"), (23, "5"), (24, "x"), (26, "3")],
            &[(4, "kard_mp"), (22, "-2")],
        ])
    }

    #[test]
    fn a_weapon_is_found_by_name_asset_key_or_with_its_suffix() {
        let table = table();
        let preview = gun_preview(&table, "an94").unwrap();
        assert_eq!(preview.reference, "an94");
        assert_eq!((preview.title_token.as_str(), preview.image.as_str()), ("@WEAPON_AN94", "img_an94"));
        assert_eq!(preview.description_lookup, "@AN94_DESC");
        assert_eq!(gun_preview(&table, "t6:weapon/an94_mp"), Some(preview));
        assert_eq!(gun_preview(&table, "kard").map(|p| p.reference), None);
        assert_eq!(gun_preview(&table, "kard_mp").map(|p| p.reference), Some("kard_mp".to_owned()));
        assert_eq!(gun_preview(&table, "nothing"), None);
    }

    #[test]
    fn only_bars_with_a_number_are_kept_in_bar_order() {
        let table = table();
        let bars = |name| gun_preview(&table, name).unwrap().bars;
        assert_eq!(bars("an94"), [(LoadoutStatMeter::Accuracy, 7), (LoadoutStatMeter::Damage, 5), (LoadoutStatMeter::Mobility, 3)]);
        assert_eq!(bars("kard_mp"), [(LoadoutStatMeter::Accuracy, -2)]);
        assert_eq!(LoadoutStatMeter::ALL_KINDS.map(LoadoutStatMeter::column), [22, 23, 24, 25, 26]);
    }
}
