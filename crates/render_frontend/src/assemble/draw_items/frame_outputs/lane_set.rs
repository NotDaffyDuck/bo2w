//! Reading the three draw lanes (static world, models, effects) as one list.
//!
//! Every lane is already sorted by draw key. The camera lists are the lanes
//! merged: smallest `(key, tie)` first, and when two lanes tie the lower lane
//! comes first, so the merge is stable across the lane order.

use crate::assemble::draw_items::kept_lanes::{HeldDrawItem, draw_order_tie};

/// Lowest lane index first on equal sort keys.
pub(super) const LANE_COUNT: usize = 3;


/// The merged rows of the lanes, in draw order, borrowed from the lanes.
pub(super) struct LaneMerge<'a> {
    lanes: [&'a [HeldDrawItem]; LANE_COUNT],
    next: [usize; LANE_COUNT],
}

impl<'a> LaneMerge<'a> {
    pub(super) fn new(lanes: [&'a [HeldDrawItem]; LANE_COUNT]) -> Self {
        Self {
            lanes,
            next: [0; LANE_COUNT],
        }
    }

    /// Rows the merge will produce in all.
    pub(super) fn total(&self) -> usize {
        self.lanes.iter().map(|lane| lane.len()).sum()
    }
}

fn order(item: &HeldDrawItem) -> (u64, u32) {
    (item.key, draw_order_tie(&item.kind))
}

impl<'a> Iterator for LaneMerge<'a> {
    type Item = &'a HeldDrawItem;

    fn next(&mut self) -> Option<&'a HeldDrawItem> {
        let mut best = None::<(usize, (u64, u32))>;
        for (lane, rows) in self.lanes.iter().enumerate() {
            let Some(row) = rows.get(self.next[lane]) else {
                continue;
            };
            let rank = order(row);
            // Strictly less: on a tie the lane found first keeps the turn.
            if best.is_none_or(|(_, held)| rank < held) {
                best = Some((lane, rank));
            }
        }
        let (lane, _) = best?;
        let row = &self.lanes[lane][self.next[lane]];
        self.next[lane] += 1;
        Some(row)
    }
}

/// Replaces `out` with the merged lanes.
pub(super) fn merge_into(out: &mut Vec<HeldDrawItem>, lanes: [&[HeldDrawItem]; LANE_COUNT]) {
    let merge = LaneMerge::new(lanes);
    out.clear();
    out.reserve(merge.total());
    out.extend(merge.copied());
}

/// Writes `rows` over `write_target` one after another. True only when they fill it
/// exactly; a row too many stops the write.
pub(super) fn fill_exact<'a>(
    write_target: &mut [HeldDrawItem],
    row_list: impl Iterator<Item = &'a HeldDrawItem>,
) -> bool {
    let mut wrote = 0;
    for row in row_list {
        let Some(seat) = write_target.get_mut(wrote) else {
            return false;
        };
        *seat = *row;
        wrote += 1;
    }
    wrote == write_target.len()
}

/// Refreshes a de-duplicated list from the rows it was built from.
///
/// `slot_of_row` says, for each incoming row in order, which slot of `write_target`
/// it stands for. A row that opens a new slot (the next one not yet written)
/// is copied into it; a row that repeats an earlier slot is skipped. True
/// only when the stream and the slot map end together and every slot of
/// `write_target` was written.
pub(super) fn fill_by_slots<'a>(
    write_target: &mut [HeldDrawItem],
    slot_of_row: &[u32],
    row_list: impl Iterator<Item = &'a HeldDrawItem>,
) -> bool {
    let mut slots = slot_of_row.iter();
    let mut opened = 0;
    for row in row_list {
        let Some(&slot) = slots.next() else {
            return false;
        };
        if slot as usize != opened {
            continue;
        }
        let Some(target) = write_target.get_mut(opened) else {
            return false;
        };
        *target = *row;
        opened += 1;
    }
    slots.next().is_none() && opened == write_target.len()
}

#[cfg(test)]
pub(super) mod tests {
    use super::*;
    use crate::assemble::draw_items::kept_lanes::HeldDrawKind;

    /// A world row with the given sort key and surface.
    pub(in super::super) fn row(key: u64, surf: u16) -> HeldDrawItem {
        let material_id = None;
        HeldDrawItem {
            material_key: material_id,
            key,
            material_order_no: 0,
            kind: HeldDrawKind::world(surf),
            facet_samplers: Default::default(),
            camera_zone: None,
        }
    }

    fn surfaces(row_list: &[HeldDrawItem]) -> Vec<u16> {
        row_list.iter()
            .map(|r| match r.kind {
                HeldDrawKind::World { surf, .. } => surf,
                _ => unreachable!(),
            })
            .collect()
    }

