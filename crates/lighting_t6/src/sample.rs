//! Reading the light grid at one point.
//!
//! A lookup walks the two compressed rows next to the point to find the cell's
//! eight corner entries, drops corners that count for too little or cannot be
//! seen, takes the strongest light among the rest, and blends the colour rows of
//! the corners that remain into one shell colour. From that shell come the
//! compressed colour and the small tile a model is lit with.
//!
//! Colour rows are three bytes per texel unless the grid's loader marks them
//! `LoaderDecoded`: such a loader runs [`light_grid_lookup`] and blends its own
//! rows.

use crate::grid_tint::{self, TINT_PAYLOAD_LEN, PackedCellTint};
use crate::point_route::{self, GridProbeFallbackBranch};
use crate::grid_entry::GridCellEntry;
use crate::tile_unpack::{self, ILLUM_TILE_LEN};
use crate::sample_ok;
use crate::illum_lookup::{
    self, GridLookupTintAccumulator, GridProbeCornerSample, GridProbeBlendFactors,
};
use crate::corner_pick::{self, IlluminationVoxelRef, GridCornerChoice};
use crate::run_rows::{self, RunQuad, RowPreamble};

/// Bytes an empty run takes in a row's run table: its two counts.
const EMPTY_RUN_LEN: usize = 2;

/// How the colour rows of a grid are stored.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GridTintStorageFormat {
    Rgb8,
    /// Another game's encoding. The loader that built the grid decodes the rows
    /// itself, after [`light_grid_lookup`].
    LoaderDecoded,
}

/// A light grid that owns its bytes.
#[derive(Clone, Debug)]
pub struct DetachedLightGrid {
    pub low: [u16; 3],
    pub high: [u16; 3],
    pub line_direction: usize,
    pub column_direction: usize,
    pub tint_key_total: u32,
    pub carries_light_volumes: bool,
    pub sunlight_lamp_slot: u32,
    pub line_bytes_origin: Vec<u8>,
    pub unparsed_line_bytes: Vec<u8>,
    pub entries: Vec<u8>,
    pub colors: Vec<u8>,
    pub color_encoding: GridTintStorageFormat,
}

impl DetachedLightGrid {
    pub fn view(&self) -> GridWindow<'_> {
        let Self {
            low: mins,
            high: maxs,
            line_direction,
            column_direction,
            tint_key_total,
            carries_light_volumes,
            sunlight_lamp_slot,
            color_encoding,
            ..
        } = *self;
        GridWindow {
            low: mins,
            high: maxs,
            line_direction,
            column_direction,
            carries_light_volumes,
            sunlight_lamp_slot,
            line_bytes_origin: self.line_bytes_origin.as_slice(),
            unparsed_line_bytes: self.unparsed_line_bytes.as_slice(),
            entries: self.entries.as_slice(),
            colors: self.colors.as_slice(),
            color_encoding,
            tint_key_total,
        }
    }
}

/// A light grid borrowed from wherever its bytes live.
#[derive(Clone, Copy, Debug)]
pub struct GridWindow<'a> {
    pub low: [u16; 3],
    pub high: [u16; 3],
    pub line_direction: usize,
    pub column_direction: usize,
    pub carries_light_volumes: bool,
    pub sunlight_lamp_slot: u32,
    /// One little-endian u16 per row: where that row's data starts, in dwords.
    pub line_bytes_origin: &'a [u8],
    pub unparsed_line_bytes: &'a [u8],
    /// Four bytes per entry: colour row (u16), primary light, trace bits.
    pub entries: &'a [u8],
    pub colors: &'a [u8],
    pub color_encoding: GridTintStorageFormat,
    pub tint_key_total: u32,
}

/// A quad with no corners; `cleared` says whether the point sat under the row.
const fn cornerless(cleared: bool) -> RunQuad {
    RunQuad {
        quad_cells: [None; 4],
        empty_run_requires_trace: cleared,
    }
}

