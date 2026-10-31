//! String tables read out of a zone: a grid of text cells.

/// A string table as stored in a zone: `rows` by `columns` text cells, row
/// by row. The first column is the row's key.
#[derive(Clone, Debug, Default)]
pub struct SnapshotStringTable {
    pub name: String,
    pub col_count: usize,
    pub height: usize,
    /// `rows * columns` cells, one row after another.
    pub grid: Vec<String>,
}

impl SnapshotStringTable {
    /// The text at `row`, `col`. Anything out of range, negative included,
    /// reads as empty text.
    #[must_use]
    pub fn cell(&self, r_ix: i32, c_ix: i32) -> &str {
        let (Ok(row), Ok(col)) = (usize::try_from(r_ix), usize::try_from(c_ix)) else {
            return "";
        };
        if row >= self.height || col >= self.col_count {
            return "";
        }
        let index = row.saturating_mul(self.col_count).saturating_add(col);
        self.grid.get(index).map_or("", String::as_str)
    }

    /// The first row whose `col` cell equals `key`, ignoring ASCII case.
    #[must_use]
    pub fn row_where_cell(&self, col: i32, wanted: &str) -> Option<i32> {
        (0..self.height)
            .map(|row| row as i32)
            .find(|&row| self.cell(row, col).eq_ignore_ascii_case(wanted))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn table() -> SnapshotStringTable {
        SnapshotStringTable {
            name: "stats".into(),
            col_count: 2,
            height: 2,
            grid: ["ak47", "AK-47", "m16", "M16A4"].map(String::from).to_vec(),
        }
    }

    #[test]
    fn cells_are_read_row_by_row() {
        let table = table();
        assert_eq!(table.cell(0, 1), "AK-47");
        assert_eq!(table.cell(1, 0), "m16");
    }

    #[test]
    fn cells_outside_the_grid_are_empty() {
        let table = table();
        assert_eq!(table.cell(-1, 0), "");
        assert_eq!(table.cell(0, -1), "");
        assert_eq!(table.cell(2, 0), "");
        assert_eq!(table.cell(0, 2), "");
    }

    #[test]
    fn a_key_matches_without_regard_to_case() {
        let table = table();
        assert_eq!(table.row_where_cell(0, "M16"), Some(1));
        assert_eq!(table.row_where_cell(1, "ak-47"), Some(0));
        assert_eq!(table.row_where_cell(0, "ppsh"), None);
    }
}
