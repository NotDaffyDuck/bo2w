//! Light-grid lookups for models, over the grid of `lighting_t6`.

pub use lighting_t6::{
    BlockedReason, GridView, LightGridColorEncoding, OwnedLightGrid, SampledLighting,
    mean_tile_rgba01,
};
use lighting_t6::{LIGHT_GRID_ATPOINT_EMPTY_PRIMARY, MODEL_LIGHTING_TILE_BYTES, sample_light_grid_at};

/// The light at `pos`, with the grid's usual fallback for an empty cell.
pub fn sample_light_grid(
    grid: &GridView<'_>,
    pos: [f32; 3],
) -> Result<SampledLighting, BlockedReason> {
    sample_light_grid_at(grid, pos, None, LIGHT_GRID_ATPOINT_EMPTY_PRIMARY)
}

/// The light at `pos`, taking `lookup_fallback` as the primary lookup of an
/// empty cell.
pub fn sample_light_grid_with_lookup_fallback(
    grid: &GridView<'_>,
    pos: [f32; 3],
    lookup_fallback: u8,
) -> Result<SampledLighting, BlockedReason> {
    sample_light_grid_at(grid, pos, None, lookup_fallback)
}

/// The lighting one placed static model was sampled with.
#[derive(Clone, Debug)]
pub struct SmodelLightingSample {
    /// The model's slot among the map's placements.
    pub authored_slot: usize,
    /// Where the grid was sampled for it.
    pub lighting_origin: [f32; 3],
    pub tile_rgba: [u8; MODEL_LIGHTING_TILE_BYTES],
    pub packed_lighting: [u8; 4],
}
