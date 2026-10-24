//! Sampling the light grid at a point: the row run-length walk to the 8 corner
//! entries, the primary light pick, the weighted colour blend, and the colour tile
//! a model is lit with. A grid's colour rows are RGB8 unless its loader marks them
//! `LoaderDecoded`; such a loader runs `light_grid_lookup` and blends its own rows.

use crate::{
    GfxLightGridEntry, LIGHT_GRID_ATPOINT_EMPTY_PRIMARY, LIGHT_GRID_COLORS_BYTE_COUNT,
    LIGHT_GRID_ROW_ABSENT, LIGHT_GRID_ROW_HEADER_SIZE, LightGridAtPointEmptyGate,
    LightGridAtPointPath, LightGridCompressedColor, LightGridEntryQuad, LightGridLookupColorAccum,
    LightGridLookupCorner, LightGridLookupWeights, LightGridPickCorner, LightGridRowHeader,
    MODEL_LIGHTING_TILE_BYTES, light_grid_accumulate_colors, light_grid_at_row_last_column,
    light_grid_atapoint_return_primary, light_grid_atapoint_select_path, light_grid_axis_lerps,
    light_grid_colors_byte_offset, light_grid_column_before_row_start_quad,
    light_grid_compress_colors, light_grid_corner_needs_trace,
    light_grid_corner_weight_keeps_entry, light_grid_corner_weights,
    light_grid_default_colors_index, light_grid_empty_run_entry_quad,
    light_grid_expand_shell_to_tile_rgba, light_grid_fixed_point_blend_weights,
    light_grid_isvalid_sample, light_grid_lookup_accumulate_corners,
    light_grid_lookup_remap_primary, light_grid_pick_default_grid_entry,
    light_grid_pick_merge_entry_quads, light_grid_pick_primary_from_corners,
    light_grid_pick_row_rle_cells, light_grid_primary_light_sun_band,
    light_grid_rle_advance_to_column, light_grid_row_contains_cell, light_grid_sample_cell,
    light_grid_solid_run_entry_quad, light_grid_solid_run_z_base,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LightGridColorEncoding {
    Rgb8,

    /// Rows in another game's encoding; the loader that built the grid decodes
    /// them after `light_grid_lookup`.
    LoaderDecoded,
}

#[derive(Clone, Debug)]
pub struct OwnedLightGrid {
    pub mins: [u16; 3],
    pub maxs: [u16; 3],
    pub row_axis: usize,
    pub col_axis: usize,
    pub color_count: u32,

    pub has_light_regions: bool,

    pub sun_primary_light_index: u32,
    pub row_data_start: Vec<u8>,
    pub raw_row_data: Vec<u8>,
    pub entries: Vec<u8>,
    pub colors: Vec<u8>,
    pub color_encoding: LightGridColorEncoding,
}

impl OwnedLightGrid {
    pub fn view(&self) -> GridView<'_> {
        GridView {
            mins: self.mins,
            maxs: self.maxs,
            row_axis: self.row_axis,
            col_axis: self.col_axis,
            has_light_regions: self.has_light_regions,
            sun_primary_light_index: self.sun_primary_light_index,
            row_data_start: &self.row_data_start,
            raw_row_data: &self.raw_row_data,
            entries: &self.entries,
            colors: &self.colors,
            color_encoding: self.color_encoding,
            color_count: self.color_count,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct GridView<'a> {
    pub mins: [u16; 3],
    pub maxs: [u16; 3],
    pub row_axis: usize,
    pub col_axis: usize,

    pub has_light_regions: bool,

    pub sun_primary_light_index: u32,

    pub row_data_start: &'a [u8],
    pub raw_row_data: &'a [u8],

    pub entries: &'a [u8],

    pub colors: &'a [u8],
    pub color_encoding: LightGridColorEncoding,
    pub color_count: u32,
}

impl GridView<'_> {
    pub fn entry_at(&self, byte_offset: u32) -> Option<GfxLightGridEntry> {
        let start = byte_offset as usize;
        let bytes = self.entries.get(start..start.checked_add(4)?)?;
        Some(GfxLightGridEntry {
            colors_index: u16::from_le_bytes([bytes[0], bytes[1]]),
            primary_light_index: bytes[2],
            needs_trace: bytes[3],
        })
    }

    pub fn colors_row(&self, index: u32) -> Option<&[u8]> {
        let start = light_grid_colors_byte_offset(index) as usize;
        self.colors
            .get(start..start.checked_add(LIGHT_GRID_COLORS_BYTE_COUNT)?)
    }
}

enum RowRleOutcome {
    OutsideGrid,
    OutsideRow { clear_needs_trace: bool },
    Truncated,
    Quad(LightGridEntryQuad),
}

impl RowRleOutcome {
    fn quad(&self) -> Option<LightGridEntryQuad> {
        match self {
            RowRleOutcome::Quad(quad) => Some(*quad),
            RowRleOutcome::OutsideGrid => Some(LightGridEntryQuad {
                corners: [None; 4],
                clear_needs_trace: false,
            }),
            RowRleOutcome::OutsideRow { clear_needs_trace } => Some(LightGridEntryQuad {
                corners: [None; 4],
                clear_needs_trace: *clear_needs_trace,
            }),
            RowRleOutcome::Truncated => None,
        }
    }
}

fn row_rle(grid: &GridView<'_>, cell: [i32; 3]) -> RowRleOutcome {
    let min = u32::from(grid.mins[grid.row_axis]);
    let max = u32::from(grid.maxs[grid.row_axis]);
    let row = (cell[grid.row_axis] as u32).wrapping_sub(min);
    if max.wrapping_sub(min).wrapping_add(1) <= row {
        return RowRleOutcome::OutsideGrid;
    }

    let word_at = (row as usize).wrapping_mul(2);
    let Some(word) = grid.row_data_start.get(word_at..word_at.wrapping_add(2)) else {
        return RowRleOutcome::Truncated;
    };
    let row_start_dwords = u16::from_le_bytes([word[0], word[1]]);
    if row_start_dwords == LIGHT_GRID_ROW_ABSENT {
        return RowRleOutcome::OutsideGrid;
    }

    let Some(payload) = grid
        .raw_row_data
        .get(usize::from(row_start_dwords).wrapping_mul(4)..)
    else {
        return RowRleOutcome::Truncated;
    };
    let Some(header) = LightGridRowHeader::from_bytes(payload) else {
        return RowRleOutcome::Truncated;
    };

    let col = (cell[grid.col_axis] as u32).wrapping_sub(u32::from(header.col_start));
    let row_in_header = (cell[2] as u32).wrapping_sub(u32::from(header.row_start));
    if !light_grid_row_contains_cell(&header, col, row_in_header) {
        let column_fits = u32::from(header.col_count) >= col.wrapping_add(1);
        return RowRleOutcome::OutsideRow {
            clear_needs_trace: column_fits && (cell[2] as u32) < u32::from(header.row_start),
        };
    }
    if col == u32::MAX {
        let rle = &payload[LIGHT_GRID_ROW_HEADER_SIZE..];
        let wide_row = header.row_count > 0xff;
        return match light_grid_column_before_row_start_quad(
            header.entry_base,
            row_in_header,
            rle,
            wide_row,
        ) {
            Some(quad) => RowRleOutcome::Quad(quad),
            None => RowRleOutcome::Truncated,
        };
    }

    let rle = &payload[LIGHT_GRID_ROW_HEADER_SIZE..];
    let Some(cursor) =
        light_grid_rle_advance_to_column(rle, header.row_count, header.entry_base, col)
    else {
        return RowRleOutcome::Truncated;
    };

    let wide_row = header.row_count > 0xff;
    let stride = if cursor.run_increment == 0 {
        2
    } else {
        header.rle_stride()
    };
    let next_run = rle
        .get(cursor.run_offset.wrapping_add(stride)..)
        .filter(|run| !run.is_empty());
    let at_last_column = light_grid_at_row_last_column(cell[grid.col_axis] as u32, &header);

    let quad = if cursor.run_increment == 0 {
        light_grid_empty_run_entry_quad(&cursor, row_in_header, at_last_column, next_run, wide_row)
    } else {
        let run = &rle[cursor.run_offset..];
        let Some(z_base) = light_grid_solid_run_z_base(run, wide_row) else {
            return RowRleOutcome::Truncated;
        };
        light_grid_solid_run_entry_quad(
            &cursor,
            row_in_header,
            z_base,
            at_last_column,
            next_run,
            wide_row,
        )
    };
    match quad {
        Some(quad) => RowRleOutcome::Quad(quad),
        None => RowRleOutcome::Truncated,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BlockedReason {
    UnmodelledRowBranch,

    TruncatedZoneData,

    NoLiveCorner,
}

#[derive(Clone, Debug)]
pub struct SampledLighting {
    pub live_corners: usize,
    pub weights: [f32; 8],
    pub path: LightGridAtPointPath,
    pub sample_count: u32,
    pub matching_primary: f32,
    pub traced_influence: f32,
    pub total: f32,
    pub picked_primary: u8,

    pub picked_before_remap: u8,

    pub corner_primaries: [Option<u8>; 8],

    /// bo2zm: each corner entry's fourth byte (legacy `needsTrace`; T6 the
    /// entry's primary light visibility, 0..255).
    pub corner_flags: [Option<u8>; 8],

    pub needs_trace_flag: bool,

    pub corners_needing_sight: u8,

    pub corners_sight_cleared: u8,

    pub corners_sight_suppressed: u8,
    pub colors: [u8; LIGHT_GRID_COLORS_BYTE_COUNT],
    pub compressed: [u8; 3],
    pub tile: [u8; MODEL_LIGHTING_TILE_BYTES],
}

/// Everything about a grid sample except its colours: which colour rows to
/// blend (`path`, `accum`, `default_grid_entry`) and the corner bookkeeping.
#[derive(Clone, Debug)]
pub struct LightGridLookup {
    pub path: LightGridAtPointPath,
    pub accum: LightGridLookupColorAccum,
    pub buckets: LightGridLookupWeights,
    pub default_grid_entry: u32,
    weights: [f32; 8],
    live_corners: usize,
    selected_primary: u8,
    picked_before_remap: u8,
    corner_primaries: [Option<u8>; 8],
    corner_flags: [Option<u8>; 8],
    needs_trace_flag: bool,
    corners_needing_sight: u8,
    corners_sight_cleared: u8,
    corners_sight_suppressed: u8,
}

impl LightGridLookup {
    pub fn into_sampled(
        self,
        colors: [u8; LIGHT_GRID_COLORS_BYTE_COUNT],
        compressed: LightGridCompressedColor,
        tile: [u8; MODEL_LIGHTING_TILE_BYTES],
    ) -> SampledLighting {
        SampledLighting {
            live_corners: self.live_corners,
            weights: self.weights,
            path: self.path,
            sample_count: self.accum.count,
            matching_primary: self.buckets.matching_primary,
            traced_influence: self.buckets.traced_influence,
            total: self.buckets.total,
            picked_primary: light_grid_atapoint_return_primary(self.path, self.selected_primary),
            picked_before_remap: self.picked_before_remap,
            corner_primaries: self.corner_primaries,
            corner_flags: self.corner_flags,
            needs_trace_flag: self.needs_trace_flag,
            corners_needing_sight: self.corners_needing_sight,
            corners_sight_cleared: self.corners_sight_cleared,
            corners_sight_suppressed: self.corners_sight_suppressed,
            colors,
            compressed: [compressed.r, compressed.g, compressed.b],
            tile,
        }
    }
}

pub fn sample_light_grid(
    grid: &GridView<'_>,
    pos: [f32; 3],
) -> Result<SampledLighting, BlockedReason> {
    sample_light_grid_at(grid, pos, None, LIGHT_GRID_ATPOINT_EMPTY_PRIMARY)
}

pub fn sample_light_grid_with_sight(
    grid: &GridView<'_>,
    pos: [f32; 3],
    sight_clear: Option<&dyn Fn([f32; 3], [f32; 3]) -> bool>,
) -> Result<SampledLighting, BlockedReason> {
    sample_light_grid_at(grid, pos, sight_clear, LIGHT_GRID_ATPOINT_EMPTY_PRIMARY)
}

pub fn sample_light_grid_with_lookup_fallback(
    grid: &GridView<'_>,
    pos: [f32; 3],
    lookup_fallback: u8,
) -> Result<SampledLighting, BlockedReason> {
    sample_light_grid_at(grid, pos, None, lookup_fallback)
}

/// Samples an RGB8 grid.
pub fn sample_light_grid_at(
    grid: &GridView<'_>,
    pos: [f32; 3],
    sight_clear: Option<&dyn Fn([f32; 3], [f32; 3]) -> bool>,
    lookup_fallback: u8,
) -> Result<SampledLighting, BlockedReason> {
    let lookup = light_grid_lookup(grid, pos, sight_clear, lookup_fallback)?;
    let (colors, compressed, tile) = rgb8_colors(grid, &lookup)?;
    Ok(lookup.into_sampled(colors, compressed, tile))
}

pub fn light_grid_lookup(
    grid: &GridView<'_>,
    pos: [f32; 3],
    sight_clear: Option<&dyn Fn([f32; 3], [f32; 3]) -> bool>,
    lookup_fallback: u8,
) -> Result<LightGridLookup, BlockedReason> {
    let cell = light_grid_sample_cell(pos);
    let lerps = light_grid_axis_lerps(pos, cell, grid.row_axis as u32, grid.col_axis as u32);
    let weights = light_grid_corner_weights(lerps);

    let Some(cells) = light_grid_pick_row_rle_cells(cell, grid.row_axis as u32) else {
        return Err(BlockedReason::TruncatedZoneData);
    };
    let low = row_rle(grid, cells.base);
    let high = row_rle(grid, cells.adjacent);
    let (Some(low_quad), Some(high_quad)) = (low.quad(), high.quad()) else {
        let truncated =
            matches!(low, RowRleOutcome::Truncated) || matches!(high, RowRleOutcome::Truncated);
        return Err(if truncated {
            BlockedReason::TruncatedZoneData
        } else {
            BlockedReason::UnmodelledRowBranch
        });
    };
    let needs_trace_flag = !(low_quad.clear_needs_trace || high_quad.clear_needs_trace);
    let slots = light_grid_pick_merge_entry_quads(low_quad, high_quad);

    let mut entries: [Option<GfxLightGridEntry>; 8] = [None; 8];
    for (i, slot) in slots.iter().enumerate() {
        if let Some(offset) = *slot {
            let Some(entry) = grid.entry_at(offset) else {
                return Err(BlockedReason::TruncatedZoneData);
            };
            entries[i] = Some(entry);
        }
    }

    let mut corners_needing_sight = 0u8;
    let mut corners_sight_cleared = 0u8;
    let mut corners_sight_suppressed = 0u8;
    let corners: [LightGridPickCorner; 8] = core::array::from_fn(|i| {
        let entry = entries[i];
        let needs_trace = entry.map_or(0, |entry| entry.needs_trace);
        let weight = weights[i];
        let asks = entry.is_some()
            && light_grid_corner_weight_keeps_entry(weight)
            && light_grid_corner_needs_trace(needs_trace, i as u32);
        let is_valid_if_traced = if !asks {
            false
        } else {
            corners_needing_sight = corners_needing_sight.saturating_add(1);
            let valid = match sight_clear {
                Some(clear) => light_grid_isvalid_sample(
                    cell,
                    i as u32,
                    grid.row_axis as u32,
                    grid.col_axis as u32,
                    pos,
                    clear,
                )
                .unwrap_or(false),

                None => false,
            };
            if valid {
                corners_sight_cleared = corners_sight_cleared.saturating_add(1);
            } else {
                corners_sight_suppressed = corners_sight_suppressed.saturating_add(1);
            }
            valid
        };
        LightGridPickCorner {
            primary_light: entry.map(|entry| entry.primary_light_index),
            needs_trace,
            weight,
            is_valid_if_traced,
        }
    });

    let sun_base = grid.sun_primary_light_index;
    let sun_band = light_grid_primary_light_sun_band(sun_base);
    let picked = light_grid_pick_primary_from_corners(&corners, sun_band);

    let selected_primary = light_grid_lookup_remap_primary(
        picked.primary_light,
        lookup_fallback,
        grid.has_light_regions,
        sun_base,
    );

    let live: Vec<LightGridLookupCorner> = (0..8)
        .filter(|&i| picked.entry_alive[i])
        .filter_map(|i| {
            entries[i].map(|entry| LightGridLookupCorner {
                colors_index: entry.colors_index,
                primary_light: entry.primary_light_index,
                weight: weights[i],

                trace_allows: false,
            })
        })
        .collect();

    let default_grid_entry =
        light_grid_pick_default_grid_entry(low_quad.clear_needs_trace, high_quad.clear_needs_trace);
    let (buckets, accum) = light_grid_lookup_accumulate_corners(&live, selected_primary, sun_band);
    let path = light_grid_atapoint_select_path(
        accum.count,
        LightGridAtPointEmptyGate {
            prefer_default_when_missing: false,
            fallback_colors_index: default_grid_entry,
            show_missing_light_grid: false,
            color_count: grid.color_count,
        },
    );

    Ok(LightGridLookup {
        path,
        accum,
        buckets,
        default_grid_entry,
        weights,
        live_corners: live.len(),
        selected_primary,
        picked_before_remap: picked.primary_light,
        corner_primaries: core::array::from_fn(|i| corners[i].primary_light),
        corner_flags: core::array::from_fn(|i| entries[i].map(|e| e.needs_trace)),
        needs_trace_flag,
        corners_needing_sight,
        corners_sight_cleared,
        corners_sight_suppressed,
    })
}

#[allow(clippy::type_complexity)]
fn rgb8_colors(
    grid: &GridView<'_>,
    lookup: &LightGridLookup,
) -> Result<
    (
        [u8; LIGHT_GRID_COLORS_BYTE_COUNT],
        LightGridCompressedColor,
        [u8; MODEL_LIGHTING_TILE_BYTES],
    ),
    BlockedReason,
> {
    let accum = &lookup.accum;
    let mut colors = [0u8; LIGHT_GRID_COLORS_BYTE_COUNT];
    match lookup.path {
        LightGridAtPointPath::SetFromIndex => {
            let Some(row) = grid.colors_row(u32::from(accum.indices[0])) else {
                return Err(BlockedReason::TruncatedZoneData);
            };
            colors.copy_from_slice(row);
        }
        LightGridAtPointPath::BlendAndSet => {
            let count = accum.count as usize;
            let mut rows: Vec<&[u8]> = Vec::with_capacity(count);
            for &index in &accum.indices[..count] {
                let Some(row) = grid.colors_row(u32::from(index)) else {
                    return Err(BlockedReason::TruncatedZoneData);
                };
                rows.push(row);
            }
            let mut fixed = [0u16; 8];
            let inverse = if lookup.buckets.total != 0.0 {
                1.0 / lookup.buckets.total
            } else {
                0.0
            };
            if light_grid_fixed_point_blend_weights(&accum.weights[..count], inverse, &mut fixed)
                .is_none()
            {
                return Err(BlockedReason::NoLiveCorner);
            }
            if !light_grid_accumulate_colors(&rows, &fixed[..count], &mut colors) {
                return Err(BlockedReason::TruncatedZoneData);
            }
        }
        LightGridAtPointPath::MissingUseIndex => {
            let Some(row) = grid.colors_row(lookup.default_grid_entry) else {
                return Err(BlockedReason::TruncatedZoneData);
            };
            colors.copy_from_slice(row);
        }
        LightGridAtPointPath::SetDefault => {
            let Some(index) = light_grid_default_colors_index(grid.color_count) else {
                return Err(BlockedReason::TruncatedZoneData);
            };
            let Some(row) = grid.colors_row(index) else {
                return Err(BlockedReason::TruncatedZoneData);
            };
            colors.copy_from_slice(row);
        }
    }

    let Some(compressed) = light_grid_compress_colors(&colors, 0xff) else {
        return Err(BlockedReason::TruncatedZoneData);
    };
    let mut tile = [0u8; MODEL_LIGHTING_TILE_BYTES];
    if !light_grid_expand_shell_to_tile_rgba(&colors, compressed.weight, &mut tile) {
        return Err(BlockedReason::TruncatedZoneData);
    }
    Ok((colors, compressed, tile))
}

pub fn mean_tile_rgba01(tile: &[u8; MODEL_LIGHTING_TILE_BYTES]) -> [f32; 3] {
    let texels = MODEL_LIGHTING_TILE_BYTES / 4;
    let mut sum = [0.0f32; 3];
    for i in 0..texels {
        let o = i * 4;
        sum[0] += f32::from(tile[o]) / 255.0;
        sum[1] += f32::from(tile[o + 1]) / 255.0;
        sum[2] += f32::from(tile[o + 2]) / 255.0;
    }
    let n = texels as f32;
    [sum[0] / n, sum[1] / n, sum[2] / n]
}