/// The corner quad from one row's header and run table, for a cell given by
/// its column and height coordinates.
fn quad_in_row(
    header: &RowPreamble,
    runs: &[u8],
    col_coord: u32,
    z_coord: u32,
) -> Option<RunQuad> {
    let col = col_coord.wrapping_sub(u32::from(header.span_first_column));
    let height = z_coord.wrapping_sub(u32::from(header.row_first_cell));
    if !run_rows::row_holds_cell(header, col, height) {
        let under_row = z_coord < u32::from(header.row_first_cell)
            && col.wrapping_add(1) <= u32::from(header.column_total);
        return Some(cornerless(under_row));
    }

    let tall = header.line_total > u16::from(u8::MAX);
    // One column to the left of the row's first column.
    if col.wrapping_add(1) == 0 {
        return run_rows::column_before_quad(
            header.first_entry_offset,
            height,
            runs,
            tall,
        );
    }

    let cursor =
        run_rows::grid_compressed_row_seek_column(runs, header.line_total, header.first_entry_offset, col)?;
    let ends_row = run_rows::on_last_column(col_coord, header);
    let solid = cursor.run_step_size > 0;
    let this_run_len = if solid {
        header.run_length_row_pitch()
    } else {
        EMPTY_RUN_LEN
    };
    let after = runs
        .get(cursor.span_start_offset + this_run_len..)
        .and_then(|rest| (!rest.is_empty()).then_some(rest));

    if solid {
        let here = &runs[cursor.span_start_offset..];
        let first_z = run_rows::solid_run_floor(here, tall)?;
        run_rows::filled_quad(&cursor, height, first_z, ends_row, after, tall)
    } else {
        run_rows::grid_vacant_span_quad(&cursor, height, ends_row, after, tall)
    }
}

impl GridWindow<'_> {
    pub fn entry_from_byte_offset(&self, byte_offset: u32) -> Option<GridCellEntry> {
        let tail = self.entries.get(usize::try_from(byte_offset).ok()?..)?;
        let &[lo, hi, light, trace] = tail.first_chunk::<4>()?;
        Some(GridCellEntry {
            vertex_color_slot: u16::from(lo) | (u16::from(hi) << 8),
            key_light_index: light,
            wants_ray_check: trace,
        })
    }

    pub fn colour_samples_at(&self, light_no: u32) -> Option<&[u8]> {
        let offset = point_route::tint_table_start(light_no) as usize;
        self.colors
            .get(offset..)?
            .get(..TINT_PAYLOAD_LEN)
    }

    /// The colour row copied out, or a truncation error.
    fn shell_row(&self, index: u32) -> Result<[u8; TINT_PAYLOAD_LEN], GridSampleBlocker> {
        match self.colour_samples_at(index).and_then(|row| row.first_chunk()) {
            Some(row) => Ok(*row),
            None => Err(GridSampleBlocker::ZoneDataCutOff),
        }
    }

    /// The bytes of the compressed row a row-axis coordinate falls in.
    /// `Some(None)` when the grid stores nothing there; `None` when the grid's
    /// bytes run out first.
    fn row_bytes(&self, row_coord: i32) -> Option<Option<&[u8]>> {
        let lowest = u32::from(self.low[self.line_direction]);
        let span = u32::from(self.high[self.line_direction])
            .wrapping_sub(lowest)
            .wrapping_add(1);
        let row = (row_coord as u32).wrapping_sub(lowest);
        if row >= span {
            return Some(None);
        }
        let table_at = row as usize * 2;
        let word = *self.line_bytes_origin.get(table_at..)?.first_chunk::<2>()?;
        match u16::from_le_bytes(word) {
            run_rows::ROW_NOT_SET => Some(None),
            dwords => self.unparsed_line_bytes.get(4 * usize::from(dwords)..).map(Some),
        }
    }

    /// The entry quad of the row holding `cell`: the entries at the cell and at
    /// the next column. A cell outside the grid or its row has no entries.
    /// `None` when the grid's bytes run out partway.
    fn row_quad(&self, cell: [i32; 3]) -> Option<RunQuad> {
        let Some(bytes) = self.row_bytes(cell[self.line_direction])? else {
            return Some(cornerless(false));
        };
        let header = RowPreamble::from_bytes(bytes)?;
        let runs = &bytes[run_rows::ROW_PREAMBLE_LEN..];
        quad_in_row(&header, runs, cell[self.column_direction] as u32, cell[2] as u32)
    }

    /// The shell colour a lookup's path calls for.
    fn shell_for(
        &self,
        found: &LightGridLookup,
    ) -> Result<[u8; TINT_PAYLOAD_LEN], GridSampleBlocker> {
        if matches!(found.path, GridProbeFallbackBranch::MixThenStore) {
            return self.blend_rows(&found.accum, found.buckets.sum);
        }
        let index = match found.path {
            GridProbeFallbackBranch::AssignedByIndex => u32::from(found.accum.picks[0]),
            GridProbeFallbackBranch::UnassignedUseSlot => found.default_grid_entry,
            _ => match point_route::fallback_tint_index(self.tint_key_total) {
                Some(last) => last,
                None => return Err(GridSampleBlocker::ZoneDataCutOff),
            },
        };
        self.shell_row(index)
    }

    /// Mixes the colour rows the lookup gathered by their weights.
    fn blend_rows(
        &self,
        sets: &GridLookupTintAccumulator,
        total: f32,
    ) -> Result<[u8; TINT_PAYLOAD_LEN], GridSampleBlocker> {
        let used = sets.count as usize;
        let mut rows: Vec<&[u8]> = Vec::with_capacity(used);
        for &set in &sets.picks[..used] {
            match self.colour_samples_at(u32::from(set)) {
                Some(row) => rows.push(row),
                None => return Err(GridSampleBlocker::ZoneDataCutOff),
            }
        }
        let scale = if total == 0.0 { 0.0 } else { total.recip() };
        let mut shares = [0u16; 8];
        if point_route::grid_quantized_mix_factors(&sets.shares[..used], scale, &mut shares)
            .is_none()
        {
            return Err(GridSampleBlocker::AllCornersDead);
        }
        let mut shell = [0u8; TINT_PAYLOAD_LEN];
        grid_tint::sum_cell_tints(&rows, &shares[..used], &mut shell)
            .then_some(shell)
            .ok_or(GridSampleBlocker::ZoneDataCutOff)
    }
}

