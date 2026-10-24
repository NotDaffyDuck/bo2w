//! The material catalog: every material, technique set and shader program,
//! plus the draw-sorted ordering built over them.

use asset_core::{AssetNamespace, AssetRef, MaterialIndex};

use super::ids::{
    MaterialAssetId, MaterialGenerationId, RuntimeShaderProgram, RuntimeShaderProgramId,
    SortedMaterialOrdinal,
};
use super::material::RuntimeMaterial;
use super::technique::RuntimeTechniqueSet;
use crate::RuntimeVertexDecl;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum RuntimeSortedMaterialTable {
    #[default]
    Missing,
    BuildFailed(CatalogBuildError),
    Ready {
        asset_ids_by_ordinal: Vec<MaterialAssetId>,
        ordinals_by_asset_id: Vec<Option<SortedMaterialOrdinal>>,
        skipped_n: u32,
        slot_gap_n: u32,
    },
}

impl RuntimeSortedMaterialTable {
    pub fn ordinal_for_asset_id(&self, asset_id: usize) -> Option<SortedMaterialOrdinal> {
        let Self::Ready {
            ordinals_by_asset_id,
            ..
        } = self
        else {
            return None;
        };
        ordinals_by_asset_id.get(asset_id).copied().flatten()
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RuntimeMaterialCatalog {
    pub generation_id: MaterialGenerationId,
    pub materials: Vec<RuntimeMaterial>,
    /// Sorted by name.
    pub material_indices_by_name: Vec<(String, usize)>,
    pub technique_sets: Vec<RuntimeTechniqueSet>,
    pub shader_programs: Vec<Option<RuntimeShaderProgram>>,
    /// Sorted by pointer identity.
    pub vertex_decls: Vec<(u32, RuntimeVertexDecl)>,
    pub sorted_materials: RuntimeSortedMaterialTable,
    pub sorted_first_skip: Option<String>,
    pub skip_tech: Option<String>,
    pub skip_cause: Option<String>,
    pub leftover_t5_arg_n: u32,
    pub leftover_t5_arg: Option<String>,
    pub leftover_t5_arg2: Option<String>,
    pub leftover_t5_arg3: Option<String>,
    pub leftover_t5_arg4: Option<String>,
    pub leftover_t5_arg5: Option<String>,
    pub leftover_t5_dest6: Option<String>,
    pub leftover_t5_dest7: Option<String>,
    pub leftover_t5_dest17: Option<String>,
    pub leftover_t5_dest18: Option<String>,
    pub leftover_t5_dest19: Option<String>,
    pub leftover_t5_dest9: Option<String>,
    pub leftover_t5_dest10: Option<String>,
    pub leftover_t5_dest11: Option<String>,
    pub leftover_t5_dest20: Option<String>,
    pub leftover_t5_dest24: Option<String>,
    pub leftover_t5_dest25: Option<String>,
    pub leftover_t5_dest26: Option<String>,
    pub leftover_t5_dest27: Option<String>,
    pub leftover_unknown_n: u32,
}

impl RuntimeMaterialCatalog {
    /// The material at an authored index, if the catalog row really is that one.
    pub fn derived(&self, id: MaterialIndex) -> Option<&RuntimeMaterial> {
        let order = id.order();
        self.materials
            .get(order)
            .filter(|row| usize::from(row.asset_id.0) == order)
    }

    pub fn ordinal_for_asset_id(&self, id: MaterialIndex) -> Option<SortedMaterialOrdinal> {
        self.derived(id)?;
        self.sorted_materials.ordinal_for_asset_id(id.order())
    }

    pub fn material_for_sorted_ordinal(&self, ordinal: u32) -> Option<&RuntimeMaterial> {
        let RuntimeSortedMaterialTable::Ready {
            asset_ids_by_ordinal,
            ..
        } = &self.sorted_materials
        else {
            return None;
        };
        let asset_id = *asset_ids_by_ordinal.get(usize::try_from(ordinal).ok()?)?;
        self.materials
            .get(usize::from(asset_id.0))
            .filter(|material| material.asset_id == asset_id)
    }

    pub fn ordinal_for_material_name(
        &self,
        name: impl AsRef<str>,
    ) -> Option<SortedMaterialOrdinal> {
        let material = self.material_for_name(name)?;
        self.sorted_materials
            .ordinal_for_asset_id(usize::from(material.asset_id.0))
    }

    pub fn material_for_name(&self, name: impl AsRef<str>) -> Option<&RuntimeMaterial> {
        let want = AssetRef::bare_name(name.as_ref());
        let found = self
            .material_indices_by_name
            .binary_search_by(|(candidate, _)| candidate.as_str().cmp(want))
            .ok()?;
        let index = self.material_indices_by_name[found].1;
        self.materials
            .get(index)
            .filter(|material| material.name == want)
    }

    pub fn shader_program(&self, id: RuntimeShaderProgramId) -> Option<&RuntimeShaderProgram> {
        let slot = usize::try_from(id.asset_slot).ok()?;
        self.shader_programs
            .get(slot)?
            .as_ref()
            .filter(|program| program.id == id)
    }

    pub fn vertex_decl(&self, pointer_identity: u32) -> Option<&RuntimeVertexDecl> {
        let found = self
            .vertex_decls
            .binary_search_by_key(&pointer_identity, |(identity, _)| *identity)
            .ok()?;
        Some(&self.vertex_decls[found].1)
    }
}

/// Why the catalog, or a table built over it, could not be assembled.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CatalogBuildError {
    TechniqueSlotCount {
        expected: usize,
        actual: usize,
    },
    MaterialAssetIdOverflow {
        index: usize,
    },
    SortedMaterialCapacity {
        count: usize,
        capacity: usize,
    },
    TechniqueSetMissing {
        material: MaterialAssetId,
    },
    TechniqueSetNamespaceMismatch {
        material: MaterialAssetId,
        want: AssetNamespace,
        got: AssetNamespace,
    },
    TechniqueGraphMissing {
        material: MaterialAssetId,
    },
    TechniqueBodyMissing {
        material: MaterialAssetId,
        slot: u8,
    },
    ShaderIdentityMissing {
        material: MaterialAssetId,
        slot: u8,
    },
    MaterialConstantMissing {
        material: MaterialAssetId,
        name_hash: u32,
    },
    LiteralPixelConstantMissing {
        material: MaterialAssetId,
        slot: u8,
    },
    ComparatorInvariant {
        material: MaterialAssetId,
        other: MaterialAssetId,
        slot: u8,
    },
    NonFiniteComparatorConstant {
        material: MaterialAssetId,
        slot: u8,
    },
}
