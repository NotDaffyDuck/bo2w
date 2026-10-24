//! Reading one grid row: its header, and the run table that says which of a
//! row's cells have entries.
//!
//! A row covers a span of columns. Its run table splits that span into runs of
//! columns that look alike. A run is `[columns, entries per column]`; when it
//! has entries it goes on with the height of the first one (a byte, or two
//! bytes once the row is taller than 255). A run with no entries is just its
//! two bytes. The entries of every column of every run sit one after the other
//! from the row's `entry_base`, so a column's entries start at the running sum
//! of `columns * entries per column` over the runs before it.

/// Bytes in a row header.
pub const LIGHT_GRID_ROW_HEADER_SIZE: usize = 12;

/// A row start that says the row does not exist.
pub const LIGHT_GRID_ROW_ABSENT: u16 = 0xffff;

const EMPTY_RUN_BYTES: usize = 2;
const ENTRY_BYTES: u32 = 4;

/// The span a row covers and where its entries begin.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LightGridRowHeader {
    pub col_start: u16,
    pub col_count: u16,
    pub row_start: u16,
    pub row_count: u16,
    pub entry_base: u32,
}

impl LightGridRowHeader {
    pub fn from_bytes(bytes: &[u8]) -> Option<Self> {
        let head = bytes.get(..LIGHT_GRID_ROW_HEADER_SIZE)?;
        let word = |at: usize| u16::from_le_bytes([head[at], head[at + 1]]);
        Some(Self {
            col_start: word(0),
            col_count: word(2),
            row_start: word(4),
            row_count: word(6),
            entry_base: u32::from_le_bytes([head[8], head[9], head[10], head[11]]),
        })
    }

    /// Bytes a run with entries takes: its two counts plus the height, which
    /// needs a second byte when the row is taller than 255.
    pub const fn rle_stride(self) -> usize {
        3 + (self.row_count > 0xff) as usize
    }
}

/// Whether a column and row, counted from the row's start, are inside it.
pub const fn light_grid_row_contains_cell(header: &LightGridRowHeader, col: u32, row: u32) -> bool {
    col.wrapping_add(1) <= header.col_count as u32 && row.wrapping_add(1) <= header.row_count as u32
}

/// Whether `cell_col` is the last column the row covers.
pub const fn light_grid_at_row_last_column(cell_col: u32, header: &LightGridRowHeader) -> bool {
    cell_col.wrapping_add(1) == (header.col_start as u32).wrapping_add(header.col_count as u32)
}

/// Where a column falls in the run table.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LightGridRleCursor {
    /// Byte offset of the run in the table.
    pub run_offset: usize,
    /// How far into the run the column is.
    pub col_in_run: u32,
    /// Index of the first entry of the run's first column.
    pub entry_index: u32,
    /// Columns in the run.
    pub run_length: u8,
    /// Entries per column in the run; zero for an empty run.
    pub run_increment: u8,
}

/// Walks the run table to the run holding column `col` of the row. `None` when
/// the table ends first.
pub fn light_grid_rle_advance_to_column(
    rle: &[u8],
    row_count: u16,
    entry_base: u32,
    col: u32,
) -> Option<LightGridRleCursor> {
    let solid_bytes = 3 + (row_count > 0xff) as usize;
    let mut run_offset = 0usize;
    let mut entry_index = entry_base;
    let mut left = col;
    loop {
        let run_length = *rle.get(run_offset)?;
        let run_increment = *rle.get(run_offset + 1)?;
        if u32::from(run_length) > left {
            return Some(LightGridRleCursor {
                run_offset,
                col_in_run: left,
                entry_index,
                run_length,
                run_increment,
            });
        }
        entry_index = entry_index.wrapping_add(u32::from(run_increment) * u32::from(run_length));
        left -= u32::from(run_length);
        run_offset += if run_increment == 0 {
            EMPTY_RUN_BYTES
        } else {
            solid_bytes
        };
    }
}

/// The height of the first entry of a run with entries, read from the run's
/// bytes.
pub fn light_grid_solid_run_z_base(run: &[u8], wide_row: bool) -> Option<u32> {
    let low = *run.get(2)?;
    if !wide_row {
        return Some(u32::from(low));
    }
    let high = *run.get(3)?;
    Some(u32::from(u16::from_le_bytes([low, high])))
}

