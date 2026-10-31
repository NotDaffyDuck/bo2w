//! Handing out lighting-atlas entries to the static models in view.
//!
//! Entries are taken in order until the atlas is full. After that a visible
//! model can take over an entry that has sat idle long enough; if there is
//! none the model goes unlit this frame.

use crate::prop_light::{
    STATIC_LIGHT_RECLAIM_AFTER_FRAMES, PROP_ENTRY0_HELD_PAST_WALK, defers_lighting,
    dirty_bit, handle_entry, model_illumination_handle_for_entry,
};

/// What to do about a static model whose lighting is out of date.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StaleIllumAction {
    /// Sample the lighting right now.
    SampleNow { prop_index: u32, entry: u16 },
    /// Queue a patch to apply later.
    QueuePatch { prop_index: u32, entry: u16 },
}

/// The action for a dirty model, or `None` if it holds no entry at all.
/// Models whose flags defer lighting get a patch; the rest are sampled now.
pub fn stale_illum_action(
    prop_index: u32,
    handle: u16,
    flags: u8,
) -> Option<StaleIllumAction> {
    use StaleIllumAction::{QueuePatch, SampleNow};
    handle_entry(handle).map(|entry| match defers_lighting(flags) {
        true => QueuePatch { prop_index, entry },
        false => SampleNow { prop_index, entry },
    })
}

/// What asking for an entry came to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SModelIllumAlloc {
    /// The model already held this entry.
    Recycled { handle: u16 },
    /// The model got a fresh entry.
    SlotFilled { handle: u16 },
    /// The model took over an idle entry from another model.
    Evicted {
        handle: u16,
        evicted_prop_index: u16,
    },
    /// No entry was available.
    Failed,
}

/// Why a [`StaticModelLightingBlob`] could not be set up.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PropLightBlobFault {
    /// No entries, or so many that handles would not fit in 16 bits (the limit asked for).
    EntryLimitOutOfRange(u32),
    /// One of the per-entry arrays is shorter than the entry limit (needed, shortest).
    EntryListUndersized(usize, usize),
    /// The dirty bitmap is too short for the model count (words needed, words given).
    LightBitsetUndersized(usize, usize),
}

/// How many 32-bit words the dirty bitmap needs for `static_model_total` models.
pub const fn prop_illum_word_count(static_model_total: u32) -> usize {
    let whole = static_model_total / 32;
    let part = (static_model_total % 32 != 0) as u32;
    (whole + part) as usize
}

/// The allocator's running state, kept between frames.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SModelIllumCounters {
    /// The frame number, for ageing entries.
    clock: i32,
    /// Entries handed out so far: the number of the next fresh entry.
    issued: u32,
    /// How much of the idle-handle list is still unspent.
    idle_left: u32,
    /// Some model was given an entry since the dirty bits were last read.
    news: bool,
}

impl SModelIllumCounters {
    /// The state after the light walk: entry 0 is taken, the rest untouched.
    pub const POST_LIGHT_TRAVERSAL: Self = Self { clock: 0, issued: PROP_ENTRY0_HELD_PAST_WALK, idle_left: 0, news: false };
}

/// Per-entry tables, all at least as long as the entry limit.
#[derive(Debug)]
struct EntryTables<'a> {
    /// Handles of entries idle long enough to take over.
    idle: &'a mut [u16],
    /// The static model holding each entry.
    holder: &'a mut [u16],
    /// The frame each entry was last asked for.
    stamp: &'a mut [i32],
}

/// The allocator's arrays and counters, borrowed for a frame.
#[derive(Debug)]
pub struct StaticModelLightingBlob<'a> {
    limit: u32,
    /// Words of `dirty` in use: one bit per static model.
    dirty_words: usize,
    state: &'a mut SModelIllumCounters,
    tables: EntryTables<'a>,
    /// A set bit means the model's lighting needs sampling.
    dirty: &'a mut [u32],
}

impl<'a> StaticModelLightingBlob<'a> {
    /// Checks the entry limit and that every array is long enough for it.
    pub fn new(
        max_entries_allowed: u32,
        static_model_total: u32,
        counters: &'a mut SModelIllumCounters,
        releasable_handles: &'a mut [u16],
        prop_index: &'a mut [u16],
        used_frame_tally: &'a mut [i32],
        illum_bits: &'a mut [u32],
    ) -> Result<Self, PropLightBlobFault> {
        if !(1..u32::from(u16::MAX)).contains(&max_entries_allowed) {
            return Err(PropLightBlobFault::EntryLimitOutOfRange(max_entries_allowed));
        }
        let want = max_entries_allowed as usize;
        let lengths = [releasable_handles.len(), prop_index.len(), used_frame_tally.len()];
        let shortest = lengths.into_iter().min().unwrap_or(0);
        if shortest < want {
            return Err(PropLightBlobFault::EntryListUndersized(want, shortest));
        }
        let dirty_words = prop_illum_word_count(static_model_total);
        if illum_bits.len() < dirty_words {
            return Err(PropLightBlobFault::LightBitsetUndersized(dirty_words, illum_bits.len()));
        }
        let tables = EntryTables { idle: releasable_handles, holder: prop_index, stamp: used_frame_tally };
        Ok(Self { limit: max_entries_allowed, dirty_words, state: counters, tables, dirty: illum_bits })
    }

