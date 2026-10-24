//! Collapsing a merged draw list, and remembering how, so the next frame can
//! redo it cheaply.
//!
//! The lanes hand over every draw the frame wants. Many of them are the same
//! draw twice (a surface listed by both the static lane and a re-lit lane, a
//! model repeated for several scene lights...). A product list keeps the first
//! of each identical draw. [`ListMemo`] does that collapse in place and keeps
//! the mapping *input row -> slot*. When the next frame's rows are the same
//! rows in the same order (only their payload may have changed), the mapping
//! is replayed instead of hashing every row again.
//!
//! In the sun product the list is two lists back to back (near and far
//! cascade); a draw may appear once in each, so the dedupe starts afresh at
//! the cascade boundary.

use bevy::platform::collections::HashMap;
use render_material::{PreparedMaterialTable, RuntimeMaterialCatalog};

use super::super::retained::{RetainedDrawItem, RetainedDrawKind};
use super::super::setup::TechType;
use super::tech::{TechRule, material_key};
use render_frame::FrameProduct;

/// What makes two draws "the same draw" for the dedupe: the sort key, the
/// technique, and the fields that pick the geometry for the draw's kind.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct RowId {
    key: u64,
    tech: u8,
    geometry: Geometry,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Geometry {
    World { surf: u16, run: u16, run_off: u32 },
    Smodel { placement: u32, surface: u32, lighting: u32 },
    XModel { surface: u32, object_id: u16 },
    CodeMesh { draw: u32, args: u8, viewmodel: bool },
    Cloud { draw: u32 },
    Mark { draw: u32 },
    Glass { draw: u32, lighting: u32 },
}

fn row_id(row: &RetainedDrawItem, tech: TechType) -> RowId {
    let geometry = match row.kind {
        RetainedDrawKind::World {
            surf, run, run_off, ..
        } => Geometry::World { surf, run, run_off },
        RetainedDrawKind::Smodel {
            placement,
            surface,
            lighting_handle,
            ..
        } => Geometry::Smodel {
            placement,
            surface,
            lighting: lighting_handle,
        },
        RetainedDrawKind::XModel {
            surface, object_id, ..
        } => Geometry::XModel { surface, object_id },
        RetainedDrawKind::CodeMesh {
            draw,
            arg_count,
            viewmodel,
            ..
        } => Geometry::CodeMesh {
            draw,
            args: arg_count,
            viewmodel,
        },
        RetainedDrawKind::ParticleCloud { draw, .. } => Geometry::Cloud { draw },
        RetainedDrawKind::MarkMesh { draw, .. } => Geometry::Mark { draw },
        RetainedDrawKind::Glass {
            draw,
            lighting_handle,
            ..
        } => Geometry::Glass {
            draw,
            lighting: lighting_handle,
        },
    };
    RowId {
        key: row.key,
        tech: tech.0,
        geometry,
    }
}

/// What the last collapse of one list produced.
#[derive(Debug, Default)]
pub(super) struct ListMemo {
    /// Scratch for the collapse; first slot of every draw seen so far.
    seen: HashMap<RowId, u32>,
    /// For each input row, the slot of the collapsed list it ended in.
    slot_of_input: Vec<u32>,
    /// Technique of each slot.
    tech_of_slot: Vec<TechType>,
    sampler_mask: u64,
    has_codemesh: bool,
    sun_near_n: usize,
    /// False until a collapse has run.
    filled: bool,
}

impl ListMemo {
    /// Collapses `rows` in place and records how. `near_n` is the length of
    /// the near cascade for a split list (0 for a plain one). `on_kept` sees
    /// each row that survives with its technique. Returns the near count of
    /// the collapsed list when the list had a near part.
    pub(super) fn fold(
        &mut self,
        rows: &mut Vec<RetainedDrawItem>,
        near_n: usize,
        tech_of: impl Fn(&RetainedDrawItem) -> TechType,
        mut on_kept: impl FnMut(&RetainedDrawItem, TechType),
    ) -> Option<usize> {
        let total = rows.len();
        let near_in = near_n.min(total);
        let boundary = (near_in > 0 && near_in < total).then_some(near_in);

        self.seen.clear();
        self.seen.reserve(total);
        self.slot_of_input.clear();
        self.slot_of_input.reserve(total);
        self.tech_of_slot.clear();
        self.tech_of_slot.reserve(total);

        let mut near_out = near_in;
        let mut kept = 0usize;
        for at in 0..total {
            if Some(at) == boundary {
                near_out = kept;
                self.seen.clear();
            }
            let row = rows[at];
            let tech = tech_of(&row);
            let id = row_id(&row, tech);
            if let Some(&slot) = self.seen.get(&id) {
                self.slot_of_input.push(slot);
                continue;
            }
            let slot = u32::try_from(kept).unwrap_or(u32::MAX);
            self.seen.insert(id, slot);
            self.slot_of_input.push(slot);
            self.tech_of_slot.push(tech);
            rows[kept] = row;
            on_kept(&row, tech);
            kept += 1;
        }
        rows.truncate(kept);
        (near_in > 0).then_some(if boundary.is_some() { near_out } else { kept })
    }