    /// Small deterministic generator for the property-style cases.
    pub(in super::super) struct Lcg(pub u64);

    impl Lcg {
        pub(in super::super) fn next(&mut self, bound: u64) -> u64 {
            self.0 = self
                .0
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            (self.0 >> 33) % bound
        }
    }

    fn sorted_lane(rng: &mut Lcg, len: usize, tag: u16) -> Vec<HeldDrawItem> {
        let mut lane: Vec<_> = (0..len)
            .map(|i| row(rng.next(12), tag * 1000 + i as u16))
            .collect();
        lane.sort_by_key(|r| (r.key, draw_order_tie(&r.kind)));
        lane
    }

    #[test]
    fn merging_empty_lanes_gives_nothing() {
        let mut out = vec![row(1, 1)];
        merge_into(&mut out, [&[], &[], &[]]);
        assert!(out.is_empty());
    }

    #[test]
    fn a_single_lane_comes_through_unchanged() {
        let lane = [row(1, 10), row(2, 11), row(2, 12)];
        let mut out = Vec::new();
        merge_into(&mut out, [&[], &lane, &[]]);
        assert_eq!(surfaces(&out), [10, 11, 12]);
    }

    #[test]
    fn lanes_interleave_by_key() {
        let a = [row(1, 1), row(5, 2)];
        let b = [row(3, 3)];
        let c = [row(0, 4), row(9, 5)];
        let mut out = Vec::new();
        merge_into(&mut out, [&a, &b, &c]);
        assert_eq!(surfaces(&out), [4, 1, 3, 2, 5]);
    }

    #[test]
    fn equal_rows_keep_the_lane_order() {
        // Same key and same tie (surface number): the lower lane goes first.
        let a = [row(7, 5)];
        let b = [row(7, 5)];
        let c = [row(7, 5)];
        let merged: Vec<_> = LaneMerge::new([&a, &b, &c]).collect();
        assert!(std::ptr::eq(merged[0], &a[0]));
        assert!(std::ptr::eq(merged[1], &b[0]));
        assert!(std::ptr::eq(merged[2], &c[0]));
    }

    #[test]
    fn the_merge_equals_a_stable_sort_of_the_lanes_joined() {
        let mut rng = Lcg(0x5eed);
        for round in 0..40 {
            let lens = [rng.next(9) as usize, rng.next(9) as usize, rng.next(9) as usize];
            let lanes: Vec<_> = lens
                .iter()
                .enumerate()
                .map(|(i, &n)| sorted_lane(&mut rng, n, i as u16))
                .collect();
            let mut expected: Vec<_> = lanes.iter().flatten().copied().collect();
            expected.sort_by_key(|r| (r.key, draw_order_tie(&r.kind)));

            let mut out = Vec::new();
            merge_into(&mut out, [&lanes[0], &lanes[1], &lanes[2]]);
            assert_eq!(surfaces(&out), surfaces(&expected), "round {round}");
        }
    }

    #[test]
    fn fill_exact_wants_the_same_count() {
        let rows = [row(1, 1), row(2, 2), row(3, 3)];
        let mut write_target = vec![row(0, 0); 3];
        assert!(fill_exact(&mut write_target, rows.iter()));
        assert_eq!(surfaces(&write_target), [1, 2, 3]);

        let mut short = vec![row(0, 0); 2];
        assert!(!fill_exact(&mut short, rows.iter()));
        let mut long = vec![row(0, 0); 4];
        assert!(!fill_exact(&mut long, rows.iter()));
    }

    #[test]
    fn fill_by_slots_copies_the_first_row_of_each_slot() {
        // Rows 0,1 are slot 0 and 1; row 2 repeats slot 0; row 3 opens slot 2.
        let rows = [row(1, 10), row(2, 11), row(3, 12), row(4, 13)];
        let mut write_target = vec![row(0, 0); 3];
        assert!(fill_by_slots(&mut write_target, &[0, 1, 0, 2], rows.iter()));
        assert_eq!(surfaces(&write_target), [10, 11, 13]);
    }

    #[test]
    fn fill_by_slots_refuses_a_stream_of_the_wrong_length() {
        let rows = [row(1, 10), row(2, 11)];
        let mut write_target = vec![row(0, 0); 2];
        // Map shorter than the stream.
        assert!(!fill_by_slots(&mut write_target, &[0], rows.iter()));
        // Map longer than the stream.
        assert!(!fill_by_slots(&mut write_target, &[0, 1, 2], rows.iter()));
        // A slot left unwritten.
        let mut wide = vec![row(0, 0); 3];
        assert!(!fill_by_slots(&mut wide, &[0, 1], rows.iter()));
    }
}
