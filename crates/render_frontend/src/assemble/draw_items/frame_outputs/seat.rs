//! A product together with the memo of how its list was last collapsed.
//!
//! Every list the frontend builds (colour, emissive, light, sun, spot) goes
//! through the same few moves, so they share this slot:
//!
//! 1. *carry over*: take last frame's published rows as they are;
//! 2. *overlay*: take last frame's rows as the shape and write this frame's
//!    payload into them straight from the lanes;
//! 3. *collapse*: merge the lanes and collapse repeats, either by replaying
//!    the memo (same shape) or by folding from scratch.

use render_frame::{FrameOutputItem, FrameOutputKind, FrameOutputState, AbsentOutputReason, OutputTarget};
use render_material::{StagedMaterialTable, LiveMaterialLedger};

use super::super::material_live::MaterialEpochId;
use super::super::kept_lanes::HeldDrawItem;
use super::super::material_setup::MethodType;
use super::bank::blank;
use super::lane_set::{LANE_COUNT, LaneMerge, fill_by_slots};
use super::memo::ListMemo;
use super::technique_ix::TechRule;

/// What every list written this frame is stamped with.
#[derive(Clone, Copy)]
pub(super) struct Build<'a> {
    pub(super) revision_stamp: MaterialEpochId,
    pub(super) digest: u64,
    pub(super) catalog: &'a LiveMaterialLedger,
    pub(super) prepared: &'a StagedMaterialTable,
}

#[derive(Debug)]
pub(super) struct ListSlot {
    pub(super) product: FrameOutputItem,
    pub(super) memo: ListMemo,
}

impl ListSlot {
    pub(super) fn new(kind: FrameOutputKind) -> Self {
        Self {
            product: blank(kind),
            memo: ListMemo::default(),
        }
    }

    /// Wipes the product for a new fill, marking it missing for `cause`.
    pub(super) fn begin(&mut self, cause: AbsentOutputReason) {
        self.product.start_fill(cause);
    }

    /// Marks the list complete and stamps it.
    pub(super) fn ready(&mut self, build: &Build<'_>, tech: MethodType, target: OutputTarget) {
        self.product.revision_stamp = build.revision_stamp;
        self.product.list_fingerprint = build.digest;
        self.product.state = FrameOutputState::Ready {
            shading_technique_kind: tech,
            target,
        };
    }

    /// Takes `previous` (last frame's published product of this kind) as is.
    pub(super) fn carry_over(&mut self, previous: &FrameOutputItem) -> bool {
        self.memo.restore_from(&mut self.product, previous)
    }

    /// Takes `previous` as the shape and rewrites each row from the lanes,
    /// skipping rows `keep` rejects. False when the lanes no longer fit the
    /// shape (the caller then collapses from scratch).
    pub(super) fn overlay(
        &mut self,
        previous: &FrameOutputItem,
        lanes: [&[HeldDrawItem]; LANE_COUNT],
        keep: impl Fn(&HeldDrawItem) -> bool,
    ) -> bool {
        self.carry_over(previous)
            && fill_by_slots(
                &mut self.product.draw_sequence,
                self.memo.slot_of_input(),
                LaneMerge::new(lanes).filter(|row| keep(row)),
            )
    }

    /// Collapses the rows now in the product. With `same_shape` the previous
    /// collapse is replayed if it still fits; otherwise the list is folded
    /// again.
    pub(super) fn collapse(
        &mut self,
        build: &Build<'_>,
        rule: &TechRule<'_>,
        status_tech: MethodType,
        target: OutputTarget,
        same_shape: bool,
    ) {
        if !(same_shape && self.memo.replay(&mut self.product)) {
            self.memo
                .compact(&mut self.product, rule, build.catalog, build.prepared);
        }
        self.ready(build, status_tech, target);
    }
}