    /// Collapses the list of `product` under `rule` and fills in the product's
    /// per-slot data (techniques, sampler mask, code-mesh flag, near count).
    pub(super) fn compact(
        &mut self,
        product: &mut FrameProduct,
        rule: &TechRule<'_>,
        catalog: &RuntimeMaterialCatalog,
        prepared: &PreparedMaterialTable,
    ) {
        let mut sampler_mask = 0u64;
        let mut has_codemesh = false;
        let near = self.fold(
            &mut product.ordered_draws,
            product.sun_near_n,
            |row| rule.tech_for(row),
            |row, tech| {
                sampler_mask |=
                    super::super::draw_code_sampler_mask(catalog, prepared, material_key(row), tech);
                has_codemesh |= matches!(row.kind, RetainedDrawKind::CodeMesh { .. });
            },
        );
        if let Some(near) = near {
            product.sun_near_n = near;
        }
        self.sampler_mask = sampler_mask;
        self.has_codemesh = has_codemesh;
        self.sun_near_n = product.sun_near_n;
        self.filled = true;
        self.write_slot_data(product);
    }

    /// The slot map of the last collapse, one entry per input row.
    pub(super) fn slot_of_input(&self) -> &[u32] {
        &self.slot_of_input
    }

    pub(super) fn is_filled(&self) -> bool {
        self.filled
    }

    fn write_slot_data(&self, product: &mut FrameProduct) {
        product.draw_tech.clone_from(&self.tech_of_slot);
        product.code_sampler_mask = self.sampler_mask;
        product.has_codemesh = self.has_codemesh;
        product.sun_near_n = self.sun_near_n;
    }

    /// Redoes the last collapse on a list of the same length: each slot takes
    /// the first input row mapped to it, so a changed payload comes through.
    /// False (rows untouched) if the list is not the length the memo is for.
    pub(super) fn replay(&self, product: &mut FrameProduct) -> bool {
        if !self.filled || self.slot_of_input.len() != product.ordered_draws.len() {
            return false;
        }
        // A slot is opened by its first row, which is never before the slot's
        // own index, so the copy below only reads rows not yet overwritten.
        let rows = &mut product.ordered_draws;
        let mut opened = 0usize;
        for (at, &slot) in self.slot_of_input.iter().enumerate() {
            if slot as usize == opened {
                rows[opened] = rows[at];
                opened += 1;
            }
        }
        debug_assert_eq!(opened, self.tech_of_slot.len());
        rows.truncate(self.tech_of_slot.len());
        self.write_slot_data(product);
        true
    }

    /// Takes the collapsed rows of `previous` (last frame's published list)
    /// as this frame's list, along with the memo's slot data. False if the
    /// memo is empty or `previous` is not the list it was made for.
    pub(super) fn restore_from(&self, product: &mut FrameProduct, previous: &FrameProduct) -> bool {
        if !self.filled || previous.ordered_draws.len() != self.tech_of_slot.len() {
            return false;
        }
        product.ordered_draws.clone_from(&previous.ordered_draws);
        self.write_slot_data(product);
        true
    }

    /// Writes the memo's slot data onto a product whose rows were just
    /// rebuilt to the collapsed shape.
    pub(super) fn apply_to(&self, product: &mut FrameProduct) {
        product.ordered_draws.truncate(self.tech_of_slot.len());
        self.write_slot_data(product);
    }
}

#[cfg(test)]
mod tests {
    use super::super::bank::blank;
    use super::super::lanes::tests::{Lcg, row};
    use super::*;
    use render_frame::FrameProductKind;

    fn by_parity(row: &RetainedDrawItem) -> TechType {
        TechType((row.key & 1) as u8)
    }

    fn surfaces(rows: &[RetainedDrawItem]) -> Vec<u16> {
        rows.iter()
            .map(|r| match r.kind {
                RetainedDrawKind::World { surf, .. } => surf,
                _ => unreachable!(),
            })
            .collect()
    }

