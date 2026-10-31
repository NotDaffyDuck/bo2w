//! What the class menu reads out of the authored tables: weapon groups, the
//! stat bars and captions of a weapon, and the perk lists.

mod placed_group;
mod ability_rows;
mod gun_showcase;

pub use placed_group::{LoadoutPlacedGroup, loadout_category_of_group};
pub use ability_rows::{LoadoutPerkEntry, LoadoutAbilitySlot, default_class_perks, perk_rows};
pub use gun_showcase::{LoadoutStatMeter, LoadoutGunShowcase, gun_preview};

/// Columns of the weapon stats table.
pub(crate) mod stats_col {
    pub const GROUP: i32 = 2;
    pub const NAME: i32 = 3;
    pub const REF: i32 = 4;
    pub const IMAGE: i32 = 6;
    pub const DESC: i32 = 7;
}

/// Whether a table is the weapon stats table, whatever folder it sits in.
#[must_use]
pub fn names_scoreboard_sheet(name: &str) -> bool {
    let unified = name.replace('\\', "/");
    let base_file = unified.rsplit('/').next().unwrap_or(&unified);
    base_file.eq_ignore_ascii_case("statstable.csv")
}

/// `@KEY` for a localized string cell, empty when the cell is.
fn loc_ref(cell: &str) -> String {
    if cell.is_empty() {
        String::new()
    } else {
        format!("@{cell}")
    }
}
