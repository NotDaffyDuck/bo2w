//! One material as the renderer sees it after load.

use asset_core::AssetNamespace;

use super::ids::{MaterialAssetId, RuntimeImageId, RuntimeTechniqueSetId, RuntimeTextureBinding};
use super::ids::TECHNIQUE_SLOT_COUNT;
use super::technique::RemapResolution;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RuntimeMaterial {
    pub asset_id: MaterialAssetId,
    pub name: String,
    pub namespace: AssetNamespace,
    pub technique_set: String,
    pub baked_draw_surf: Option<u64>,
    pub local_technique_set: RuntimeTechniqueSetId,
    pub remap: RemapResolution,
    pub state_bits_entry: Option<[u8; TECHNIQUE_SLOT_COUNT]>,
    pub state_bits_table: Vec<[u32; 2]>,
    pub camera_region: u8,
    pub sort_key: u8,
    pub info_game_flags: u8,
    pub state_flags: u8,
    pub surface_type_bits: Option<u32>,
    pub unlit: bool,
    pub uses_model_lighting_const: bool,
    pub square_color_map: bool,
    pub shadow_only: bool,
    pub t6_draw: Option<asset_core::T6Draw>,
    pub cull_mode: Option<u8>,
    pub uv_anim_bits: [u32; 4],
    pub falloff_parms_bits: [u32; 4],
    pub falloff_begin_bits: [u32; 4],
    pub falloff_end_bits: [u32; 4],
    pub env_map_parms_bits: [u32; 4],
    /// Sorted by name hash.
    pub textures: Vec<(u32, Option<RuntimeTextureBinding>)>,
    /// Sorted by name hash.
    pub constants: Vec<(u32, [u32; 4])>,
}

fn floats(bits: [u32; 4]) -> [f32; 4] {
    bits.map(f32::from_bits)
}

impl RuntimeMaterial {
    pub fn uv_anim(&self) -> [f32; 4] {
        floats(self.uv_anim_bits)
    }

    pub fn falloff_parms(&self) -> [f32; 4] {
        floats(self.falloff_parms_bits)
    }

    pub fn falloff_begin(&self) -> [f32; 4] {
        floats(self.falloff_begin_bits)
    }

    pub fn falloff_end(&self) -> [f32; 4] {
        floats(self.falloff_end_bits)
    }

    pub fn env_map_parms(&self) -> [f32; 4] {
        floats(self.env_map_parms_bits)
    }

    /// The first bound image carrying the given texture semantic.
    pub fn texture_semantic(&self, semantic: u8) -> Option<RuntimeImageId> {
        self.textures
            .iter()
            .filter_map(|(_, texture)| texture.as_ref())
            .find(|binding| binding.semantic == semantic)
            .map(|binding| binding.image)
    }
}
