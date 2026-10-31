//! Deciding how a run of cached static models reaches the screen.
//!
//! The cached models of one bucket either become one "pretess" command that
//! points at an index range copied for them, or one "cached" command listing
//! the cache indices. A run that spans more than one cache bank is cut at the
//! bank edge, because a command addresses one bank only.

use crate::smodel_bucket::PropFacetPath;

/// Size of a pretess command: header, instance count, first cache index and
/// the first index in the destination buffer.
const STATIC_MODEL_PREDRAW_RECORD_BYTES: usize = 12;

/// Leaves per cache bank, as a shift of the zero-based cache index.
const CACHE_INDEX_BANK_SHIFT: u32 = 12;

/// Which command a plan asks for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StaticModelCmdVariety {
    Pretess,
    Cached,
    /// The models are left for the caller to handle on their own.
    Unchanged,
}

/// A pretess command and the index range it covers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PropPretessGrant {
    /// Where the copied indices start in the destination buffer.
    pub first_idx: u32,
    pub index_count: u32,
    pub command: [u8; STATIC_MODEL_PREDRAW_RECORD_BYTES],
}

/// What to do with a run of cache indices.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PropCmdReplay {
    /// The path the models take from here.
    pub write_target: PropFacetPath,
    /// How many of the leading indices the plan covers.
    pub models_taken: usize,
    pub cmd_kind: StaticModelCmdVariety,
    pub command_payload_size: usize,
    pub grant: Option<PropPretessGrant>,
}

/// Bytes of a cached command naming `count` cache indices: a four-byte header,
/// the count, then two bytes per index, rounded down to four.
fn cached_cmd_bytes(count: u32) -> usize {
    (count.saturating_mul(2).saturating_add(9) & !3) as usize
}

/// The bank a cache index lives in; `None` for the empty index.
fn bank_of(cache_slot_no: u16) -> Option<u16> {
    Some(cache_slot_no.checked_sub(1)? >> CACHE_INDEX_BANK_SHIFT)
}

/// How many indices at the front of the list share the first one's bank.
/// Zero when the list is empty or starts with the empty index.
pub fn same_bank_run(residency_slots: &[u16]) -> usize {
    let Some(bank) = residency_slots.first().and_then(|&first| bank_of(first)) else {
        return 0;
    };
    residency_slots
        .iter()
        .take_while(|&&index| bank_of(index) == Some(bank))
        .count()
}

/// The pretess command bytes: a zeroed header word, then the instance count,
/// the first cache index and the first destination index, little-endian.
fn pretess_cmd(instances: u16, cache_slot_no: u16, start: u32) -> [u8; STATIC_MODEL_PREDRAW_RECORD_BYTES] {
    let [n0, n1] = instances.to_le_bytes();
    let [c0, c1] = cache_slot_no.to_le_bytes();
    let [s0, s1, s2, s3] = start.to_le_bytes();
    [0, 0, 0, 0, n0, n1, c0, c1, s0, s1, s2, s3]
}

/// How many indices the runs hold together; `None` when there are none, or a
/// run is empty or is not made of whole triangles.
fn presplit_index_total(runs: &[&[u16]]) -> Option<u32> {
    let total = runs.iter().try_fold(0u32, |sum, run| {
        let whole_triangles = !run.is_empty() && run.len() % 3 == 0;
        let len = u32::try_from(run.len()).ok().filter(|_| whole_triangles)?;
        sum.checked_add(len)
    })?;
    (total != 0).then_some(total)
}

/// How full the command buffer and the destination index buffer are.
#[derive(Clone, Copy)]
struct Budget {
    bytes_taken: usize,
    byte_room: usize,
    indices_taken: u32,
    index_room: u32,
}

impl Budget {
    fn fits_cmd(self, bytes: usize) -> bool {
        self.bytes_taken.saturating_add(bytes) <= self.byte_room
    }

