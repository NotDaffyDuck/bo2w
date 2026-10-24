//! BO2's dyn ents in cells and primary lights: dynEntCellBits[type] (one row of
//! dynEntClientWordCount[type] words per cell, MSB first), the per-view vis bytes a cell's
//! dyn brushes are written into (R_CullDynBrushInCell), and the primaryLightDynEntShadowVis
//! bits (R_GetPrimaryLightDynEntShadowBit, LSB first).

use crate::aabb::{Bounds, bounds_culled};
use crate::cell::DpvsPlanes;
use crate::scene_ent::filter_cells;
use crate::vis::msb_get;

/// R_CullDynBrushInCell for one brush of the cell: a vis byte already set stays; otherwise
/// a box that survives every plane gets 1. BO2's vis byte is 0 or 1 only.
pub fn write_dyn_brush_vis(vis: &mut u8, bounds: Bounds, planes: &[[f32; 4]]) {
    if *vis == 0 && !bounds_culled(bounds, planes) {
        *vis = 1;
    }
}

/// Whether a dyn brush joins sceneDynBrush: vis & 1 and at least one surface.
#[must_use]
pub fn dyn_brush_scene_list_admits(vis: u8, surface_count: u16) -> bool {
    vis & 1 != 0 && surface_count != 0
}

/// dynEntClientWordCount: words per cell row for this many dyn ents.
#[must_use]
pub fn dyn_ent_client_word_count(client_count: u32) -> usize {
    (client_count as usize).div_ceil(32)
}

#[must_use]
pub fn dyn_ent_cell_bits_len(word_count: usize, cell_count: usize) -> usize {
    word_count.saturating_mul(cell_count)
}

/// Clears the dyn ent's bit in every cell row.
pub fn unfilter_dyn_ent_from_cells(
    bits: &mut [u32],
    word_count: usize,
    cell_count: usize,
    dyn_ent_id: u32,
) {
    if word_count == 0 {
        return;
    }
    let word = (dyn_ent_id >> 5) as usize;
    let mask = !(0x8000_0000u32 >> (dyn_ent_id & 31));
    for cell in 0..cell_count {
        if let Some(slot) = bits.get_mut(cell * word_count + word) {
            *slot &= mask;
        }
    }
}

pub fn add_dyn_ent_to_cell(bits: &mut [u32], word_count: usize, cell: usize, dyn_ent_id: u32) {
    if word_count == 0 {
        return;
    }
    let index = cell * word_count + (dyn_ent_id >> 5) as usize;
    if let Some(slot) = bits.get_mut(index) {
        *slot |= 0x8000_0000u32 >> (dyn_ent_id & 31);
    }
}

#[must_use]
pub fn cell_dyn_ent_words(bits: &[u32], word_count: usize, cell: usize) -> &[u32] {
    if word_count == 0 {
        return &[];
    }
    let start = cell * word_count;
    bits.get(start..start + word_count).unwrap_or(&[])
}

#[must_use]
pub fn dyn_ent_in_cell(bits: &[u32], word_count: usize, cell: usize, dyn_ent_id: u32) -> bool {
    msb_get(cell_dyn_ent_words(bits, word_count, cell), dyn_ent_id as usize)
}

/// The dyn ent leaves every row, then joins the row of each cell its box reaches. BO2
/// unknown: how BO2 walks the nodes for a dyn ent was not read, so this keeps MW2's walk
/// (both children on a straddle, the box not narrowed), TODO(dynent-filter-t6).
pub fn filter_dyn_ent_into_cells(
    planes: &DpvsPlanes<'_>,
    bounds: Bounds,
    bits: &mut [u32],
    word_count: usize,
    dyn_ent_id: u32,
) {
    unfilter_dyn_ent_from_cells(bits, word_count, planes.cell_count as usize, dyn_ent_id);
    if word_count == 0 {
        return;
    }
    filter_cells(planes, bounds, false, &mut |cell| {
        add_dyn_ent_to_cell(bits, word_count, cell as usize, dyn_ent_id);
    });
}

/// R_GetPrimaryLightDynEntShadowBit: (primaryLightCount - (sun + 1)) * id - (sun + 1) + light,
/// for lights after the sun's.
#[must_use]
pub fn unlink_dyn_ent_primary_light_bit_index(
    dyn_ent_id: u32,
    light_index: u32,
    sun_primary: u32,
    primary_count: u32,
) -> Option<u32> {
    let sun1 = sun_primary.wrapping_add(1);
    if light_index < sun1 || light_index >= primary_count {
        return None;
    }
    let span = primary_count.wrapping_sub(sun1) as i32;
    let i = span
        .wrapping_mul(dyn_ent_id as i32)
        .wrapping_sub(sun1 as i32)
        .wrapping_add(light_index as i32);
    u32::try_from(i).ok()
}

