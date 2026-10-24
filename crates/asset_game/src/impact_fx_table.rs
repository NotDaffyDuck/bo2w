//! Which effect a bullet hit plays: one row per kind of hit, one cell per
//! surface the bullet can land on.

use fx_t6::runtime::{
    FX_IMPACT_FLESH_COUNT, FX_IMPACT_NONFLESH_COUNT, FX_SURF_TYPE_FLESH, impact_table_row,
};

use crate::{AssetNamespace, FxName};

/// The kind of hit that the armour-piercing rounds of the Zombies guns
/// register as.
const ARMOUR_PIERCING_HIT: i32 = 3;

/// The kind of hit whose row an armour-piercing hit plays instead: the large
/// bullet's. Zombies' own armour-piercing row holds the 20 mm cannon's
/// hits (fireball, light, smoke out to 175 units), far too big for the HAMR,
/// LSAT, DSR-50 and SMR, which hit as large bullets as the campaign's own
/// armour-piercing hits are sized.
const LARGE_BULLET_HIT: i32 = 2;

/// One row of the table: the effect to play on each surface.
#[derive(Clone, Debug, Default)]
pub struct OwnedFxImpactEntry {
    /// By surface type; the 32nd is the riot shield.
    pub nonflesh: [String; FX_IMPACT_NONFLESH_COUNT],
    /// By flesh slot.
    pub flesh: [String; FX_IMPACT_FLESH_COUNT],
}

impl OwnedFxImpactEntry {
    /// The effect for a non-flesh `surf_type`, if the row has such a surface.
    /// An empty name means the surface plays nothing.
    pub fn nonflesh_name(&self, surf_type: usize) -> Option<&str> {
        self.nonflesh.get(surf_type).map(String::as_str)
    }
}

/// A map's impact effect table with every cell already named.
#[derive(Clone, Debug, Default)]
pub struct OwnedFxImpactTable {
    /// Set for tables built from Black Ops II's own data.
    pub t6: bool,
    pub name: String,
    pub entries: Vec<OwnedFxImpactEntry>,
}

impl OwnedFxImpactTable {
    /// Builds the table from the zone's impact table, padding any row's flesh
    /// cells that are missing.
    pub fn from_t6(table: &asset_t6::ImpactTableRef) -> Self {
        let entries = table
            .rows
            .iter()
            .map(|(nonflesh, flesh)| OwnedFxImpactEntry {
                nonflesh: nonflesh.clone(),
                flesh: std::array::from_fn(|slot| flesh.get(slot).cloned().unwrap_or_default()),
            })
            .collect();
        Self {
            t6: true,
            name: table.name.clone(),
            entries,
        }
    }

    pub fn row_count(&self) -> usize {
        self.entries.len()
    }

    /// The namespace the table's effect names belong to.
    pub fn namespace(&self) -> AssetNamespace {
        AssetNamespace::T6
    }

    /// The effect to play for `row` landing on `surf_type`. A flesh surface
    /// also needs the `flesh_slot`. Empty cells play nothing.
    pub fn effect_name(
        &self,
        row: usize,
        surf_type: usize,
        flesh_slot: Option<usize>,
    ) -> Option<FxName<'_>> {
        let entry = self.entries.get(row)?;
        let name = if surf_type == FX_SURF_TYPE_FLESH {
            entry.flesh.get(flesh_slot?)?.as_str()
        } else {
            entry.nonflesh_name(surf_type)?
        };
        (!name.is_empty()).then(|| FxName::new(self.namespace(), name))
    }

    /// The row to read for a kind of hit, on the way in or (with `exit`) out.
    pub fn impact_row(&self, impact_type: i32, exit: bool) -> Option<usize> {
        let kind = if self.t6 && impact_type == ARMOUR_PIERCING_HIT {
            LARGE_BULLET_HIT
        } else {
            impact_type
        };
        impact_table_row(kind, exit)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn table_with(nonflesh: [String; FX_IMPACT_NONFLESH_COUNT]) -> OwnedFxImpactTable {
        OwnedFxImpactTable::from_t6(&asset_t6::ImpactTableRef {
            name: "table".into(),
            rows: vec![(nonflesh, Default::default())],
        })
    }

    #[test]
    fn the_riot_shield_surface_is_named_and_the_next_one_is_not() {
        let mut nonflesh: [String; FX_IMPACT_NONFLESH_COUNT] = Default::default();
        nonflesh[2] = "brick".into();
        nonflesh[31] = "shield".into();
        let table = table_with(nonflesh);
        assert_eq!(table.effect_name(0, 2, None).map(|n| n.name), Some("brick"));
        assert_eq!(table.effect_name(0, 31, None).map(|n| n.name), Some("shield"));
        assert!(table.effect_name(0, 32, None).is_none());
        assert!(table.effect_name(0, 5, None).is_none());
        assert!(table.effect_name(1, 2, None).is_none());
    }

    #[test]
    fn armour_piercing_hits_read_the_large_bullet_rows() {
        let table = table_with(Default::default());
        assert_eq!(table.impact_row(3, false), Some(3));
        assert_eq!(table.impact_row(3, true), Some(4));
        assert_eq!(table.impact_row(2, true), Some(4));
        assert_eq!(table.impact_row(15, false), Some(20));
    }

    #[test]
    fn a_table_not_from_black_ops_ii_keeps_the_armour_piercing_row() {
        let table = OwnedFxImpactTable::default();
        assert_eq!(table.impact_row(3, false), impact_table_row(3, false));
    }
}