    /// A pretess command for `instances` models from `cache_slot_no`, with its
    /// indices placed right after the ones already in the destination.
    fn pretess(self, index_runs: &[&[u16]], slot_no: u16, instances: u16) -> Option<PropPretessGrant> {
        if !self.fits_cmd(STATIC_MODEL_PREDRAW_RECORD_BYTES) {
            return None;
        }
        let index_count = presplit_index_total(index_runs)?;
        let end = self.indices_taken.checked_add(index_count)?;
        (end <= self.index_room).then(|| PropPretessGrant {
            first_idx: self.indices_taken,
            index_count,
            command: pretess_cmd(instances, slot_no, self.indices_taken),
        })
    }
}

/// Copies the runs, one after another, into the range the allocation names.
/// `false` when the range does not fit in `write_target` or the runs do not fill it
/// exactly; `write_target` is left untouched then.
pub fn copy_presplit_indices(
    write_target: &mut [u16],
    alloc: PropPretessGrant,
    pieces: &[&[u16]],
) -> bool {
    let runs = pieces;
    let start = alloc.first_idx as usize;
    let end = start.saturating_add(alloc.index_count as usize);
    let Some(window) = write_target.get_mut(start..end) else {
        return false;
    };
    let filled = runs.iter().try_fold(0usize, |len, run| len.checked_add(run.len()));
    if filled != Some(window.len()) {
        return false;
    }
    let mut rest = window;
    for piece in runs {
        let (head, tail) = rest.split_at_mut(piece.len());
        head.copy_from_slice(piece);
        rest = tail;
    }
    true
}

impl PropCmdReplay {
    fn of(write_target: PropFacetPath, consumed: usize, cmd_kind: StaticModelCmdVariety, command_payload_size: usize) -> Self {
        Self { write_target, models_taken: consumed, cmd_kind, command_payload_size, grant: None }
    }

    /// The models go on along `write_target` with no command.
    fn untouched(write_target: PropFacetPath, consumed: usize) -> Self {
        Self::of(write_target, consumed, StaticModelCmdVariety::Unchanged, 0)
    }
}

