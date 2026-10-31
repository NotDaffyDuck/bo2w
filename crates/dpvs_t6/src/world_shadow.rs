//! The visible BSP surfaces of one sun-shadow range, as a flat list in index order.
//!
//! BO2's own run rule for the sun range and its sm_fastSunShadow reuse are not read yet
//! (facts dpvs-drawsurf-t6.md lines 201-203), so this keeps the admit rule the port had:
//! visible, a sun-shadow caster, and a non-zero key (a guard on host data).

use alloc::vec::Vec;

use crate::SortedDraw;

#[inline]
fn shadow_caster_flag(w_bits: &[u32], ix: usize) -> bool {
    w_bits
        .get(ix / 32)
        .is_some_and(|w| (*w & (1u32 << (ix % 32))) != 0)
}

/// Clears `queue` and fills it with the admitted surfaces of `begin..end`.
pub fn add_bsp_draw_surfs_range_sun_shadow(
    begin: u32,
    end: u32,
    facet_vis: &[u8],
    caster_bitwords: &[u32],
    facet_draws: &[SortedDraw],
    queue: &mut Vec<u16>,
) {
    queue.clear();
    for face in begin as usize..end as usize {
        if facet_vis.get(face).copied().unwrap_or(0) == 0 || !shadow_caster_flag(caster_bitwords, face) {
            continue;
        }
        if facet_draws.get(face).is_none_or(|key| key.packed == 0) {
            continue;
        }
        let Ok(id) = u16::try_from(face) else {
            continue;
        };
        queue.push(id);
    }
}

#[cfg(test)]
mod tests {
    use alloc::vec;

    use super::*;

    #[test]
    fn admits_visible_casters_with_a_key() {
        let vis = [1, 1, 0, 1, 1];
        let casters = [0b11011u32];
        let keys = [
            SortedDraw::unpack_from_bits(1),
            SortedDraw::unpack_from_bits(1),
            SortedDraw::unpack_from_bits(1),
            SortedDraw::unpack_from_bits(0),
            SortedDraw::unpack_from_bits(2),
        ];
        let mut out = vec![7];
        add_bsp_draw_surfs_range_sun_shadow(0, 6, &vis, &casters, &keys, &mut out);
        assert_eq!(out, vec![0, 1, 4]);
    }
}
