//! Reading one grid row: its header, and the run table that says which of a
//! row's cells have entries.
//!
//! A row covers a span of columns. Its run table splits that span into runs of
//! columns that look alike. A run is `[columns, entries per column]`; when it
//! has entries it goes on with the height of the first one (a byte, or two
//! bytes once the row is taller than 255). A run with no entries is just its
//! two bytes. The entries of every column of every run sit one after the other
//! from the row's `first_entry_offset`, so a column's entries start at the running sum
//! of `columns * entries per column` over the runs before it.

/// Bytes in a row header.
pub const ROW_PREAMBLE_LEN: usize = 12;

/// A row start that says the row does not exist.
pub const ROW_NOT_SET: u16 = 0xffff;

/// Rows taller than this store run heights in two bytes.
const ONE_BYTE_HEIGHT_MAX: u16 = 0xff;

/// Size of one grid entry in the entry table.
const ENTRY_SIZE: u32 = 4;

/// Bytes a run with entries takes in a row of `line_total` cells: the two counts and a
/// one- or two-byte height.
const fn solid_run_bytes(line_total: u16) -> usize {
    if line_total > ONE_BYTE_HEIGHT_MAX { 4 } else { 3 }
}

/// The span a row covers and where its entries begin.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RowPreamble {
    pub span_first_column: u16,
    pub column_total: u16,
    pub row_first_cell: u16,
    pub line_total: u16,
    pub first_entry_offset: u32,
}

impl RowPreamble {
    /// Reads the four little-endian u16 fields and the u32 entry base.
    pub fn from_bytes(raw: &[u8]) -> Option<Self> {
        let head: &[u8; ROW_PREAMBLE_LEN] = raw.get(..ROW_PREAMBLE_LEN)?.try_into().ok()?;
        let [c0, c1, c2, c3, r0, r1, r2, r3, e0, e1, e2, e3] = *head;
        Some(Self {
            span_first_column: u16::from_le_bytes([c0, c1]),
            column_total: u16::from_le_bytes([c2, c3]),
            row_first_cell: u16::from_le_bytes([r0, r1]),
            line_total: u16::from_le_bytes([r2, r3]),
            first_entry_offset: u32::from_le_bytes([e0, e1, e2, e3]),
        })
    }

    /// Bytes a run with entries takes: its two counts plus the height, which
    /// needs a second byte when the row is taller than 255.
    pub const fn run_length_row_pitch(self) -> usize {
        solid_run_bytes(self.line_total)
    }
}

/// Whether a column and row, counted from the row's start, are inside it.
pub const fn row_holds_cell(preamble: &RowPreamble, col_no: u32, row_no: u32) -> bool {
    below_count(col_no, preamble.column_total) && below_count(row_no, preamble.line_total)
}

/// `at + 1 <= count`, with `at + 1` wrapping.
const fn below_count(at: u32, count: u16) -> bool {
    at.wrapping_add(1) <= count as u32
}

/// Whether `cell_col` is the last column the row covers.
pub const fn on_last_column(cell_col: u32, header: &RowPreamble) -> bool {
    let span_end = (header.span_first_column as u32).wrapping_add(header.column_total as u32);
    span_end == cell_col.wrapping_add(1)
}

/// Where a column falls in the run table.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RunWalker {
    /// Byte offset of the run in the table.
    pub span_start_offset: usize,
    /// How far into the run the column is.
    pub column_within_span: u32,
    /// Index of the first entry of the run's first column.
    pub run_entry_ordinal: u32,
    /// Columns in the run.
    pub span_cell_count: u8,
    /// Entries per column in the run; zero for an empty run.
    pub run_step_size: u8,
}

/// Walks the run table to the run holding column `col` of the row. `None` when
/// the table ends first.
pub fn grid_compressed_row_seek_column(
    rle: &[u8],
    line_total: u16,
    first_entry_offset: u32,
    col: u32,
) -> Option<RunWalker> {
    let mut at = RunWalker { span_start_offset: 0, column_within_span: col, run_entry_ordinal: first_entry_offset, span_cell_count: 0, run_step_size: 0 };
    loop {
        let [columns, per_column] = rle.get(at.span_start_offset..)?.first_chunk::<2>().copied()?;
        (at.span_cell_count, at.run_step_size) = (columns, per_column);
        let Some(past) = at.column_within_span.checked_sub(u32::from(columns)) else {
            return Some(at);
        };
        at.column_within_span = past;
        at.run_entry_ordinal = at.run_entry_ordinal.wrapping_add(u32::from(per_column) * u32::from(columns));
        at.span_start_offset += match per_column {
            0 => 2,
            _ => solid_run_bytes(line_total),
        };
    }
}

