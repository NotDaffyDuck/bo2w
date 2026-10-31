//! Which cells each scene entity is in, kept as bit rows, and the BSP walk
//! that files an entity's box into cells.
//!
//! Each (bank, cell) pair owns a row of 0x100 words with one bit per entity
//! number, counted from the high end of each word. Bank 0 (animated and
//! plain models) uses rows `0..cell_count`; bank 1 (brush models) follows it.
//! Only the one local view's slice is kept.
//!
//! The walk tests the box against each node's plane: wholly in front goes
//! on to the next node, wholly behind to the back child, and a box across
//! the plane goes both ways. Across an axis-aligned plane the box is cut at
//! the plane first, so each side only sees its own part.

use crate::box_tree::Bounds;
use crate::cell::{PlaneEquation, VisibilityCellPlanes};

pub const ENTITY_CELL_BITSET_WORDS: usize = 0x100;

pub const ENTITY_CELL_VISIBILITY_SETS: usize = 2;

/// Words for every row of both banks.
#[must_use]
pub fn entity_cell_bitset_length(cell_count: usize) -> usize {
    cell_count.saturating_mul(ENTITY_CELL_BITSET_WORDS * ENTITY_CELL_VISIBILITY_SETS)
}

/// Word offset within a row, and the mask, of entity `actor_id`.
fn entity_bit(actor_id: u32) -> (usize, u32) {
    ((actor_id / 32) as usize, (1u32 << 31) >> (actor_id % 32))
}

/// Takes the entity out of every row of both banks.
pub fn clear_actor_cells_main_view(vis_words: &mut [u32], row_cells: usize, actor_id: u32) {
    let (offset, mask) = entity_bit(actor_id);
    let row_total = row_cells.saturating_mul(ENTITY_CELL_VISIBILITY_SETS);
    let words = vis_words.iter_mut().skip(offset).step_by(ENTITY_CELL_BITSET_WORDS).take(row_total);
    words.for_each(|word| *word &= !mask);
}

/// Puts the entity into the bank 0 row of `cell`.
pub fn mark_actor_in_cell(vis_words: &mut [u32], cell: usize, actor_id: u32) {
    mark_in_row(vis_words, cell, actor_id);
}

fn mark_in_row(bits: &mut [u32], row: usize, actor_id: u32) {
    let (offset, mask) = entity_bit(actor_id);
    let at = row.saturating_mul(ENTITY_CELL_BITSET_WORDS).saturating_add(offset);
    if let Some(bit_word) = bits.get_mut(at) {
        *bit_word |= mask;
    }
}

/// Row `cell` (bank 0 for `cell < cell_count`); empty when out of range.
#[must_use]
pub fn entity_cell_bitset_row(bitset: &[u32], cell: usize) -> &[u32] {
    let first = cell * ENTITY_CELL_BITSET_WORDS;
    match bitset.get(first..first + ENTITY_CELL_BITSET_WORDS) {
        Some(row) => row,
        None => &[],
    }
}

/// The cell's brush-model row (bank 1).
#[must_use]
pub fn entity_cell_row_later_pass(bit_row: &[u32], n_cells: usize, cell: usize) -> &[u32] {
    entity_cell_bitset_row(bit_row, n_cells.saturating_add(cell))
}

/// How many bits of a row the per-cell visit reads: the configured entity
/// total rounded down to whole words.
#[must_use]
pub fn actor_cell_mask(configured_entity_total: u32) -> usize {
    ((configured_entity_total as usize) >> 5).saturating_mul(32)
}

/// Refiles an animated or plain model: out of every row, then into the bank 0
/// row of each cell its box reaches.
pub fn sort_scene_entity_into_cells(
    planes: &VisibilityCellPlanes<'_>,
    ent_box: Bounds,
    cell_mask: &mut [u32],
    actor_id: u32,
) {
    refile(planes, ent_box, cell_mask, actor_id, Bank::Models);
}

/// Refiles a brush model the same way, into bank 1.
pub fn sort_brush_model_into_cells(
    planes: &VisibilityCellPlanes<'_>,
    ent_box: Bounds,
    row_bits: &mut [u32],
    actor_id: u32,
) {
    refile(planes, ent_box, row_bits, actor_id, Bank::BrushModels);
}

#[derive(Clone, Copy)]
enum Bank {
    Models,
    BrushModels,
}

fn refile(planes: &VisibilityCellPlanes<'_>, ent_box: Bounds, vis_words: &mut [u32], actor_id: u32, bank: Bank) {
    let row_cells = planes.leaf_cells as usize;
    let first_row = match bank {
        Bank::Models => 0,
        Bank::BrushModels => row_cells,
    };
    clear_actor_cells_main_view(vis_words, row_cells, actor_id);
    scene_ent_cells(planes, ent_box, &mut |cell| mark_in_row(vis_words, first_row + cell as usize, actor_id));
}

