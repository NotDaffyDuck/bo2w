//! The bindless texture table every pass shares.

use d3d9_sm3::{
    TEX_TABLE_SLOT_2D, TEX_TABLE_SLOT_3D, TEX_TABLE_SLOT_CUBE,
    TEX_TABLE_SLOT_SAMPLERS, TEX_TABLE_BIND_GROUP,
};

use super::model::{TextureTableDimension, GpuBindSlotDescriptor, GpuBindResourceKind, GpuStageVisibility};

pub const FLAT_TEXTURE_TABLE_SLOTS: u32 = 8192;
pub const CUBEMAP_TEXTURE_TABLE_SIZE: u32 = 256;
pub const VOLUME_TEXTURE_TABLE_SIZE: u32 = 32;
pub const SAMPLER_TABLE_SIZE: u32 = 128;

/// The table's four bindings: 2D textures, cube textures, 3D textures, samplers.
pub fn table_bind_entries() -> [GpuBindSlotDescriptor; 4] {
    let group = u8::try_from(TEX_TABLE_BIND_GROUP).expect("group 1");
    let slot = |binding: u32, kind: GpuBindResourceKind| GpuBindSlotDescriptor {
        band: group,
        binding: u16::try_from(binding).expect("table binding is over u16::MAX"),
        visibility: GpuStageVisibility::VertexAndPixelStage,
        kind,
        register_packed: None,
    };
    let textures = |dim, count| GpuBindResourceKind::TextureList { dimension: dim, count };
    [
        slot(TEX_TABLE_SLOT_2D, textures(TextureTableDimension::D2, FLAT_TEXTURE_TABLE_SLOTS)),
        slot(TEX_TABLE_SLOT_CUBE, textures(TextureTableDimension::Cube, CUBEMAP_TEXTURE_TABLE_SIZE)),
        slot(TEX_TABLE_SLOT_3D, textures(TextureTableDimension::D3, VOLUME_TEXTURE_TABLE_SIZE)),
        slot(
            TEX_TABLE_SLOT_SAMPLERS,
            GpuBindResourceKind::SamplerArray { count: SAMPLER_TABLE_SIZE },
        ),
    ]
}
