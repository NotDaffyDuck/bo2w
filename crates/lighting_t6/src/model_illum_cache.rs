//! The cache that gives each dynamic model (a player body, a piece of glass)
//! a slot of lit texels in the model lighting atlas.
//!
//! Slots are handed out from "free" bitmaps, one per recent frame. A slot is
//! free only when it was free in the last three frames and not yet taken in
//! this one, so a texel is never rewritten while an older frame might still
//! be drawing from it.

use crate::prop_alloc::prop_illum_word_count;
use crate::prop_light::MODEL_ILLUMINATION_WARN_SLOT_GRANT_FAILED;

/// How many frames of free bitmaps are kept (the frame counter wraps at 4).
pub const MODEL_LIGHT_VACANCY_BITSET_COUNT: usize = 4;

/// What asking the cache for a slot came to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IllumSlotGrant {
    /// The model kept its slot and had not moved; `info` is what was stored.
    Recycled { owner: u16, bits_info: u16 },
    /// The model has a slot whose texels must be sampled again.
    SlotFilled { owner: u16, slot_no: u32 },
    /// Nothing was free.
    Failed,
}

/// Why a [`ModelLightCacheBundle`] could not be set up.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IllumBlobFault {
    /// No slots, or the last slot's handle would not fit in 16 bits (base, slot count).
    BoundsBeyondRange(u32, u32),
    /// One of the free bitmaps is too short for the slot count (words needed, shortest).
    FreeBitsWordCountMismatch(usize, usize),
    /// The origin or lighting-info array is too short for the slot count (needed, shortest).
    EntryListUndersized(usize, usize),
}

/// How many 32-bit words a free bitmap needs for `figure_entry_limit` slots.
pub const fn illum_free_word_count(figure_entry_limit: u32) -> usize {
    prop_illum_word_count(figure_entry_limit)
}

/// Which of the four bitmaps belongs to the frame `age` frames before
/// `mod_frame_tally`.
pub const fn free_texel_word(frame_count: u32, frames_old: u32) -> usize {
    (frame_count.wrapping_sub(frames_old) & 3) as usize
}

/// Moves on to the next frame: the counter steps (wrapping at 4), a failed
/// allocation is forgiven, and the new frame's bitmap starts all free.
pub fn toggle_debris_model_illum_frame(
    ps_free_bits: &mut [&mut [u32]; MODEL_LIGHT_VACANCY_BITSET_COUNT],
    mod_frame_tally: &mut u32,
    alloc_failures: &mut bool,
) {
    let next = free_texel_word(*mod_frame_tally + 1, 0);
    ps_free_bits[next].iter_mut().for_each(|word| *word = !0);
    (*mod_frame_tally, *alloc_failures) = (next as u32, false);
}

/// The handle a model holds for a slot. Handle 0 means "no slot", so handles
/// start one past `base_index`. `None` when it does not fit in 16 bits.
fn slot_handle(first_slot: u32, slot_no: u32) -> Option<u16> {
    let wide = u64::from(first_slot) + u64::from(slot_no) + 1;
    u16::try_from(wide).ok()
}

/// The slot behind a handle; `None` for the empty handle or one that belongs
/// to the static models below `base_index`.
pub fn illum_entry_slot(first_slot: u32, owner: u16) -> Option<u32> {
    u32::from(owner).checked_sub(first_slot)?.checked_sub(1)
}

/// The bit for a slot inside its word: slot 0 is the top bit.
const fn slot_bit(slot: u32) -> u32 {
    (1u32 << 31) >> (slot % 32)
}

/// A slot's stored lighting info: the scene light index in the low byte and
/// the reflection probe index in the high byte.
pub const fn illum_word_from_bytes(key_light_index: u8, env_probe_index: u8) -> u16 {
    u16::from_le_bytes([key_light_index, env_probe_index])
}

/// The scene light index stored in a slot's lighting info.
pub const fn light_info_to_scene_light_slot(info: u16) -> u8 {
    info.to_le_bytes()[0]
}

/// The reflection probe index stored in a slot's lighting info.
pub const fn lighting_word_reflection_probe(info: u16) -> u8 {
    info.to_le_bytes()[1]
}

