//! The 3D texture that holds every model's lighting tile, and the queue of tile
//! uploads the render world copies into it.

use bevy::asset::RenderAssetUsages;
use bevy::prelude::*;
use bevy::render::render_resource::{
    Extent3d, TextureDimension, TextureFormat, TextureViewDescriptor, TextureViewDimension,
};
use lighting_t6::{
    MODEL_LIGHTING_ATLAS_DEPTH, MODEL_LIGHTING_ATLAS_WIDTH, MODEL_LIGHTING_TILE_BYTES,
    ModelLightingAtlasDims, ModelLightingTileIndex, model_lighting_write_tile_to_atlas,
};

#[derive(Resource)]
pub struct WorldModelLightingAtlas {
    pub image: Handle<Image>,
    pub dims: ModelLightingAtlasDims,
}

impl WorldModelLightingAtlas {
    pub fn new(images: &mut Assets<Image>, dims: ModelLightingAtlasDims) -> Self {
        Self { image: images.add(model_lighting_atlas_image(dims)), dims }
    }
}

/// An empty atlas: `MODEL_LIGHTING_ATLAS_WIDTH` x `dims.image_height` texels,
/// `MODEL_LIGHTING_ATLAS_DEPTH` layers deep, four bytes a texel.
pub fn model_lighting_atlas_image(dims: ModelLightingAtlasDims) -> Image {
    let size = Extent3d {
        width: MODEL_LIGHTING_ATLAS_WIDTH,
        height: dims.image_height,
        depth_or_array_layers: MODEL_LIGHTING_ATLAS_DEPTH,
    };
    let byte_len = size.width as usize * size.height as usize * size.depth_or_array_layers as usize * 4;
    let mut image = Image::new(
        size,
        TextureDimension::D3,
        vec![0; byte_len],
        TextureFormat::Rgba8Unorm,
        RenderAssetUsages::MAIN_WORLD | RenderAssetUsages::RENDER_WORLD,
    );
    image.texture_view_descriptor = Some(TextureViewDescriptor {
        dimension: Some(TextureViewDimension::D3),
        ..default()
    });
    image
}

/// Copies a tile into the CPU side of the atlas. `false` when the image is not
/// an atlas of these dimensions, has no CPU data, or the tile is out of range.
pub fn model_lighting_atlas_write_tile(
    atlas: &mut Image,
    dims: ModelLightingAtlasDims,
    entry: ModelLightingTileIndex,
    tile: &[u8; MODEL_LIGHTING_TILE_BYTES],
) -> bool {
    let size = atlas.texture_descriptor.size;
    let is_atlas = size.width == MODEL_LIGHTING_ATLAS_WIDTH
        && size.height == dims.image_height
        && size.depth_or_array_layers == MODEL_LIGHTING_ATLAS_DEPTH;
    match atlas.data.as_mut() {
        Some(data) if is_atlas => model_lighting_write_tile_to_atlas(data, dims.image_height, entry, tile),
        _ => false,
    }
}

/// Tiles written this frame that the render world still has to upload.
#[derive(Resource, Default)]
pub struct ModelLightingAtlasTileWrites {
    pub tiles: Vec<ModelLightingAtlasTileWrite>,
}

pub struct ModelLightingAtlasTileWrite {
    pub image: AssetId<Image>,
    pub origin: (u32, u32),
    pub texels: [u8; MODEL_LIGHTING_TILE_BYTES],
}

impl ModelLightingAtlasTileWrites {
    pub fn push(
        &mut self,
        image: AssetId<Image>,
        entry: ModelLightingTileIndex,
        tile: &[u8; MODEL_LIGHTING_TILE_BYTES],
    ) {
        self.tiles.push(ModelLightingAtlasTileWrite {
            image,
            origin: entry.texel_origin(),
            texels: *tile,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DIMS: ModelLightingAtlasDims = ModelLightingAtlasDims {
        xmodel_entry_limit: 1,
        smodel_entry_limit: 1,
        image_height: 8,
    };

    #[test]
    fn the_atlas_image_is_a_zeroed_3d_texture_of_the_stated_size() {
        let image = model_lighting_atlas_image(DIMS);
        let size = image.texture_descriptor.size;
        assert_eq!(
            (size.width, size.height, size.depth_or_array_layers),
            (MODEL_LIGHTING_ATLAS_WIDTH, 8, MODEL_LIGHTING_ATLAS_DEPTH)
        );
        assert_eq!(image.texture_descriptor.dimension, TextureDimension::D3);
        let data = image.data.expect("cpu data");
        assert_eq!(data.len(), (size.width * size.height * size.depth_or_array_layers * 4) as usize);
        assert!(data.iter().all(|&b| b == 0));
    }

    #[test]
    fn writing_a_tile_to_a_differently_sized_image_is_refused() {
        let mut other = model_lighting_atlas_image(ModelLightingAtlasDims { image_height: 16, ..DIMS });
        let tile = [1; MODEL_LIGHTING_TILE_BYTES];
        assert!(!model_lighting_atlas_write_tile(&mut other, DIMS, ModelLightingTileIndex { raw: 0 }, &tile));
    }

    #[test]
    fn writing_a_tile_to_an_image_with_no_cpu_data_is_refused() {
        let mut atlas = model_lighting_atlas_image(DIMS);
        atlas.data = None;
        let tile = [1; MODEL_LIGHTING_TILE_BYTES];
        assert!(!model_lighting_atlas_write_tile(&mut atlas, DIMS, ModelLightingTileIndex { raw: 0 }, &tile));
    }

    #[test]
    fn a_tile_that_fits_lands_in_the_atlas_data() {
        let mut atlas = model_lighting_atlas_image(DIMS);
        let tile = [0xAB; MODEL_LIGHTING_TILE_BYTES];
        assert!(model_lighting_atlas_write_tile(&mut atlas, DIMS, ModelLightingTileIndex { raw: 0 }, &tile));
        assert!(atlas.data.expect("cpu data").contains(&0xAB));
    }

    #[test]
    fn queued_writes_keep_the_tile_texels_and_the_entry_origin() {
        let mut writes = ModelLightingAtlasTileWrites::default();
        let entry = ModelLightingTileIndex { raw: 3 };
        writes.push(AssetId::default(), entry, &[7; MODEL_LIGHTING_TILE_BYTES]);
        assert_eq!(writes.tiles.len(), 1);
        assert_eq!(writes.tiles[0].origin, entry.texel_origin());
        assert!(writes.tiles[0].texels.iter().all(|&b| b == 7));
    }
}
