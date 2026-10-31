//! Reading a map's light grid at the spot a model stands.

pub use lighting_t6::{DetachedLightGrid, GridSampleBlocker, GridWindow, LightProbeReading};

/// The grid's reading at `spot`, using the grid's own fallback when the cell
/// there is empty.
pub fn probe_light_cells(grid: &GridWindow<'_>, spot: [f32; 3]) -> Result<LightProbeReading, GridSampleBlocker> {
    probe_light_cells_with_backup(grid, spot, lighting_t6::GRID_SAMPLE_VACANT_SUN_BAND)
}

/// The grid's reading at `spot`, using `empty_cell_lookup` first when the
/// cell there is empty. The grid works out the cell itself.
pub fn probe_light_cells_with_backup(grid: &GridWindow<'_>, spot: [f32; 3], empty_cell_lookup: u8) -> Result<LightProbeReading, GridSampleBlocker> {
    let no_hint = None;
    lighting_t6::sample_light_grid_at(grid, spot, no_hint, empty_cell_lookup)
}

/// What one placed static model's lighting came out as.
#[derive(Clone, Debug)]
pub struct StaticPropLightSample {
    /// The model's index in the map's list of placed models.
    pub designer_slot: usize,
    /// The point the grid was read at.
    pub illum_origin: [f32; 3],
    pub tile_px: [u8; lighting_t6::ILLUM_TILE_LEN],
    pub compact_illum: [u8; 4],
}
