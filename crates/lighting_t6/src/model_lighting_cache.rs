//! The cache that gives each dynamic model (a player body, a piece of glass)
//! a slot of lit texels in the model lighting atlas.
//!
//! Slots are handed out from "free" bitmaps, one per recent frame. A slot is
//! free only when it was free in the last three frames and not yet taken in
//! this one, so a texel is never rewritten while an older frame might still
//! be drawing from it.

use crate::smodel_lighting::MODEL_LIGHTING_WARN_CACHE_ALLOC_FAILED;

/// How many frames of free bitmaps are kept (the frame counter wraps at 4).
pub const MODEL_LIGHTING_PIXEL_FREE_BITS_BUFFERS: usize = 4;

/// What asking the cache for a slot came to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ModelLightingCacheAlloc {
    /// The model kept its slot and had not moved; `info` is what was stored.
    Reused { handle: u16, info: u16 },
    /// The model has a slot whose texels must be sampled again.
    Assigned { handle: u16, slot: u32 },
    /// Nothing was free.
    Failed,
}

/// Why a [`ModelLightingCacheGlob`] could not be set up.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ModelLightingCacheGlobError {
    /// No slots, or the last slot's handle would not fit in 16 bits.
    LimitsOutOfRange {
        base_index: u32,
        xmodel_entry_limit: u32,
    },
    /// One of the free bitmaps is too short for the slot count.
    FreeBitsWordCountMismatch { needed: usize, got: usize },
    /// The origin or lighting-info array is too short for the slot count.
    EntryArrayTooShort { needed: usize, got: usize },
}

/// How many 32-bit words a free bitmap needs for `xmodel_entry_limit` slots.
pub const fn model_lighting_cache_free_bits_words(xmodel_entry_limit: u32) -> usize {
    xmodel_entry_limit.div_ceil(32) as usize
}

/// Which of the four bitmaps belongs to the frame `age` frames before
/// `mod_frame_count`.
pub const fn dyn_pixel_free_bits_index(mod_frame_count: u32, age: u32) -> usize {
    (mod_frame_count.wrapping_sub(age) & 3) as usize
}

/// Moves on to the next frame: the counter steps (wrapping at 4), a failed
/// allocation is forgiven, and the new frame's bitmap starts all free.
pub fn toggle_dyn_model_lighting_frame(
    pixel_free_bits: &mut [&mut [u32]; MODEL_LIGHTING_PIXEL_FREE_BITS_BUFFERS],
    mod_frame_count: &mut u32,
    alloc_fail: &mut bool,
) {
    *mod_frame_count = (*mod_frame_count + 1) & 3;
    *alloc_fail = false;
    pixel_free_bits[*mod_frame_count as usize].fill(u32::MAX);
}

/// The handle a model holds for a slot. Handle 0 means "no slot", so handles
/// start one past `base_index`. `None` when it does not fit in 16 bits.
fn slot_handle(base_index: u32, slot: u32) -> Option<u16> {
    let handle = base_index.checked_add(slot)?.checked_add(1)?;
    u16::try_from(handle).ok()
}

/// The slot behind a handle; `None` for the empty handle or one that belongs
/// to the static models below `base_index`.
pub fn model_lighting_cache_slot(base_index: u32, handle: u16) -> Option<u32> {
    let handle = u32::from(handle);
    if handle <= base_index {
        return None;
    }
    Some(handle - base_index - 1)
}

/// The bit for a slot inside its word: slot 0 is the top bit.
const fn slot_bit(slot: u32) -> u32 {
    0x8000_0000 >> (slot & 31)
}

/// A slot's stored lighting info: the scene light index in the low byte and
/// the reflection probe index in the high byte.
pub const fn lighting_info_from_bytes(primary_light_index: u8, reflection_probe_index: u8) -> u16 {
    u16::from_le_bytes([primary_light_index, reflection_probe_index])
}

/// The scene light index stored in a slot's lighting info.
pub const fn lighting_info_scene_light_index(info: u16) -> u8 {
    info.to_le_bytes()[0]
}

/// The reflection probe index stored in a slot's lighting info.
pub const fn lighting_info_reflection_probe_index(info: u16) -> u8 {
    info.to_le_bytes()[1]
}

/// The cache's state borrowed for one frame's allocations.
#[derive(Debug)]
pub struct ModelLightingCacheGlob<'a> {
    base_index: u32,
    xmodel_entry_limit: u32,
    rover: &'a mut u32,
    alloc_fail: &'a mut bool,
    origins: &'a mut [[f32; 3]],
    lighting_info: &'a mut [u16],
    prev_prev: &'a [u32],
    prev: &'a [u32],
    curr: &'a mut [u32],
}

