//! The set of product lists the renderer reads, kept as three snapshots in a
//! ring so the frontend can fill one while the renderer still holds another.
//!
//! One snapshot is *live* (the one readers see). Each frame the frontend
//! picks a different one to *fill*, hands the finished products into it and
//! makes it live. A snapshot some reader still holds is never written: it is
//! skipped, or replaced by a fresh empty one if both candidates are held.

use std::sync::Arc;

use bevy::prelude::*;
use render_frame::{
    FrameOutputItem, FrameOutputKind, FrameOutputsSnapshot, AbsentOutputReason, FocusedFrameSnapshot,
};

const RING: usize = 3;

pub(super) const COLOUR_MISSING: AbsentOutputReason = AbsentOutputReason::DrawlistWriterAbsent;
pub(super) const SUN_MISSING: AbsentOutputReason = AbsentOutputReason::SunShadowSourceAbsent;
pub(super) const SPOT_MISSING: AbsentOutputReason = AbsentOutputReason::ConeShadowSourceAbsent;

/// An empty product of the given kind, as a fresh snapshot carries it.
pub(super) fn blank(kind: FrameOutputKind) -> FrameOutputItem {
    FrameOutputsSnapshot::empty()
        .products
        .into_iter()
        .find(|prod| prod.kind == kind)
        .expect("every declared product kind is in an empty snapshot")
}

#[derive(Resource, Debug)]
pub struct FrameRenderOutputs {
    /// The frame the products being assembled belong to.
    pub frame_seq_no: u64,
    banks: [Arc<FrameOutputsSnapshot>; RING],
    live: usize,
    filling: usize,
    /// Counts changes of the world-run list, so a reader can tell it moved
    /// without comparing the list.
    level_run_edition: u64,
}

impl Default for FrameRenderOutputs {
    fn default() -> Self {
        Self {
            frame_seq_no: 0,
            banks: std::array::from_fn(|_| Arc::new(FrameOutputsSnapshot::empty())),
            live: 0,
            filling: 0,
            level_run_edition: 0,
        }
    }
}

impl FrameRenderOutputs {
    /// A product of the live snapshot.
    pub fn product(&self, kind: FrameOutputKind) -> &FrameOutputItem {
        self.banks[self.live].product(kind)
    }

    /// The live snapshot, for a reader that keeps it past this frame.
    pub fn published(&self) -> Arc<FrameOutputsSnapshot> {
        Arc::clone(&self.banks[self.live])
    }

    /// Chooses the snapshot to fill this frame and wipes its frame header.
    pub(super) fn start_fill(&mut self) {
        let next = (self.live + 1) % RING;
        let after = (self.live + 2) % RING;
        self.filling = if Arc::strong_count(&self.banks[next]) == 1 {
            next
        } else {
            after
        };
        if Arc::strong_count(&self.banks[self.filling]) > 1 {
            self.banks[self.filling] = Arc::new(FrameOutputsSnapshot::empty());
        }
        let snap = self.filling_snapshot();
        snap.frame_seq_no = 0;
        snap.focus.take();
    }

    fn filling_snapshot(&mut self) -> &mut FrameOutputsSnapshot {
        Arc::get_mut(&mut self.banks[self.filling]).expect("the snapshot being filled is unique")
    }

    /// Swaps `product` into its slot of the snapshot being filled; the old
    /// slot contents come back in `product` for next frame's reuse.
    pub(super) fn adopt(&mut self, product: &mut FrameOutputItem) {
        let sort = product.kind;
        let seat = self
            .filling_snapshot()
            .products
            .iter_mut()
            .find(|slot| slot.kind == sort)
            .expect("every published product kind has a slot");
        std::mem::swap(seat, product);
    }

    /// Records which world surfaces the static draw lane ran this frame.
    pub(super) fn set_world_run_surfs(&mut self, surf_list: &[u16]) {
        if self.banks[self.live].level_run_facets.as_slice() != surf_list {
            self.level_run_edition = self.level_run_edition.wrapping_add(1);
        }
        let revision = self.level_run_edition;
        let snap = self.filling_snapshot();
        if snap.level_run_facets.as_slice() != surf_list {
            snap.level_run_facets.clear();
            snap.level_run_facets.extend_from_slice(surf_list);
        }
        snap.level_run_edition = revision;
    }