/// Why a point could not be sampled.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GridSampleBlocker {
    /// The grid's bytes end before the data a lookup needs.
    ZoneDataCutOff,
    /// No corner has weight to blend.
    AllCornersDead,
}

/// A finished sample of the grid at one point.
#[derive(Clone, Debug)]
pub struct LightProbeReading {
    /// Corners that took part in the blend.
    pub active_corner_total: usize,
    pub mix: [f32; 8],
    pub path: GridProbeFallbackBranch,
    pub chosen_key_light: u8,
    /// Each corner's strongest light; `None` for a corner with no entry.
    pub corner_dominant_lights: [Option<u8>; 8],
    /// Each corner entry's fourth byte: the old trace bits, which in BO2 are the
    /// entry's primary-light visibility, 0..255.
    pub corner_flags: [Option<u8>; 8],
    pub packed: [u8; 3],
    pub texels: [u8; ILLUM_TILE_LEN],
}

/// Everything about a sample except the colours: which colour rows to blend
/// (`path`, `accum`, `default_grid_entry`) and what the corners looked like.
#[derive(Clone, Debug)]
pub struct LightGridLookup {
    pub path: GridProbeFallbackBranch,
    pub accum: GridLookupTintAccumulator,
    pub buckets: GridProbeBlendFactors,
    pub default_grid_entry: u32,
    corner_entries: [Option<GridCellEntry>; 8],
    corner_weights: [f32; 8],
    kept: usize,
    chosen_light: u8,
}