/// What each slot remembers.
#[derive(Debug)]
struct SlotTables<'a> {
    /// Where the model was when its texels were sampled.
    origin: &'a mut [[f32; 3]],
    info: &'a mut [u16],
}

/// The free bitmaps of the three frames a slot must be free in.
#[derive(Debug)]
struct FreeMaps<'a> {
    /// Two and one frames back; only read.
    older: [&'a [u32]; 2],
    /// This frame; slots are cleared from it as they are taken.
    now: &'a mut [u32],
}

impl FreeMaps<'_> {
    /// The slots in `word` that are free in all three frames.
    fn free(&self, word: usize) -> u32 {
        self.older.iter().fold(self.now[word], |free, map| free & map[word])
    }

    /// Marks a slot as taken this frame.
    fn claim(&mut self, slot_no: u32) {
        self.now[(slot_no / 32) as usize] &= !slot_bit(slot_no);
    }
}

/// The cache's state borrowed for one frame's allocations.
#[derive(Debug)]
pub struct ModelLightCacheBundle<'a> {
    base: u32,
    slot_total: u32,
    /// The word the last slot came from; the next search starts there.
    scan_cursor: &'a mut u32,
    /// Set once a search fails; every later search this frame fails at once.
    failed: &'a mut bool,
    slots: SlotTables<'a>,
    maps: FreeMaps<'a>,
}