    pub(super) fn set_focus(&mut self, focus: Option<FocusedFrameSnapshot>) {
        self.filling_snapshot().focus = focus;
    }

    /// Stamps the filled snapshot with the frame and makes it the live one.
    pub(super) fn complete_fill(&mut self) {
        let frame_seq_no = self.frame_seq_no;
        self.filling_snapshot().frame_seq_no = frame_seq_no;
        self.live = self.filling;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fill_one_frame(products: &mut FrameRenderOutputs, frame: u64) {
        products.frame_seq_no = frame;
        products.start_fill();
        products.complete_fill();
    }

    #[test]
    fn a_new_ring_has_every_product_missing() {
        let products = FrameRenderOutputs::default();
        for kind in FrameOutputKind::ALL_KINDS {
            assert_eq!(products.product(kind).kind, kind);
        }
        assert_eq!(products.published().frame_seq_no, 0);
    }

    #[test]
    fn finishing_a_fill_makes_it_live() {
        let mut products = FrameRenderOutputs::default();
        fill_one_frame(&mut products, 7);
        assert_eq!(products.published().frame_seq_no, 7);
        fill_one_frame(&mut products, 8);
        assert_eq!(products.published().frame_seq_no, 8);
    }

    #[test]
    fn the_snapshot_a_reader_holds_is_never_written() {
        let mut products = FrameRenderOutputs::default();
        fill_one_frame(&mut products, 1);
        let held = products.published();
        for frame in 2..10 {
            fill_one_frame(&mut products, frame);
            assert_eq!(held.frame_seq_no, 1, "frame {frame} wrote the held snapshot");
        }
    }

    #[test]
    fn two_held_snapshots_still_leave_a_bank_to_fill() {
        let mut products = FrameRenderOutputs::default();
        fill_one_frame(&mut products, 1);
        let first = products.published();
        fill_one_frame(&mut products, 2);
        let second = products.published();
        for frame in 3..12 {
            fill_one_frame(&mut products, frame);
            assert_eq!((first.frame_seq_no, second.frame_seq_no), (1, 2));
            assert_eq!(products.published().frame_seq_no, frame);
        }
    }

    #[test]
    fn an_adopted_product_lands_in_its_slot_and_the_old_one_comes_back() {
        let mut products = FrameRenderOutputs::default();
        products.start_fill();
        let mut colour = blank(FrameOutputKind::Colour);
        colour.list_fingerprint = 99;
        products.adopt(&mut colour);
        assert_eq!(colour.list_fingerprint, 0, "the slot's old contents come back");
        products.complete_fill();
        assert_eq!(products.product(FrameOutputKind::Colour).list_fingerprint, 99);
    }

    #[test]
    fn the_world_run_revision_moves_only_when_the_list_does() {
        let mut products = FrameRenderOutputs::default();
        let revision = |products: &FrameRenderOutputs| products.published().level_run_edition;

        products.start_fill();
        products.set_world_run_surfs(&[1, 2, 3]);
        products.complete_fill();
        let first = revision(&products);
        assert_ne!(first, 0);

        products.start_fill();
        products.set_world_run_surfs(&[1, 2, 3]);
        products.complete_fill();
        assert_eq!(revision(&products), first);

        products.start_fill();
        products.set_world_run_surfs(&[1, 2]);
        products.complete_fill();
        assert_ne!(revision(&products), first);
        assert_eq!(products.published().level_run_facets, vec![1, 2]);
    }

    #[test]
    fn the_focus_is_cleared_when_a_fill_begins() {
        let mut products = FrameRenderOutputs::default();
        products.start_fill();
        products.set_focus(Some(FocusedFrameSnapshot::never_found(5, "test")));
        products.complete_fill();
        assert!(products.published().focus().is_some());
        products.start_fill();
        products.complete_fill();
        assert!(products.published().focus().is_none());
    }
}