/// Words of primaryLightDynEntShadowVis for this many dyn ents.
#[must_use]
pub fn dyn_ent_primary_light_vis_word_count(
    client_count: u32,
    sun_primary: u32,
    primary_count: u32,
) -> usize {
    if client_count == 0 {
        return 0;
    }
    unlink_dyn_ent_primary_light_bit_index(
        client_count - 1,
        primary_count.saturating_sub(1),
        sun_primary,
        primary_count,
    )
    .map_or(0, |bit| (bit >> 5) as usize + 1)
}

/// Sets (a hit) or clears (a miss) the dyn ent's bit for one light.
pub fn link_dyn_ent_primary_light_bit(
    words: &mut [u32],
    dyn_ent_id: u32,
    light_index: u32,
    sun_primary: u32,
    primary_count: u32,
    set: bool,
) {
    let Some(bit) =
        unlink_dyn_ent_primary_light_bit_index(dyn_ent_id, light_index, sun_primary, primary_count)
    else {
        return;
    };
    let mask = 1u32 << (bit & 31);
    if let Some(word) = words.get_mut((bit >> 5) as usize) {
        if set {
            *word |= mask;
        } else {
            *word &= !mask;
        }
    }
}

/// R_UnlinkDynEntFromPrimaryLights: clears all the dyn ent's light bits.
pub fn unlink_dyn_ent_from_primary_lights(
    words: &mut [u32],
    dyn_ent_id: u32,
    sun_primary: u32,
    primary_count: u32,
) {
    for light in sun_primary.saturating_add(1)..primary_count {
        link_dyn_ent_primary_light_bit(words, dyn_ent_id, light, sun_primary, primary_count, false);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cell::CPlane;

    #[test]
    fn vis_byte_is_zero_or_one() {
        // x >= 0 kept.
        let planes = [[1.0, 0.0, 0.0, 0.0]];
        let mut vis = 0u8;
        write_dyn_brush_vis(&mut vis, Bounds::from_mins_maxs([-2.0; 3], [-1.0; 3]), &planes);
        assert_eq!(vis, 0);
        write_dyn_brush_vis(&mut vis, Bounds::from_mins_maxs([1.0; 3], [2.0; 3]), &planes);
        assert_eq!(vis, 1);
        write_dyn_brush_vis(&mut vis, Bounds::from_mins_maxs([-2.0; 3], [-1.0; 3]), &planes);
        assert_eq!(vis, 1);
        assert!(dyn_brush_scene_list_admits(vis, 1));
        assert!(!dyn_brush_scene_list_admits(vis, 0));
    }

    #[test]
    fn cells_rows_and_refilter() {
        let p = [CPlane { normal: [1.0, 0.0, 0.0], dist: 0.0, r#type: 0 }];
        let n = [3u16, 3, 1, 2];
        let planes = DpvsPlanes { planes: &p, nodes: &n, cell_count: 2 };
        let wc = dyn_ent_client_word_count(40);
        assert_eq!(wc, 2);
        let mut bits = alloc::vec![0u32; dyn_ent_cell_bits_len(wc, 2)];
        filter_dyn_ent_into_cells(&planes, Bounds::from_mins_maxs([-1.0; 3], [1.0; 3]), &mut bits, wc, 33);
        assert!(dyn_ent_in_cell(&bits, wc, 0, 33) && dyn_ent_in_cell(&bits, wc, 1, 33));
        assert_eq!(bits[1], 0x8000_0000 >> 1);
        filter_dyn_ent_into_cells(&planes, Bounds::from_mins_maxs([1.0; 3], [2.0; 3]), &mut bits, wc, 33);
        assert!(dyn_ent_in_cell(&bits, wc, 0, 33) && !dyn_ent_in_cell(&bits, wc, 1, 33));
    }

    #[test]
    fn primary_light_bits() {
        // Sun is light 0; lights 1..4 follow, 3 bits per dyn ent.
        assert_eq!(unlink_dyn_ent_primary_light_bit_index(0, 0, 0, 4), None);
        assert_eq!(unlink_dyn_ent_primary_light_bit_index(2, 1, 0, 4), Some(6));
        assert_eq!(dyn_ent_primary_light_vis_word_count(11, 0, 4), 2);
        let mut words = [0u32; 2];
        link_dyn_ent_primary_light_bit(&mut words, 2, 3, 0, 4, true);
        assert_eq!(words[0], 1 << 8);
        unlink_dyn_ent_from_primary_lights(&mut words, 2, 0, 4);
        assert_eq!(words, [0, 0]);
    }
}