    /// Clears every model's dirty bit.
    pub fn reset_changed_flags(&mut self) {
        self.dirty.iter_mut().take(self.dirty_words).for_each(|word| *word = 0);
        self.state.news = false;
    }

    /// Starts a new frame: ages the entries and lists the ones idle for at
    /// least the freeable age, ready to be taken over.
    pub fn flip_frame(&mut self) {
        let clock = self.state.clock.wrapping_add(1);
        let stamps = &*self.tables.stamp;
        let stale = (PROP_ENTRY0_HELD_PAST_WALK..self.state.issued)
            .filter(|&entry| clock.wrapping_sub(stamps[entry as usize]) >= STATIC_LIGHT_RECLAIM_AFTER_FRAMES)
            .map(|entry| model_illumination_handle_for_entry(entry as u16));
        let mut listed = 0;
        for (place, handle) in self.tables.idle.iter_mut().zip(stale) {
            *place = handle;
            listed += 1;
        }
        self.state.clock = clock;
        self.state.idle_left = listed;
    }

    /// Calls `visit` with each model whose bit is set (lowest index first,
    /// clearing the bits) and returns how many there were. Does nothing when
    /// no model has been given an entry since the last call.
    pub fn update_dirty_smodel_lighting(&mut self, mut visit: impl FnMut(u32)) -> u32 {
        if !std::mem::take(&mut self.state.news) {
            return 0;
        }
        let mut count = 0;
        for (first, word) in (0u32..).step_by(32).zip(self.dirty.iter_mut().take(self.dirty_words)) {
            let bits = std::mem::take(word);
            // The top bit of a word is its first model.
            for model in (0..32).filter(|n| bits & (0x8000_0000u32 >> n) != 0) {
                visit(first + model);
                count += 1;
            }
        }
        count
    }

    /// Gives a visible static model an entry for this frame.
    ///
    /// A model that holds one keeps it (and the entry is stamped as used).
    /// Otherwise the next fresh entry is used, and once those run out the
    /// most recently listed freeable entry that was not used this frame is
    /// taken over from its holder. Either way the model is marked dirty.
    pub fn reserve_static_model_light_slot(
        &mut self,
        prop_index: u32,
        held_key: u16,
    ) -> SModelIllumAlloc {
        if let Some(held) = handle_entry(held_key) {
            if let Some(stamp) = self.tables.stamp.get_mut(usize::from(held)) {
                *stamp = self.state.clock;
            }
            return SModelIllumAlloc::Recycled { handle: held_key };
        }
        let Some((entry, previous)) = self.fresh_entry().or_else(|| self.idle_entry()) else {
            return SModelIllumAlloc::Failed;
        };
        self.hand_over(entry, prop_index);
        let handle = model_illumination_handle_for_entry(entry);
        match previous {
            None => SModelIllumAlloc::SlotFilled { handle },
            Some(evicted_prop_index) => SModelIllumAlloc::Evicted { handle, evicted_prop_index },
        }
    }

    /// The next never-used entry, while the atlas has some.
    fn fresh_entry(&mut self) -> Option<(u16, Option<u16>)> {
        let next = self.state.issued;
        (next < self.limit).then(|| {
            self.state.issued = next + 1;
            (next as u16, None)
        })
    }

    /// Spends idle handles from the end of the list until one names an entry
    /// nobody asked for this frame; that entry and the model holding it.
    fn idle_entry(&mut self) -> Option<(u16, Option<u16>)> {
        while let Some(top) = self.state.idle_left.checked_sub(1) {
            self.state.idle_left = top;
            let entry = handle_entry(self.tables.idle[top as usize]);
            if let Some(entry) = entry.filter(|&e| self.tables.stamp[usize::from(e)] != self.state.clock) {
                return Some((entry, Some(self.tables.holder[usize::from(entry)])));
            }
        }
        None
    }