/// The height of the first entry of a run with entries, read from the run's
/// bytes.
pub fn solid_run_floor(run: &[u8], long_row_layout: bool) -> Option<u32> {
    let height = run.get(2..)?;
    match long_row_layout {
        true => height.first_chunk::<2>().map(|&two| u32::from(u16::from_le_bytes(two))),
        false => height.first().map(|&one| u32::from(one)),
    }
}

/// The entries around a point, as byte offsets into the entry table: the
/// column the point is in (height, then the one above) and the next column.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RunQuad {
    pub quad_cells: [Option<u32>; 4],
    /// The point sits below the row's first entry, so its trace bits are
    /// cleared.
    pub empty_run_requires_trace: bool,
}

/// One column's stack of entries, seen from the point's height.
#[derive(Clone, Copy)]
struct Stack {
    /// Entry index at the point's height (the stack's first entry plus `above`).
    at: u32,
    /// How many cells above the stack's first entry the point is (wrapping).
    above: u32,
    /// Entries in the stack.
    height: u8,
}

impl Stack {
    /// The column whose stack of `height` entries starts at entry `first` and at
    /// height `z_base`, seen from `row`.
    fn of(first_at: u32, floor_z: u32, tall: u8, row_no: u32) -> Self {
        let above = row_no.wrapping_sub(floor_z);
        Stack { at: first_at.wrapping_add(above), above, height: tall }
    }

    /// Byte offsets of the entry at the point's height and of the one over it,
    /// each only when the stack reaches it.
    fn pair(self) -> [Option<u32>; 2] {
        let top = u32::from(self.height);
        [0u32, 1].map(|up| (self.above.wrapping_add(up) < top).then(|| self.at.wrapping_add(up).wrapping_mul(ENTRY_SIZE)))
    }
}

/// A quad from the point's column and the next one.
fn quad(left: [Option<u32>; 2], right: [Option<u32>; 2], empty_run_requires_trace: bool) -> RunQuad {
    let [a, b] = left;
    let [c, d] = right;
    RunQuad { quad_cells: [a, b, c, d], empty_run_requires_trace }
}

/// The first column of the run after the cursor's, whose first entry is
/// `first`, seen from `row`.
fn next_run_stack(first_at: u32, after_run: Option<&[u8]>, long_rows: bool, row_no: u32) -> Option<Stack> {
    let (z_base, height) = run_head(after_run?, long_rows)?;
    Some(Stack::of(first_at, z_base, height, row_no))
}

/// A run's first-entry height and its entries per column.
fn run_head(run: &[u8], long_row_layout: bool) -> Option<(u32, u8)> {
    let z_base = solid_run_floor(run, long_row_layout)?;
    Some((z_base, *run.get(1)?))
}

/// Whether the column after the cursor's still belongs to the cursor's run.
fn next_column_in_run(cursor: &RunWalker) -> bool {
    cursor.column_within_span.wrapping_add(1) < u32::from(cursor.span_cell_count)
}

/// Entries around a point in a run that has them.
pub fn filled_quad(
    cursor: &RunWalker,
    header_row: u32,
    floor_z: u32,
    on_last_col: bool,
    run_after: Option<&[u8]>,
    long_row_layout: bool,
) -> Option<RunQuad> {
    let per_column = u32::from(cursor.run_step_size);
    let column_first = |col: u32| cursor.run_entry_ordinal.wrapping_add(per_column.wrapping_mul(col));
    let here = Stack::of(column_first(cursor.column_within_span), floor_z, cursor.run_step_size, header_row);

    let right = if next_column_in_run(cursor) {
        Stack { at: here.at.wrapping_add(per_column), ..here }.pair()
    } else if on_last_col {
        [None; 2]
    } else {
        let after_run = column_first(u32::from(cursor.span_cell_count));
        next_run_stack(after_run, run_after, long_row_layout, header_row)?.pair()
    };
    Some(quad(here.pair(), right, header_row < floor_z))
}

