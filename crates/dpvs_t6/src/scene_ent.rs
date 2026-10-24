//! BO2's scene-ent cell rows (GfxWorldDpvsPlanes.sceneEntCellBits) and the BSP node filter
//! that fills them (R_FilterDObjIntoCells, R_FilterBModelIntoCells, R_UnfilterEntFromCells and
//! the recursive filter under them).
//!
//! One row of 0x100 words per (bank, cell), one bit per entnum, MSB first. Bank 0 holds DObjs
//! and models; bank 1 holds brush models and starts at row `cell_count`. Only client 0's slice
//! is kept (one local view); client c's slice would start at word c * (entCount >> 5).
//!
//! The filter tests the box against each node plane: wholly in front goes to the next node,
//! wholly behind to the back child, a straddle visits both. On a straddled axial plane
//! (type < 3) the box is narrowed to the part on each side before going on.

use crate::aabb::Bounds;
use crate::cell::{CPlane, DpvsPlanes};

pub const SCENE_ENT_CELL_ROW_WORDS: usize = 0x100;

pub const SCENE_ENT_CELL_VIEW_BANKS: usize = 2;

#[must_use]
pub fn scene_ent_cell_bits_len(cell_count: usize) -> usize {
    cell_count.saturating_mul(SCENE_ENT_CELL_ROW_WORDS * SCENE_ENT_CELL_VIEW_BANKS)
}

/// Clears the ent's bit in every row of both banks.
pub fn unfilter_scene_ent_from_cells_view0(bits: &mut [u32], cell_count: usize, ent_id: u32) {
    let word = (ent_id >> 5) as usize;
    let mask = !(0x8000_0000u32 >> (ent_id & 31));
    let rows = cell_count.saturating_mul(SCENE_ENT_CELL_VIEW_BANKS);
    for row in 0..rows {
        if let Some(slot) = bits.get_mut(row * SCENE_ENT_CELL_ROW_WORDS + word) {
            *slot &= mask;
        }
    }
}

pub fn add_scene_ent_to_cell(bits: &mut [u32], cell: usize, ent_id: u32) {
    add_to_row(bits, cell, ent_id);
}

fn add_to_row(bits: &mut [u32], row: usize, ent_id: u32) {
    let index = row
        .saturating_mul(SCENE_ENT_CELL_ROW_WORDS)
        .saturating_add((ent_id >> 5) as usize);
    if let Some(slot) = bits.get_mut(index) {
        *slot |= 0x8000_0000u32 >> (ent_id & 31);
    }
}

#[must_use]
pub fn scene_ent_cell_row(bits: &[u32], cell: usize) -> &[u32] {
    let start = cell * SCENE_ENT_CELL_ROW_WORDS;
    bits.get(start..start + SCENE_ENT_CELL_ROW_WORDS)
        .unwrap_or(&[])
}

/// The cell's brush-model row (bank 1).
#[must_use]
pub fn scene_ent_cell_row_second_pass(bits: &[u32], cell_count: usize, cell: usize) -> &[u32] {
    scene_ent_cell_row(bits, cell_count.saturating_add(cell))
}

/// Bits the per-cell visit walks in a row: entCount >> 5 words.
#[must_use]
pub fn scene_ent_cell_walk_bits(gfx_cfg_ent_count: u32) -> usize {
    ((gfx_cfg_ent_count as usize) >> 5).saturating_mul(32)
}

/// R_FilterDObjIntoCells: the ent leaves every row, then joins the bank 0 row of each cell
/// its box reaches.
pub fn filter_scene_ent_into_cells(
    planes: &DpvsPlanes<'_>,
    bounds: Bounds,
    bits: &mut [u32],
    ent_id: u32,
) {
    filter_into_bank(planes, bounds, bits, ent_id, 0);
}

/// R_FilterBModelIntoCells: as the DObj filter, into bank 1.
pub fn filter_bmodel_into_cells(
    planes: &DpvsPlanes<'_>,
    bounds: Bounds,
    bits: &mut [u32],
    ent_id: u32,
) {
    filter_into_bank(planes, bounds, bits, ent_id, planes.cell_count as usize);
}