impl LightGridLookup {
    /// Joins the lookup with the colours its loader blended.
    pub fn into_sampled(
        self,
        _colors: [u8; TINT_PAYLOAD_LEN],
        compressed: PackedCellTint,
        tile: [u8; ILLUM_TILE_LEN],
    ) -> LightProbeReading {
        let PackedCellTint { r, g, b, .. } = compressed;
        let entries = self.corner_entries;
        LightProbeReading {
            chosen_key_light: point_route::point_primary_light(
                self.path,
                self.chosen_light,
            ),
            corner_dominant_lights: entries.map(|slot| Some(slot?.key_light_index)),
            corner_flags: entries.map(|slot| Some(slot?.wants_ray_check)),
            active_corner_total: self.kept,
            mix: self.corner_weights,
            path: self.path,
            packed: [r, g, b],
            texels: tile,
        }
    }
}

pub fn probe_light_cells(
    grid: &GridWindow<'_>,
    pos: [f32; 3],
) -> Result<LightProbeReading, GridSampleBlocker> {
    grid_sample_with_line_of_sight(grid, pos, None)
}

pub fn grid_sample_with_line_of_sight(
    lattice: &GridWindow<'_>,
    where_at: [f32; 3],
    line_of_sight_open: Option<&dyn Fn([f32; 3], [f32; 3]) -> bool>,
) -> Result<LightProbeReading, GridSampleBlocker> {
    let fallback = point_route::GRID_SAMPLE_VACANT_SUN_BAND;
    sample_light_grid_at(lattice, where_at, line_of_sight_open, fallback)
}

/// The compressed colour and model tile made from a shell colour.
fn finish_shell(
    shell: &[u8; TINT_PAYLOAD_LEN],
) -> Option<(PackedCellTint, [u8; ILLUM_TILE_LEN])> {
    let small = grid_tint::pack_cell_tints(shell, u8::MAX)?;
    let mut texels = [0u8; ILLUM_TILE_LEN];
    tile_unpack::widen_shell_into_tile(shell, small.share, &mut texels)
        .then_some((small, texels))
}

/// Samples a grid whose colour rows are RGB.
pub fn sample_light_grid_at(
    grid: &GridWindow<'_>,
    pos: [f32; 3],
    line_of_sight_open: Option<&dyn Fn([f32; 3], [f32; 3]) -> bool>,
    lookup_backup: u8,
) -> Result<LightProbeReading, GridSampleBlocker> {
    let found = light_grid_lookup(grid, pos, line_of_sight_open, lookup_backup)?;
    let shell = grid.shell_for(&found)?;
    match finish_shell(&shell) {
        Some((small, tile)) => Ok(found.into_sampled(shell, small, tile)),
        None => Err(GridSampleBlocker::ZoneDataCutOff),
    }
}

/// The entries at the eight corners of `cell`, plus the colour set to fall
/// back on when none of them counts.
fn corner_entries(
    grid: &GridWindow<'_>,
    cell: IlluminationVoxelRef,
    line_direction: u32,
) -> Result<([Option<GridCellEntry>; 8], u32), GridSampleBlocker> {
    const SHORT: GridSampleBlocker = GridSampleBlocker::ZoneDataCutOff;
    let pair = corner_pick::packed_row_cells(cell, line_direction).ok_or(SHORT)?;
    let near = grid.row_quad(pair.first).ok_or(SHORT)?;
    let far = grid.row_quad(pair.neighbor_cell).ok_or(SHORT)?;

    let mut found = [None; 8];
    let offsets = corner_pick::grid_pick_combine_cell_quartets(near, far);
    for (slot, offset) in found.iter_mut().zip(offsets) {
        let Some(offset) = offset else { continue };
        *slot = Some(grid.entry_from_byte_offset(offset).ok_or(SHORT)?);
    }
    let fallback =
        corner_pick::fallback_cell_entry(near.empty_run_requires_trace, far.empty_run_requires_trace);
    Ok((found, fallback))
}

/// What the light pick sees of one corner. `can_see` runs only when the corner
/// has an entry, carries enough weight and its trace bit asks for a check.
fn pick_corner(
    entry: Option<GridCellEntry>,
    part: f32,
    corner: u32,
    can_see: impl FnOnce() -> bool,
) -> GridCornerChoice {
    let mut out = GridCornerChoice {
        dominant_light: None,
        wants_ray_check: 0,
        share: part,
        valid_when_traced: false,
    };
    if let Some(entry) = entry {
        out.dominant_light = Some(entry.key_light_index);
        out.wants_ray_check = entry.wants_ray_check;
        let wants_check = corner_pick::corner_keeps_weight(part)
            && corner_pick::corner_wants_trace(entry.wants_ray_check, corner);
        out.valid_when_traced = wants_check && can_see();
    }
    out
}

