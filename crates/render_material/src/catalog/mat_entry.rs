//! One material as the renderer sees it after load.

use asset_core::AssetCategoryScope;

use super::ids::{MaterialAssetKey, LiveImageId, LiveTechniqueBundleKey, LiveTextureBinding};
use super::ids::RENDER_RECIPE_SLOT_TOTAL;
use super::technique::MaterialRemapOutcome;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LiveMaterial {
    pub asset_id: MaterialAssetKey,
    pub name: String,
    pub namespace: AssetCategoryScope,
    pub method_set: String,
    pub baked_facet_draw: Option<u64>,
    pub local_technique_set: LiveTechniqueBundleKey,
    pub remap: MaterialRemapOutcome,
    pub state_row_for: Option<[u8; RENDER_RECIPE_SLOT_TOTAL]>,
    pub state_rows: Vec<[u32; 2]>,
    pub camera_zone: u8,
    pub draw_sort_key: u8,
    pub game_flag_info: u8,
    pub condition_bits: u8,
    pub facet_type_bits: Option<u32>,
    pub unlit: bool,
    pub uses_model_illum_const: bool,
    pub colour_map_is_square: bool,
    pub occluder_exclusive: bool,
    pub t6_draw: Option<asset_core::T6Draw>,
    pub cull_mode: Option<u8>,
    pub uv_scroll_bits: [u32; 4],
    pub attenuation_terms_raw: [u32; 4],
    pub attenuation_start_raw: [u32; 4],
    pub attenuation_stop_raw: [u32; 4],
    pub reflection_map_param_bits: [u32; 4],
    /// A cloud sheet's constants: 1 / `FadeFactor`, `ColorSpeed.xy`, `ColorScale.x` and `hdrAmount` squared.
    pub sheet_bits: [u32; 5],
    /// Sorted by name hash.
    pub textures: Vec<(u32, Option<LiveTextureBinding>)>,
    /// Sorted by name hash.
    pub constants: Vec<(u32, [u32; 4])>,
}

fn floats(bits: [u32; 4]) -> [f32; 4] {
    bits.map(f32::from_bits)
}

impl LiveMaterial {
    pub fn uv_motion(&self) -> [f32; 4] {
        floats(self.uv_scroll_bits)
    }

    pub fn attenuation_terms(&self) -> [f32; 4] {
        floats(self.attenuation_terms_raw)
    }

    pub fn attenuation_start(&self) -> [f32; 4] {
        floats(self.attenuation_start_raw)
    }

    pub fn attenuation_stop(&self) -> [f32; 4] {
        floats(self.attenuation_stop_raw)
    }

    pub fn env_map_params(&self) -> [f32; 4] {
        floats(self.reflection_map_param_bits)
    }

    pub fn sheet(&self) -> [f32; 5] {
        self.sheet_bits.map(f32::from_bits)
    }

    /// The first bound image carrying the given texture semantic.
    pub fn semantic_for_image(&self, semantic: u8) -> Option<LiveImageId> {
        self.textures
            .iter()
            .filter_map(|pair| pair.1.as_ref())
            .find(|binding| binding.semantic == semantic)
            .map(|binding| binding.image)
    }
}