/// Entries around a point in a run with none: only the next column can
/// contribute.
pub fn grid_vacant_span_quad(
    cursor: &RunWalker,
    header_row: u32,
    on_last_col: bool,
    after_run: Option<&[u8]>,
    long_row_layout: bool,
) -> Option<RunQuad> {
    // Below the top of a following stack with entries, the trace bits go.
    let following_has_entries = match after_run {
        Some(next) => *next.get(1)? != 0,
        None => false,
    };
    let clear = if let (true, Some(next)) = (following_has_entries, after_run) {
        let (z_base, height) = run_head(next, long_row_layout)?;
        header_row < z_base.wrapping_add(u32::from(height))
    } else {
        false
    };

    let right = if next_column_in_run(cursor) || on_last_col {
        [None; 2]
    } else {
        next_run_stack(cursor.run_entry_ordinal, after_run, long_row_layout, header_row)?.pair()
    };
    Some(quad([None; 2], right, clear))
}

/// Entries around a point one column before the row starts: only the row's
/// first column is near.
pub fn column_before_quad(
    first_entry_offset: u32,
    header_row: u32,
    lead_run: &[u8],
    long_row_layout: bool,
) -> Option<RunQuad> {
    let (floor_z, tall) = run_head(lead_run, long_row_layout)?;
    let first_at = Stack::of(first_entry_offset, floor_z, tall, header_row);
    Some(quad([None; 2], first_at.pair(), header_row < floor_z))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn header(line_total: u16) -> RowPreamble {
        RowPreamble { span_first_column: 10, column_total: 6, row_first_cell: 3, line_total, first_entry_offset: 0 }
    }

    fn cursor(column_within_span: u32, run_entry_ordinal: u32, span_cell_count: u8, run_step_size: u8) -> RunWalker {
        RunWalker { span_start_offset: 0, column_within_span, run_entry_ordinal, span_cell_count, run_step_size }
    }

    /// Column `col` of a three-column run of two entries per column, from entry 10.
    fn solid_cursor(col: u32) -> RunWalker {
        cursor(col, 10, 3, 2)
    }

    fn solid(col: u32, row: u32, last: bool, next: Option<&[u8]>) -> Option<RunQuad> {
        filled_quad(&solid_cursor(col), row, 4, last, next, false)
    }

    const RUNS: [u8; 5] = [2, 3, 1, 4, 0];

    #[test]
    fn a_header_reads_little_endian_fields() {
        let bytes = [1, 0, 2, 0, 3, 0, 4, 1, 5, 0, 0, 0, 99];
        let want = RowPreamble { span_first_column: 1, column_total: 2, row_first_cell: 3, line_total: 260, first_entry_offset: 5 };
        assert_eq!(RowPreamble::from_bytes(&bytes), Some(want));
        assert_eq!(RowPreamble::from_bytes(&bytes[..11]), None);
    }

    #[test]
    fn a_tall_row_has_a_wider_run_stride() {
        assert_eq!([255, 256].map(|rows| header(rows).run_length_row_pitch()), [3, 4]);
    }

    #[test]
    fn contains_cell_counts_from_the_row_start() {
        let h = header(4);
        assert!(row_holds_cell(&h, 5, 3));
        assert!(!row_holds_cell(&h, 6, 0));
        assert!(!row_holds_cell(&h, 0, 4));
    }

    #[test]
    fn the_last_column_is_the_one_before_the_span_ends() {
        let h = header(4);
        assert!(on_last_column(15, &h));
        assert!(!on_last_column(14, &h));
    }

    #[test]
    fn advancing_inside_the_first_run_keeps_the_entry_base() {
        let got = grid_compressed_row_seek_column(&RUNS, 8, 100, 1);
        assert_eq!(got, Some(cursor(1, 100, 2, 3)));
    }

    #[test]
    fn advancing_past_a_run_skips_its_entries_and_bytes() {
        let got = grid_compressed_row_seek_column(&RUNS, 8, 100, 3).unwrap();
        assert_eq!((got.span_start_offset, got.column_within_span, got.run_entry_ordinal, got.run_step_size), (3, 1, 106, 0));
    }

    #[test]
    fn a_tall_row_run_takes_four_bytes() {
        let got = grid_compressed_row_seek_column(&[1, 1, 5, 0, 2, 0], 300, 0, 1).unwrap();
        assert_eq!((got.span_start_offset, got.run_entry_ordinal), (4, 1));
    }

    #[test]
    fn advancing_off_the_end_of_the_table_finds_nothing() {
        assert_eq!(grid_compressed_row_seek_column(&RUNS, 8, 0, 6), None);
        assert_eq!(grid_compressed_row_seek_column(&[], 8, 0, 0), None);
        assert_eq!(grid_compressed_row_seek_column(&[5], 8, 0, 0), None);
    }

    #[test]
    fn the_z_base_is_one_byte_or_two() {
        assert_eq!(solid_run_floor(&[1, 1, 7, 9], false), Some(7));
        assert_eq!(solid_run_floor(&[1, 1, 0x34, 0x12], true), Some(0x1234));
        assert_eq!(solid_run_floor(&[1, 1, 7], true), None);
        assert_eq!(solid_run_floor(&[1, 1], false), None);
    }

    #[test]
    fn a_solid_run_quad_reads_two_columns_of_the_same_run() {
        let got = solid(1, 5, false, None).unwrap();
        assert_eq!(got, RunQuad { quad_cells: [Some(52), None, Some(60), None], empty_run_requires_trace: false });
    }

    #[test]
    fn a_solid_run_quad_one_row_below_the_first_entry_only_reaches_up_to_it() {
        let got = solid(1, 3, false, None).unwrap();
        assert_eq!(got, RunQuad { quad_cells: [None, Some(48), None, Some(56)], empty_run_requires_trace: true });
    }

    #[test]
    fn a_solid_run_quad_well_below_the_first_entry_is_empty_and_clears_the_trace() {
        let got = solid(1, 1, false, None).unwrap();
        assert_eq!(got, RunQuad { quad_cells: [None; 4], empty_run_requires_trace: true });
    }

    #[test]
    fn a_solid_run_quad_at_the_run_end_reads_the_next_run() {
        let got = solid(2, 5, false, Some(&[2, 3, 5])).unwrap();
        assert_eq!(got.quad_cells, [Some(60), None, Some(64), Some(68)]);
    }

    #[test]
    fn a_solid_run_quad_at_the_row_end_has_no_right_column() {
        assert_eq!(solid(2, 5, true, None).unwrap().quad_cells, [Some(60), None, None, None]);
    }

    #[test]
    fn a_solid_run_quad_at_the_run_end_needs_the_next_run() {
        assert_eq!(solid(2, 5, false, None), None);
    }

    #[test]
    fn an_empty_run_quad_only_sees_the_next_run() {
        let at = cursor(1, 40, 2, 0);
        let next: &[u8] = &[3, 2, 1];
        let low = grid_vacant_span_quad(&at, 1, false, Some(next), false).unwrap();
        assert_eq!(low.quad_cells, [None, None, Some(160), Some(164)]);
        let high = grid_vacant_span_quad(&at, 3, false, Some(next), false).unwrap();
        assert_eq!(high, RunQuad { quad_cells: [None; 4], empty_run_requires_trace: false });
    }

    #[test]
    fn an_empty_run_quad_clears_the_trace_when_the_row_is_under_the_next_stack() {
        let got = grid_vacant_span_quad(&cursor(0, 40, 2, 0), 2, false, Some(&[3, 2, 4]), false).unwrap();
        assert_eq!(got, RunQuad { quad_cells: [None; 4], empty_run_requires_trace: true });
    }

    #[test]
    fn an_empty_run_quad_inside_the_run_has_no_entries() {
        let got = grid_vacant_span_quad(&cursor(0, 0, 2, 0), 0, false, None, false).unwrap();
        assert_eq!(got, RunQuad { quad_cells: [None; 4], empty_run_requires_trace: false });
    }

    #[test]
    fn the_column_before_the_row_starts_sees_the_first_run() {
        let first = [2, 3, 2];
        let level = column_before_quad(50, 3, &first, false).unwrap();
        assert_eq!(level, RunQuad { quad_cells: [None, None, Some(204), Some(208)], empty_run_requires_trace: false });
        let under = column_before_quad(50, 1, &first, false).unwrap();
        assert!(under.empty_run_requires_trace);
    }
}