/// The path for a lookup that found `sets` colour sets. With no sets the
/// fallback set stands in when the grid has it, else the default colours.
fn path_for(sets: u32, fallback: u32, tint_key_total: u32) -> GridProbeFallbackBranch {
    match (sets, fallback < tint_key_total) {
        (0, true) => GridProbeFallbackBranch::UnassignedUseSlot,
        (0, false) => GridProbeFallbackBranch::UseDefaults,
        (1, _) => GridProbeFallbackBranch::AssignedByIndex,
        _ => GridProbeFallbackBranch::MixThenStore,
    }
}

/// Finds the corners around `pos` and everything about them that the colour
/// blend needs.
pub fn light_grid_lookup(
    grid: &GridWindow<'_>,
    pos: [f32; 3],
    line_of_sight_open: Option<&dyn Fn([f32; 3], [f32; 3]) -> bool>,
    lookup_backup: u8,
) -> Result<LightGridLookup, GridSampleBlocker> {
    let line_direction = grid.line_direction as u32;
    let column_direction = grid.column_direction as u32;
    let cell = corner_pick::cell_under(pos);
    let lerps = corner_pick::axis_mix(pos, cell, line_direction, column_direction);
    let corner_weights = corner_pick::corner_mix(lerps);
    let (corner_entries, fallback_set) = corner_entries(grid, cell, line_direction)?;

    let sees_corner = |corner: u32| {
        line_of_sight_open.is_some_and(|clear| {
            sample_ok::sample_counts(cell, corner, line_direction, column_direction, pos, clear)
                == Some(true)
        })
    };
    let mut pick_input = [pick_corner(None, 0.0, 0, || false); 8];
    for corner in 0..8u32 {
        let at = corner as usize;
        pick_input[at] = pick_corner(corner_entries[at], corner_weights[at], corner, || {
            sees_corner(corner)
        });
    }

    let sun_first = grid.sunlight_lamp_slot;
    let sun_band = corner_pick::sun_band_light(sun_first);
    let choice = corner_pick::grid_choose_main_light_from_corners(&pick_input, sun_band);
    let chosen_light = illum_lookup::grid_remap_main_light(
        choice.dominant_light,
        lookup_backup,
        grid.carries_light_volumes,
        sun_first,
    );

    let mut kept = Vec::with_capacity(8);
    for ((alive, entry), weight) in choice
        .entry_in_use
        .into_iter()
        .zip(corner_entries)
        .zip(corner_weights)
    {
        if let (true, Some(entry)) = (alive, entry) {
            kept.push(GridProbeCornerSample {
                vertex_color_slot: entry.vertex_color_slot,
                dominant_light: entry.key_light_index,
                share: weight,
                ray_check_permits: false,
            });
        }
    }

    let (sums, sets) =
        illum_lookup::grid_sum_corner_contributions(&kept, chosen_light, sun_band);
    Ok(LightGridLookup {
        path: path_for(sets.count, fallback_set, grid.tint_key_total),
        accum: sets,
        buckets: sums,
        default_grid_entry: fallback_set,
        corner_entries,
        corner_weights,
        kept: kept.len(),
        chosen_light,
    })
}

/// The average colour of a model-lighting tile, each channel 0 to 1.
pub fn mean_tile_rgba01(texels: &[u8; ILLUM_TILE_LEN]) -> [f32; 3] {
    let texel_count = (ILLUM_TILE_LEN / 4) as f32;
    core::array::from_fn(|channel| {
        let channel_sum = texels
            .iter()
            .skip(channel)
            .step_by(4)
            .fold(0.0f32, |acc, &byte| acc + f32::from(byte) / 255.0);
        channel_sum / texel_count
    })
}