    fn fold_plain(memo: &mut ListMemo, rows: &mut Vec<RetainedDrawItem>, near: usize) -> Option<usize> {
        memo.fold(rows, near, by_parity, |_, _| {})
    }

    #[test]
    fn identical_draws_collapse_to_the_first() {
        let mut memo = ListMemo::default();
        let mut rows = vec![row(2, 1), row(2, 1), row(4, 2), row(2, 1), row(4, 2)];
        let near = fold_plain(&mut memo, &mut rows, 0);
        assert_eq!(near, None);
        assert_eq!(surfaces(&rows), [1, 2]);
        assert_eq!(memo.slot_of_input(), [0, 0, 1, 0, 1]);
    }

    #[test]
    fn a_different_technique_keeps_the_draw_apart() {
        // Same surface, odd and even keys: two techniques, two slots.
        let mut memo = ListMemo::default();
        let mut rows = vec![row(2, 1), row(3, 1), row(2, 1)];
        fold_plain(&mut memo, &mut rows, 0);
        assert_eq!(rows.len(), 2);
        assert_eq!(memo.tech_of_slot, [TechType(0), TechType(1)]);
    }

    #[test]
    fn the_same_key_with_another_surface_is_another_draw() {
        let mut memo = ListMemo::default();
        let mut rows = vec![row(2, 1), row(2, 2)];
        fold_plain(&mut memo, &mut rows, 0);
        assert_eq!(surfaces(&rows), [1, 2]);
    }

    #[test]
    fn the_dedupe_restarts_at_the_cascade_boundary() {
        let mut memo = ListMemo::default();
        // near = [a, a, b]; far = [a, b, b]
        let mut rows = vec![row(2, 1), row(2, 1), row(4, 2), row(2, 1), row(4, 2), row(4, 2)];
        let near = fold_plain(&mut memo, &mut rows, 3);
        assert_eq!(near, Some(2));
        assert_eq!(surfaces(&rows), [1, 2, 1, 2]);
        assert_eq!(memo.slot_of_input(), [0, 0, 1, 2, 3, 3]);
    }

    #[test]
    fn a_near_count_covering_the_whole_list_is_not_a_split() {
        let mut memo = ListMemo::default();
        let mut rows = vec![row(2, 1), row(2, 1), row(4, 2)];
        assert_eq!(fold_plain(&mut memo, &mut rows, 3), Some(2));
        let mut rows = vec![row(2, 1), row(2, 1), row(4, 2)];
        // More than the list holds counts as the whole list.
        assert_eq!(fold_plain(&mut memo, &mut rows, 99), Some(2));
    }

    #[test]
    fn no_near_part_reports_nothing() {
        let mut memo = ListMemo::default();
        let mut rows = vec![row(2, 1), row(4, 2)];
        assert_eq!(fold_plain(&mut memo, &mut rows, 0), None);
    }

    #[test]
    fn the_kept_callback_sees_each_survivor_once() {
        let mut memo = ListMemo::default();
        let mut rows = vec![row(2, 1), row(2, 1), row(3, 7), row(2, 1)];
        let mut seen = Vec::new();
        memo.fold(&mut rows, 0, by_parity, |row, tech| {
            seen.push((row.key, tech.0));
        });
        assert_eq!(seen, [(2, 0), (3, 1)]);
    }

    #[test]
    fn an_empty_list_folds_to_an_empty_list() {
        let mut memo = ListMemo::default();
        let mut rows = Vec::new();
        assert_eq!(fold_plain(&mut memo, &mut rows, 0), None);
        assert!(rows.is_empty());
        assert!(memo.slot_of_input().is_empty());
    }

    fn product_with(rows: Vec<RetainedDrawItem>, near: usize) -> FrameProduct {
        let mut product = blank(FrameProductKind::SunShadow);
        product.ordered_draws = rows;
        product.sun_near_n = near;
        product
    }

    /// Runs the fold on a product the way `compact` does, without a catalog.
    fn fold_product(memo: &mut ListMemo, product: &mut FrameProduct) {
        let near = memo.fold(&mut product.ordered_draws, product.sun_near_n, by_parity, |_, _| {});
        if let Some(near) = near {
            product.sun_near_n = near;
        }
        memo.sun_near_n = product.sun_near_n;
        memo.filled = true;
        memo.write_slot_data(product);
    }

