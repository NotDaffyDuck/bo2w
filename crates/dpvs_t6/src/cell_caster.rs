//! cellCasterBits: for each cell, the cells between it and the sun through the portals
//! (R_GenerateShadowMapCasterCells), and the sm_strictCull union of the rows of the cells the
//! camera sees.

use crate::cell::DpvsWorld;
use crate::vis::{bit_get, msb_get, msb_set, words_for_bits};
use crate::walk::{WalkScratch, visit_portals_no_frustum};

/// One cell's row: (cellCount + 31) >> 5 words, MSB first.
#[must_use]
pub fn cell_caster_row_words(cell_count: usize) -> usize {
    words_for_bits(cell_count)
}

/// One row per cell.
#[must_use]
pub fn cell_caster_matrix_words(cell_count: usize) -> usize {
    cell_count.saturating_mul(cell_caster_row_words(cell_count))
}

/// R_GenerateShadowMapCasterCells: one R_VisitPortalsNoFrustum per seed cell, written into
/// that cell's row of `out` (each row holds its own cell). `sun_dir` points toward the
/// sun. BO2 builds it only for a map with a sun (sunPrimaryLightIndex != 0); that check is
/// the caller's. `out` is cleared first and left clear when it is shorter than the matrix.
pub fn generate_shadow_map_caster_cells(
    world: &DpvsWorld<'_>,
    sun_dir: [f32; 3],
    scratch: &mut WalkScratch,
    out: &mut [u32],
) {
    let cell_count = world.cells.len();
    let row = cell_caster_row_words(cell_count);
    out.fill(0);
    if out.len() < cell_caster_matrix_words(cell_count) {
        return;
    }
    for seed in 0..cell_count {
        visit_portals_no_frustum(world, seed, sun_dir, scratch);
        let dst = &mut out[seed * row..(seed + 1) * row];
        for (word_index, &word) in scratch.visible_bits().iter().enumerate() {
            let mut w = word;
            while w != 0 {
                msb_set(dst, word_index * 32 + w.trailing_zeros() as usize);
                w &= w - 1;
            }
        }
    }
}

/// sm_strictCull: the union of the caster rows of the cells the camera sees (`visible_lsb`,
/// the walk's LSB-first bits), or of every row when `vis_all`. Writes `out_msb` (one row,
/// MSB first) and returns how many cells it holds.
pub fn or_caster_rows_for_visible(
    matrix: &[u32],
    cell_count: usize,
    visible_lsb: &[u32],
    vis_all: bool,
    out_msb: &mut [u32],
) -> u32 {
    let row = cell_caster_row_words(cell_count);
    out_msb.fill(0);
    if row == 0 || matrix.len() < cell_count * row || out_msb.len() < row {
        return 0;
    }
    for seed in 0..cell_count {
        if !vis_all && !bit_get(visible_lsb, seed) {
            continue;
        }
        let start = seed * row;
        for (dst, src) in out_msb.iter_mut().zip(&matrix[start..start + row]) {
            *dst |= *src;
        }
    }
    (0..cell_count).filter(|&cell| msb_get(out_msb, cell)).count() as u32
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
        let mut verts = Vec::new();
        let mut portals = Vec::new();
        let mut door = |x: f32, dir: f32, to: u16| {
            let vert_start = verts.len() as u32;
            for (y, z) in [(-1.0, -1.0), (-1.0, 1.0), (1.0, 1.0), (1.0, -1.0)] {
                verts.push([x, y, z]);
            }
            portals.push(Portal {
                plane: [dir, 0.0, 0.0, -dir * x],
                cell: to,
                vert_start,
                vert_count: 4,
                hull_axis: [[0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
            });
        };
        door(10.0, 1.0, 1);
        door(20.0, 1.0, 2);
        door(10.0, -1.0, 0);
        door(20.0, -1.0, 1);
        let cell = |x0: f32, portal_start: u32, portal_count: u32| Cell {
            mins: [x0, -5.0, -5.0],
            maxs: [x0 + 10.0, 5.0, 5.0],
            portal_start,
            portal_count,
        };
        // Portals by cell: 0 -> [0], 1 -> [1, 2], 2 -> [3].
        let cells = vec![cell(0.0, 0, 1), cell(10.0, 1, 2), cell(20.0, 3, 1)];
        (cells, portals, verts)
    }

    fn rows(sun_dir: [f32; 3]) -> Vec<u32> {
        let (cells, portals, verts) = hall();
        let world = DpvsWorld {
            cells: &cells,
            portals: &portals,
            verts: &verts,
            planes: &[],
            nodes: &[],
        };
        let mut out = vec![0u32; cell_caster_matrix_words(cells.len())];
        generate_shadow_map_caster_cells(&world, sun_dir, &mut WalkScratch::default(), &mut out);
        out
    }

    #[test]
    fn each_row_holds_the_cells_toward_the_sun() {
        assert_eq!(cell_caster_row_words(3), 1);
        assert_eq!(cell_caster_row_words(33), 2);
        let m = rows([1.0, 0.0, 0.0]);
        assert_eq!(m, [0xE000_0000, 0x6000_0000, 0x2000_0000]);
        let m = rows([-1.0, 0.0, 0.0]);
        assert_eq!(m, [0x8000_0000, 0xC000_0000, 0xE000_0000]);
    }

    #[test]
    fn strict_cull_ors_the_rows_of_the_visible_cells() {
        let m = rows([1.0, 0.0, 0.0]);
        let mut out = [0u32; 1];
        assert_eq!(or_caster_rows_for_visible(&m, 3, &[0b100], false, &mut out), 1);
        assert_eq!(out, [0x2000_0000]);
        assert_eq!(or_caster_rows_for_visible(&m, 3, &[0b010], false, &mut out), 2);
        assert_eq!(or_caster_rows_for_visible(&m, 3, &[], true, &mut out), 3);
        assert_eq!(out, [0xE000_0000]);
    }
}