/// Plans the command for the front of a run of cache indices.
///
/// Only models on the cached path get a command; any other path is passed
/// through untouched. The pretess form is tried first, when `pretess_on`
/// and the device is not lost; it needs room for its command (`bytes_taken` of
/// `byte_room` bytes taken) and for the runs' indices (`indices_taken` of
/// `index_room` taken). Otherwise the cached form is used. With no room for
/// either, nothing is consumed.
#[allow(clippy::too_many_arguments)]
pub fn plan_cached_prop_cmds(
    source: PropFacetPath,
    residency_slots: &[u16],
    pretess_on: bool,
    gpu_lost: bool,
    bytes_taken: usize,
    byte_room: usize,
    indices_taken: u32,
    index_room: u32,
    index_runs: &[&[u16]],
) -> PropCmdReplay {
    use PropFacetPath::{Cached, Pretess};
    if source != Cached {
        return PropCmdReplay::untouched(source, residency_slots.len());
    }
    let budget = Budget { bytes_taken, byte_room, indices_taken, index_room };
    let models = same_bank_run(residency_slots);
    let instances = u16::try_from(models).unwrap_or(u16::MAX);

    let try_pretess = models != 0 && pretess_on && !gpu_lost;
    if let Some(blocks) = try_pretess.then(|| budget.pretess(index_runs, residency_slots[0], instances)).flatten() {
        let plan = PropCmdReplay::of(Pretess, models, StaticModelCmdVariety::Pretess, STATIC_MODEL_PREDRAW_RECORD_BYTES);
        return PropCmdReplay { grant: Some(blocks), ..plan };
    }
    let bytes = cached_cmd_bytes(u32::from(instances));
    if models == 0 || !budget.fits_cmd(bytes) {
        return PropCmdReplay::untouched(Cached, 0);
    }
    PropCmdReplay::of(Cached, models, StaticModelCmdVariety::Cached, bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    const TRI: [u16; 3] = [0, 1, 2];
    const QUAD: [u16; 6] = [0, 1, 2, 2, 1, 3];

    /// One planner call: cached models, pretess off, room for 1000 command
    /// bytes and 100 indices, both empty. Tests change what they need.
    #[derive(Clone, Copy)]
    struct Ask<'a> {
        source: PropFacetPath,
        ids: &'a [u16],
        with_pretess: bool,
        lost: bool,
        budget: (usize, usize, u32, u32),
    }

    fn ask(ids: &[u16]) -> Ask<'_> {
        Ask { source: PropFacetPath::Cached, ids, with_pretess: false, lost: false, budget: (0, 1000, 0, 100) }
    }

    impl Ask<'_> {
        fn with_pretess(self) -> Self {
            Ask { with_pretess: true, ..self }
        }

        fn plan(self, runs: &[&[u16]]) -> PropCmdReplay {
            let (bytes_taken, byte_room, indices_taken, index_room) = self.budget;
            plan_cached_prop_cmds(self.source, self.ids, self.with_pretess, self.lost, bytes_taken, byte_room, indices_taken, index_room, runs)
        }

        fn kind(self, runs: &[&[u16]]) -> StaticModelCmdVariety {
            self.plan(runs).cmd_kind
        }
    }

    fn plan(picks: &[u16], on: bool, runs: &[&[u16]]) -> PropCmdReplay {
        Ask { with_pretess: on, ..ask(picks) }.plan(runs)
    }

    fn alloc_at(indices_taken: u32, runs: &[&[u16]]) -> PropPretessGrant {
        let budget = Budget { bytes_taken: 0, byte_room: 100, indices_taken, index_room: 100 };
        budget.pretess(runs, 1, 1).unwrap()
    }

    #[test]
    fn the_empty_index_ends_a_bank_run() {
        let lists: [&[u16]; 3] = [&[], &[0, 1, 2], &[1, 2, 0, 3]];
        assert_eq!(lists.map(same_bank_run), [0, 0, 2]);
    }

    #[test]
    fn a_run_stops_where_the_bank_changes() {
        let lists: [&[u16]; 5] = [
            &[1, 2, 3],
            &[0x1000, 0x1001],
            &[0x1001, 0x1002, 0x2000, 0x2001],
            &[0x2001, 0x1001],
            &[u16::MAX, u16::MAX - 1],
        ];
        assert_eq!(lists.map(same_bank_run), [3, 1, 3, 1, 2]);
    }

    #[test]
    fn a_pretess_command_names_the_count_the_index_and_the_range() {
        let cmd = pretess_cmd(3, 0x0102, 0x0a0b_0c0d);
        assert_eq!(cmd, [0, 0, 0, 0, 3, 0, 0x02, 0x01, 0x0d, 0x0c, 0x0b, 0x0a]);
    }

    #[test]
    fn a_cached_command_is_sized_for_its_indices() {
        assert_eq!([1, 2, 3, 4].map(cached_cmd_bytes), [8, 12, 12, 16]);
        assert_eq!(cached_cmd_bytes(u32::MAX), (u32::MAX & !3) as usize);
    }

    #[test]
    fn other_paths_pass_through_whole() {
        use PropFacetPath::{Pretess, Rigid, Skinned};
        for source in [Rigid, Skinned, Pretess] {
            let got = Ask { source, ..ask(&[1, 2, 3]).with_pretess() }.plan(&[&TRI]);
            assert_eq!(got, PropCmdReplay::untouched(source, 3));
        }
    }

    #[test]
    fn nothing_is_consumed_when_there_is_no_first_index() {
        let nothing = PropCmdReplay::untouched(PropFacetPath::Cached, 0);
        assert_eq!(plan(&[], true, &[]), nothing);
        assert_eq!(plan(&[0, 1], true, &[&TRI]), nothing);
    }

    #[test]
    fn a_run_with_room_becomes_one_pretess_command() {
        let got = plan(&[5, 6], true, &[&TRI, &QUAD]);
        assert_eq!((got.write_target, got.cmd_kind), (PropFacetPath::Pretess, StaticModelCmdVariety::Pretess));
        assert_eq!((got.models_taken, got.command_payload_size), (2, 12));
        let alloc = got.grant.unwrap();
        assert_eq!((alloc.first_idx, alloc.index_count), (0, 9));
        assert_eq!(alloc.command, pretess_cmd(2, 5, 0));
    }

    #[test]
    fn the_pretess_range_starts_where_the_last_one_ended() {
        let alloc = Ask { budget: (24, 1000, 30, 100), ..ask(&[7]).with_pretess() }.plan(&[&TRI]).grant.unwrap();
        assert_eq!(alloc.first_idx, 30);
        assert_eq!(alloc.command, pretess_cmd(1, 7, 30));
    }

    #[test]
    fn without_pretess_the_run_becomes_a_cached_command() {
        let got = plan(&[5, 6, 7], false, &[&TRI]);
        assert_eq!((got.write_target, got.cmd_kind), (PropFacetPath::Cached, StaticModelCmdVariety::Cached));
        assert_eq!((got.models_taken, got.command_payload_size), (3, 12));
        assert_eq!(got.grant, None);
    }

    #[test]
    fn a_lost_device_never_plans_pretess() {
        let lost = Ask { lost: true, ..ask(&[5]).with_pretess() };
        assert_eq!(lost.kind(&[&TRI]), StaticModelCmdVariety::Cached);
    }

    #[test]
    fn pretess_falls_back_to_cached_when_its_ranges_do_not_fit() {
        // No runs at all, an empty run, a run of a partial triangle, and a
        // destination that is too full.
        let one = ask(&[5]).with_pretess();
        let bad_runs: [&[&[u16]]; 3] = [&[], &[&[]], &[&[1, 2]]];
        for runs in bad_runs {
            assert_eq!(one.kind(runs), StaticModelCmdVariety::Cached);
        }
        let filled = |indices_taken: u32, index_room: u32| Ask { budget: (0, 1000, indices_taken, index_room), ..one };
        assert_eq!(filled(95, 100).kind(&[&QUAD]), StaticModelCmdVariety::Cached);
        assert_eq!(filled(94, 100).kind(&[&QUAD]), StaticModelCmdVariety::Pretess);
        assert_eq!(filled(u32::MAX, u32::MAX).kind(&[&TRI]), StaticModelCmdVariety::Cached);
    }

    #[test]
    fn pretess_falls_back_when_the_command_buffer_is_nearly_full() {
        let capped = |byte_room: usize| Ask { budget: (0, byte_room, 0, 100), ..ask(&[5]).with_pretess() };
        assert_eq!(capped(11).kind(&[&TRI]), StaticModelCmdVariety::Cached);
        assert_eq!(capped(12).kind(&[&TRI]), StaticModelCmdVariety::Pretess);
    }

    #[test]
    fn nothing_is_consumed_when_neither_command_fits() {
        let capped = |byte_room: usize| Ask { budget: (0, byte_room, 0, 100), ..ask(&[5]) }.plan(&[]);
        assert_eq!(capped(7), PropCmdReplay::untouched(PropFacetPath::Cached, 0));
        assert_eq!(capped(8).cmd_kind, StaticModelCmdVariety::Cached);
    }

    #[test]
    fn a_plan_covers_only_the_models_of_the_first_bank() {
        assert_eq!(plan(&[0x0fff, 0x1000, 0x1001], false, &[]).models_taken, 2);
    }

    #[test]
    fn copying_lays_the_runs_end_to_end() {
        let runs: &[&[u16]] = &[&TRI, &QUAD];
        let mut write_target = [9u16; 14];
        assert!(copy_presplit_indices(&mut write_target, alloc_at(2, runs), runs));
        assert_eq!(write_target, [9, 9, 0, 1, 2, 0, 1, 2, 2, 1, 3, 9, 9, 9]);
    }

    #[test]
    fn copying_fails_when_the_range_or_the_runs_do_not_match() {
        let alloc = alloc_at(4, &[&QUAD]);
        let copies = |len: usize, runs: &[&[u16]]| copy_presplit_indices(&mut vec![0; len], alloc, runs);
        assert!(!copies(9, &[&QUAD]));
        assert!(!copies(10, &[&TRI]));
        assert!(!copies(10, &[&QUAD, &TRI]));
        assert!(copies(10, &[&TRI, &TRI]));
    }
}