    #[test]
    fn replay_gives_the_fold_again_with_fresh_payload() {
        let mut memo = ListMemo::default();
        let mut first = product_with(vec![row(2, 1), row(2, 1), row(4, 2), row(2, 1)], 0);
        fold_product(&mut memo, &mut first);

        // The same rows next frame, but each carries a new material rank.
        let mut next = vec![row(2, 1), row(2, 1), row(4, 2), row(2, 1)];
        for (i, r) in next.iter_mut().enumerate() {
            r.material_rank = 100 + i as u32;
        }
        let mut again = product_with(next, 0);
        assert!(memo.replay(&mut again));
        assert_eq!(surfaces(&again.ordered_draws), [1, 2]);
        assert_eq!(again.ordered_draws[0].material_rank, 100);
        assert_eq!(again.ordered_draws[1].material_rank, 102);
        assert_eq!(again.draw_tech, first.draw_tech);
    }

    #[test]
    fn replay_refuses_a_list_of_another_length() {
        let mut memo = ListMemo::default();
        let mut first = product_with(vec![row(2, 1), row(2, 1)], 0);
        fold_product(&mut memo, &mut first);

        let mut longer = product_with(vec![row(2, 1), row(2, 1), row(4, 2)], 0);
        assert!(!memo.replay(&mut longer));
        assert_eq!(longer.ordered_draws.len(), 3, "rows are left alone");
        assert!(!ListMemo::default().replay(&mut first), "an empty memo replays nothing");
    }

    #[test]
    fn replay_restores_a_split_near_count() {
        let mut memo = ListMemo::default();
        let rows = vec![row(2, 1), row(2, 1), row(2, 1), row(4, 2)];
        let mut first = product_with(rows.clone(), 2);
        fold_product(&mut memo, &mut first);
        assert_eq!(first.sun_near_n, 1);

        let mut again = product_with(rows, 2);
        assert!(memo.replay(&mut again));
        assert_eq!(again.sun_near_n, 1);
        assert_eq!(surfaces(&again.ordered_draws), [1, 1, 2]);
    }

    #[test]
    fn restore_takes_the_previous_collapsed_rows() {
        let mut memo = ListMemo::default();
        let mut first = product_with(vec![row(2, 1), row(2, 1), row(4, 2)], 0);
        fold_product(&mut memo, &mut first);

        let mut fresh = product_with(Vec::new(), 0);
        assert!(memo.restore_from(&mut fresh, &first));
        assert_eq!(surfaces(&fresh.ordered_draws), [1, 2]);
        assert_eq!(fresh.draw_tech, first.draw_tech);

        let mut other = product_with(vec![row(2, 1)], 0);
        assert!(!memo.restore_from(&mut fresh, &other), "length differs");
        other.ordered_draws.push(row(4, 2));
        assert!(memo.restore_from(&mut fresh, &other));
    }

    #[test]
    fn replay_matches_a_fresh_fold_on_random_lists() {
        let mut rng = Lcg(0xfeed);
        for round in 0..60 {
            let len = rng.next(24) as usize;
            let near = rng.next(len as u64 + 2) as usize;
            let rows: Vec<_> = (0..len)
                .map(|_| row(rng.next(4), rng.next(4) as u16))
                .collect();

            let mut memo = ListMemo::default();
            let mut folded = product_with(rows.clone(), near);
            fold_product(&mut memo, &mut folded);

            let mut replayed = product_with(rows.clone(), near);
            assert!(memo.replay(&mut replayed), "round {round}");
            assert_eq!(
                surfaces(&replayed.ordered_draws),
                surfaces(&folded.ordered_draws),
                "round {round}"
            );
            assert_eq!(replayed.sun_near_n, folded.sun_near_n, "round {round}");
            assert_eq!(replayed.draw_tech, folded.draw_tech, "round {round}");

            // No two surviving rows are the same draw inside one cascade.
            let split = folded.sun_near_n.min(folded.ordered_draws.len());
            let cascades = if near > 0 && near < len {
                vec![&folded.ordered_draws[..split], &folded.ordered_draws[split..]]
            } else {
                vec![&folded.ordered_draws[..]]
            };
            for cascade in cascades {
                let mut ids: Vec<_> = cascade
                    .iter()
                    .map(|r| (r.key, surfaces(std::slice::from_ref(r))[0]))
                    .collect();
                ids.sort_unstable();
                let before = ids.len();
                ids.dedup();
                assert_eq!(ids.len(), before, "round {round}: a repeat survived");
            }
        }
    }
}