/// Every cell the walk files the box into, in walk order (a cell may repeat).
/// Callers that cache cells instead of walking get the same rows.
pub fn scene_ent_cells(planes: &VisibilityCellPlanes<'_>, ent_box: Bounds, on_cell: &mut impl FnMut(u32)) {
    filter_cells(planes, ent_box, true, on_cell);
}

/// Whether the walk files the box into `cell`.
#[must_use]
pub fn actor_box_touches_cell(planes: &VisibilityCellPlanes<'_>, ent_box: Bounds, cell: usize) -> bool {
    let mut touched = false;
    scene_ent_cells(planes, ent_box, &mut |found| touched = touched || found as usize == cell);
    touched
}

/// The BSP walk; with `narrow` the box is cut at axis-aligned planes it lies across.
pub(crate) fn filter_cells(
    planes: &VisibilityCellPlanes<'_>,
    ent_box: Bounds,
    narrow: bool,
    on_cell: &mut impl FnMut(u32),
) {
    let walk = Walk { tree: planes, narrow, cells_total: planes.leaf_cells as usize };
    if walk.cells_total > 0 && !planes.nodes.is_empty() {
        walk.visit(0, ent_box, on_cell, 0);
    }
}

/// Where a box lies relative to a plane.
enum Side {
    Front,
    Back,
    Across,
}

fn side_of(extent_box: Bounds, plane: &PlaneEquation) -> Side {
    let (c0, ext) = (extent_box.mid(), extent_box.half());
    let [nx, ny, nz] = plane.normal;
    let centre_dist = c0[0] * nx + c0[1] * ny + c0[2] * nz - plane.offset;
    let reach = ext[0] * nx.abs() + ext[1] * ny.abs() + ext[2] * nz.abs();
    if centre_dist >= reach {
        Side::Front
    } else if -reach >= centre_dist {
        Side::Back
    } else {
        Side::Across
    }
}

/// The box cut at `axis = at`: the part in front, then the part behind.
fn split_at(bounds: Bounds, ax: usize, cur: f32) -> (Bounds, Bounds) {
    let (low, high) = (bounds.low_corner(), bounds.high_corner());
    let (mut front_low, mut back_high) = (low, high);
    front_low[ax] = cur;
    back_high[ax] = cur;
    (Bounds::from_corner_extremes(front_low, high), Bounds::from_corner_extremes(low, back_high))
}

struct Walk<'t, 'p> {
    tree: &'t VisibilityCellPlanes<'p>,
    narrow: bool,
    cells_total: usize,
}