fn filter_into_bank(
    planes: &DpvsPlanes<'_>,
    bounds: Bounds,
    bits: &mut [u32],
    ent_id: u32,
    row_offset: usize,
) {
    unfilter_scene_ent_from_cells_view0(bits, planes.cell_count as usize, ent_id);
    scene_ent_cells(planes, bounds, &mut |cell| {
        add_to_row(bits, row_offset + cell as usize, ent_id);
    });
}

/// The cells the filter puts a box in, in walk order. Callers that cache cells in place of
/// filtering get the same rows.
pub fn scene_ent_cells(planes: &DpvsPlanes<'_>, bounds: Bounds, on_cell: &mut impl FnMut(u32)) {
    filter_cells(planes, bounds, true, on_cell);
}

/// Whether the filter puts the box in `cell`.
#[must_use]
pub fn scene_ent_box_reaches_cell(planes: &DpvsPlanes<'_>, bounds: Bounds, cell: usize) -> bool {
    let mut reached = false;
    scene_ent_cells(planes, bounds, &mut |c| reached |= c as usize == cell);
    reached
}

/// The node walk; `narrow` cuts the box at a straddled axial plane (the scene-ent filter does).
pub(crate) fn filter_cells(
    planes: &DpvsPlanes<'_>,
    bounds: Bounds,
    narrow: bool,
    on_cell: &mut impl FnMut(u32),
) {
    if planes.nodes.is_empty() || planes.cell_count == 0 {
        return;
    }
    filter_r(planes, 0, bounds, narrow, on_cell, 0);
}

/// Box on plane: 1 wholly in front, 2 wholly behind, 3 both.
fn box_on_plane_side(bounds: Bounds, plane: &CPlane) -> u8 {
    let (mid, half) = (bounds.mid(), bounds.half());
    let n = plane.normal;
    let d = mid[0] * n[0] + mid[1] * n[1] + mid[2] * n[2] - plane.dist;
    let r = half[0] * n[0].abs() + half[1] * n[1].abs() + half[2] * n[2].abs();
    if d >= r {
        1
    } else if d <= -r {
        2
    } else {
        3
    }
}

fn narrowed(bounds: Bounds, axis: usize, dist: f32, front: bool) -> Bounds {
    let (mut mins, mut maxs) = (bounds.mins(), bounds.maxs());
    if front {
        mins[axis] = dist;
    } else {
        maxs[axis] = dist;
    }
    Bounds::from_mins_maxs(mins, maxs)
}

