use std::ops::Range;
use std::sync::Arc;

use bevy::prelude::*;
use lighting_t6::{SpotShadowEmittedSlot, SpotShadowHistory};
use render_frame::{PackedFrontendLists, SpotShadowFrameSlot, SpotShadowReceiver};

use super::super::list;
use super::super::retained::RetainedDrawItem;

/// The per-frame shadow caster state: the light selection carried between
/// frames, and for every light that got a shadow slot this frame its packed
/// lists, the span of `items` they were cut from, and the slot itself.
#[derive(Resource, Clone, Debug, Default)]
pub struct SpotShadowCasterPlan {
    pub history: SpotShadowHistory,
    pub packed: Vec<PackedFrontendLists>,
    pub items: Vec<RetainedDrawItem>,
    emitted: Vec<SpotShadowEmittedSlot>,
    spans: Vec<Range<usize>>,
}

/// The six index lists in a packed set that point back into the draw items.
fn index_lists(packed: &mut PackedFrontendLists) -> [&mut Vec<u32>; 6] {
    [
        &mut packed.world_draw_indices,
        &mut packed.xmodel_draw_indices,
        &mut packed.smodel_draw_indices,
        &mut packed.smodel_cached_draw_indices,
        &mut packed.smodel_pretess_draw_indices,
        &mut packed.smodel_skinned_draw_indices,
    ]
}

fn index_lists_ref(packed: &PackedFrontendLists) -> [&Vec<u32>; 6] {
    [
        &packed.world_draw_indices,
        &packed.xmodel_draw_indices,
        &packed.smodel_draw_indices,
        &packed.smodel_cached_draw_indices,
        &packed.smodel_pretess_draw_indices,
        &packed.smodel_skinned_draw_indices,
    ]
}

/// Rewrites indices that count from the start of a slot's span into positions
/// in the compacted draw list.
fn translate_into_compact(indices: &mut [u32], span: &Range<usize>, compact: &[u32]) {
    for entry in indices.iter_mut() {
        let local = usize::try_from(*entry).expect("slot index fits usize");
        let source = span
            .start
            .checked_add(local)
            .filter(|source| *source < span.end)
            .expect("slot index stays inside its span");
        *entry = *compact
            .get(source)
            .expect("compaction covers every input of the span");
    }
}

fn mix_list(id: &mut u64, list_values: &[u32]) {
    list::mix_content_id(id, list_values.len() as u64);
    list_values
        .iter()
        .for_each(|&value| list::mix_content_id(id, u64::from(value)));
}

impl SpotShadowCasterPlan {
    pub(crate) fn clear_frame(&mut self) {
        self.items.clear();
        self.packed.clear();
        self.emitted.clear();
        self.spans.clear();
    }

    /// Adds one slot: its draws are appended to `items` and remembered as a span.
    pub(super) fn record_slot(
        &mut self,
        draws: Vec<RetainedDrawItem>,
        slot: SpotShadowEmittedSlot,
        packed: PackedFrontendLists,
    ) {
        let first = self.items.len();
        self.items.extend(draws);
        self.spans.push(first..self.items.len());
        self.emitted.push(slot);
        self.packed.push(packed);
    }

    pub(crate) fn shadowed_light_indices(&self) -> Vec<u8> {
        self.emitted.iter().map(|slot| slot.light_index).collect()
    }

    pub(crate) fn receivers_by_light_index(&self) -> Vec<Option<SpotShadowReceiver>> {
        let mut table: Vec<Option<SpotShadowReceiver>> = Vec::new();
        for slot in &self.emitted {
            let Some(lookup) = slot.lookup else { continue };
            let at = usize::from(slot.light_index);
            if table.len() <= at {
                table.resize(at + 1, None);
            }
            table[at] = Some(SpotShadowReceiver {
                lookup,
                pixel_adjust: slot.plan.pixel_adjust(),
            });
        }
        table
    }

    /// Moves the slots out with their index lists translated through `compact`.
    fn drain_slots(&mut self, compact: &[u32]) -> Vec<SpotShadowFrameSlot> {
        assert_eq!(self.packed.len(), self.emitted.len());
        assert_eq!(self.packed.len(), self.spans.len());
        let packed = std::mem::take(&mut self.packed);
        let emitted = std::mem::take(&mut self.emitted);
        let spans = std::mem::take(&mut self.spans);
        let mut out = Vec::with_capacity(packed.len());
        for ((mut lists, slot), span) in packed.into_iter().zip(emitted).zip(spans) {
            for indices in index_lists(&mut lists) {
                translate_into_compact(indices, &span, compact);
            }
            out.push(SpotShadowFrameSlot {
                emitted: slot,
                packed: Arc::new(lists),
            });
        }
        out
    }

    /// A number that changes when the compaction or any slot's lists change.
    fn content_id(&self, compact: &[u32]) -> u64 {
        let mut id = list::CONTENT_ID_SEED;
        mix_list(&mut id, compact);
        list::mix_content_id(&mut id, self.packed.len() as u64);
        for ((lists, slot), span) in self.packed.iter().zip(&self.emitted).zip(&self.spans) {
            list::mix_content_id(&mut id, u64::from(slot.slot_index));
            list::mix_content_id(&mut id, u64::from(slot.light_index));
            list::mix_content_id(&mut id, span.start as u64);
            list::mix_content_id(&mut id, span.end as u64);
            for indices in index_lists_ref(lists) {
                mix_list(&mut id, indices);
            }
        }
        id
    }

    /// The frame's slots, reusing last frame's packed sets when nothing they
    /// were built from changed.
    pub(crate) fn adopt_frame_slots(
        &mut self,
        compact: &[u32],
        last_id: &mut Option<u64>,
        last_packed: &mut Vec<Arc<PackedFrontendLists>>,
    ) -> Vec<SpotShadowFrameSlot> {
        let id = self.content_id(compact);
        if *last_id == Some(id) && last_packed.len() == self.emitted.len() {
            self.packed.clear();
            self.spans.clear();
            return std::mem::take(&mut self.emitted)
                .into_iter()
                .zip(last_packed.iter())
                .map(|(slot, kept)| SpotShadowFrameSlot {
                    emitted: slot,
                    packed: Arc::clone(kept),
                })
                .collect();
        }
        let slots = self.drain_slots(compact);
        *last_id = Some(id);
        last_packed.clear();
        last_packed.extend(slots.iter().map(|slot| Arc::clone(&slot.packed)));
        slots
    }
}
