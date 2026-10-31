//! BO2's dyn ents in cells and primary lights: dynEntCellBits[type] (one row of
//! dynEntClientWordCount[type] words per cell, MSB first), the per-view vis bytes a cell's dyn
//! brushes are written into (R_CullDynBrushInCell), whether a seen one joins the scene list, and the primaryLightDynEntShadowVis bits (R_GetPrimaryLightDynEntShadowBit,
//! LSB first).

use crate::box_tree::{Bounds, box_outside_planes};
use crate::cell::VisibilityCellPlanes;
use crate::stage_actor::filter_cells;
use crate::cell_bits::top_bit_get;

/// R_CullDynBrushInCell for one brush of the cell: a brush already seen stays seen; an
/// unseen one is seen (1) when no plane culls its box. BO2's vis byte is 0 or 1 only.
pub fn write_debris_solid_vis(vis: &mut u8, cell_box: Bounds, cull_planes: &[[f32; 4]]) {
    let newly_seen = *vis == 0 && !box_outside_planes(cell_box, cull_planes);
    *vis |= u8::from(newly_seen);
}

/// Whether a dyn brush joins sceneDynBrush: vis & 1 and at least one surface.
#[must_use]
pub fn debris_brush_listed(seen_flag: u8, facet_tally: u16) -> bool {
    facet_tally > 0 && seen_flag % 2 == 1
}

/// dynEntClientWordCount: words per cell row for this many dyn ents.
#[must_use]
pub fn debris_viewer_words(view_n: u32) -> usize {
    let ents = view_n as usize;
    ents / 32 + usize::from(ents % 32 != 0)
}

#[must_use]
pub fn debris_cell_mask_len(dw_n: usize, cells_total: usize) -> usize {
    cells_total.saturating_mul(dw_n)
}

/// Where a dyn ent sits inside a cell row: the word, and its bit (MSB first).
fn row_place(moving_entity_number: u32) -> (usize, u32) {
    ((moving_entity_number / 32) as usize, (1u32 << 31) >> (moving_entity_number % 32))
}

/// The word of `cell`'s row that holds the dyn ent, if the rows reach that far.
fn row_word(bitset: &mut [u32], dwords_n: usize, cell: usize, ent_no: u32) -> Option<(&mut u32, u32)> {
    if dwords_n == 0 {
        return None;
    }
    let (word, bit) = row_place(ent_no);
    bitset.get_mut(cell * dwords_n + word).map(|slot| (slot, bit))
}

/// Clears the dyn ent's bit in every cell row.
pub fn unfilter_dynamic_ent_from_cells(
    bit_row: &mut [u32],
    dword_tally: usize,
    cell_count: usize,
    moving_entity_number: u32,
) {
    for cell in 0..cell_count {
        if let Some((slot, bit)) = row_word(bit_row, dword_tally, cell, moving_entity_number) {
            *slot &= !bit;
        }
    }
}

pub fn mark_debris_in_cell(vis_words: &mut [u32], dword_tally: usize, cell: usize, moving_entity_number: u32) {
    if let Some((slot, bit)) = row_word(vis_words, dword_tally, cell, moving_entity_number) {
        *slot |= bit;
    }
}

/// One cell's row, or nothing when the rows are empty or do not reach the cell.
#[must_use]
pub fn debris_words_of_cell(vis_words: &[u32], dword_tally: usize, cell: usize) -> &[u32] {
    let mut rows = vis_words.chunks_exact(dword_tally.max(1));
    match dword_tally {
        0 => &[],
        _ => rows.nth(cell).unwrap_or(&[]),
    }
}

#[must_use]
pub fn debris_marked_in_cell(vis_words: &[u32], dword_tally: usize, cell: usize, moving_entity_number: u32) -> bool {
    let line_ix = debris_words_of_cell(vis_words, dword_tally, cell);
    top_bit_get(line_ix, moving_entity_number as usize)
}

/// The dyn ent leaves every row, then joins the row of each cell its box reaches. BO2
/// unknown: how BO2 walks the nodes for a dyn ent was not read, so this keeps MW2's walk
/// (both children on a straddle, the box not narrowed), TODO(debris_bit-filter-t6).
pub fn filter_debris_into_rooms(
    planes: &VisibilityCellPlanes<'_>,
    cell_box: Bounds,
    flag_words: &mut [u32],
    dword_tally: usize,
    moving_entity_number: u32,
) {
    let cells = planes.leaf_cells as usize;
    unfilter_dynamic_ent_from_cells(flag_words, dword_tally, cells, moving_entity_number);
    if dword_tally != 0 {
        let mut join = |cell: u32| mark_debris_in_cell(flag_words, dword_tally, cell as usize, moving_entity_number);
        filter_cells(planes, cell_box, false, &mut join);
    }
}

/// R_GetPrimaryLightDynEntShadowBit: (primaryLightCount - (sun + 1)) * id - (sun + 1) + light,
/// for lights after the sun's. The sum runs in 32-bit wrapping arithmetic; a result that
/// is negative as a signed number has no bit.
#[must_use]
pub fn detach_dynamic_prop_key_light_slot(
    moving_entity_number: u32,
    shadow_light_slot: u32,
    sunlight_primary_flag_index: u32,
    main_light_total: u32,
) -> Option<u32> {
    let first_light = sunlight_primary_flag_index.wrapping_add(1);
    if !(first_light..main_light_total).contains(&shadow_light_slot) {
        return None;
    }
    let per_ent = main_light_total.wrapping_sub(first_light);
    let bit = per_ent.wrapping_mul(moving_entity_number).wrapping_sub(first_light).wrapping_add(shadow_light_slot);
    (bit <= i32::MAX as u32).then_some(bit)
}