impl Walk<'_, '_> {
    /// Follows the back side in place and recurses into the front side of a
    /// box across a plane. `depth` and the step count both stop a broken tree.
    fn visit(&self, first_ix: usize, mut piece: Bounds, per_cell: &mut impl FnMut(u32), level: usize) {
        let nodes = self.tree.nodes;
        if level > nodes.len() {
            return;
        }
        let mut at = first_ix;
        for _ in nodes {
            let Some(&code) = nodes.get(at) else { return };
            let code = usize::from(code);
            let Some(plane_slot) = code.checked_sub(self.cells_total + 1) else {
                if let Some(cell) = code.checked_sub(1) {
                    per_cell(cell as u32);
                }
                return;
            };
            let (Some(split_plane), Some(&back_offset)) = (self.tree.planes.get(plane_slot), nodes.get(at + 1)) else {
                return;
            };
            let front = at + 2;
            let back = at + usize::from(back_offset);
            at = match side_of(piece, split_plane) {
                Side::Front => front,
                Side::Back => back,
                Side::Across => {
                    let axes = usize::from(split_plane.r#type);
                    if self.narrow && axes < 3 {
                        let (front_part, back_part) = split_at(piece, axes, split_plane.offset);
                        self.visit(front, front_part, per_cell, level + 1);
                        piece = back_part;
                    } else {
                        self.visit(front, piece, per_cell, level + 1);
                    }
                    back
                }
            };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cell_bits::top_bit_get;
    use alloc::vec::Vec;

    fn axial(along: u8, dist: f32) -> PlaneEquation {
        let mut facing = [0.0; 3];
        facing[usize::from(along)] = 1.0;
        PlaneEquation { normal: facing, offset: dist, r#type: along }
    }

    // x = 0 splits cells 0 (front) and 1 (back); behind it x = -10 splits 1 and 2.
    // nodes: [plane0, back@3, leaf 1, plane1, back@3, leaf 2, leaf 3]
    fn two_planes() -> ([PlaneEquation; 2], [u16; 7]) {
        ([axial(0, 0.0), axial(0, -10.0)], [4, 3, 1, 5, 3, 2, 3])
    }

    fn covered_cells(planes: &VisibilityCellPlanes<'_>, lo: [f32; 3], hi: [f32; 3]) -> Vec<u32> {
        let mut dst = Vec::new();
        scene_ent_cells(planes, Bounds::from_corner_extremes(lo, hi), &mut |c| dst.push(c));
        dst
    }

    #[test]
    fn straddle_visits_both_sides() {
        let (p, n) = two_planes();
        let planes = VisibilityCellPlanes { planes: &p, nodes: &n, leaf_cells: 3 };
        assert_eq!(covered_cells(&planes, [1.0; 3], [2.0; 3]), [0]);
        assert_eq!(covered_cells(&planes, [-5.0, 0.0, 0.0], [5.0, 1.0, 1.0]), [0, 1]);
        assert_eq!(covered_cells(&planes, [-20.0, 0.0, 0.0], [5.0, 1.0, 1.0]), [0, 1, 2]);
    }

    #[test]
    fn axial_straddle_narrows_the_box() {
        // A slanted root plane, then x = 0 behind it. A box straddling both reaches the
        // back of x = 0 only through its narrowed part.
        let tilted = PlaneEquation { normal: [0.0, 1.0, 0.0], offset: 0.0, r#type: 3 };
        let p = [tilted, axial(0, 0.0)];
        // nodes: [slanted, back@3, leaf 1, x=0, back@3, leaf 2, leaf 3]
        let n = [4u16, 3, 1, 5, 3, 2, 3];
        let planes = VisibilityCellPlanes { planes: &p, nodes: &n, leaf_cells: 3 };
        assert_eq!(covered_cells(&planes, [-1.0, -1.0, 0.0], [1.0, 1.0, 1.0]), [0, 1, 2]);
        assert_eq!(covered_cells(&planes, [1.0, -1.0, 0.0], [2.0, 1.0, 1.0]), [0, 1]);
    }

    #[test]
    fn rows_and_banks() {
        let (p, n) = two_planes();
        let planes = VisibilityCellPlanes { planes: &p, nodes: &n, leaf_cells: 3 };
        let mut set_bits = alloc::vec![0u32; entity_cell_bitset_length(3)];
        let b = Bounds::from_corner_extremes([-5.0, 0.0, 0.0], [5.0, 1.0, 1.0]);
        sort_scene_entity_into_cells(&planes, b, &mut set_bits, 37);
        sort_brush_model_into_cells(&planes, b, &mut set_bits, 40);
        assert!(top_bit_get(entity_cell_bitset_row(&set_bits, 0), 37));
        assert!(top_bit_get(entity_cell_bitset_row(&set_bits, 1), 37));
        assert!(!top_bit_get(entity_cell_bitset_row(&set_bits, 2), 37));
        assert!(top_bit_get(entity_cell_row_later_pass(&set_bits, 3, 1), 40));
        assert!(!top_bit_get(entity_cell_bitset_row(&set_bits, 1), 40));
        assert_eq!(set_bits[ENTITY_CELL_BITSET_WORDS + 1], 0x8000_0000 >> 5);
        assert!(actor_box_touches_cell(&planes, b, 1));
        assert!(!actor_box_touches_cell(&planes, b, 2));

        sort_scene_entity_into_cells(&planes, Bounds::from_corner_extremes([-20.0; 3], [-15.0; 3]), &mut set_bits, 37);
        assert!(!top_bit_get(entity_cell_bitset_row(&set_bits, 0), 37));
        assert!(top_bit_get(entity_cell_bitset_row(&set_bits, 2), 37));
        // Re-filtering the DObj clears its bit in bank 1 too, as R_UnfilterEntFromCells does.
        assert!(top_bit_get(entity_cell_row_later_pass(&set_bits, 3, 1), 40));
        clear_actor_cells_main_view(&mut set_bits, 3, 40);
        assert!(set_bits.iter().all(|&w| w & (0x8000_0000 >> 8) == 0 || w == 0x8000_0000 >> 5));
    }

    #[test]
    fn empty_tree_reaches_nothing() {
        let planes = VisibilityCellPlanes { planes: &[], nodes: &[], leaf_cells: 0 };
        assert!(covered_cells(&planes, [0.0; 3], [1.0; 3]).is_empty());
        assert_eq!(actor_cell_mask(2048), 2048);
    }
}