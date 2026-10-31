//! Which effect a bullet hit plays: one row per kind of hit, one cell per
//! surface the bullet can land on.

use fx_t6::runtime::{
    BODY_HIT_KIND_TOTAL, SOLID_HIT_KIND_TOTAL, MATERIAL_KIND_BODY, impact_table_row,
};

use crate::{AssetCategoryScope, FxName};

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
pub struct SelfContainedImpactFxRecord {
    /// By surface type; the 32nd is the riot shield.
    pub hard_hit: [String; SOLID_HIT_KIND_TOTAL],
    /// By flesh slot.
    pub body_hit: [String; BODY_HIT_KIND_TOTAL],
}

impl SelfContainedImpactFxRecord {
    /// The effect for a non-flesh `surf_type`, if the row has such a surface.
    /// An empty name means the surface plays nothing.
    pub fn nonflesh_name(&self, surf_type: usize) -> Option<&str> {
        self.hard_hit.get(surf_type).map(String::as_str)
    }

    /// The cell for a surface: by flesh slot when the surface is flesh.
    fn cell(&self, surface_kind: usize, body_material_slot: Option<usize>) -> Option<&str> {
        if surface_kind != MATERIAL_KIND_BODY {
            return self.nonflesh_name(surface_kind);
        }
        self.body_hit.get(body_material_slot?).map(String::as_str)
    }
}

/// A map's impact effect table with every cell already named.
#[derive(Clone, Debug, Default)]
pub struct SelfContainedImpactFxTable {
    /// Set for tables built from Black Ops II's own data.
    pub t6: bool,
    pub name: String,
    pub entries: Vec<SelfContainedImpactFxRecord>,
}

impl SelfContainedImpactFxTable {
    /// Builds the table from the zone's impact table, padding any row's flesh
    /// cells that are missing.
    pub fn from_t6(table: &asset_t6::ImpactTableRef) -> Self {
        let entries = table
            .rows
            .iter()
            .map(|(nonflesh, flesh)| SelfContainedImpactFxRecord {
                hard_hit: nonflesh.clone(),
                body_hit: std::array::from_fn(|slot| flesh.get(slot).cloned().unwrap_or_default()),
            })
            .collect();
        Self {
            t6: true,
            name: table.name.clone(),
            entries,
        }
    }

    pub fn line_total(&self) -> usize {
        self.entries.len()
    }

    /// The namespace the table's effect names belong to.
    pub fn namespace(&self) -> AssetCategoryScope {
        AssetCategoryScope::T6
    }

    /// The effect to play for `row` landing on `surf_type`. A flesh surface
    /// also needs the `body_material_slot`. Empty cells play nothing.
    pub fn effect_label(
        &self,
        row: usize,
        surf_kind: usize,
        body_material_slot: Option<usize>,
    ) -> Option<FxName<'_>> {
        let name = self.entries.get(row)?.cell(surf_kind, body_material_slot)?;
        Some(FxName::new(self.namespace(), name)).filter(|_| !name.is_empty())
    }

    /// The row to read for a kind of hit, on the way in or (with `exit`) out.
    pub fn hit_fx_row(&self, impact_kind_id: i32, outbound: bool) -> Option<usize> {
        let kind = if self.t6 && impact_kind_id == ARMOUR_PIERCING_HIT {
            LARGE_BULLET_HIT
        } else {
            impact_kind_id
        };
        impact_table_row(kind, outbound)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn table_with(nonflesh: [String; SOLID_HIT_KIND_TOTAL]) -> SelfContainedImpactFxTable {
        SelfContainedImpactFxTable::from_t6(&asset_t6::ImpactTableRef {
            name: "table".into(),
            rows: vec![(nonflesh, Default::default())],
        })
    }

    #[test]
    fn the_riot_shield_surface_is_named_and_the_next_one_is_not() {
        let mut nonflesh: [String; SOLID_HIT_KIND_TOTAL] = Default::default();
        nonflesh[2] = "brick".into();
        nonflesh[31] = "shield".into();
        let table = table_with(nonflesh);
        assert_eq!(table.effect_label(0, 2, None).map(|n| n.name), Some("brick"));
        assert_eq!(table.effect_label(0, 31, None).map(|n| n.name), Some("shield"));
        assert!(table.effect_label(0, 32, None).is_none());
        assert!(table.effect_label(0, 5, None).is_none());
        assert!(table.effect_label(1, 2, None).is_none());
    }

    #[test]
    fn a_flesh_surface_is_read_by_slot_and_needs_one() {
        let mut table = table_with(Default::default());
        table.entries[0].body_hit[1] = "blood".into();
        assert_eq!(table.effect_label(0, MATERIAL_KIND_BODY, Some(1)).map(|n| n.name), Some("blood"));
        assert!(table.effect_label(0, MATERIAL_KIND_BODY, Some(0)).is_none());
        assert!(table.effect_label(0, MATERIAL_KIND_BODY, None).is_none());
        assert!(table.effect_label(0, MATERIAL_KIND_BODY, Some(BODY_HIT_KIND_TOTAL)).is_none());
    }

    #[test]
    fn armour_piercing_hits_read_the_large_bullet_rows() {
        let table = table_with(Default::default());
        assert_eq!(table.hit_fx_row(3, false), Some(3));
        assert_eq!(table.hit_fx_row(3, true), Some(4));
        assert_eq!(table.hit_fx_row(2, true), Some(4));
        assert_eq!(table.hit_fx_row(15, false), Some(20));
    }

    #[test]
    fn a_table_not_from_black_ops_ii_keeps_the_armour_piercing_row() {
        let table = SelfContainedImpactFxTable::default();
        assert_eq!(table.hit_fx_row(3, false), impact_table_row(3, false));
    }
}
