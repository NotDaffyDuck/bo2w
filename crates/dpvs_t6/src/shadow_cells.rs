//! cellCasterBits: for each cell, the cells between it and the sun through the portals
//! (R_GenerateShadowMapCasterCells), and the sm_strictCull union of the rows of the cells the
//! camera sees.

use crate::cell::DpvsWorld;
use crate::cell_bits::{bit_get, top_bit_get, top_bit_set, word_span};
use crate::portal_crawl::{TraversalScratchpad, crawl_portals_unbounded};

/// One cell's row: (cellCount + 31) >> 5 words, MSB first.
#[must_use]
pub fn cell_caster_bitset_width(cell_count: usize) -> usize {
    word_span(cell_count)
}

/// One row per cell.
#[must_use]
pub fn cell_caster_grid_word_total(cell_count: usize) -> usize {
    cell_count.saturating_mul(word_span(cell_count))
}

/// R_GenerateShadowMapCasterCells: one R_VisitPortalsNoFrustum per seed cell, written into
/// that cell's row of `out` (each row holds its own cell). `sun_dir` points toward the
/// sun. BO2 builds it only for a map with a sun (sunPrimaryLightIndex != 0); that check is
/// the caller's. `out` is cleared first and left clear when it is shorter than the matrix.
pub fn collect_shadow_caster_cells(
    world: &DpvsWorld<'_>,
    toward_sun: [f32; 3],
    scratch: &mut TraversalScratchpad,
    dst_buf: &mut [u32],
) {
    let cell_count = world.cells.len();
    let row = word_span(cell_count);
    dst_buf.fill(0);
    if dst_buf.len() < cell_caster_grid_word_total(cell_count) {
        return;
    }
    for start_cell in 0..cell_count {
        crawl_portals_unbounded(world, start_cell, toward_sun, scratch);
        let dst = &mut dst_buf[start_cell * row..(start_cell + 1) * row];
        for (word_index, &bit_word) in scratch.visible_bits().iter().enumerate() {
            let mut w = bit_word;
            while w != 0 {
                top_bit_set(dst, word_index * 32 + w.trailing_zeros() as usize);
                w &= w - 1;
            }
        }
    }
}

/// sm_strictCull: the union of the caster rows of the cells the camera sees (`visible_lsb`,
/// the walk's LSB-first bits), or of every row when `sight_all`. Writes `out_msb` (one row,
/// MSB first) and returns how many cells it holds.
pub fn merge_caster_rows_of_visible(
    grid: &[u32],
    cell_count: usize,
    low_seen_bit: &[u32],
    sight_all: bool,
    top_out: &mut [u32],
) -> u32 {
    let line_ix = word_span(cell_count);
    top_out.fill(0);
    if line_ix == 0 || grid.len() < cell_count * line_ix || top_out.len() < line_ix {
        return 0;
    }
    for origin_ix in 0..cell_count {
        if !sight_all && !bit_get(low_seen_bit, origin_ix) {
            continue;
        }
        let start = origin_ix * line_ix;
        for (out_buf, src) in top_out.iter_mut().zip(&grid[start..start + line_ix]) {
            *out_buf |= *src;
        }
    }
    (0..cell_count).filter(|&cell| top_bit_get(top_out, cell)).count() as u32
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cell::{Cell, Portal};
    use alloc::vec;
    use alloc::vec::Vec;

    // Three rooms along +x joined by 2x2 doorways at x 10 and 20, each portal's plane facing
    // into the room it leads to.
    fn hall() -> (Vec<Cell>, Vec<Portal>, Vec<[f32; 3]>) {
        let mut vertex_points = Vec::new();
        let mut portals = Vec::new();
        let mut door = |x: f32, dir: f32, to: u16| {
            let vertex_first = vertex_points.len() as u32;
            for (y, z) in [(-1.0, -1.0), (-1.0, 1.0), (1.0, 1.0), (1.0, -1.0)] {
                vertex_points.push([x, y, z]);
            }
            portals.push(Portal {
                plane: [dir, 0.0, 0.0, -dir * x],
                cell: to,
                vertex_first,
                vertex_tally: 4,
                hull_axes_dir: [[0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
            });
        };
        door(10.0, 1.0, 1);
        door(20.0, 1.0, 2);
        door(10.0, -1.0, 0);
        door(20.0, -1.0, 1);
        let cell = |x0: f32, portal_start: u32, portal_opening_total: u32| Cell {
            lo: [x0, -5.0, -5.0],
            hi: [x0 + 10.0, 5.0, 5.0],
            portal_start,
            portal_opening_total,
        };
        // Portals by cell: 0 -> [0], 1 -> [1, 2], 2 -> [3].
        let cells = vec![cell(0.0, 0, 1), cell(10.0, 1, 2), cell(20.0, 3, 1)];
        (cells, portals, vertex_points)
    }

    fn rows(sun_dir: [f32; 3]) -> Vec<u32> {
        let (cells, portals, vertex_points) = hall();
        let world = DpvsWorld {
            cells: &cells,
            portals: &portals,
            vertex_points: &vertex_points,
            planes: &[],
            nodes: &[],
        };
        let mut out_pts = vec![0u32; cell_caster_grid_word_total(cells.len())];
        collect_shadow_caster_cells(&world, sun_dir, &mut TraversalScratchpad::default(), &mut out_pts);
        out_pts
    }

    #[test]
    fn each_row_holds_the_cells_toward_the_sun() {
        assert_eq!(word_span(3), 1);
        assert_eq!(word_span(33), 2);
        let m = rows([1.0, 0.0, 0.0]);
        assert_eq!(m, [0xE000_0000, 0x6000_0000, 0x2000_0000]);
        let m = rows([-1.0, 0.0, 0.0]);
        assert_eq!(m, [0x8000_0000, 0xC000_0000, 0xE000_0000]);
    }

    #[test]
    fn strict_cull_ors_the_rows_of_the_visible_cells() {
        let m = rows([1.0, 0.0, 0.0]);
        let mut out = [0u32; 1];
        assert_eq!(merge_caster_rows_of_visible(&m, 3, &[0b100], false, &mut out), 1);
        assert_eq!(out, [0x2000_0000]);
        assert_eq!(merge_caster_rows_of_visible(&m, 3, &[0b010], false, &mut out), 2);
        assert_eq!(merge_caster_rows_of_visible(&m, 3, &[], true, &mut out), 3);
        assert_eq!(out, [0xE000_0000]);
    }
}
