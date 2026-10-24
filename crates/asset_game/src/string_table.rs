//! String tables read out of a zone: a grid of text cells.

/// A string table as stored in a zone: `rows` by `columns` text cells, row
/// by row. The first column is the row's key.
#[derive(Clone, Debug, Default)]
pub struct CapturedStringTable {
    pub name: String,
    pub columns: usize,
    pub rows: usize,
    /// `rows * columns` cells, one row after another.
    pub cells: Vec<String>,
}

impl CapturedStringTable {
    /// The text at `row`, `col`. Anything out of range, negative included,
    /// reads as empty text.
    #[must_use]
    pub fn cell(&self, row: i32, col: i32) -> &str {
        let (Ok(row), Ok(col)) = (usize::try_from(row), usize::try_from(col)) else {
            return "";
        };
        if row >= self.rows || col >= self.columns {
            return "";
        }
        let index = row.saturating_mul(self.columns).saturating_add(col);
        self.cells.get(index).map_or("", String::as_str)
    }

    /// The first row whose `col` cell equals `key`, ignoring ASCII case.
    #[must_use]
    pub fn lookup_row_in_col(&self, col: i32, key: &str) -> Option<i32> {
        (0..self.rows)
            .map(|row| row as i32)
            .find(|&row| self.cell(row, col).eq_ignore_ascii_case(key))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn table() -> CapturedStringTable {
        CapturedStringTable {
            name: "stats".into(),
            columns: 2,
            rows: 2,
            cells: ["ak47", "AK-47", "m16", "M16A4"].map(String::from).to_vec(),
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
        assert_eq!(table.lookup_row_in_col(0, "M16"), Some(1));
        assert_eq!(table.lookup_row_in_col(1, "ak-47"), Some(0));
        assert_eq!(table.lookup_row_in_col(0, "ppsh"), None);
    }
}
