use std::ops::Range;
use std::sync::Arc;

use bevy::prelude::*;
use lighting_t6::{SpotShadowSubmittedSlot, ConePickMemory};
use render_frame::{CompactFrontendLists, ConeShadowFrameSlot, ConeShadowSink};

use super::super::list;
use super::super::kept_lanes::HeldDrawItem;

/// The per-frame shadow caster state: the light selection carried between
/// frames, and for every light that got a shadow slot this frame its packed
/// lists, the span of `items` they were cut from, and the slot itself.
#[derive(Resource, Clone, Debug, Default)]
pub struct ConeShadowCasterPlan {
    pub history: ConePickMemory,
    pub packed: Vec<CompactFrontendLists>,
    pub items: Vec<HeldDrawItem>,
    granted: Vec<SpotShadowSubmittedSlot>,
    spans: Vec<Range<usize>>,
}

/// The six index lists in a packed set that point back into the draw items.
fn index_lists(packed: &mut CompactFrontendLists) -> [&mut Vec<u32>; 6] {
    [
        &mut packed.level_draw_indices,
        &mut packed.model_draw_ix,
        &mut packed.prop_draw_indices,
        &mut packed.prop_cached_draw_indices,
        &mut packed.prop_prebuilt_draw_indices,
        &mut packed.prop_skinned_draw_indices,
    ]
}

fn index_lists_ref(packed: &CompactFrontendLists) -> [&Vec<u32>; 6] {
    [
        &packed.level_draw_indices,
        &packed.model_draw_ix,
        &packed.prop_draw_indices,
        &packed.prop_cached_draw_indices,
        &packed.prop_prebuilt_draw_indices,
        &packed.prop_skinned_draw_indices,
    ]
}

/// Rewrites indices that count from the start of a slot's span into positions
/// in the compacted draw list.
fn translate_into_compact(indices: &mut [u32], span: &Range<usize>, squeeze: &[u32]) {
    for entry in indices.iter_mut() {
        let offset = usize::try_from(*entry).expect("slot index fits usize");
        let pos = span
            .start
            .checked_add(offset)
            .filter(|pos| *pos < span.end)
            .expect("slot index stays inside its span");
        *entry = *squeeze.get(pos).expect("compaction covers every input of the span");
    }
}

fn mix_list(id: &mut u64, list_values: &[u32]) {
    list::blend_content_id(id, list_values.len() as u64);
    list_values
        .iter()
        .for_each(|&value| list::blend_content_id(id, u64::from(value)));
}

impl ConeShadowCasterPlan {
    pub(crate) fn wipe_frame(&mut self) {
        self.items.truncate(0);
        self.packed.truncate(0);
        self.granted.truncate(0);
        self.spans.truncate(0);
    }

    /// Adds one slot: its draws are appended to `items` and remembered as a span.
    pub(super) fn record_slot(
        &mut self,
        draws: Vec<HeldDrawItem>,
        slot: SpotShadowSubmittedSlot,
        packed: CompactFrontendLists,
    ) {
        let first = self.items.len();
        self.items.extend(draws);
        self.spans.push(first..self.items.len());
        self.granted.push(slot);
        self.packed.push(packed);
    }

    pub(crate) fn shadow_light_slots(&self) -> Vec<u8> {
        self.granted.iter().map(|slot| slot.shadow_light_slot).collect()
    }

    pub(crate) fn receivers_by_light_index(&self) -> Vec<Option<ConeShadowSink>> {
        let mut table: Vec<Option<ConeShadowSink>> = Vec::new();
        for slot in &self.granted {
            let Some(lookup) = slot.lookup else { continue };
            let at = usize::from(slot.shadow_light_slot);
            if table.len() <= at {
                table.resize(at + 1, None);
            }
            table[at] = Some(ConeShadowSink {
                lookup,
                texel_offset_correction: slot.plan.texel_offset_correction(),
            });
        }
        table
    }

    /// Moves the slots out with their index lists translated through `compact`.
    fn drain_slots(&mut self, compact: &[u32]) -> Vec<ConeShadowFrameSlot> {
        let slots = self.packed.len();
        assert_eq!(slots, self.granted.len());
        assert_eq!(slots, self.spans.len());
        let sets = std::mem::take(&mut self.packed);
        let granted = std::mem::take(&mut self.granted);
        let ranges = std::mem::take(&mut self.spans);
        let mut out = Vec::with_capacity(sets.len());
        for ((mut lists, slot), span) in sets.into_iter().zip(granted).zip(ranges) {
            for indices in index_lists(&mut lists) {
                translate_into_compact(indices, &span, compact);
            }
            out.push(ConeShadowFrameSlot {
                emitted: slot,
                packed: Arc::new(lists),
            });
        }
        out
    }

    /// A number that changes when the compaction or any slot's lists change.
    fn content_id(&self, squeeze: &[u32]) -> u64 {
        let mut id = list::CONTENT_KEY_SEED;
        mix_list(&mut id, squeeze);
        let slots = self.packed.len() as u64;
        list::blend_content_id(&mut id, slots);
        for ((lists, slot), span) in self.packed.iter().zip(&self.granted).zip(&self.spans) {
            list::blend_content_id(&mut id, u64::from(slot.slot_no));
            list::blend_content_id(&mut id, u64::from(slot.shadow_light_slot));
            list::blend_content_id(&mut id, span.start as u64);
            list::blend_content_id(&mut id, span.end as u64);
            for indices in index_lists_ref(lists) {
                mix_list(&mut id, indices);
            }
        }
        id
    }

    /// The frame's slots, reusing last frame's packed sets when nothing they
    /// were built from changed.
    pub(crate) fn take_frame_slots(
        &mut self,
        order: &[u32],
        prev_id: &mut Option<u64>,
        last_compact: &mut Vec<Arc<CompactFrontendLists>>,
    ) -> Vec<ConeShadowFrameSlot> {
        let id = self.content_id(order);
        if *prev_id == Some(id) && last_compact.len() == self.granted.len() {
            self.packed.truncate(0);
            self.spans.truncate(0);
            return std::mem::take(&mut self.granted)
                .into_iter()
                .zip(last_compact.iter())
                .map(|(seat, kept)| ConeShadowFrameSlot {
                    emitted: seat,
                    packed: Arc::clone(kept),
                })
                .collect();
        }
        let slots = self.drain_slots(order);
        prev_id.replace(id);
        last_compact.clear();
        last_compact.extend(slots.iter().map(|slot| Arc::clone(&slot.packed)));
        slots
    }
}
