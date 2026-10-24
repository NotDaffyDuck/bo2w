//! Handing out lighting-atlas entries to the static models in view.
//!
//! Entries are taken in order until the atlas is full. After that a visible
//! model can take over an entry that has sat idle long enough; if there is
//! none the model goes unlit this frame.

use crate::smodel_lighting::{
    SMODEL_LIGHTING_FREEABLE_AGE_FRAMES, SMODEL_LIGHTING_RESERVED_ENTRY0_AFTER_WALK,
    defers_lighting, dirty_bit, handle_entry, model_lighting_handle_from_entry,
};

/// What to do about a static model whose lighting is out of date.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SModelDirtyLightingAction {
    /// Sample the lighting right now.
    SampleImmediate { smodel_index: u32, entry: u16 },
    /// Queue a patch to apply later.
    EnqueuePatch { smodel_index: u32, entry: u16 },
}

/// The action for a dirty model, or `None` if it holds no entry at all.
pub fn dirty_smodel_lighting_action(
    smodel_index: u32,
    handle: u16,
    flags: u8,
) -> Option<SModelDirtyLightingAction> {
    let entry = handle_entry(handle)?;
    Some(if defers_lighting(flags) {
        SModelDirtyLightingAction::EnqueuePatch {
            smodel_index,
            entry,
        }
    } else {
        SModelDirtyLightingAction::SampleImmediate {
            smodel_index,
            entry,
        }
    })
}

/// What asking for an entry came to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SModelLightingAlloc {
    /// The model already held this entry.
    Reused { handle: u16 },
    /// The model got a fresh entry.
    Assigned { handle: u16 },
    /// The model took over an idle entry from another model.
    Evicted {
        handle: u16,
        evicted_smodel_index: u16,
    },
    /// No entry was available.
    Failed,
}

/// Why a [`SModelLightingGlob`] could not be set up.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SModelLightingGlobError {
    /// No entries, or so many that handles would not fit in 16 bits.
    EntryLimitOutOfRange { entry_limit: u32 },
    /// One of the per-entry arrays is shorter than the entry limit.
    EntryArrayTooShort { needed: usize, got: usize },
    /// The dirty bitmap is too short for the model count.
    LightingBitsTooShort { needed: usize, got: usize },
}

/// How many 32-bit words the dirty bitmap needs for `smodel_count` models.
pub const fn smodel_lighting_bits_words(smodel_count: u32) -> usize {
    smodel_count.div_ceil(32) as usize
}

/// The allocator's running state, kept between frames.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SModelLightingCounters {
    /// Entries handed out so far (the next fresh entry's number).
    pub assigned_count: u32,
    /// How many of the freeable-handle list are still available.
    pub freeable_count: u32,
    /// The frame number, for ageing entries.
    pub frame_count: i32,
    /// Whether any model was given an entry since the dirty bits were read.
    pub any_new_lighting: bool,
}

impl SModelLightingCounters {
    /// The state after the light walk: entry 0 is taken, the rest untouched.
    pub const AFTER_LIGHT_WALK: Self = Self {
        assigned_count: SMODEL_LIGHTING_RESERVED_ENTRY0_AFTER_WALK,
        freeable_count: 0,
        frame_count: 0,
        any_new_lighting: false,
    };
}

/// The allocator's arrays and counters, borrowed for a frame.
#[derive(Debug)]
pub struct SModelLightingGlob<'a> {
    entry_limit: u32,
    smodel_count: u32,
    counters: &'a mut SModelLightingCounters,
    /// Handles of entries idle long enough to take over.
    freeable_handles: &'a mut [u16],
    /// Which static model currently holds each entry.
    smodel_index: &'a mut [u16],
    /// The frame each entry was last asked for.
    used_frame_count: &'a mut [i32],
    /// One bit per static model: its lighting needs sampling.
    lighting_bits: &'a mut [u32],
}

