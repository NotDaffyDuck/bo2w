use super::loc_ref;
use crate::CapturedStringTable;

/// Columns of the perk table.
mod col {
    pub const REF: i32 = 1;
    pub const NAME: i32 = 2;
    pub const ICON: i32 = 3;
    pub const DESC: i32 = 4;
    pub const SLOT: i32 = 5;
}

/// The class slots perks go in; the fourth is the deathstreak.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CacPerkSlot {
    Perk1,
    Perk2,
    Perk3,
    Deathstreak,
}

impl CacPerkSlot {
    fn from_cell(cell: &str) -> Option<Self> {
        match cell {
            "perk1" => Some(Self::Perk1),
            "perk2" => Some(Self::Perk2),
            "perk3" => Some(Self::Perk3),
            "perk4" => Some(Self::Deathstreak),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CacPerkRow {
    pub reference: String,
    pub name_key: String,
    pub image: String,
    pub desc_key: String,
    pub slot: CacPerkSlot,
}

/// The perks a class can pick, in table order. Rows with no slot, no
/// reference or an underscore-led reference are not offered.
#[must_use]
pub fn perk_rows(table: &CapturedStringTable) -> Vec<CacPerkRow> {
    (0..table.rows as i32)
        .filter_map(|row| {
            let slot = CacPerkSlot::from_cell(table.cell(row, col::SLOT))?;
            let reference = table.cell(row, col::REF);
            if reference.is_empty() || reference.starts_with('_') {
                return None;
            }
            Some(CacPerkRow {
                reference: reference.to_owned(),
                name_key: loc_ref(table.cell(row, col::NAME)),
                image: table.cell(row, col::ICON).to_owned(),
                desc_key: loc_ref(table.cell(row, col::DESC)),
                slot,
            })
        })
        .collect()
}