impl<'a> ModelLightingCacheGlob<'a> {
    /// Checks the slot count and that every array is long enough for it.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        base_index: u32,
        xmodel_entry_limit: u32,
        rover: &'a mut u32,
        alloc_fail: &'a mut bool,
        origins: &'a mut [[f32; 3]],
        lighting_info: &'a mut [u16],
        prev_prev: &'a [u32],
        prev: &'a [u32],
        curr: &'a mut [u32],
    ) -> Result<Self, ModelLightingCacheGlobError> {
        let last_slot_fits =
            xmodel_entry_limit != 0 && slot_handle(base_index, xmodel_entry_limit - 1).is_some();
        if !last_slot_fits {
            return Err(ModelLightingCacheGlobError::LimitsOutOfRange {
                base_index,
                xmodel_entry_limit,
            });
        }
        let slots = xmodel_entry_limit as usize;
        let entries = origins.len().min(lighting_info.len());
        if entries < slots {
            return Err(ModelLightingCacheGlobError::EntryArrayTooShort {
                needed: slots,
                got: entries,
            });
        }
        let words = model_lighting_cache_free_bits_words(xmodel_entry_limit);
        let bitmap_words = prev_prev.len().min(prev.len()).min(curr.len());
        if bitmap_words < words {
            return Err(ModelLightingCacheGlobError::FreeBitsWordCountMismatch {
                needed: words,
                got: bitmap_words,
            });
        }
        Ok(Self {
            base_index,
            xmodel_entry_limit,
            rover,
            alloc_fail,
            origins,
            lighting_info,
            prev_prev,
            prev,
            curr,
        })
    }

    /// The warning index to report when [`alloc`](Self::alloc) fails.
    pub const fn warn_index_on_fail() -> u32 {
        MODEL_LIGHTING_WARN_CACHE_ALLOC_FAILED
    }

    /// The slots in `word` that are free in all three recent frames.
    fn free_in_word(&self, word: usize) -> u32 {
        self.prev_prev[word] & self.prev[word] & self.curr[word]
    }

    /// Marks a slot as taken this frame.
    fn take(&mut self, slot: u32) {
        self.curr[(slot >> 5) as usize] &= !slot_bit(slot);
    }

    /// Gives a model a slot for this frame. A model that already holds one
    /// keeps it when it has not moved (or, with `allow_moved_reuse`, takes the
    /// same slot to be resampled at its new origin); otherwise the first free
    /// slot at or after the rover is used. After one failure every further
    /// request fails until the next frame.
    pub fn alloc(
        &mut self,
        current_handle: u16,
        origin: [f32; 3],
        allow_moved_reuse: bool,
    ) -> ModelLightingCacheAlloc {
        let held = model_lighting_cache_slot(self.base_index, current_handle)
            .filter(|&slot| slot < self.xmodel_entry_limit);
        if let Some(slot) = held {
            let at = slot as usize;
            if self.origins[at] == origin {
                self.take(slot);
                return ModelLightingCacheAlloc::Reused {
                    handle: current_handle,
                    info: self.lighting_info[at],
                };
            }
            if allow_moved_reuse {
                self.take(slot);
                self.origins[at] = origin;
                return ModelLightingCacheAlloc::Assigned {
                    handle: current_handle,
                    slot,
                };
            }
        }
        if *self.alloc_fail {
            return ModelLightingCacheAlloc::Failed;
        }

        let words = model_lighting_cache_free_bits_words(self.xmodel_entry_limit);
        let first = *self.rover as usize % words;
        for step in 0..words {
            let word = (first + step) % words;
            let free = self.free_in_word(word);
            if free == 0 {
                continue;
            }
            // The lowest free slot in the word is the topmost set bit.
            let slot = word as u32 * 32 + free.leading_zeros();
            if slot >= self.xmodel_entry_limit {
                continue;
            }
            let Some(handle) = slot_handle(self.base_index, slot) else {
                break;
            };
            self.take(slot);
            *self.rover = word as u32;
            self.origins[slot as usize] = origin;
            return ModelLightingCacheAlloc::Assigned { handle, slot };
        }
        *self.alloc_fail = true;
        ModelLightingCacheAlloc::Failed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Owns everything a glob borrows.
    struct Cache {
        rover: u32,
        alloc_fail: bool,
        origins: Vec<[f32; 3]>,
        info: Vec<u16>,
        prev_prev: Vec<u32>,
        prev: Vec<u32>,
        curr: Vec<u32>,
    }

    impl Cache {
        fn all_free(slots: u32) -> Self {
            let words = model_lighting_cache_free_bits_words(slots);
            Self {
                rover: 0,
                alloc_fail: false,
                origins: vec![[0.0; 3]; slots as usize],
                info: vec![0; slots as usize],
                prev_prev: vec![u32::MAX; words],
                prev: vec![u32::MAX; words],
                curr: vec![u32::MAX; words],
            }
        }

        fn glob(&mut self, base: u32, slots: u32) -> ModelLightingCacheGlob<'_> {
            ModelLightingCacheGlob::new(
                base,
                slots,
                &mut self.rover,
                &mut self.alloc_fail,
                &mut self.origins,
                &mut self.info,
                &self.prev_prev,
                &self.prev,
                &mut self.curr,
            )
            .unwrap()
        }
    }

    #[test]
    fn a_bitmap_needs_a_word_per_thirty_two_slots() {
        assert_eq!(model_lighting_cache_free_bits_words(0), 0);
        assert_eq!(model_lighting_cache_free_bits_words(1), 1);
        assert_eq!(model_lighting_cache_free_bits_words(32), 1);
        assert_eq!(model_lighting_cache_free_bits_words(33), 2);
    }

    #[test]
    fn older_frames_step_back_around_the_four_bitmaps() {
        assert_eq!(dyn_pixel_free_bits_index(2, 0), 2);
        assert_eq!(dyn_pixel_free_bits_index(2, 1), 1);
        assert_eq!(dyn_pixel_free_bits_index(0, 1), 3);
        assert_eq!(dyn_pixel_free_bits_index(1, 2), 3);
        assert_eq!(dyn_pixel_free_bits_index(3, 3), 0);
    }

    #[test]
    fn the_next_frame_starts_with_every_slot_free_and_failure_forgiven() {
        let mut bufs = [vec![0u32; 2], vec![0u32; 2], vec![0u32; 2], vec![0u32; 2]];
        let [a, b, c, d] = &mut bufs;
        let mut frame = 0;
        let mut failed = true;
        toggle_dyn_model_lighting_frame(
            &mut [
                a.as_mut_slice(),
                b.as_mut_slice(),
                c.as_mut_slice(),
                d.as_mut_slice(),
            ],
            &mut frame,
            &mut failed,
        );
        assert_eq!(frame, 1);
        assert!(!failed);
        assert_eq!(bufs[1], vec![u32::MAX; 2]);
        assert_eq!(bufs[0], vec![0; 2]);
        assert_eq!(bufs[2], vec![0; 2]);
    }

    #[test]
    fn the_frame_counter_wraps_after_three() {
        let mut bufs = [vec![0u32], vec![0u32], vec![0u32], vec![0u32]];
        let [a, b, c, d] = &mut bufs;
        let mut frame = 3;
        let mut failed = false;
        toggle_dyn_model_lighting_frame(
            &mut [
                a.as_mut_slice(),
                b.as_mut_slice(),
                c.as_mut_slice(),
                d.as_mut_slice(),
            ],
            &mut frame,
            &mut failed,
        );
        assert_eq!(frame, 0);
        assert_eq!(bufs[0], vec![u32::MAX]);
    }

    #[test]
    fn handles_start_one_past_the_base() {
        assert_eq!(slot_handle(10, 0), Some(11));
        assert_eq!(slot_handle(10, 5), Some(16));
        assert_eq!(slot_handle(0xfffe, 0), Some(0xffff));
        assert_eq!(slot_handle(0xffff, 0), None);
        assert_eq!(slot_handle(u32::MAX, 0), None);
    }

    #[test]
    fn a_handle_maps_back_to_its_slot() {
        assert_eq!(model_lighting_cache_slot(10, 11), Some(0));
        assert_eq!(model_lighting_cache_slot(10, 16), Some(5));
        for slot in 0..50 {
            let handle = slot_handle(7, slot).unwrap();
            assert_eq!(model_lighting_cache_slot(7, handle), Some(slot));
        }
    }

    #[test]
    fn the_empty_handle_and_static_model_handles_have_no_slot() {
        assert_eq!(model_lighting_cache_slot(0, 0), None);
        assert_eq!(model_lighting_cache_slot(10, 0), None);
        assert_eq!(model_lighting_cache_slot(10, 10), None);
        assert_eq!(model_lighting_cache_slot(10, 3), None);
    }

    #[test]
    fn slot_zero_is_the_top_bit_of_its_word() {
        assert_eq!(slot_bit(0), 0x8000_0000);
        assert_eq!(slot_bit(1), 0x4000_0000);
        assert_eq!(slot_bit(31), 1);
        assert_eq!(slot_bit(32), 0x8000_0000);
    }

    #[test]
    fn lighting_info_keeps_the_light_low_and_the_probe_high() {
        let info = lighting_info_from_bytes(0x12, 0x34);
        assert_eq!(info, 0x3412);
        assert_eq!(lighting_info_scene_light_index(info), 0x12);
        assert_eq!(lighting_info_reflection_probe_index(info), 0x34);
    }

    #[test]
    fn setup_refuses_no_slots_and_handles_that_overflow() {
        let mut c = Cache::all_free(4);
        for (base, slots) in [(0, 0), (0xffff, 1), (0xfffe, 2)] {
            let err = ModelLightingCacheGlob::new(
                base,
                slots,
                &mut c.rover,
                &mut c.alloc_fail,
                &mut c.origins,
                &mut c.info,
                &c.prev_prev,
                &c.prev,
                &mut c.curr,
            )
            .unwrap_err();
            assert_eq!(
                err,
                ModelLightingCacheGlobError::LimitsOutOfRange {
                    base_index: base,
                    xmodel_entry_limit: slots
                }
            );
        }
    }

    #[test]
    fn setup_refuses_arrays_that_are_too_short() {
        let mut c = Cache::all_free(40);
        c.origins.truncate(39);
        let err = ModelLightingCacheGlob::new(
            0,
            40,
            &mut c.rover,
            &mut c.alloc_fail,
            &mut c.origins,
            &mut c.info,
            &c.prev_prev,
            &c.prev,
            &mut c.curr,
        )
        .unwrap_err();
        assert_eq!(
            err,
            ModelLightingCacheGlobError::EntryArrayTooShort {
                needed: 40,
                got: 39
            }
        );

        let mut c = Cache::all_free(40);
        c.prev.truncate(1);
        let err = ModelLightingCacheGlob::new(
            0,
            40,
            &mut c.rover,
            &mut c.alloc_fail,
            &mut c.origins,
            &mut c.info,
            &c.prev_prev,
            &c.prev,
            &mut c.curr,
        )
        .unwrap_err();
        assert_eq!(
            err,
            ModelLightingCacheGlobError::FreeBitsWordCountMismatch { needed: 2, got: 1 }
        );
    }

    #[test]
    fn new_models_take_slots_in_order_and_mark_them_used() {
        let mut c = Cache::all_free(40);
        let mut glob = c.glob(100, 40);
        let first = glob.alloc(0, [1.0, 0.0, 0.0], false);
        let second = glob.alloc(0, [2.0, 0.0, 0.0], false);
        assert_eq!(
            first,
            ModelLightingCacheAlloc::Assigned {
                handle: 101,
                slot: 0
            }
        );
        assert_eq!(
            second,
            ModelLightingCacheAlloc::Assigned {
                handle: 102,
                slot: 1
            }
        );
        assert_eq!(c.curr[0], u32::MAX & !0xc000_0000);
        assert_eq!(c.origins[0], [1.0, 0.0, 0.0]);
        assert_eq!(c.origins[1], [2.0, 0.0, 0.0]);
    }

    #[test]
    fn a_slot_used_in_a_recent_frame_is_skipped() {
        let mut c = Cache::all_free(40);
        c.prev[0] &= !slot_bit(0);
        c.prev_prev[0] &= !slot_bit(1);
        let mut glob = c.glob(0, 40);
        assert_eq!(
            glob.alloc(0, [0.0; 3], false),
            ModelLightingCacheAlloc::Assigned { handle: 3, slot: 2 }
        );
    }

    #[test]
    fn a_model_that_has_not_moved_keeps_its_slot_and_its_info() {
        let mut c = Cache::all_free(8);
        c.origins[3] = [5.0, 6.0, 7.0];
        c.info[3] = 0x0a0b;
        c.curr[0] |= slot_bit(3);
        c.curr[0] &= !slot_bit(3);
        c.curr[0] |= slot_bit(3);
        let mut glob = c.glob(0, 8);
        let got = glob.alloc(4, [5.0, 6.0, 7.0], false);
        assert_eq!(
            got,
            ModelLightingCacheAlloc::Reused {
                handle: 4,
                info: 0x0a0b
            }
        );
        assert_eq!(c.curr[0] & slot_bit(3), 0);
    }

    #[test]
    fn a_model_that_moved_is_resampled_in_place_when_allowed() {
        let mut c = Cache::all_free(8);
        c.origins[3] = [5.0, 6.0, 7.0];
        let mut glob = c.glob(0, 8);
        let got = glob.alloc(4, [9.0, 9.0, 9.0], true);
        assert_eq!(
            got,
            ModelLightingCacheAlloc::Assigned { handle: 4, slot: 3 }
        );
        assert_eq!(c.origins[3], [9.0, 9.0, 9.0]);
        assert_eq!(c.curr[0] & slot_bit(3), 0);
    }

    #[test]
    fn a_model_that_moved_gets_a_fresh_slot_when_not_allowed() {
        let mut c = Cache::all_free(8);
        c.origins[3] = [5.0, 6.0, 7.0];
        let mut glob = c.glob(0, 8);
        let got = glob.alloc(4, [9.0, 9.0, 9.0], false);
        assert_eq!(
            got,
            ModelLightingCacheAlloc::Assigned { handle: 1, slot: 0 }
        );
        assert_eq!(c.origins[3], [5.0, 6.0, 7.0]);
    }

    #[test]
    fn a_stale_handle_past_the_slot_count_falls_back_to_a_fresh_slot() {
        let mut c = Cache::all_free(8);
        let mut glob = c.glob(0, 8);
        let got = glob.alloc(50, [1.0; 3], true);
        assert_eq!(
            got,
            ModelLightingCacheAlloc::Assigned { handle: 1, slot: 0 }
        );
    }

    #[test]
    fn slots_past_the_limit_are_never_handed_out() {
        let mut c = Cache::all_free(3);
        let mut glob = c.glob(0, 3);
        for slot in 0..3 {
            let got = glob.alloc(0, [slot as f32; 3], false);
            assert_eq!(
                got,
                ModelLightingCacheAlloc::Assigned {
                    handle: slot as u16 + 1,
                    slot
                }
            );
        }
        assert_eq!(
            glob.alloc(0, [9.0; 3], false),
            ModelLightingCacheAlloc::Failed
        );
    }

    #[test]
    fn running_out_marks_the_frame_failed_until_the_next_toggle() {
        let mut c = Cache::all_free(2);
        {
            let mut glob = c.glob(0, 2);
            glob.alloc(0, [1.0; 3], false);
            glob.alloc(0, [2.0; 3], false);
            assert_eq!(
                glob.alloc(0, [3.0; 3], false),
                ModelLightingCacheAlloc::Failed
            );
        }
        assert!(c.alloc_fail);
        // Freeing a slot does not help: the failure sticks.
        c.curr[0] = u32::MAX;
        let mut glob = c.glob(0, 2);
        assert_eq!(
            glob.alloc(0, [4.0; 3], false),
            ModelLightingCacheAlloc::Failed
        );
    }

    #[test]
    fn a_failed_frame_still_serves_models_that_hold_a_slot() {
        let mut c = Cache::all_free(2);
        c.alloc_fail = true;
        c.origins[1] = [8.0; 3];
        let mut glob = c.glob(0, 2);
        assert_eq!(
            glob.alloc(2, [8.0; 3], false),
            ModelLightingCacheAlloc::Reused { handle: 2, info: 0 }
        );
    }

    #[test]
    fn the_search_starts_at_the_rover_word_and_wraps() {
        let mut c = Cache::all_free(96);
        c.rover = 1;
        c.curr[1] = 0;
        c.curr[2] = 0;
        let mut glob = c.glob(0, 96);
        let got = glob.alloc(0, [1.0; 3], false);
        assert_eq!(
            got,
            ModelLightingCacheAlloc::Assigned { handle: 1, slot: 0 }
        );
        assert_eq!(c.rover, 0);
    }

    #[test]
    fn the_rover_follows_the_last_word_that_gave_a_slot() {
        let mut c = Cache::all_free(96);
        c.curr[0] = 0;
        let mut glob = c.glob(0, 96);
        let got = glob.alloc(0, [1.0; 3], false);
        assert_eq!(
            got,
            ModelLightingCacheAlloc::Assigned {
                handle: 33,
                slot: 32
            }
        );
        assert_eq!(c.rover, 1);
    }

    #[test]
    fn the_warning_index_is_the_cache_alloc_one() {
        assert_eq!(
            ModelLightingCacheGlob::warn_index_on_fail(),
            MODEL_LIGHTING_WARN_CACHE_ALLOC_FAILED
        );
    }
}