impl<'a> SModelLightingGlob<'a> {
    /// Checks the entry limit and that every array is long enough for it.
    pub fn new(
        entry_limit: u32,
        smodel_count: u32,
        counters: &'a mut SModelLightingCounters,
        freeable_handles: &'a mut [u16],
        smodel_index: &'a mut [u16],
        used_frame_count: &'a mut [i32],
        lighting_bits: &'a mut [u32],
    ) -> Result<Self, SModelLightingGlobError> {
        if entry_limit == 0 || entry_limit >= u32::from(u16::MAX) {
            return Err(SModelLightingGlobError::EntryLimitOutOfRange { entry_limit });
        }
        let needed = entry_limit as usize;
        let shortest = freeable_handles
            .len()
            .min(smodel_index.len())
            .min(used_frame_count.len());
        if shortest < needed {
            return Err(SModelLightingGlobError::EntryArrayTooShort {
                needed,
                got: shortest,
            });
        }
        let words = smodel_lighting_bits_words(smodel_count);
        if lighting_bits.len() < words {
            return Err(SModelLightingGlobError::LightingBitsTooShort {
                needed: words,
                got: lighting_bits.len(),
            });
        }
        Ok(Self {
            entry_limit,
            smodel_count,
            counters,
            freeable_handles,
            smodel_index,
            used_frame_count,
            lighting_bits,
        })
    }

    /// Clears every model's dirty bit.
    pub fn clear_dirty_bits(&mut self) {
        let words = smodel_lighting_bits_words(self.smodel_count);
        self.lighting_bits[..words].fill(0);
        self.counters.any_new_lighting = false;
    }

    /// Starts a new frame: ages the entries and lists the ones idle for at
    /// least the freeable age, ready to be taken over.
    pub fn toggle_frame(&mut self) {
        let counters = &mut *self.counters;
        counters.frame_count = counters.frame_count.wrapping_add(1);
        counters.freeable_count = 0;
        for entry in SMODEL_LIGHTING_RESERVED_ENTRY0_AFTER_WALK..counters.assigned_count {
            let idle = counters
                .frame_count
                .wrapping_sub(self.used_frame_count[entry as usize]);
            if idle >= SMODEL_LIGHTING_FREEABLE_AGE_FRAMES {
                self.freeable_handles[counters.freeable_count as usize] =
                    model_lighting_handle_from_entry(entry as u16);
                counters.freeable_count += 1;
            }
        }
    }

    /// Calls `visit` with each model whose bit is set (lowest index first,
    /// clearing the bits as it goes) and returns how many there were. Does
    /// nothing when no model has been given an entry since the last call.
    pub fn update_dirty_smodel_lighting(&mut self, mut visit: impl FnMut(u32)) -> u32 {
        if !self.counters.any_new_lighting {
            return 0;
        }
        self.counters.any_new_lighting = false;
        let words = smodel_lighting_bits_words(self.smodel_count);
        let mut visited = 0u32;
        for (word_index, word) in self.lighting_bits[..words].iter_mut().enumerate() {
            while *word != 0 {
                let bit = word.leading_zeros();
                *word &= !(0x8000_0000 >> bit);
                visit(word_index as u32 * 32 + bit);
                visited += 1;
            }
        }
        visited
    }

    /// Gives a visible static model an entry for this frame.
    ///
    /// A model that holds one keeps it (and the entry is stamped as used).
    /// Otherwise the next fresh entry is used, and once those run out the
    /// most recently listed freeable entry that was not used this frame is
    /// taken over from its holder. Either way the model is marked dirty.
    pub fn alloc_static_model_lighting(
        &mut self,
        smodel_index: u32,
        current_handle: u16,
    ) -> SModelLightingAlloc {
        if let Some(entry) = handle_entry(current_handle) {
            if let Some(stamp) = self.used_frame_count.get_mut(usize::from(entry)) {
                *stamp = self.counters.frame_count;
            }
            return SModelLightingAlloc::Reused {
                handle: current_handle,
            };
        }

        let (entry, evicted) = if self.counters.assigned_count < self.entry_limit {
            let entry = self.counters.assigned_count;
            self.counters.assigned_count += 1;
            (entry, None)
        } else {
            match self.take_freeable_entry() {
                Some(taken) => taken,
                None => return SModelLightingAlloc::Failed,
            }
        };

        let slot = entry as usize;
        self.smodel_index[slot] = smodel_index as u16;
        self.used_frame_count[slot] = self.counters.frame_count;
        let (word, bit) = dirty_bit(smodel_index);
        if let Some(bits) = self.lighting_bits.get_mut(word) {
            *bits |= bit;
        }
        self.counters.any_new_lighting = true;

        let handle = model_lighting_handle_from_entry(entry as u16);
        match evicted {
            Some(evicted_smodel_index) => SModelLightingAlloc::Evicted {
                handle,
                evicted_smodel_index,
            },
            None => SModelLightingAlloc::Assigned { handle },
        }
    }