/// The entries around a point, as byte offsets into the entry table: the
/// column the point is in (height, then the one above) and the next column.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LightGridEntryQuad {
    pub corners: [Option<u32>; 4],
    /// The point sits below the row's first entry, so its trace bits are
    /// cleared.
    pub clear_needs_trace: bool,
}

/// Byte offset of entry `index` if a stack of `height` entries reaches
/// `above_base` cells over its first.
fn entry_offset(index: u32, above_base: u32, height: u8) -> Option<u32> {
    (above_base < u32::from(height)).then(|| index.wrapping_mul(ENTRY_BYTES))
}

/// The entry at a height in a column and the one over it.
fn stack_pair(index: u32, above_base: u32, height: u8) -> [Option<u32>; 2] {
    [
        entry_offset(index, above_base, height),
        entry_offset(index.wrapping_add(1), above_base.wrapping_add(1), height),
    ]
}

/// `(height of first entry, entries per column)` of a run that follows.
fn following_run(run: &[u8], wide_row: bool) -> Option<(u32, u8)> {
    let z_base = light_grid_solid_run_z_base(run, wide_row)?;
    Some((z_base, *run.get(1)?))
}

/// Entries around a point in a run that has them.
pub fn light_grid_solid_run_entry_quad(
    cursor: &LightGridRleCursor,
    row_in_header: u32,
    z_base: u32,
    at_row_last_column: bool,
    next_run: Option<&[u8]>,
    wide_row: bool,
) -> Option<LightGridEntryQuad> {
    let above = row_in_header.wrapping_sub(z_base);
    let step = cursor.run_increment;
    let here = cursor
        .entry_index
        .wrapping_add(u32::from(step).wrapping_mul(cursor.col_in_run))
        .wrapping_add(above);
    let left = stack_pair(here, above, step);

    let right = if cursor.col_in_run.wrapping_add(1) < u32::from(cursor.run_length) {
        stack_pair(here.wrapping_add(u32::from(step)), above, step)
    } else if at_row_last_column {
        [None, None]
    } else {
        let (next_z, next_step) = following_run(next_run?, wide_row)?;
        let past_run = cursor
            .entry_index
            .wrapping_add(u32::from(step).wrapping_mul(u32::from(cursor.run_length)));
        let next_above = row_in_header.wrapping_sub(next_z);
        stack_pair(past_run.wrapping_add(next_above), next_above, next_step)
    };

    Some(LightGridEntryQuad {
        corners: [left[0], left[1], right[0], right[1]],
        clear_needs_trace: row_in_header < z_base,
    })
}

/// Entries around a point in a run with none: only the next column can
/// contribute.
pub fn light_grid_empty_run_entry_quad(
    cursor: &LightGridRleCursor,
    row_in_header: u32,
    at_row_last_column: bool,
    next_run: Option<&[u8]>,
    wide_row: bool,
) -> Option<LightGridEntryQuad> {
    let clear_needs_trace = match next_run {
        Some(next) if *next.get(1)? != 0 => {
            let (next_z, next_step) = following_run(next, wide_row)?;
            row_in_header < u32::from(next_step).wrapping_add(next_z)
        }
        _ => false,
    };
    let mut corners = [None; 4];

    let next_column_is_in_this_run =
        cursor.col_in_run.wrapping_add(1) < u32::from(cursor.run_length);
    if !next_column_is_in_this_run && !at_row_last_column {
        let (next_z, next_step) = following_run(next_run?, wide_row)?;
        let next_above = row_in_header.wrapping_sub(next_z);
        let first = cursor.entry_index.wrapping_add(next_above);
        let right = stack_pair(first, next_above, next_step);
        corners[2] = right[0];
        corners[3] = right[1];
    }
    Some(LightGridEntryQuad {
        corners,
        clear_needs_trace,
    })
}

