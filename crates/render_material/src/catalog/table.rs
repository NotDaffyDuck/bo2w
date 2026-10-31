//! The material catalog: every material, technique set and shader program,
//! plus the draw-sorted ordering built over them.

use asset_core::AssetCategoryScope;
use asset_core::AssetPointer;
use asset_core::SurfaceMaterialId;

use super::ids::{
    MaterialAssetKey, MaterialEpochId, ActiveShaderCombo, LiveShaderProgramId,
    SortedMaterialRank,
};
use super::mat_entry::LiveMaterial;
use super::technique::LiveTechniqueBundle;
use crate::LiveVertexFormat;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum LiveSortedMaterialTable {
    #[default]
    Missing,
    BuildFailed(LedgerBuildError),
    /// Built: the asset at each sorted place, the place of each asset, and how many
    /// were left out or left as gaps.
    Ready { asset_ids_by_ordinal: Vec<MaterialAssetKey>, ranks_by_asset_id: Vec<Option<SortedMaterialRank>>, skipped_total: u32, gap_total: u32 },
}

impl LiveSortedMaterialTable {
    pub fn rank_for_asset_id(&self, asset_id: usize) -> Option<SortedMaterialRank> {
        let Self::Ready {
            ranks_by_asset_id,
            ..
        } = self
        else {
            return None;
        };
        ranks_by_asset_id.get(asset_id).copied().flatten()
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LiveMaterialLedger {
    pub revision_stamp: MaterialEpochId, pub materials: Vec<LiveMaterial>,
    /// Sorted by name.
    pub material_slots_by_label: Vec<(String, usize)>, pub technique_sets: Vec<LiveTechniqueBundle>,
    pub shader_set: Vec<Option<ActiveShaderCombo>>,
    /// Sorted by pointer identity.
    pub decl_rows: Vec<(u32, LiveVertexFormat)>, pub ranked_materials: LiveSortedMaterialTable,
    pub first_ordered_skip_reason: Option<String>, pub dropped_recipes: Option<String>,
    pub drop_reasons: Option<String>,
}

impl LiveMaterialLedger {
    /// The material at an authored index, if the catalog row really is that one.
    pub fn derived(&self, id: SurfaceMaterialId) -> Option<&LiveMaterial> {
        let order = id.order();
        let rows = &self.materials;
        rows
            .get(order)
            .filter(|row| usize::from(row.asset_id.0) == order)
    }

    pub fn rank_for_asset_id(&self, id: SurfaceMaterialId) -> Option<SortedMaterialRank> {
        // Only a material the catalog holds has a place.
        self.derived(id).and_then(|_| self.ranked_materials.rank_for_asset_id(id.order()))
    }

    pub fn by_sort_position(&self, ordinal: u32) -> Option<&LiveMaterial> {
        let LiveSortedMaterialTable::Ready {
            asset_ids_by_ordinal: ids,
            ..
        } = &self.ranked_materials
        else {
            return None;
        };
        let key = *ids.get(usize::try_from(ordinal).ok()?)?;
        let held = self.materials.get(usize::from(key.0))?;
        (held.asset_id == key).then_some(held)
    }

    pub fn rank_for_material_name(&self, label: impl AsRef<str>) -> Option<SortedMaterialRank> {
        let at = usize::from(self.material_by_name(label)?.asset_id.0);
        self.ranked_materials.rank_for_asset_id(at)
    }

    pub fn material_by_name(&self, name: impl AsRef<str>) -> Option<&LiveMaterial> {
        let want = AssetPointer::bare_name(name.as_ref());
        // The label list is sorted by name; it gives the slot, the slot must still carry that name.
        let labels = &self.material_slots_by_label;
        let slot = labels[labels.binary_search_by(|(label, _)| label.as_str().cmp(want)).ok()?].1;
        self.materials.get(slot).filter(|held| held.name == want)
    }

    pub fn program_bytes(&self, id: LiveShaderProgramId) -> Option<&ActiveShaderCombo> {
        let slot = usize::try_from(id.asset_table_position).ok()?;
        let program = self.shader_set.get(slot)?.as_ref()?;
        (program.id == id).then_some(program)
    }

    pub fn decl_for(&self, address_fingerprint: u32) -> Option<&LiveVertexFormat> {
        let found = self
            .decl_rows
            .binary_search_by_key(&address_fingerprint, |pair| pair.0)
            .ok()?;
        Some(&self.decl_rows[found].1)
    }
}

/// Why the catalog, or a table built over it, could not be assembled.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LedgerBuildError {
    RenderTechniqueSlotTotal { want_slots: usize, got_slots: usize },
    MaterialAssetNumberTooLarge { index: usize },
    RankTableFull { count: usize, capacity: usize },
    TechniqueSetMissing { material: MaterialAssetKey },
    SetNamespaceDiffers { material: MaterialAssetKey, want: AssetCategoryScope, got: AssetCategoryScope },
    TechniqueGraphMissing { material: MaterialAssetKey },
    RecipeBodyAbsent { material: MaterialAssetKey, slot: u8 },
    ShaderPairAbsent { material: MaterialAssetKey, slot: u8 },
    LocalConstantAbsent { material: MaterialAssetKey, label_digest: u32 },
    LiteralFragmentAbsent { material: MaterialAssetKey, slot: u8 },
    CompareRuleContradiction { material: MaterialAssetKey, other: MaterialAssetKey, slot: u8 },
    CompareConstantNotFinite { material: MaterialAssetKey, slot: u8 },
}