/// Words of primaryLightDynEntShadowVis for this many dyn ents: enough to hold the last
/// dyn ent's bit for the last light.
#[must_use]
pub fn moving_entity_main_light_visibility_words(
    viewer_n: u32,
    sunlight_primary_flag_index: u32,
    main_light_total: u32,
) -> usize {
    let Some(last_ent) = viewer_n.checked_sub(1) else {
        return 0;
    };
    let last_light = main_light_total.saturating_sub(1);
    detach_dynamic_prop_key_light_slot(last_ent, last_light, sunlight_primary_flag_index, main_light_total)
        .map_or(0, |bit| bit as usize / 32 + 1)
}

/// Sets (a hit) or clears (a miss) the dyn ent's bit for one light.
pub fn attach_dynamic_prop_key_light_bit(
    mask_words: &mut [u32],
    moving_entity_number: u32,
    shadow_light_slot: u32,
    sunlight_primary_flag_index: u32,
    main_light_total: u32,
    set: bool,
) {
    let bit = detach_dynamic_prop_key_light_slot(moving_entity_number, shadow_light_slot, sunlight_primary_flag_index, main_light_total);
    let Some((word, mask)) = bit.and_then(|b| Some((mask_words.get_mut((b / 32) as usize)?, 1u32 << (b % 32)))) else {
        return;
    };
    *word = if set { *word | mask } else { *word & !mask };
}

/// R_UnlinkDynEntFromPrimaryLights: clears all the dyn ent's light bits.
pub fn detach_moving_entity_from_main_lights(
    slot_words: &mut [u32],
    moving_entity_number: u32,
    sunlight_primary_flag_index: u32,
    main_light_total: u32,
) {
    let lights = sunlight_primary_flag_index.saturating_add(1)..main_light_total;
    lights.for_each(|light| {
        attach_dynamic_prop_key_light_bit(slot_words, moving_entity_number, light, sunlight_primary_flag_index, main_light_total, false);
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cell::PlaneEquation;

    #[test]
    fn a_seen_brush_with_surfaces_joins_the_scene_list() {
        assert!(debris_brush_listed(1, 1));
        assert!(debris_brush_listed(3, 1));
        assert!(!debris_brush_listed(1, 0));
        assert!(!debris_brush_listed(2, 5));
    }

    #[test]
    fn a_vis_byte_is_set_once_by_a_box_inside_the_planes() {
        let keep_x_positive = [[1.0, 0.0, 0.0, 0.0]];
        let behind = Bounds::from_corner_extremes([-2.0; 3], [-1.0; 3]);
        let ahead = Bounds::from_corner_extremes([1.0; 3], [2.0; 3]);
        let mut vis = 0u8;
        let seen: [u8; 3] = [behind, ahead, behind].map(|b| {
            write_debris_solid_vis(&mut vis, b, &keep_x_positive);
            vis
        });
        assert_eq!(seen, [0, 1, 1]);
    }

    #[test]
    fn cells_rows_and_refilter() {
        let p = [PlaneEquation { normal: [1.0, 0.0, 0.0], offset: 0.0, r#type: 0 }];
        let n = [3u16, 3, 1, 2];
        let planes = VisibilityCellPlanes { planes: &p, nodes: &n, leaf_cells: 2 };
        let words_n = debris_viewer_words(40);
        assert_eq!(words_n, 2);
        let mut cell_mask = alloc::vec![0u32; debris_cell_mask_len(words_n, 2)];
        let straddle = Bounds::from_corner_extremes([-1.0; 3], [1.0; 3]);
        filter_debris_into_rooms(&planes, straddle, &mut cell_mask, words_n, 33);
        assert!(debris_marked_in_cell(&cell_mask, words_n, 0, 33) && debris_marked_in_cell(&cell_mask, words_n, 1, 33));
        assert_eq!(cell_mask[1], 0x8000_0000 >> 1);
        let front = Bounds::from_corner_extremes([1.0; 3], [2.0; 3]);
        filter_debris_into_rooms(&planes, front, &mut cell_mask, words_n, 33);
        assert!(debris_marked_in_cell(&cell_mask, words_n, 0, 33) && !debris_marked_in_cell(&cell_mask, words_n, 1, 33));
        assert!(debris_words_of_cell(&cell_mask, 0, 0).is_empty() && debris_words_of_cell(&cell_mask, words_n, 2).is_empty());
    }

    #[test]
    fn primary_light_bits() {
        // Sun is light 0; lights 1..4 follow, 3 bits per dyn ent.
        assert_eq!(detach_dynamic_prop_key_light_slot(0, 0, 0, 4), None);
        assert_eq!(detach_dynamic_prop_key_light_slot(2, 1, 0, 4), Some(6));
        assert_eq!(moving_entity_main_light_visibility_words(11, 0, 4), 2);
        assert_eq!(moving_entity_main_light_visibility_words(0, 0, 4), 0);
        let mut row_words = [0u32; 2];
        attach_dynamic_prop_key_light_bit(&mut row_words, 2, 3, 0, 4, true);
        assert_eq!(row_words[0], 1 << 8);
        detach_moving_entity_from_main_lights(&mut row_words, 2, 0, 4);
        assert_eq!(row_words, [0, 0]);
    }
}
