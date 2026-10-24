use super::{loc_ref, stats_col};
use crate::CapturedStringTable;

/// The five bars the class menu draws under a weapon.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum CacStatBar {
    Accuracy,
    Damage,
    Range,
    FireRate,
    Mobility,
}

impl CacStatBar {
    pub const ALL: [Self; 5] = [Self::Accuracy, Self::Damage, Self::Range, Self::FireRate, Self::Mobility];

    /// Where the stats table keeps this bar's value.
    fn column(self) -> i32 {
        22 + self as i32
    }
}

/// A weapon's caption, picture and bars as the stats table authors them.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CacWeaponPreview {
    pub reference: String,
    pub name_key: String,
    pub image: String,
    pub desc_key: String,
    pub bars: Vec<(CacStatBar, i32)>,
}

/// The stats-table row of a weapon, found by its reference with or without
/// the `_mp` suffix. `weapon_name` may be an asset key.
#[must_use]
pub fn weapon_preview(table: &CapturedStringTable, weapon_name: &str) -> Option<CacWeaponPreview> {
    let leaf = weapon_name.rsplit(['/', ':']).next().unwrap_or(weapon_name);
    let stem = leaf.strip_suffix("_mp").unwrap_or(leaf);
    let (row, reference) = [leaf, stem]
        .into_iter()
        .find_map(|candidate| Some((table.lookup_row_in_col(stats_col::REF, candidate)?, candidate)))?;

    let bars = CacStatBar::ALL
        .into_iter()
        .filter_map(|bar| Some((bar, table.cell(row, bar.column()).parse::<i32>().ok()?)))
        .collect();
    Some(CacWeaponPreview {
        reference: reference.to_owned(),
        name_key: loc_ref(table.cell(row, stats_col::NAME)),
        image: table.cell(row, stats_col::IMAGE).to_owned(),
        desc_key: loc_ref(table.cell(row, stats_col::DESC)),
        bars,
    })
}
