//! The 3D texture that holds every model's lighting tile, and the queue of tile
//! uploads the render world copies into it.

use bevy::asset::RenderAssetUsages;
use bevy::prelude::*;
use bevy::render::render_resource;
use lighting_t6::ILLUM_TILE_LEN;
use lighting_t6::MODEL_LIGHT_ATLAS_LAYERS;
use lighting_t6::MODEL_LIGHT_ATLAS_TEXELS_ACROSS;
use lighting_t6::ModelIllumAtlasDims;
use lighting_t6::IllumTileNo;
use lighting_t6::blit_illum_tile;

#[derive(Resource)]
pub struct LevelModelIllumAtlas {
    pub image: Handle<Image>,
    pub dims: ModelIllumAtlasDims,
}

impl LevelModelIllumAtlas {
    pub fn new(store: &mut Assets<Image>, dims: ModelIllumAtlasDims) -> Self {
        let image = store.add(model_lighting_atlas_image(dims));
        Self { image, dims }
    }
}

/// An empty atlas: `MODEL_LIGHT_ATLAS_TEXELS_ACROSS` x `dims.image_height` texels,
/// `MODEL_LIGHT_ATLAS_LAYERS` layers deep, four bytes a texel.
pub fn model_lighting_atlas_image(dims: ModelIllumAtlasDims) -> Image {
    let rows = dims.rows_tall;
    let extent = render_resource::Extent3d {
        width: MODEL_LIGHT_ATLAS_TEXELS_ACROSS,
        height: rows,
        depth_or_array_layers: MODEL_LIGHT_ATLAS_LAYERS,
    };
    let byte_len = extent.width as usize * extent.height as usize * extent.depth_or_array_layers as usize * 4;
    let mut blank = Image::new(
        extent,
        render_resource::TextureDimension::D3,
        vec![0; byte_len],
        render_resource::TextureFormat::Rgba8Unorm,
        RenderAssetUsages::MAIN_WORLD | RenderAssetUsages::RENDER_WORLD,
    );
    blank.texture_view_descriptor = Some(render_resource::TextureViewDescriptor {
        dimension: Some(render_resource::TextureViewDimension::D3),
        ..default()
    });
    blank
}

/// Copies a tile into the CPU side of the atlas. `false` when the image is not
/// an atlas of these dimensions, has no CPU data, or the tile is out of range.
pub fn light_atlas_store_tile(
    target: &mut Image,
    dims: ModelIllumAtlasDims,
    entry: IllumTileNo,
    tile: &[u8; ILLUM_TILE_LEN],
) -> bool {
    let rows = dims.rows_tall;
    let size = target.texture_descriptor.size;
    let is_atlas = size.width == MODEL_LIGHT_ATLAS_TEXELS_ACROSS
        && size.height == rows
        && size.depth_or_array_layers == MODEL_LIGHT_ATLAS_LAYERS;
    match target.data.as_mut() {
        Some(data) if is_atlas => blit_illum_tile(data, rows, entry, tile),
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
    pub texels: [u8; ILLUM_TILE_LEN],
}

impl ModelLightingAtlasTileWrites {
    pub fn push(
        &mut self,
        image: AssetId<Image>,
        entry: IllumTileNo,
        tile: &[u8; ILLUM_TILE_LEN],
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

    const DIMS: ModelIllumAtlasDims = ModelIllumAtlasDims {
        figure_entry_limit: 1,
        prop_entry_limit: 1,
        rows_tall: 8,
    };

    #[test]
    fn the_atlas_image_is_a_zeroed_3d_texture_of_the_stated_size() {
        let image = model_lighting_atlas_image(DIMS);
        let size = image.texture_descriptor.size;
        assert_eq!(
            (size.width, size.height, size.depth_or_array_layers),
            (MODEL_LIGHT_ATLAS_TEXELS_ACROSS, 8, MODEL_LIGHT_ATLAS_LAYERS)
        );
        assert_eq!(image.texture_descriptor.dimension, render_resource::TextureDimension::D3);
        let data = image.data.expect("cpu data");
        assert_eq!(data.len(), (size.width * size.height * size.depth_or_array_layers * 4) as usize);
        assert!(data.iter().all(|&b| b == 0));
    }

    #[test]
    fn writing_a_tile_to_a_differently_sized_image_is_refused() {
        let mut other = model_lighting_atlas_image(ModelIllumAtlasDims { rows_tall: 16, ..DIMS });
        let tile = [1; ILLUM_TILE_LEN];
        assert!(!light_atlas_store_tile(&mut other, DIMS, IllumTileNo { raw: 0 }, &tile));
    }

    #[test]
    fn writing_a_tile_to_an_image_with_no_cpu_data_is_refused() {
        let mut target = model_lighting_atlas_image(DIMS);
        target.data = None;
        let tile = [1; ILLUM_TILE_LEN];
        assert!(!light_atlas_store_tile(&mut target, DIMS, IllumTileNo { raw: 0 }, &tile));
    }

    #[test]
    fn a_tile_that_fits_lands_in_the_atlas_data() {
        let mut target = model_lighting_atlas_image(DIMS);
        let tile = [0xAB; ILLUM_TILE_LEN];
        assert!(light_atlas_store_tile(&mut target, DIMS, IllumTileNo { raw: 0 }, &tile));
        assert!(target.data.expect("cpu data").contains(&0xAB));
    }

    #[test]
    fn queued_writes_keep_the_tile_texels_and_the_entry_origin() {
        let mut writes = ModelLightingAtlasTileWrites::default();
        let entry = IllumTileNo { raw: 3 };
        writes.push(AssetId::default(), entry, &[7; ILLUM_TILE_LEN]);
        assert_eq!(writes.tiles.len(), 1);
        assert_eq!(writes.tiles[0].origin, entry.texel_origin());
        assert!(writes.tiles[0].texels.iter().all(|&b| b == 7));
    }
}
