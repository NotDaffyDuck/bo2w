//! Model lighting tiles: small RGBA blocks the frame hands over, copied into the atlas
//! texture that models sample their baked lighting from.

use bevy::prelude::*;
use bevy::render::render_asset::RenderAssets;
use bevy::render::render_resource::{
    Extent3d, Origin3d, TexelCopyBufferLayout, TexelCopyTextureInfo, TextureAspect,
};
use bevy::render::renderer::RenderQueue;
use bevy::render::texture::GpuImage;
use bevy::render::{Render, RenderApp, RenderSystems};
use lighting_t6::{MODEL_LIGHT_ATLAS_LAYERS, ILLUM_TILE_LEN, ILLUM_TILE_SIDE};

/// Side of a tile in texels.
const TILE_SIDE: u32 = ILLUM_TILE_SIDE as u32;
/// Bytes in one row of a tile (RGBA8).
const TILE_ROW_BYTES: u32 = TILE_SIDE * 4;

/// One tile and where in which atlas it goes.
pub struct ModelLightingTileUpload {
    pub image: AssetId<Image>,
    pub origin: (u32, u32),
    pub texels: [u8; ILLUM_TILE_LEN],
}

/// Tiles waiting to be copied; drained every frame.
#[derive(Resource, Default)]
pub struct ModelLightingTileUploads {
    pub tiles: Vec<ModelLightingTileUpload>,
}

pub(super) fn register(host: &mut App) {
    if let Some(render_sub) = host.get_sub_app_mut(RenderApp) {
        render_sub
            .init_resource::<ModelLightingTileUploads>()
            .add_systems(
                Render,
                copy_tiles_to_atlas.in_set(RenderSystems::PrepareResources),
            );
    }
}

/// Writes one tile into its atlas texture. The copy covers the tile's whole depth range.
fn write_tile(gpu_queue: &RenderQueue, atlas: &GpuImage, tile: &ModelLightingTileUpload) {
    let (x, y) = tile.origin;
    let destination = TexelCopyTextureInfo {
        texture: &atlas.texture,
        mip_level: 0,
        origin: Origin3d { x, y, z: 0 },
        aspect: TextureAspect::All,
    };
    let origin_layout = TexelCopyBufferLayout {
        offset: 0,
        bytes_per_row: Some(TILE_ROW_BYTES),
        rows_per_image: Some(TILE_SIDE),
    };
    let extent = Extent3d {
        width: TILE_SIDE,
        height: TILE_SIDE,
        depth_or_array_layers: MODEL_LIGHT_ATLAS_LAYERS,
    };
    gpu_queue.write_texture(destination, &tile.texels, origin_layout, extent);
}

/// Runs after `PrepareAssets`. A tile whose atlas has no GPU texture yet is dropped: the
/// texture is then created from data that already contains it.
fn copy_tiles_to_atlas(
    mut pending: ResMut<ModelLightingTileUploads>,
    atlases: Res<RenderAssets<GpuImage>>,
    gpu_queue: Res<RenderQueue>,
) {
    for tile in pending.tiles.drain(..) {
        if let Some(atlas) = atlases.get(tile.image) {
            write_tile(&gpu_queue, atlas, &tile);
        }
    }
}