fn filter_r(
    planes: &DpvsPlanes<'_>,
    mut i: usize,
    mut bounds: Bounds,
    narrow: bool,
    on_cell: &mut impl FnMut(u32),
    depth: usize,
) {
    if depth > planes.nodes.len() {
        return;
    }
    let cell_count = planes.cell_count as usize;
    for _ in 0..planes.nodes.len() {
        let Some(&node) = planes.nodes.get(i) else {
            return;
        };
        let value = usize::from(node);
        if value <= cell_count {
            if let Some(cell) = value.checked_sub(1).filter(|&c| c < cell_count) {
                on_cell(cell as u32);
            }
            return;
        }
        let Some(plane) = planes.planes.get(value - (cell_count + 1)) else {
            return;
        };
        let Some(&offset) = planes.nodes.get(i + 1) else {
            return;
        };
        let (front, back) = (i + 2, i + usize::from(offset));
        match box_on_plane_side(bounds, plane) {
            1 => i = front,
            2 => i = back,
            _ => {
                let axis = usize::from(plane.r#type);
                if narrow && axis < 3 {
                    let front_box = narrowed(bounds, axis, plane.dist, true);
                    filter_r(planes, front, front_box, narrow, on_cell, depth + 1);
                    bounds = narrowed(bounds, axis, plane.dist, false);
                } else {
                    filter_r(planes, front, bounds, narrow, on_cell, depth + 1);
                }
                i = back;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vis::msb_get;
    use alloc::vec::Vec;

    fn axial(axis: u8, dist: f32) -> CPlane {
        let mut normal = [0.0; 3];
        normal[usize::from(axis)] = 1.0;
        CPlane { normal, dist, r#type: axis }
    }

    // x = 0 splits cells 0 (front) and 1 (back); behind it x = -10 splits 1 and 2.
    // nodes: [plane0, back@3, leaf 1, plane1, back@3, leaf 2, leaf 3]
    fn two_planes() -> ([CPlane; 2], [u16; 7]) {
        ([axial(0, 0.0), axial(0, -10.0)], [4, 3, 1, 5, 3, 2, 3])
    }

    fn cells(planes: &DpvsPlanes<'_>, mins: [f32; 3], maxs: [f32; 3]) -> Vec<u32> {
        let mut out = Vec::new();
        scene_ent_cells(planes, Bounds::from_mins_maxs(mins, maxs), &mut |c| out.push(c));
        out
    }

    #[test]
    fn straddle_visits_both_sides() {
        let (p, n) = two_planes();
        let planes = DpvsPlanes { planes: &p, nodes: &n, cell_count: 3 };
        assert_eq!(cells(&planes, [1.0; 3], [2.0; 3]), [0]);
        assert_eq!(cells(&planes, [-5.0, 0.0, 0.0], [5.0, 1.0, 1.0]), [0, 1]);
        assert_eq!(cells(&planes, [-20.0, 0.0, 0.0], [5.0, 1.0, 1.0]), [0, 1, 2]);
    }

    #[test]
    fn axial_straddle_narrows_the_box() {
        // A slanted root plane, then x = 0 behind it. A box straddling both reaches the
        // back of x = 0 only through its narrowed part.
        let slanted = CPlane { normal: [0.0, 1.0, 0.0], dist: 0.0, r#type: 3 };
        let p = [slanted, axial(0, 0.0)];
        // nodes: [slanted, back@3, leaf 1, x=0, back@3, leaf 2, leaf 3]
        let n = [4u16, 3, 1, 5, 3, 2, 3];
        let planes = DpvsPlanes { planes: &p, nodes: &n, cell_count: 3 };
        assert_eq!(cells(&planes, [-1.0, -1.0, 0.0], [1.0, 1.0, 1.0]), [0, 1, 2]);
        assert_eq!(cells(&planes, [1.0, -1.0, 0.0], [2.0, 1.0, 1.0]), [0, 1]);
    }

    #[test]
    fn rows_and_banks() {
        let (p, n) = two_planes();
        let planes = DpvsPlanes { planes: &p, nodes: &n, cell_count: 3 };
        let mut bits = alloc::vec![0u32; scene_ent_cell_bits_len(3)];
        let b = Bounds::from_mins_maxs([-5.0, 0.0, 0.0], [5.0, 1.0, 1.0]);
        filter_scene_ent_into_cells(&planes, b, &mut bits, 37);
        filter_bmodel_into_cells(&planes, b, &mut bits, 40);
        assert!(msb_get(scene_ent_cell_row(&bits, 0), 37));
        assert!(msb_get(scene_ent_cell_row(&bits, 1), 37));
        assert!(!msb_get(scene_ent_cell_row(&bits, 2), 37));
        assert!(msb_get(scene_ent_cell_row_second_pass(&bits, 3, 1), 40));
        assert!(!msb_get(scene_ent_cell_row(&bits, 1), 40));
        assert_eq!(bits[SCENE_ENT_CELL_ROW_WORDS + 1], 0x8000_0000 >> 5);
        assert!(scene_ent_box_reaches_cell(&planes, b, 1));
        assert!(!scene_ent_box_reaches_cell(&planes, b, 2));

        filter_scene_ent_into_cells(&planes, Bounds::from_mins_maxs([-20.0; 3], [-15.0; 3]), &mut bits, 37);
        assert!(!msb_get(scene_ent_cell_row(&bits, 0), 37));
        assert!(msb_get(scene_ent_cell_row(&bits, 2), 37));
        // Re-filtering the DObj clears its bit in bank 1 too, as R_UnfilterEntFromCells does.
        assert!(msb_get(scene_ent_cell_row_second_pass(&bits, 3, 1), 40));
        unfilter_scene_ent_from_cells_view0(&mut bits, 3, 40);
        assert!(bits.iter().all(|&w| w & (0x8000_0000 >> 8) == 0 || w == 0x8000_0000 >> 5));
    }

    #[test]
    fn empty_tree_reaches_nothing() {
        let planes = DpvsPlanes { planes: &[], nodes: &[], cell_count: 0 };
        assert!(cells(&planes, [0.0; 3], [1.0; 3]).is_empty());
        assert_eq!(scene_ent_cell_walk_bits(2048), 2048);
    }
}
