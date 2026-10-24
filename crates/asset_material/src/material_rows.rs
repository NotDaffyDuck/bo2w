//! The rows a material population is made of: images, materials, and what
//! binds the one to the other.

use std::sync::Arc;

use bevy::prelude::Image;

use crate::{
    AssetNamespace, AssetRef, MaterialDrawRoute, TECHNIQUE_SLOT_COUNT, TechniqueTable, ZoneOwner,
};

pub const TS_2D: u8 = 0;
pub const TS_FUNCTION: u8 = 1;
pub const TS_COLOR_MAP: u8 = 2;
pub const TS_NORMAL_MAP: u8 = 5;
pub const TS_SPECULAR_MAP: u8 = 8;
pub const TS_WATER_MAP: u8 = 0x0B;

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
pub struct AuthoredImage {
    pub namespace: AssetNamespace,
    pub name: AssetRef,
    pub map_type: u8,
    pub semantic: u8,
    pub category: u8,
    pub use_srgb_reads: bool,
    pub width: u16,
    pub height: u16,
    pub depth: u16,
    pub level_count: u8,
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
pub struct MaterialTextureBinding {
    pub name_hash: u32,
    pub name_start: u8,
    pub name_end: u8,
    pub sampler_state: u8,
    pub semantic: u8,
    /// Row in the image list; `None` while the image is not in the population.
    pub image: Option<usize>,
}

#[derive(Clone, Debug)]
pub struct MaterialConstant {
    pub name_hash: u32,
    pub name: [u8; 12],
    pub literal: [f32; 4],
}

#[derive(Clone, Debug)]
pub struct AuthoredMaterial {
    pub name: AssetRef,
    pub namespace: AssetNamespace,
    /// The technique set the material names; Black Ops II rows leave it empty.
    pub technique_set: AssetRef,
    pub sort_key: u8,
    pub info_game_flags: u8,
    pub state_flags: u8,
    pub camera_region: u8,
    pub surface_type_bits: Option<u32>,
    /// The state-bit pairs the passes index.
    pub state_bits: Vec<[u32; 2]>,
    /// Which pair each technique slot uses.
    pub state_bits_entry: Option<[u8; TECHNIQUE_SLOT_COUNT]>,
    pub technique_table: Option<TechniqueTable>,
    pub route: Option<MaterialDrawRoute>,
    pub textures: Vec<MaterialTextureBinding>,
    pub constants: Vec<MaterialConstant>,
    pub zone: ZoneOwner,
    /// The draw the renderer follows; `None` on a placeholder row.
    pub t6_draw: Option<asset_core::T6Draw>,
}
