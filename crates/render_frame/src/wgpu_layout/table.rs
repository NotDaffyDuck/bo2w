//! The bindless texture table every pass shares.

use d3d9_sm3::{
    TEXTURE_TABLE_BINDING_2D, TEXTURE_TABLE_BINDING_3D, TEXTURE_TABLE_BINDING_CUBE,
    TEXTURE_TABLE_BINDING_SAMPLERS, TEXTURE_TABLE_GROUP,
};

use super::model::{TextureTableDimension, WgpuBindLayoutEntry, WgpuBindingKind, WgpuShaderVisibility};

pub const TEXTURE_TABLE_2D_CAPACITY: u32 = 8192;
pub const TEXTURE_TABLE_CUBE_CAPACITY: u32 = 256;
pub const TEXTURE_TABLE_3D_CAPACITY: u32 = 32;
pub const TEXTURE_TABLE_SAMPLER_CAPACITY: u32 = 128;

/// The table's four bindings: 2D textures, cube textures, 3D textures, samplers.
pub fn texture_table_bind_entries() -> [WgpuBindLayoutEntry; 4] {
    let group = u8::try_from(TEXTURE_TABLE_GROUP).expect("group 1");
    let slot = |binding: u32, kind: WgpuBindingKind| WgpuBindLayoutEntry {
        group,
        binding: u16::try_from(binding).expect("table binding fits u16"),
        visibility: WgpuShaderVisibility::VertexFragment,
        kind,
        register_packed: None,
    };
    let textures = |dimension, count| WgpuBindingKind::TextureArray { dimension, count };
    [
        slot(TEXTURE_TABLE_BINDING_2D, textures(TextureTableDimension::D2, TEXTURE_TABLE_2D_CAPACITY)),
        slot(TEXTURE_TABLE_BINDING_CUBE, textures(TextureTableDimension::Cube, TEXTURE_TABLE_CUBE_CAPACITY)),
        slot(TEXTURE_TABLE_BINDING_3D, textures(TextureTableDimension::D3, TEXTURE_TABLE_3D_CAPACITY)),
        slot(
            TEXTURE_TABLE_BINDING_SAMPLERS,
            WgpuBindingKind::SamplerArray { count: TEXTURE_TABLE_SAMPLER_CAPACITY },
        ),
    ]
}