/// Entries around a point one column before the row starts: only the row's
/// first column is near.
pub fn light_grid_column_before_row_start_quad(
    entry_base: u32,
    row_in_header: u32,
    first_run: &[u8],
    wide_row: bool,
) -> Option<LightGridEntryQuad> {
    let (z_base, step) = following_run(first_run, wide_row)?;
    let above = row_in_header.wrapping_sub(z_base);
    let right = stack_pair(entry_base.wrapping_add(above), above, step);
    Some(LightGridEntryQuad {
        corners: [None, None, right[0], right[1]],
        clear_needs_trace: row_in_header < z_base,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn header(row_count: u16) -> LightGridRowHeader {
        LightGridRowHeader {
            col_start: 10,
            col_count: 6,
            row_start: 3,
            row_count,
            entry_base: 0,
        }
    }

    fn solid_cursor(col_in_run: u32) -> LightGridRleCursor {
        LightGridRleCursor {
            run_offset: 0,
            col_in_run,
            entry_index: 10,
            run_length: 3,
            run_increment: 2,
        }
    }

    #[test]
    fn a_header_reads_little_endian_fields() {
        let bytes = [1, 0, 2, 0, 3, 0, 4, 1, 5, 0, 0, 0, 99];
        let parsed = LightGridRowHeader::from_bytes(&bytes).unwrap();
        assert_eq!(
            parsed,
            LightGridRowHeader {
                col_start: 1,
                col_count: 2,
                row_start: 3,
                row_count: 260,
                entry_base: 5,
            }
        );
        assert_eq!(LightGridRowHeader::from_bytes(&bytes[..11]), None);
    }

    #[test]
    fn a_tall_row_has_a_wider_run_stride() {
        assert_eq!(header(255).rle_stride(), 3);
        assert_eq!(header(256).rle_stride(), 4);
    }

    #[test]
    fn contains_cell_counts_from_the_row_start() {
        let h = header(4);
        assert!(light_grid_row_contains_cell(&h, 5, 3));
        assert!(!light_grid_row_contains_cell(&h, 6, 0));
        assert!(!light_grid_row_contains_cell(&h, 0, 4));
    }

    #[test]
    fn the_last_column_is_the_one_before_the_span_ends() {
        let h = header(4);
        assert!(light_grid_at_row_last_column(15, &h));
        assert!(!light_grid_at_row_last_column(14, &h));
    }

    #[test]
    fn advancing_inside_the_first_run_keeps_the_entry_base() {
        let rle = [2, 3, 1, 4, 0];
        let cursor = light_grid_rle_advance_to_column(&rle, 8, 100, 1).unwrap();
        assert_eq!(
            cursor,
            LightGridRleCursor {
                run_offset: 0,
                col_in_run: 1,
                entry_index: 100,
                run_length: 2,
                run_increment: 3,
            }
        );
    }

    #[test]
    fn advancing_past_a_run_skips_its_entries_and_bytes() {
        let rle = [2, 3, 1, 4, 0];
        let cursor = light_grid_rle_advance_to_column(&rle, 8, 100, 3).unwrap();
        assert_eq!(cursor.run_offset, 3);
        assert_eq!(cursor.col_in_run, 1);
        assert_eq!(cursor.entry_index, 106);
        assert_eq!(cursor.run_increment, 0);
    }

    #[test]
    fn a_tall_row_run_takes_four_bytes() {
        let rle = [1, 1, 5, 0, 2, 0];
        let cursor = light_grid_rle_advance_to_column(&rle, 300, 0, 1).unwrap();
        assert_eq!(cursor.run_offset, 4);
        assert_eq!(cursor.entry_index, 1);
    }

    #[test]
    fn advancing_off_the_end_of_the_table_finds_nothing() {
        let rle = [2, 3, 1, 4, 0];
        assert_eq!(light_grid_rle_advance_to_column(&rle, 8, 0, 6), None);
        assert_eq!(light_grid_rle_advance_to_column(&[], 8, 0, 0), None);
        assert_eq!(light_grid_rle_advance_to_column(&[5], 8, 0, 0), None);
    }

    #[test]
    fn the_z_base_is_one_byte_or_two() {
        assert_eq!(light_grid_solid_run_z_base(&[1, 1, 7, 9], false), Some(7));
        assert_eq!(
            light_grid_solid_run_z_base(&[1, 1, 0x34, 0x12], true),
            Some(0x1234)
        );
        assert_eq!(light_grid_solid_run_z_base(&[1, 1, 7], true), None);
        assert_eq!(light_grid_solid_run_z_base(&[1, 1], false), None);
    }

    #[test]
    fn a_solid_run_quad_reads_two_columns_of_the_same_run() {
        let quad =
            light_grid_solid_run_entry_quad(&solid_cursor(1), 5, 4, false, None, false).unwrap();
        assert_eq!(quad.corners, [Some(52), None, Some(60), None]);
        assert!(!quad.clear_needs_trace);
    }

    #[test]
    fn a_solid_run_quad_one_row_below_the_first_entry_only_reaches_up_to_it() {
        let quad =
            light_grid_solid_run_entry_quad(&solid_cursor(1), 3, 4, false, None, false).unwrap();
        assert_eq!(quad.corners, [None, Some(48), None, Some(56)]);
        assert!(quad.clear_needs_trace);
    }

    #[test]
    fn a_solid_run_quad_well_below_the_first_entry_is_empty_and_clears_the_trace() {
        let quad =
            light_grid_solid_run_entry_quad(&solid_cursor(1), 1, 4, false, None, false).unwrap();
        assert_eq!(quad.corners, [None; 4]);
        assert!(quad.clear_needs_trace);
    }

    #[test]
    fn a_solid_run_quad_at_the_run_end_reads_the_next_run() {
        let next = [2, 3, 5];
        let quad =
            light_grid_solid_run_entry_quad(&solid_cursor(2), 5, 4, false, Some(&next), false)
                .unwrap();
        assert_eq!(quad.corners, [Some(60), None, Some(64), Some(68)]);
    }

    #[test]
    fn a_solid_run_quad_at_the_row_end_has_no_right_column() {
        let quad =
            light_grid_solid_run_entry_quad(&solid_cursor(2), 5, 4, true, None, false).unwrap();
        assert_eq!(quad.corners, [Some(60), None, None, None]);
    }

    #[test]
    fn a_solid_run_quad_at_the_run_end_needs_the_next_run() {
        assert_eq!(
            light_grid_solid_run_entry_quad(&solid_cursor(2), 5, 4, false, None, false),
            None
        );
    }

    #[test]
    fn an_empty_run_quad_only_sees_the_next_run() {
        let cursor = LightGridRleCursor {
            run_offset: 0,
            col_in_run: 1,
            entry_index: 40,
            run_length: 2,
            run_increment: 0,
        };
        let next = [3, 2, 1];
        let quad = light_grid_empty_run_entry_quad(&cursor, 1, false, Some(&next), false).unwrap();
        assert_eq!(quad.corners, [None, None, Some(160), Some(164)]);
        let above = light_grid_empty_run_entry_quad(&cursor, 3, false, Some(&next), false).unwrap();
        assert_eq!(above.corners, [None; 4]);
        assert!(!above.clear_needs_trace);
    }

    #[test]
    fn an_empty_run_quad_clears_the_trace_when_the_row_is_under_the_next_stack() {
        let cursor = LightGridRleCursor {
            run_offset: 0,
            col_in_run: 0,
            entry_index: 40,
            run_length: 2,
            run_increment: 0,
        };
        let next = [3, 2, 4];
        let quad = light_grid_empty_run_entry_quad(&cursor, 2, false, Some(&next), false).unwrap();
        assert!(quad.clear_needs_trace);
        assert_eq!(quad.corners, [None; 4]);
    }

    #[test]
    fn an_empty_run_quad_inside_the_run_has_no_entries() {
        let cursor = LightGridRleCursor {
            run_offset: 0,
            col_in_run: 0,
            entry_index: 0,
            run_length: 2,
            run_increment: 0,
        };
        let quad = light_grid_empty_run_entry_quad(&cursor, 0, false, None, false).unwrap();
        assert_eq!(quad.corners, [None; 4]);
        assert!(!quad.clear_needs_trace);
    }

    #[test]
    fn the_column_before_the_row_starts_sees_the_first_run() {
        let first = [2, 3, 2];
        let quad = light_grid_column_before_row_start_quad(50, 3, &first, false).unwrap();
        assert_eq!(quad.corners, [None, None, Some(204), Some(208)]);
        assert!(!quad.clear_needs_trace);
        let under = light_grid_column_before_row_start_quad(50, 1, &first, false).unwrap();
        assert!(under.clear_needs_trace);
    }
}