impl<'a> ModelLightCacheBundle<'a> {
    /// Checks the slot count and that every array is long enough for it.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        first_slot: u32,
        figure_entry_limit: u32,
        scan_cursor: &'a mut u32,
        alloc_failures: &'a mut bool,
        points: &'a mut [[f32; 3]],
        illum_info: &'a mut [u16],
        two_frames_ago: &'a [u32],
        before: &'a [u32],
        now_bits: &'a mut [u32],
    ) -> Result<Self, IllumBlobFault> {
        let fits = figure_entry_limit
            .checked_sub(1)
            .and_then(|last| slot_handle(first_slot, last))
            .is_some();
        if !fits {
            return Err(IllumBlobFault::BoundsBeyondRange(first_slot, figure_entry_limit));
        }
        let want = figure_entry_limit as usize;
        let have = points.len().min(illum_info.len());
        if have < want {
            return Err(IllumBlobFault::EntryListUndersized(want, have));
        }
        let want_words = illum_free_word_count(figure_entry_limit);
        let have_words = [two_frames_ago.len(), before.len(), now_bits.len()].into_iter().min().unwrap_or(0);
        if have_words < want_words {
            return Err(IllumBlobFault::FreeBitsWordCountMismatch(want_words, have_words));
        }
        Ok(Self {
            base: first_slot,
            slot_total: figure_entry_limit,
            scan_cursor,
            failed: alloc_failures,
            slots: SlotTables { origin: points, info: illum_info },
            maps: FreeMaps { older: [two_frames_ago, before], now: now_bits },
        })
    }

    /// The warning index to report when [`alloc`](Self::alloc) fails.
    pub const fn warn_index_on_fail() -> u32 {
        MODEL_ILLUMINATION_WARN_SLOT_GRANT_FAILED
    }

    /// The first free slot, searching whole words from the scan_cursor's word and
    /// wrapping; within a word the lowest slot wins. With the word it is in.
    fn find_free(&self) -> Option<(usize, u32)> {
        let words = illum_free_word_count(self.slot_total);
        let start = *self.scan_cursor as usize % words;
        (start..words).chain(0..start).find_map(|word| {
            let free = self.maps.free(word);
            let slot = word as u32 * 32 + free.leading_zeros();
            (free != 0 && slot < self.slot_total).then_some((word, slot))
        })
    }

    /// Gives a model a slot for this frame. A model that already holds one
    /// keeps it when it has not moved (or, with `allow_moved_recycle`, takes the
    /// same slot to be resampled at its new origin); otherwise the first free
    /// slot at or after the scan_cursor is used. After one failure every further
    /// request fails until the next frame.
    pub fn alloc(
        &mut self,
        held_key: u16,
        spot: [f32; 3],
        allow_moved_recycle: bool,
    ) -> IllumSlotGrant {
        let held = illum_entry_slot(self.base, held_key).filter(|&s| s < self.slot_total);
        if let Some(slot_no) = held {
            let at = slot_no as usize;
            let unmoved = self.slots.origin[at] == spot;
            if unmoved || allow_moved_recycle {
                self.maps.claim(slot_no);
                if unmoved {
                    let bits_info = self.slots.info[at];
                    return IllumSlotGrant::Recycled { owner: held_key, bits_info };
                }
                self.slots.origin[at] = spot;
                return IllumSlotGrant::SlotFilled { owner: held_key, slot_no };
            }
        }

        let found = if *self.failed { None } else { self.find_free() };
        let claimed = found.and_then(|(word, slot_no)| Some((word, slot_no, slot_handle(self.base, slot_no)?)));
        match claimed {
            Some((word, slot_no, owner)) => {
                self.maps.claim(slot_no);
                *self.scan_cursor = word as u32;
                self.slots.origin[slot_no as usize] = spot;
                IllumSlotGrant::SlotFilled { owner, slot_no }
            }
            None => {
                *self.failed = true;
                IllumSlotGrant::Failed
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Owns everything a glob borrows.
    struct Cache {
        scan_cursor: u32,
        alloc_failures: bool,
        spots: Vec<[f32; 3]>,
        info: Vec<u16>,
        two_frames_ago: Vec<u32>,
        prev: Vec<u32>,
        curr: Vec<u32>,
    }

    impl Cache {
        fn all_free(slots: u32) -> Self {
            let free = vec![u32::MAX; illum_free_word_count(slots)];
            Self {
                scan_cursor: 0,
                alloc_failures: false,
                spots: vec![[0.0; 3]; slots as usize],
                info: vec![0; slots as usize],
                two_frames_ago: free.clone(),
                prev: free.clone(),
                curr: free,
            }
        }

        fn try_glob(&mut self, base: u32, slots: u32) -> Result<ModelLightCacheBundle<'_>, IllumBlobFault> {
            let Cache { scan_cursor, alloc_failures, spots, info, two_frames_ago, prev, curr } = self;
            ModelLightCacheBundle::new(base, slots, scan_cursor, alloc_failures, spots, info, two_frames_ago, prev, curr)
        }

        fn glob(&mut self, base: u32, slots: u32) -> ModelLightCacheBundle<'_> {
            self.try_glob(base, slots).unwrap()
        }
    }

    fn assigned(owner: u16, slot_no: u32) -> IllumSlotGrant {
        IllumSlotGrant::SlotFilled { owner, slot_no }
    }

    /// Steps the frame over four bitmaps of `words` zeroed words.
    fn toggle_from(frame_no: u32, gave_up: bool, word_count: usize) -> (u32, bool, [Vec<u32>; 4]) {
        let mut bufs: [Vec<u32>; 4] = std::array::from_fn(|_| vec![0; word_count]);
        let (mut frame_no, mut gave_up) = (frame_no, gave_up);
        let [a, b, c, d] = &mut bufs;
        toggle_debris_model_illum_frame(&mut [a, b, c, d].map(Vec::as_mut_slice), &mut frame_no, &mut gave_up);
        (frame_no, gave_up, bufs)
    }

    #[test]
    fn a_bitmap_needs_a_word_per_thirty_two_slots() {
        let words: Vec<_> = [0, 1, 32, 33].into_iter().map(illum_free_word_count).collect();
        assert_eq!(words, [0, 1, 1, 2]);
    }

    #[test]
    fn older_frames_step_back_around_the_four_bitmaps() {
        let cases = [((2, 0), 2), ((2, 1), 1), ((0, 1), 3), ((1, 2), 3), ((3, 3), 0)];
        for ((frame, age), want) in cases {
            assert_eq!(free_texel_word(frame, age), want, "{frame} {age}");
        }
    }

    #[test]
    fn the_next_frame_starts_with_every_slot_free_and_failure_forgiven() {
        let (frame, failed, bufs) = toggle_from(0, true, 2);
        assert_eq!(frame, 1);
        assert!(!failed);
        assert_eq!(bufs[1], [u32::MAX; 2]);
        assert_eq!(bufs[0], [0; 2]);
        assert_eq!(bufs[2], [0; 2]);
    }

    #[test]
    fn the_frame_counter_wraps_after_three() {
        let (frame, _, bufs) = toggle_from(3, false, 1);
        assert_eq!(frame, 0);
        assert_eq!(bufs[0], [u32::MAX]);
    }

    #[test]
    fn handles_start_one_past_the_base() {
        let cases = [((10, 0), Some(11)), ((10, 5), Some(16)), ((0xfffe, 0), Some(0xffff))];
        for ((base, slot), want) in cases {
            assert_eq!(slot_handle(base, slot), want);
        }
        assert_eq!(slot_handle(0xffff, 0), None);
        assert_eq!(slot_handle(u32::MAX, 0), None);
    }

    #[test]
    fn a_handle_maps_back_to_its_slot() {
        assert_eq!(illum_entry_slot(10, 11), Some(0));
        assert_eq!(illum_entry_slot(10, 16), Some(5));
        assert!((0..50).all(|slot| illum_entry_slot(7, slot_handle(7, slot).unwrap()) == Some(slot)));
    }

    #[test]
    fn the_empty_handle_and_static_model_handles_have_no_slot() {
        for (base, handle) in [(0, 0), (10, 0), (10, 10), (10, 3)] {
            assert_eq!(illum_entry_slot(base, handle), None);
        }
    }

    #[test]
    fn slot_zero_is_the_top_bit_of_its_word() {
        let bits = [0, 1, 31, 32].map(slot_bit);
        assert_eq!(bits, [0x8000_0000, 0x4000_0000, 1, 0x8000_0000]);
    }

    #[test]
    fn lighting_info_keeps_the_light_low_and_the_probe_high() {
        let info = illum_word_from_bytes(0x12, 0x34);
        assert_eq!(info, 0x3412);
        assert_eq!(light_info_to_scene_light_slot(info), 0x12);
        assert_eq!(lighting_word_reflection_probe(info), 0x34);
    }

    #[test]
    fn setup_refuses_no_slots_and_handles_that_overflow() {
        let mut c = Cache::all_free(4);
        for (base, slots) in [(0, 0), (0xffff, 1), (0xfffe, 2)] {
            let err = c.try_glob(base, slots).unwrap_err();
            assert_eq!(err, IllumBlobFault::BoundsBeyondRange(base, slots));
        }
    }

    #[test]
    fn setup_refuses_arrays_that_are_too_short() {
        let mut c = Cache::all_free(40);
        c.spots.truncate(39);
        assert_eq!(c.try_glob(0, 40).unwrap_err(), IllumBlobFault::EntryListUndersized(40, 39));

        let mut c = Cache::all_free(40);
        c.prev.truncate(1);
        assert_eq!(c.try_glob(0, 40).unwrap_err(), IllumBlobFault::FreeBitsWordCountMismatch(2, 1));
    }

    #[test]
    fn new_models_take_slots_in_order_and_mark_them_used() {
        let mut c = Cache::all_free(40);
        let mut glob = c.glob(100, 40);
        let got = [1.0, 2.0].map(|x| glob.alloc(0, [x, 0.0, 0.0], false));
        assert_eq!(got, [assigned(101, 0), assigned(102, 1)]);
        assert_eq!(c.curr[0], u32::MAX & !0xc000_0000);
        assert_eq!(&c.spots[..2], &[[1.0, 0.0, 0.0], [2.0, 0.0, 0.0]]);
    }

    #[test]
    fn a_slot_used_in_a_recent_frame_is_skipped() {
        let mut c = Cache::all_free(40);
        c.prev[0] &= !slot_bit(0);
        c.two_frames_ago[0] &= !slot_bit(1);
        assert_eq!(c.glob(0, 40).alloc(0, [0.0; 3], false), assigned(3, 2));
    }

    #[test]
    fn a_model_that_has_not_moved_keeps_its_slot_and_its_info() {
        let mut c = Cache::all_free(8);
        c.spots[3] = [5.0, 6.0, 7.0];
        c.info[3] = 0x0a0b;
        let got = c.glob(0, 8).alloc(4, [5.0, 6.0, 7.0], false);
        assert_eq!(got, IllumSlotGrant::Recycled { owner: 4, bits_info: 0x0a0b });
        assert_eq!(c.curr[0] & slot_bit(3), 0);
    }

    #[test]
    fn a_model_that_moved_is_resampled_in_place_when_allowed() {
        let mut c = Cache::all_free(8);
        c.spots[3] = [5.0, 6.0, 7.0];
        assert_eq!(c.glob(0, 8).alloc(4, [9.0; 3], true), assigned(4, 3));
        assert_eq!(c.spots[3], [9.0; 3]);
        assert_eq!(c.curr[0] & slot_bit(3), 0);
    }

    #[test]
    fn a_model_that_moved_gets_a_fresh_slot_when_not_allowed() {
        let mut c = Cache::all_free(8);
        c.spots[3] = [5.0, 6.0, 7.0];
        assert_eq!(c.glob(0, 8).alloc(4, [9.0; 3], false), assigned(1, 0));
        assert_eq!(c.spots[3], [5.0, 6.0, 7.0]);
    }

    #[test]
    fn a_stale_handle_past_the_slot_count_falls_back_to_a_fresh_slot() {
        let mut c = Cache::all_free(8);
        assert_eq!(c.glob(0, 8).alloc(50, [1.0; 3], true), assigned(1, 0));
    }

    #[test]
    fn slots_past_the_limit_are_never_handed_out() {
        let mut c = Cache::all_free(3);
        let mut glob = c.glob(0, 3);
        for slot in 0..3u32 {
            assert_eq!(glob.alloc(0, [slot as f32; 3], false), assigned(slot as u16 + 1, slot));
        }
        assert_eq!(glob.alloc(0, [9.0; 3], false), IllumSlotGrant::Failed);
    }

    #[test]
    fn running_out_marks_the_frame_failed_until_the_next_toggle() {
        let mut c = Cache::all_free(2);
        let mut glob = c.glob(0, 2);
        let got = [1.0, 2.0, 3.0].map(|x| glob.alloc(0, [x; 3], false));
        assert_eq!(got[2], IllumSlotGrant::Failed);
        assert!(c.alloc_failures);
        // Freeing a slot does not help: the failure sticks.
        c.curr[0] = u32::MAX;
        assert_eq!(c.glob(0, 2).alloc(0, [4.0; 3], false), IllumSlotGrant::Failed);
    }

    #[test]
    fn a_failed_frame_still_serves_models_that_hold_a_slot() {
        let mut c = Cache::all_free(2);
        c.alloc_failures = true;
        c.spots[1] = [8.0; 3];
        let got = c.glob(0, 2).alloc(2, [8.0; 3], false);
        assert_eq!(got, IllumSlotGrant::Recycled { owner: 2, bits_info: 0 });
    }

    #[test]
    fn the_search_starts_at_the_rover_word_and_wraps() {
        let mut c = Cache::all_free(96);
        c.scan_cursor = 1;
        c.curr[1] = 0;
        c.curr[2] = 0;
        assert_eq!(c.glob(0, 96).alloc(0, [1.0; 3], false), assigned(1, 0));
        assert_eq!(c.scan_cursor, 0);
    }

    #[test]
    fn the_rover_follows_the_last_word_that_gave_a_slot() {
        let mut c = Cache::all_free(96);
        c.curr[0] = 0;
        assert_eq!(c.glob(0, 96).alloc(0, [1.0; 3], false), assigned(33, 32));
        assert_eq!(c.scan_cursor, 1);
    }

    #[test]
    fn the_warning_index_is_the_cache_alloc_one() {
        assert_eq!(ModelLightCacheBundle::warn_index_on_fail(), MODEL_ILLUMINATION_WARN_SLOT_GRANT_FAILED);
    }
}