    /// Records `smodel` as holding `entry` this frame and marks it dirty.
    fn hand_over(&mut self, entry: u16, smodel: u32) {
        let at = usize::from(entry);
        self.tables.holder[at] = smodel as u16;
        self.tables.stamp[at] = self.state.clock;
        let (word, bitmask) = dirty_bit(smodel);
        if let Some(bits) = self.dirty.get_mut(word) {
            *bits |= bitmask;
        }
        self.state.news = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Owns everything a glob borrows.
    struct Store {
        counters: SModelIllumCounters,
        freeable: Vec<u16>,
        owners: Vec<u16>,
        used: Vec<i32>,
        bits: Vec<u32>,
    }

    impl Store {
        fn new(entries: usize, fixed_props: u32) -> Self {
            Self {
                counters: SModelIllumCounters::POST_LIGHT_TRAVERSAL,
                freeable: vec![0; entries],
                owners: vec![0; entries],
                used: vec![0; entries],
                bits: vec![0; prop_illum_word_count(fixed_props)],
            }
        }

        fn try_glob(&mut self, limit: u32, fixed_props: u32) -> Result<StaticModelLightingBlob<'_>, PropLightBlobFault> {
            let Store { counters, freeable, owners, used, bits } = self;
            StaticModelLightingBlob::new(limit, fixed_props, counters, freeable, owners, used, bits)
        }

        fn glob(&mut self, limit: u32, fixed_props: u32) -> StaticModelLightingBlob<'_> {
            self.try_glob(limit, fixed_props).unwrap()
        }

