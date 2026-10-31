//! Effect and elem draw order.

use crate::bolt::FX_BOLT_SORT_DEFAULT;

/// Two effects whose squared camera distances are within this ratio tie.
pub const FX_SORT_TIE_LOW: f32 = 0.9999;
pub const FX_SORT_TIE_HIGH: f32 = 1.0001;

#[inline]
pub fn dist_to_cam_sq(camera: [f32; 3], pos: [f32; 3]) -> f32 {
    let d = [camera[0] - pos[0], camera[1] - pos[1], camera[2] - pos[2]];
    d[0] * d[0] + d[1] * d[1] + d[2] * d[2]
}

/// Effects are drawn far to near; within the tie band, effects with the same
/// owner break the tie on their sort-order byte.
// BO2 unknown: which way the sort-order tie breaks (see facts/QUESTIONS.md),
// so this only says whether two distances tie.
#[inline]
pub fn effect_dists_tie(a_dist_sq: f32, b_dist_sq: f32) -> bool {
    let ratio = a_dist_sq / b_dist_sq;
    (FX_SORT_TIE_LOW..=FX_SORT_TIE_HIGH).contains(&ratio)
}

/// A runner parent (sort byte 0xff) takes the average sort order of its
/// non-runner elems, clamped to 1..=0xfe, or 0 when it has none.
// BO2 unknown: the average is read here as a truncating integer division
// (see facts/QUESTIONS.md).
pub fn runner_parent_sort_order(non_runner_sort_orders: impl IntoIterator<Item = u8>) -> u8 {
    let (sum, count) = non_runner_sort_orders
        .into_iter()
        .fold((0u32, 0u32), |(s, c), o| (s + u32::from(o), c + 1));
    if count == 0 {
        return 0;
    }
    (sum / count).clamp(1, u32::from(FX_BOLT_SORT_DEFAULT) - 1) as u8
}

/// Sprite elems sort by their def's sort order, then farther first. True
/// when the existing elem stays ahead of the new one.
#[inline]
pub fn existing_elem_sorts_before_new(
    resident_rank: u8,
    existing_dist_sq: f32,
    incoming_rank: u8,
    new_dist_sq: f32,
) -> bool {
    if resident_rank != incoming_rank {
        return resident_rank < incoming_rank;
    }
    existing_dist_sq > new_dist_sq
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ties() {
        assert!(effect_dists_tie(10000.0, 10000.5));
        assert!(!effect_dists_tie(10000.0, 10002.0));
        assert_eq!(dist_to_cam_sq([0.0; 3], [3.0, 4.0, 0.0]), 25.0);
    }

    #[test]
    fn runner_parents() {
        assert_eq!(runner_parent_sort_order([]), 0);
        assert_eq!(runner_parent_sort_order([0, 0]), 1);
        assert_eq!(runner_parent_sort_order([0xff, 0xff]), 0xfe);
        assert_eq!(runner_parent_sort_order([4, 7]), 5);
    }

    #[test]
    fn elem_order() {
        assert!(existing_elem_sorts_before_new(1, 0.0, 2, 100.0));
        assert!(!existing_elem_sorts_before_new(3, 100.0, 2, 0.0));
        assert!(existing_elem_sorts_before_new(2, 100.0, 2, 50.0));
        assert!(!existing_elem_sorts_before_new(2, 50.0, 2, 100.0));
    }
}