    /// Pops freeable handles until one names an entry nobody asked for this
    /// frame; returns it with the model that was holding it.
    fn take_freeable_entry(&mut self) -> Option<(u32, Option<u16>)> {
        while self.counters.freeable_count > 0 {
            self.counters.freeable_count -= 1;
            let handle = self.freeable_handles[self.counters.freeable_count as usize];
            let Some(entry) = handle_entry(handle) else {
                continue;
            };
            let entry = u32::from(entry);
            if self.used_frame_count[entry as usize] != self.counters.frame_count {
                return Some((entry, Some(self.smodel_index[entry as usize])));
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Owns everything a glob borrows.
    struct Store {
        counters: SModelLightingCounters,
        freeable: Vec<u16>,
        owners: Vec<u16>,
        used: Vec<i32>,
        bits: Vec<u32>,
    }

    impl Store {
        fn new(entries: usize, smodels: u32) -> Self {
            Self {
                counters: SModelLightingCounters::AFTER_LIGHT_WALK,
                freeable: vec![0; entries],
                owners: vec![0; entries],
                used: vec![0; entries],
                bits: vec![0; smodel_lighting_bits_words(smodels)],
            }
        }

        fn glob(&mut self, entry_limit: u32, smodels: u32) -> SModelLightingGlob<'_> {
            SModelLightingGlob::new(
                entry_limit,
                smodels,
                &mut self.counters,
                &mut self.freeable,
                &mut self.owners,
                &mut self.used,
                &mut self.bits,
            )
            .unwrap()
        }
    }

    #[test]
    fn a_model_with_no_entry_has_no_dirty_action() {
        assert_eq!(dirty_smodel_lighting_action(3, 0, 0), None);
    }

    #[test]
    fn a_dirty_model_is_sampled_at_once_unless_its_flags_defer() {
        assert_eq!(
            dirty_smodel_lighting_action(3, 8, 0),
            Some(SModelDirtyLightingAction::SampleImmediate {
                smodel_index: 3,
                entry: 7
            })
        );
        assert_eq!(
            dirty_smodel_lighting_action(3, 8, 0x20),
            Some(SModelDirtyLightingAction::EnqueuePatch {
                smodel_index: 3,
                entry: 7
            })
        );
    }

    #[test]
    fn the_dirty_bitmap_needs_a_word_per_thirty_two_models() {
        assert_eq!(smodel_lighting_bits_words(0), 0);
        assert_eq!(smodel_lighting_bits_words(32), 1);
        assert_eq!(smodel_lighting_bits_words(33), 2);
    }

    #[test]
    fn the_light_walk_leaves_entry_zero_taken() {
        let start = SModelLightingCounters::AFTER_LIGHT_WALK;
        assert_eq!(start.assigned_count, 1);
        assert_eq!(start.freeable_count, 0);
        assert_eq!(start.frame_count, 0);
        assert!(!start.any_new_lighting);
    }

    #[test]
    fn setup_refuses_a_bad_entry_limit() {
        let mut s = Store::new(4, 8);
        for limit in [0, u32::from(u16::MAX), u32::MAX] {
            let err = SModelLightingGlob::new(
                limit,
                8,
                &mut s.counters,
                &mut s.freeable,
                &mut s.owners,
                &mut s.used,
                &mut s.bits,
            )
            .unwrap_err();
            assert_eq!(
                err,
                SModelLightingGlobError::EntryLimitOutOfRange { entry_limit: limit }
            );
        }
    }

    #[test]
    fn setup_refuses_arrays_that_are_too_short() {
        let mut s = Store::new(4, 8);
        s.used.truncate(3);
        let err = SModelLightingGlob::new(
            4,
            8,
            &mut s.counters,
            &mut s.freeable,
            &mut s.owners,
            &mut s.used,
            &mut s.bits,
        )
        .unwrap_err();
        assert_eq!(
            err,
            SModelLightingGlobError::EntryArrayTooShort { needed: 4, got: 3 }
        );

        let mut s = Store::new(4, 8);
        let err = SModelLightingGlob::new(
            4,
            100,
            &mut s.counters,
            &mut s.freeable,
            &mut s.owners,
            &mut s.used,
            &mut s.bits,
        )
        .unwrap_err();
        assert_eq!(
            err,
            SModelLightingGlobError::LightingBitsTooShort { needed: 4, got: 1 }
        );
    }

    #[test]
    fn fresh_entries_are_handed_out_in_order_starting_after_entry_zero() {
        let mut s = Store::new(4, 8);
        let mut glob = s.glob(4, 8);
        assert_eq!(
            glob.alloc_static_model_lighting(5, 0),
            SModelLightingAlloc::Assigned { handle: 2 }
        );
        assert_eq!(
            glob.alloc_static_model_lighting(6, 0),
            SModelLightingAlloc::Assigned { handle: 3 }
        );
        assert_eq!(s.owners[1], 5);
        assert_eq!(s.owners[2], 6);
        assert_eq!(s.counters.assigned_count, 3);
        assert!(s.counters.any_new_lighting);
    }

    #[test]
    fn a_new_assignment_marks_the_model_dirty() {
        let mut s = Store::new(4, 40);
        s.glob(4, 40).alloc_static_model_lighting(33, 0);
        assert_eq!(s.bits, vec![0, 0x4000_0000]);
    }

    #[test]
    fn a_model_that_holds_an_entry_keeps_it_and_stamps_it() {
        let mut s = Store::new(4, 8);
        s.counters.frame_count = 9;
        let mut glob = s.glob(4, 8);
        assert_eq!(
            glob.alloc_static_model_lighting(5, 3),
            SModelLightingAlloc::Reused { handle: 3 }
        );
        assert_eq!(s.used[2], 9);
        assert!(!s.counters.any_new_lighting);
        assert_eq!(s.bits, vec![0]);
    }

    #[test]
    fn a_handle_past_the_arrays_is_still_reused_without_a_stamp() {
        let mut s = Store::new(4, 8);
        let mut glob = s.glob(4, 8);
        assert_eq!(
            glob.alloc_static_model_lighting(5, 500),
            SModelLightingAlloc::Reused { handle: 500 }
        );
    }

    #[test]
    fn a_full_atlas_with_nothing_freeable_fails() {
        let mut s = Store::new(3, 8);
        let mut glob = s.glob(3, 8);
        glob.alloc_static_model_lighting(0, 0);
        glob.alloc_static_model_lighting(1, 0);
        assert_eq!(
            glob.alloc_static_model_lighting(2, 0),
            SModelLightingAlloc::Failed
        );
    }

    #[test]
    fn entries_idle_for_four_frames_become_freeable() {
        let mut s = Store::new(5, 8);
        {
            let mut glob = s.glob(5, 8);
            for model in 0..4 {
                glob.alloc_static_model_lighting(model, 0);
            }
        }
        // Entries 1..=4 were used in frame 0. Keep entry 2 alive.
        for frame in 1..=4 {
            s.counters.frame_count = frame - 1;
            let mut glob = s.glob(5, 8);
            glob.alloc_static_model_lighting(1, 3);
            glob.toggle_frame();
            if frame < 4 {
                assert_eq!(s.counters.freeable_count, 0, "frame {frame}");
            }
        }
        assert_eq!(s.counters.frame_count, 4);
        assert_eq!(s.counters.freeable_count, 3);
        assert_eq!(&s.freeable[..3], &[2, 4, 5]);
    }

    #[test]
    fn a_full_atlas_takes_over_the_last_freeable_entry() {
        let mut s = Store::new(3, 8);
        s.counters.assigned_count = 3;
        s.counters.frame_count = 10;
        s.owners = vec![0, 6, 7];
        s.used = vec![0, 2, 3];
        s.freeable = vec![2, 3, 0];
        s.counters.freeable_count = 2;
        let mut glob = s.glob(3, 8);
        assert_eq!(
            glob.alloc_static_model_lighting(4, 0),
            SModelLightingAlloc::Evicted {
                handle: 3,
                evicted_smodel_index: 7
            }
        );
        assert_eq!(s.owners[2], 4);
        assert_eq!(s.used[2], 10);
        assert_eq!(s.counters.freeable_count, 1);
        assert_eq!(s.bits[0], 0x0800_0000);
    }

    #[test]
    fn an_entry_used_this_frame_is_not_taken_over() {
        let mut s = Store::new(3, 8);
        s.counters.assigned_count = 3;
        s.counters.frame_count = 10;
        s.owners = vec![0, 6, 7];
        s.used = vec![0, 2, 10];
        s.freeable = vec![2, 3, 0];
        s.counters.freeable_count = 2;
        let mut glob = s.glob(3, 8);
        // Entry 2 (handle 3) is popped first but was used this frame, so
        // entry 1 (handle 2) is taken instead.
        assert_eq!(
            glob.alloc_static_model_lighting(4, 0),
            SModelLightingAlloc::Evicted {
                handle: 2,
                evicted_smodel_index: 6
            }
        );
        assert_eq!(s.counters.freeable_count, 0);
    }

    #[test]
    fn the_freeable_list_is_spent_as_entries_are_taken() {
        let mut s = Store::new(2, 8);
        s.counters.assigned_count = 2;
        s.counters.frame_count = 10;
        s.owners = vec![0, 6];
        s.used = vec![0, 1];
        s.freeable = vec![2, 0];
        s.counters.freeable_count = 1;
        let mut glob = s.glob(2, 8);
        assert!(matches!(
            glob.alloc_static_model_lighting(3, 0),
            SModelLightingAlloc::Evicted { .. }
        ));
        assert_eq!(
            glob.alloc_static_model_lighting(4, 0),
            SModelLightingAlloc::Failed
        );
    }

    #[test]
    fn dirty_models_are_visited_lowest_first_and_cleared() {
        let mut s = Store::new(8, 70);
        {
            let mut glob = s.glob(8, 70);
            for model in [65, 3, 40, 0] {
                glob.alloc_static_model_lighting(model, 0);
            }
        }
        let mut seen = Vec::new();
        let visited = s.glob(8, 70).update_dirty_smodel_lighting(|m| seen.push(m));
        assert_eq!(visited, 4);
        assert_eq!(seen, vec![0, 3, 40, 65]);
        assert_eq!(s.bits, vec![0, 0, 0]);
        assert!(!s.counters.any_new_lighting);
    }

    #[test]
    fn a_dirty_walk_with_nothing_new_visits_nothing() {
        let mut s = Store::new(4, 8);
        s.bits[0] = 0xffff_ffff;
        let visited = s
            .glob(4, 8)
            .update_dirty_smodel_lighting(|_| panic!("visited"));
        assert_eq!(visited, 0);
        assert_eq!(s.bits[0], 0xffff_ffff);
    }

    #[test]
    fn clearing_the_dirty_bits_forgets_the_new_lighting() {
        let mut s = Store::new(4, 40);
        s.glob(4, 40).alloc_static_model_lighting(35, 0);
        s.glob(4, 40).clear_dirty_bits();
        assert_eq!(s.bits, vec![0, 0]);
        assert!(!s.counters.any_new_lighting);
    }
}
