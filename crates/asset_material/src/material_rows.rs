//! The rows a material population is made of: images, materials, and what
//! binds the one to the other.

use std::sync::Arc;

use bevy::prelude::Image;

use crate::{
    AssetCategoryScope, AssetPointer, SortRoute, RENDER_RECIPE_SLOT_TOTAL, TechniqueSlotTable, ZoneCustodian,
};

pub const TS_2D: u8 = 0x00;
pub const SEMANTIC_FUNCTION: u8 = 0x01;
pub const SEMANTIC_COLOR: u8 = 0x02;
pub const SEMANTIC_NORMAL: u8 = 0x05;
pub const SEMANTIC_SPECULAR: u8 = 0x08;
pub const SEMANTIC_LIQUID: u8 = 11;

/// Which prepared variant of an image a decode produced.
///
/// A name alone cannot say whether one plan's decode would have answered
/// another's, because two sources can ship different images under one name.
/// The two halves keep the decode apart from what is wrapped around it: a
/// claim that matches on `payload` and differs on `usage` wanted the same
/// texels, so it is repeated work and not a second source.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ImageVariantId {
    /// Digest of everything that decides a byte of the decoded texels: the
    /// archive entries read, in order, and the map type (2D or cube).
    pub payload: u64,
    /// The options that leave the texels alone but change the image around
    /// them: the colour space the view reads in, and the sampler.
    pub usage: u32,
}

#[derive(Clone, Debug)]
pub struct HandBuiltTexture {
    pub namespace: AssetCategoryScope,
    pub name: AssetPointer,
    pub texture_kind: u8,
    pub semantic: u8,
    pub category: u8,
    pub srgb_sampled: bool,
    pub width: u16,
    pub height: u16,
    pub depth: u16,
    pub tier_total: u8,
    pub format: u32,
    pub payload: Arc<Vec<u8>>,
    pub decoded: Option<Arc<Image>>,
    pub common_owned: bool,
    /// The variant `decoded` is, when a decode plan produced it; `None` for
    /// an inline body or a row nothing has answered yet.
    pub decoded_variant: Option<ImageVariantId>,
}

/// One texture a material samples, by the name hash its shader asks for.
#[derive(Clone, Debug)]
pub struct SurfaceImageAttachment {
    pub label_digest: u32,
    pub label_head_offset: u8,
    pub label_tail_offset: u8,
    pub texture_sampling_mode: u8,
    pub semantic: u8,
    /// Row in the image list; `None` while the image is not in the population.
    pub image: Option<usize>,
}

#[derive(Clone, Debug)]
pub struct ShaderParameterValue {
    pub label_digest: u32,
    pub name: [u8; 12],
    pub literal: [f32; 4],
}

#[derive(Clone, Debug)]
pub struct DesignerMaterial {
    pub name: AssetPointer,
    pub namespace: AssetCategoryScope,
    /// The technique set the material names; Black Ops II rows leave it empty.
    pub method_set: AssetPointer,
    pub draw_order: u8,
    pub game_flag_info: u8,
    pub condition_bits: u8,
    pub camera_zone: u8,
    pub facet_type_bits: Option<u32>,
    /// The state-bit pairs the passes index.
    pub raster_bits: Vec<[u32; 2]>,
    /// Which pair each technique slot uses.
    pub raster_slot: Option<[u8; RENDER_RECIPE_SLOT_TOTAL]>,
    pub method_table: Option<TechniqueSlotTable>,
    pub route: Option<SortRoute>,
    pub textures: Vec<SurfaceImageAttachment>,
    pub constants: Vec<ShaderParameterValue>,
    pub zone: ZoneCustodian,
    /// The draw the renderer follows; `None` on a placeholder row.
    pub t6_draw: Option<asset_core::T6Draw>,
}