        /// A full atlas at frame 10 whose entries are held by `owners`, last
        /// used on `used`, with the first `listed` of `freeable` spendable.
        fn full(owners: &[u16], used: &[i32], freeable: &[u16], listed: u32) -> Self {
            let mut s = Store::new(owners.len(), 8);
            s.counters.issued = owners.len() as u32;
            s.counters.clock = 10;
            s.counters.idle_left = listed;
            (s.owners, s.used, s.freeable) = (owners.to_vec(), used.to_vec(), freeable.to_vec());
            s
        }
    }

    #[test]
    fn a_model_with_no_entry_has_no_dirty_action() {
        assert_eq!(stale_illum_action(3, 0, 0), None);
    }

    #[test]
    fn a_dirty_model_is_sampled_at_once_unless_its_flags_defer() {
        use StaleIllumAction::{QueuePatch, SampleNow};
        let now = stale_illum_action(3, 8, 0);
        let later = stale_illum_action(3, 8, 0x20);
        assert_eq!(now, Some(SampleNow { prop_index: 3, entry: 7 }));
        assert_eq!(later, Some(QueuePatch { prop_index: 3, entry: 7 }));
    }

    #[test]
    fn the_dirty_bitmap_needs_a_word_per_thirty_two_models() {
        let words: Vec<_> = [0, 32, 33].into_iter().map(prop_illum_word_count).collect();
        assert_eq!(words, [0, 1, 2]);
    }

    #[test]
    fn the_light_walk_leaves_entry_zero_taken() {
        let start = SModelIllumCounters::POST_LIGHT_TRAVERSAL;
        assert_eq!((start.issued, start.idle_left, start.clock, start.news), (1, 0, 0, false));
    }

    #[test]
    fn setup_refuses_a_bad_entry_limit() {
        let mut s = Store::new(4, 8);
        for limit in [0, u32::from(u16::MAX), u32::MAX] {
            let err = s.try_glob(limit, 8).unwrap_err();
            assert_eq!(err, PropLightBlobFault::EntryLimitOutOfRange(limit));
        }
    }

    #[test]
    fn setup_refuses_arrays_that_are_too_short() {
        let mut s = Store::new(4, 8);
        s.used.truncate(3);
        assert_eq!(s.try_glob(4, 8).unwrap_err(), PropLightBlobFault::EntryListUndersized(4, 3));

        let mut s = Store::new(4, 8);
        assert_eq!(s.try_glob(4, 100).unwrap_err(), PropLightBlobFault::LightBitsetUndersized(4, 1));
    }

    #[test]
    fn fresh_entries_are_handed_out_in_order_starting_after_entry_zero() {
        let mut s = Store::new(4, 8);
        let mut glob = s.glob(4, 8);
        let got = [5, 6].map(|model| glob.reserve_static_model_light_slot(model, 0));
        assert_eq!(got, [2, 3].map(|handle| SModelIllumAlloc::SlotFilled { handle }));
        assert_eq!(&s.owners[1..3], &[5, 6]);
        assert_eq!(s.counters.issued, 3);
        assert!(s.counters.news);
    }

    #[test]
    fn a_new_assignment_marks_the_model_dirty() {
        let mut s = Store::new(4, 40);
        s.glob(4, 40).reserve_static_model_light_slot(33, 0);
        assert_eq!(s.bits, [0, 0x4000_0000]);
    }

    #[test]
    fn a_model_that_holds_an_entry_keeps_it_and_stamps_it() {
        let mut s = Store::new(4, 8);
        s.counters.clock = 9;
        let got = s.glob(4, 8).reserve_static_model_light_slot(5, 3);
        assert_eq!(got, SModelIllumAlloc::Recycled { handle: 3 });
        assert_eq!(s.used[2], 9);
        assert!(!s.counters.news);
        assert_eq!(s.bits, [0]);
    }

    #[test]
    fn a_handle_past_the_arrays_is_still_reused_without_a_stamp() {
        let mut s = Store::new(4, 8);
        let got = s.glob(4, 8).reserve_static_model_light_slot(5, 500);
        assert_eq!(got, SModelIllumAlloc::Recycled { handle: 500 });
    }

    #[test]
    fn a_full_atlas_with_nothing_freeable_fails() {
        let mut s = Store::new(3, 8);
        let mut glob = s.glob(3, 8);
        let got: Vec<_> = (0..3).map(|model| glob.reserve_static_model_light_slot(model, 0)).collect();
        assert_eq!(got[2], SModelIllumAlloc::Failed);
    }

    #[test]
    fn entries_idle_for_four_frames_become_freeable() {
        let mut s = Store::new(5, 8);
        let mut glob = s.glob(5, 8);
        (0..4).for_each(|model| {
            glob.reserve_static_model_light_slot(model, 0);
        });
        // Entries 1..=4 were used in frame 0. Keep entry 2 alive.
        for frame in 1..=4 {
            s.counters.clock = frame - 1;
            let mut glob = s.glob(5, 8);
            glob.reserve_static_model_light_slot(1, 3);
            glob.flip_frame();
            if frame < 4 {
                assert_eq!(s.counters.idle_left, 0, "frame {frame}");
            }
        }
        assert_eq!((s.counters.clock, s.counters.idle_left), (4, 3));
        assert_eq!(&s.freeable[..3], &[2, 4, 5]);
    }

    #[test]
    fn a_full_atlas_takes_over_the_last_freeable_entry() {
        let mut s = Store::full(&[0, 6, 7], &[0, 2, 3], &[2, 3, 0], 2);
        let got = s.glob(3, 8).reserve_static_model_light_slot(4, 0);
        assert_eq!(got, SModelIllumAlloc::Evicted { handle: 3, evicted_prop_index: 7 });
        assert_eq!((s.owners[2], s.used[2]), (4, 10));
        assert_eq!(s.counters.idle_left, 1);
        assert_eq!(s.bits[0], 0x0800_0000);
    }

    #[test]
    fn an_entry_used_this_frame_is_not_taken_over() {
        let mut s = Store::full(&[0, 6, 7], &[0, 2, 10], &[2, 3, 0], 2);
        // Entry 2 (handle 3) is popped first but was used this frame, so
        // entry 1 (handle 2) is taken instead.
        let got = s.glob(3, 8).reserve_static_model_light_slot(4, 0);
        assert_eq!(got, SModelIllumAlloc::Evicted { handle: 2, evicted_prop_index: 6 });
        assert_eq!(s.counters.idle_left, 0);
    }

    #[test]
    fn the_freeable_list_is_spent_as_entries_are_taken() {
        let mut s = Store::full(&[0, 6], &[0, 1], &[2, 0], 1);
        let mut glob = s.glob(2, 8);
        let first = glob.reserve_static_model_light_slot(3, 0);
        assert!(matches!(first, SModelIllumAlloc::Evicted { .. }));
        assert_eq!(glob.reserve_static_model_light_slot(4, 0), SModelIllumAlloc::Failed);
    }

    #[test]
    fn dirty_models_are_visited_lowest_first_and_cleared() {
        let mut s = Store::new(8, 70);
        let mut glob = s.glob(8, 70);
        for model in [65, 3, 40, 0] {
            glob.reserve_static_model_light_slot(model, 0);
        }
        let mut seen = Vec::new();
        assert_eq!(glob.update_dirty_smodel_lighting(|m| seen.push(m)), 4);
        assert_eq!(seen, [0, 3, 40, 65]);
        assert_eq!(s.bits, [0, 0, 0]);
        assert!(!s.counters.news);
    }

    #[test]
    fn a_dirty_walk_with_nothing_new_visits_nothing() {
        let mut s = Store::new(4, 8);
        s.bits[0] = u32::MAX;
        assert_eq!(s.glob(4, 8).update_dirty_smodel_lighting(|_| panic!("visited")), 0);
        assert_eq!(s.bits[0], u32::MAX);
    }

    #[test]
    fn clearing_the_dirty_bits_forgets_the_new_lighting() {
        let mut s = Store::new(4, 40);
        s.glob(4, 40).reserve_static_model_light_slot(35, 0);
        s.glob(4, 40).reset_changed_flags();
        assert_eq!(s.bits, [0, 0]);
        assert!(!s.counters.news);
    }
}
