//! BO2's mark density limit: world marks are bucketed into 20-unit cells, and a new
//! mark is refused when its cell already holds 8. MW2 has no such limit.

use crate::FX_MARKS_LIMIT;

/// Cell size is 20 units on every axis (position times 0.05, floored).
pub const FX_MARKS_GRID_CELL_SCALE: f32 = 0.05;

pub const FX_MARKS_GRID_BUCKETS: usize = 256;

/// A new mark is refused once its cell holds this many (BO2: 8).
pub const FX_MARKS_GRID_CELL_LIMIT: u16 = 8;

/// How much frame time passes between grid rebuilds (BO2: 500 ms).
pub const FX_MARKS_GRID_UPDATE_MSEC: i32 = 500;

const ENTRY_NONE: u16 = 0xffff;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FxMarksGridCell {
    pub x: i16,
    pub y: i16,
    pub z: i16,
}

fn grid_axis(v: f32) -> Option<i16> {
    let s = v * FX_MARKS_GRID_CELL_SCALE;
    // Also refuses NaN.
    if !(s >= i16::MIN as f32 && s < i16::MAX as f32 + 1.0) {
        return None;
    }
    let t = s as i32;
    let f = if (t as f32) > s { t - 1 } else { t };
    Some(f as i16)
}

/// The cell holding `pos`, or None when an axis does not fit in an i16.
pub fn marks_grid_cell_from_position(pos: [f32; 3]) -> Option<FxMarksGridCell> {
    Some(FxMarksGridCell {
        x: grid_axis(pos[0])?,
        y: grid_axis(pos[1])?,
        z: grid_axis(pos[2])?,
    })
}

#[inline]
pub const fn marks_grid_hash_from_cell(cell: FxMarksGridCell) -> usize {
    let h = (cell.z as i32 * 61) - (cell.y as i32 * 95) - (cell.x as i32 * 75);
    (h & 0xff) as usize
}

/// The cell table: 256 buckets chained through at most one entry per live mark.
#[derive(Clone, Debug)]
pub struct FxMarksGrid {
    heads: [u16; FX_MARKS_GRID_BUCKETS],
    cells: [FxMarksGridCell; FX_MARKS_LIMIT as usize],
    counts: [u16; FX_MARKS_LIMIT as usize],
    next: [u16; FX_MARKS_LIMIT as usize],
    len: u16,
    pending_msec: i32,
}

impl Default for FxMarksGrid {
    fn default() -> Self {
        Self::new()
    }
}

impl FxMarksGrid {
    pub const fn new() -> Self {
        Self {
            heads: [ENTRY_NONE; FX_MARKS_GRID_BUCKETS],
            cells: [FxMarksGridCell { x: 0, y: 0, z: 0 }; FX_MARKS_LIMIT as usize],
            counts: [0; FX_MARKS_LIMIT as usize],
            next: [ENTRY_NONE; FX_MARKS_LIMIT as usize],
            len: 0,
            pending_msec: 0,
        }
    }

    pub fn clear(&mut self) {
        self.heads = [ENTRY_NONE; FX_MARKS_GRID_BUCKETS];
        self.len = 0;
    }

    fn find(&self, cell: FxMarksGridCell) -> Option<usize> {
        let mut e = self.heads[marks_grid_hash_from_cell(cell)];
        while e != ENTRY_NONE {
            if self.cells[e as usize] == cell {
                return Some(e as usize);
            }
            e = self.next[e as usize];
        }
        None
    }

    /// Counts one mark at `pos`.
    pub fn add(&mut self, pos: [f32; 3]) {
        let Some(cell) = marks_grid_cell_from_position(pos) else {
            return;
        };
        if let Some(e) = self.find(cell) {
            self.counts[e] = self.counts[e].saturating_add(1);
            return;
        }
        if self.len as usize >= self.cells.len() {
            return;
        }
        let e = self.len as usize;
        let bucket = marks_grid_hash_from_cell(cell);
        self.cells[e] = cell;
        self.counts[e] = 1;
        self.next[e] = self.heads[bucket];
        self.heads[bucket] = e as u16;
        self.len += 1;
    }

    pub fn count_at(&self, pos: [f32; 3]) -> u16 {
        marks_grid_cell_from_position(pos)
            .and_then(|cell| self.find(cell))
            .map_or(0, |e| self.counts[e])
    }

    /// True when a new mark at `pos` must be refused; always false with the grid off.
    pub fn too_many_marks_in_vicinity(&self, grid_enable: bool, pos: [f32; 3]) -> bool {
        grid_enable && self.count_at(pos) >= FX_MARKS_GRID_CELL_LIMIT
    }

    /// Adds a frame's time; true when the grid is due a rebuild.
    pub fn advance(&mut self, frame_msec: i32) -> bool {
        self.pending_msec = self.pending_msec.saturating_add(frame_msec.max(0));
        if self.pending_msec < FX_MARKS_GRID_UPDATE_MSEC {
            return false;
        }
        // BO2 detail unknown: whether the remainder carries over.
        self.pending_msec = 0;
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cells_are_twenty_units_and_floor_negative() {
        let c = marks_grid_cell_from_position([19.9, 20.0, -0.1]).unwrap();
        assert_eq!((c.x, c.y, c.z), (0, 1, -1));
        assert!(marks_grid_cell_from_position([1.0e9, 0.0, 0.0]).is_none());
        assert!(marks_grid_cell_from_position([f32::NAN, 0.0, 0.0]).is_none());
    }

    #[test]
    fn hash_masks_to_a_byte() {
        let c = FxMarksGridCell { x: 1, y: 2, z: 3 };
        assert_eq!(
            marks_grid_hash_from_cell(c),
            ((183 - 190 - 75) & 0xff) as usize
        );
    }

    #[test]
    fn eighth_mark_in_a_cell_is_refused() {
        let mut g = FxMarksGrid::new();
        for i in 0..7 {
            g.add([i as f32, 5.0, 5.0]);
        }
        assert!(!g.too_many_marks_in_vicinity(true, [10.0, 10.0, 10.0]));
        g.add([1.0, 1.0, 1.0]);
        assert!(g.too_many_marks_in_vicinity(true, [10.0, 10.0, 10.0]));
        assert!(!g.too_many_marks_in_vicinity(false, [10.0, 10.0, 10.0]));
        assert!(!g.too_many_marks_in_vicinity(true, [30.0, 10.0, 10.0]));
        g.clear();
        assert_eq!(g.count_at([1.0, 1.0, 1.0]), 0);
    }

    #[test]
    fn rebuild_every_500_ms() {
        let mut g = FxMarksGrid::new();
        assert!(!g.advance(250));
        assert!(!g.advance(249));
        assert!(g.advance(1));
        assert!(!g.advance(16));
    }
}
