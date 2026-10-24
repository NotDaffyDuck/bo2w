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
    FrameProduct, FrameProductKind, FrameProductsSnapshot, MissingProductCause, RenderFocusFrame,
};

const RING: usize = 3;

pub(super) const COLOUR_MISSING: MissingProductCause = MissingProductCause::MissingFrontendEmitter;
pub(super) const SUN_MISSING: MissingProductCause = MissingProductCause::MissingSunShadowProducer;
pub(super) const SPOT_MISSING: MissingProductCause = MissingProductCause::MissingSpotShadowProducer;

/// An empty product of the given kind, as a fresh snapshot carries it.
pub(super) fn blank(kind: FrameProductKind) -> FrameProduct {
    FrameProductsSnapshot::empty()
        .products
        .into_iter()
        .find(|product| product.kind == kind)
        .expect("every declared product kind is in an empty snapshot")
}

#[derive(Resource, Debug)]
pub struct RenderFrameProducts {
    /// The frame the products being assembled belong to.
    pub frame_id: u64,
    banks: [Arc<FrameProductsSnapshot>; RING],
    live: usize,
    filling: usize,
    /// Counts changes of the world-run list, so a reader can tell it moved
    /// without comparing the list.
    world_run_revision: u64,
}

impl Default for RenderFrameProducts {
    fn default() -> Self {
        Self {
            frame_id: 0,
            banks: std::array::from_fn(|_| Arc::new(FrameProductsSnapshot::empty())),
            live: 0,
            filling: 0,
            world_run_revision: 0,
        }
    }
}

impl RenderFrameProducts {
    /// A product of the live snapshot.
    pub fn product(&self, kind: FrameProductKind) -> &FrameProduct {
        self.banks[self.live].product(kind)
    }

    /// The live snapshot, for a reader that keeps it past this frame.
    pub fn published(&self) -> Arc<FrameProductsSnapshot> {
        Arc::clone(&self.banks[self.live])
    }

    /// Chooses the snapshot to fill this frame and wipes its frame header.
    pub(super) fn begin_fill(&mut self) {
        let next = (self.live + 1) % RING;
        let after = (self.live + 2) % RING;
        self.filling = if Arc::strong_count(&self.banks[next]) == 1 {
            next
        } else {
            after
        };
        if Arc::strong_count(&self.banks[self.filling]) > 1 {
            self.banks[self.filling] = Arc::new(FrameProductsSnapshot::empty());
        }
        let snap = self.filling_snapshot();
        snap.frame_id = 0;
        snap.focus = None;
    }

    fn filling_snapshot(&mut self) -> &mut FrameProductsSnapshot {
        Arc::get_mut(&mut self.banks[self.filling]).expect("the snapshot being filled is unique")
    }

    /// Swaps `product` into its slot of the snapshot being filled; the old
    /// slot contents come back in `product` for next frame's reuse.
    pub(super) fn adopt(&mut self, product: &mut FrameProduct) {
        let kind = product.kind;
        let slot = self
            .filling_snapshot()
            .products
            .iter_mut()
            .find(|slot| slot.kind == kind)
            .expect("every published product kind has a slot");
        std::mem::swap(slot, product);
    }

    /// Records which world surfaces the static draw lane ran this frame.
    pub(super) fn set_world_run_surfs(&mut self, surfs: &[u16]) {
        if self.banks[self.live].world_run_surfs.as_slice() != surfs {
            self.world_run_revision = self.world_run_revision.wrapping_add(1);
        }
        let revision = self.world_run_revision;
        let snap = self.filling_snapshot();
        if snap.world_run_surfs.as_slice() != surfs {
            snap.world_run_surfs.clear();
            snap.world_run_surfs.extend_from_slice(surfs);
        }
        snap.world_run_revision = revision;
    }

    pub(super) fn set_focus(&mut self, focus: Option<RenderFocusFrame>) {
        self.filling_snapshot().focus = focus;
    }

    /// Stamps the filled snapshot with the frame and makes it the live one.
    pub(super) fn finish_fill(&mut self) {
        let frame_id = self.frame_id;
        self.filling_snapshot().frame_id = frame_id;
        self.live = self.filling;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fill_one_frame(products: &mut RenderFrameProducts, frame: u64) {
        products.frame_id = frame;
        products.begin_fill();
        products.finish_fill();
    }

    #[test]
    fn a_new_ring_has_every_product_missing() {
        let products = RenderFrameProducts::default();
        for kind in FrameProductKind::ALL {
            assert_eq!(products.product(kind).kind, kind);
        }
        assert_eq!(products.published().frame_id, 0);
    }

    #[test]
    fn finishing_a_fill_makes_it_live() {
        let mut products = RenderFrameProducts::default();
        fill_one_frame(&mut products, 7);
        assert_eq!(products.published().frame_id, 7);
        fill_one_frame(&mut products, 8);
        assert_eq!(products.published().frame_id, 8);
    }

    #[test]
    fn the_snapshot_a_reader_holds_is_never_written() {
        let mut products = RenderFrameProducts::default();
        fill_one_frame(&mut products, 1);
        let held = products.published();
        for frame in 2..10 {
            fill_one_frame(&mut products, frame);
            assert_eq!(held.frame_id, 1, "frame {frame} wrote the held snapshot");
        }
    }

    #[test]
    fn two_held_snapshots_still_leave_a_bank_to_fill() {
        let mut products = RenderFrameProducts::default();
        fill_one_frame(&mut products, 1);
        let first = products.published();
        fill_one_frame(&mut products, 2);
        let second = products.published();
        for frame in 3..12 {
            fill_one_frame(&mut products, frame);
            assert_eq!((first.frame_id, second.frame_id), (1, 2));
            assert_eq!(products.published().frame_id, frame);
        }
    }

    #[test]
    fn an_adopted_product_lands_in_its_slot_and_the_old_one_comes_back() {
        let mut products = RenderFrameProducts::default();
        products.begin_fill();
        let mut colour = blank(FrameProductKind::Colour);
        colour.list_digest = 99;
        products.adopt(&mut colour);
        assert_eq!(colour.list_digest, 0, "the slot's old contents come back");
        products.finish_fill();
        assert_eq!(products.product(FrameProductKind::Colour).list_digest, 99);
    }

    #[test]
    fn the_world_run_revision_moves_only_when_the_list_does() {
        let mut products = RenderFrameProducts::default();
        let revision = |products: &RenderFrameProducts| products.published().world_run_revision;

        products.begin_fill();
        products.set_world_run_surfs(&[1, 2, 3]);
        products.finish_fill();
        let first = revision(&products);
        assert_ne!(first, 0);

        products.begin_fill();
        products.set_world_run_surfs(&[1, 2, 3]);
        products.finish_fill();
        assert_eq!(revision(&products), first);

        products.begin_fill();
        products.set_world_run_surfs(&[1, 2]);
        products.finish_fill();
        assert_ne!(revision(&products), first);
        assert_eq!(products.published().world_run_surfs, vec![1, 2]);
    }

    #[test]
    fn the_focus_is_cleared_when_a_fill_begins() {
        let mut products = RenderFrameProducts::default();
        products.begin_fill();
        products.set_focus(Some(RenderFocusFrame::unresolved(5, "test")));
        products.finish_fill();
        assert!(products.published().focus().is_some());
        products.begin_fill();
        products.finish_fill();
        assert!(products.published().focus().is_none());
    }
}
