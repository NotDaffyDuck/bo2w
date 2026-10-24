//! The visible BSP surfaces of one sun-shadow range, as a flat list in index order.
//!
//! BO2's own run rule for the sun range and its sm_fastSunShadow reuse are not read yet
//! (facts dpvs-drawsurf-t6.md lines 201-203), so this keeps the admit rule the port had:
//! visible, a sun-shadow caster, and a non-zero key (a guard on host data).

use alloc::vec::Vec;

use crate::GfxDrawSurf;

#[inline]
fn caster_bit(words: &[u32], index: usize) -> bool {
    words
        .get(index / 32)
        .is_some_and(|w| (*w & (1u32 << (index % 32))) != 0)
}

/// Clears `out` and fills it with the admitted surfaces of `begin..end`.
pub fn add_bsp_draw_surfs_range_sun_shadow(
    begin: u32,
    end: u32,
    surface_vis: &[u8],
    caster_words: &[u32],
    draw_surfs: &[GfxDrawSurf],
    out: &mut Vec<u16>,
) {
    out.clear();
    for surf in begin as usize..end as usize {
        if surface_vis.get(surf).copied().unwrap_or(0) == 0 || !caster_bit(caster_words, surf) {
            continue;
        }
        if draw_surfs.get(surf).is_none_or(|key| key.packed == 0) {
            continue;
        }
        let Ok(id) = u16::try_from(surf) else {
            continue;
        };
        out.push(id);
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
            GfxDrawSurf::from_packed(1),
            GfxDrawSurf::from_packed(1),
            GfxDrawSurf::from_packed(1),
            GfxDrawSurf::from_packed(0),
            GfxDrawSurf::from_packed(2),
        ];
        let mut out = vec![7];
        add_bsp_draw_surfs_range_sun_shadow(0, 6, &vis, &casters, &keys, &mut out);
        assert_eq!(out, vec![0, 1, 4]);
    }
}
