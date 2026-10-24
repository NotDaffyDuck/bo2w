//! Deciding how a run of cached static models reaches the screen.
//!
//! The cached models of one bucket either become one "pretess" command that
//! points at an index range copied for them, or one "cached" command listing
//! the cache indices. A run that spans more than one cache bank is cut at the
//! bank edge, because a command addresses one bank only.

use crate::smodel_bucket::SmodelSurfPath;

/// Size of a pretess command: header, instance count, first cache index and
/// the first index in the destination buffer.
const SMODEL_PRETESS_CMD_BYTES: usize = 12;

/// Leaves per cache bank, as a shift of the zero-based cache index.
const CACHE_INDEX_BANK_SHIFT: u32 = 12;

/// Which command a plan asks for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SmodelCmdKind {
    Pretess,
    Cached,
    /// The models are left for the caller to handle on their own.
    Unchanged,
}

/// A pretess command and the index range it covers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SmodelPretessAlloc {
    /// Where the copied indices start in the destination buffer.
    pub first_index: u32,
    pub index_count: u32,
    pub cmd: [u8; SMODEL_PRETESS_CMD_BYTES],
}

/// What to do with a run of cache indices.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SmodelCachedCmdPlan {
    /// The path the models take from here.
    pub dest: SmodelSurfPath,
    /// How many of the leading indices the plan covers.
    pub consumed: usize,
    pub cmd_kind: SmodelCmdKind,
    pub cmd_bytes: usize,
    pub pretess: Option<SmodelPretessAlloc>,
}

/// Bytes of a cached command naming `count` cache indices: a four-byte header,
/// the count, then two bytes per index, rounded down to four.
fn cached_cmd_bytes(count: u32) -> usize {
    (count.saturating_mul(2).saturating_add(9) & !3) as usize
}

/// The bank a cache index lives in; `None` for the empty index.
fn bank_of(cache_index: u16) -> Option<u16> {
    Some(cache_index.checked_sub(1)? >> CACHE_INDEX_BANK_SHIFT)
}

/// How many indices at the front of the list share the first one's bank.
/// Zero when the list is empty or starts with the empty index.
pub fn smodel_same_bank_count(cache_indices: &[u16]) -> usize {
    let Some(bank) = cache_indices.first().and_then(|&first| bank_of(first)) else {
        return 0;
    };
    cache_indices
        .iter()
        .take_while(|&&index| bank_of(index) == Some(bank))
        .count()
}

fn pretess_cmd(
    instances: u16,
    first_cache_index: u16,
    first_index: u32,
) -> [u8; SMODEL_PRETESS_CMD_BYTES] {
    let mut cmd = [0; SMODEL_PRETESS_CMD_BYTES];
    cmd[4..6].copy_from_slice(&instances.to_le_bytes());
    cmd[6..8].copy_from_slice(&first_cache_index.to_le_bytes());
    cmd[8..].copy_from_slice(&first_index.to_le_bytes());
    cmd
}

/// How many indices the runs hold together; `None` when there are none, or a
/// run is empty or is not made of whole triangles.
fn pretess_index_count(runs: &[&[u16]]) -> Option<u32> {
    let mut total = 0u32;
    for run in runs {
        if run.is_empty() || run.len() % 3 != 0 {
            return None;
        }
        total = total.checked_add(u32::try_from(run.len()).ok()?)?;
    }
    (total > 0).then_some(total)
}

/// Room for a pretess command plus the runs' indices, if there is any.
fn reserve_pretess(
    cmd_used: usize,
    cmd_cap: usize,
    dest_used: u32,
    dest_total: u32,
    runs: &[&[u16]],
    first_cache_index: u16,
    instances: u16,
) -> Option<SmodelPretessAlloc> {
    if cmd_used.saturating_add(SMODEL_PRETESS_CMD_BYTES) > cmd_cap {
        return None;
    }
    let index_count = pretess_index_count(runs)?;
    if dest_used.checked_add(index_count)? > dest_total {
        return None;
    }
    Some(SmodelPretessAlloc {
        first_index: dest_used,
        index_count,
        cmd: pretess_cmd(instances, first_cache_index, dest_used),
    })
}

/// Copies the runs, one after another, into the range the allocation names.
/// `false` when the range does not fit in `dest` or the runs do not fill it
/// exactly; `dest` may be partly written then.
pub fn smodel_pretess_indices_copy(
    dest: &mut [u16],
    alloc: SmodelPretessAlloc,
    runs: &[&[u16]],
) -> bool {
    let first = alloc.first_index as usize;
    let Some(range) = dest.get_mut(first..first.saturating_add(alloc.index_count as usize)) else {
        return false;
    };
    let mut rest = range;
    for run in runs {
        let Some((head, tail)) = rest.split_at_mut_checked(run.len()) else {
            return false;
        };
        head.copy_from_slice(run);
        rest = tail;
    }
    rest.is_empty()
}

fn unchanged(dest: SmodelSurfPath, consumed: usize) -> SmodelCachedCmdPlan {
    SmodelCachedCmdPlan {
        dest,
        consumed,
        cmd_kind: SmodelCmdKind::Unchanged,
        cmd_bytes: 0,
        pretess: None,
    }
}

/// Plans the command for the front of a run of cache indices.
///
/// Only models on the cached path get a command; any other path is passed
/// through untouched. The pretess form is tried first, when `pretess_enabled`
/// and the device is not lost; it needs room for its command (`cmd_used` of
/// `cmd_cap` bytes taken) and for the runs' indices (`dest_used` of
/// `dest_total` taken). Otherwise the cached form is used. With no room for
/// either, nothing is consumed.
#[allow(clippy::too_many_arguments)]
pub fn smodel_cached_cmd_plan(
    source: SmodelSurfPath,
    cache_indices: &[u16],
    pretess_enabled: bool,
    device_lost: bool,
    cmd_used: usize,
    cmd_cap: usize,
    dest_used: u32,
    dest_total: u32,
    runs: &[&[u16]],
) -> SmodelCachedCmdPlan {
    if source != SmodelSurfPath::Cached {
        return unchanged(source, cache_indices.len());
    }
    let consumed = smodel_same_bank_count(cache_indices);
    if consumed == 0 {
        return unchanged(SmodelSurfPath::Cached, 0);
    }
    let instances = u16::try_from(consumed).unwrap_or(u16::MAX);
    if pretess_enabled && !device_lost {
        let alloc = reserve_pretess(
            cmd_used,
            cmd_cap,
            dest_used,
            dest_total,
            runs,
            cache_indices[0],
            instances,
        );
        if let Some(alloc) = alloc {
            return SmodelCachedCmdPlan {
                dest: SmodelSurfPath::Pretess,
                consumed,
                cmd_kind: SmodelCmdKind::Pretess,
                cmd_bytes: SMODEL_PRETESS_CMD_BYTES,
                pretess: Some(alloc),
            };
        }
    }
    let cmd_bytes = cached_cmd_bytes(u32::from(instances));
    if cmd_used.saturating_add(cmd_bytes) > cmd_cap {
        return unchanged(SmodelSurfPath::Cached, 0);
    }
    SmodelCachedCmdPlan {
        dest: SmodelSurfPath::Cached,
        consumed,
        cmd_kind: SmodelCmdKind::Cached,
        cmd_bytes,
        pretess: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TRI: [u16; 3] = [0, 1, 2];
    const QUAD: [u16; 6] = [0, 1, 2, 2, 1, 3];

    fn plan(indices: &[u16], enabled: bool, runs: &[&[u16]]) -> SmodelCachedCmdPlan {
        smodel_cached_cmd_plan(
            SmodelSurfPath::Cached,
            indices,
            enabled,
            false,
            0,
            1000,
            0,
            100,
            runs,
        )
    }

    #[test]
    fn the_empty_index_ends_a_bank_run() {
        assert_eq!(smodel_same_bank_count(&[]), 0);
        assert_eq!(smodel_same_bank_count(&[0, 1, 2]), 0);
        assert_eq!(smodel_same_bank_count(&[1, 2, 0, 3]), 2);
    }

    #[test]
    fn a_run_stops_where_the_bank_changes() {
        assert_eq!(smodel_same_bank_count(&[1, 2, 3]), 3);
        assert_eq!(smodel_same_bank_count(&[0x1000, 0x1001]), 1);
        assert_eq!(smodel_same_bank_count(&[0x1001, 0x1002, 0x2000, 0x2001]), 3);
        assert_eq!(smodel_same_bank_count(&[0x2001, 0x1001]), 1);
        assert_eq!(smodel_same_bank_count(&[u16::MAX, u16::MAX - 1]), 2);
    }

    #[test]
    fn a_pretess_command_names_the_count_the_index_and_the_range() {
        let cmd = pretess_cmd(3, 0x0102, 0x0a0b_0c0d);
        assert_eq!(cmd, [0, 0, 0, 0, 3, 0, 0x02, 0x01, 0x0d, 0x0c, 0x0b, 0x0a]);
    }

    #[test]
    fn a_cached_command_is_sized_for_its_indices() {
        assert_eq!(cached_cmd_bytes(1), 8);
        assert_eq!(cached_cmd_bytes(2), 12);
        assert_eq!(cached_cmd_bytes(3), 12);
        assert_eq!(cached_cmd_bytes(4), 16);
        assert_eq!(cached_cmd_bytes(u32::MAX), (u32::MAX & !3) as usize);
    }

    #[test]
    fn other_paths_pass_through_whole() {
        for path in [
            SmodelSurfPath::Rigid,
            SmodelSurfPath::Skinned,
            SmodelSurfPath::Pretess,
        ] {
            let got =
                smodel_cached_cmd_plan(path, &[1, 2, 3], true, false, 0, 1000, 0, 100, &[&TRI]);
            assert_eq!(got, unchanged(path, 3));
        }
    }

    #[test]
    fn nothing_is_consumed_when_there_is_no_first_index() {
        assert_eq!(plan(&[], true, &[]), unchanged(SmodelSurfPath::Cached, 0));
        assert_eq!(
            plan(&[0, 1], true, &[&TRI]),
            unchanged(SmodelSurfPath::Cached, 0)
        );
    }

    #[test]
    fn a_run_with_room_becomes_one_pretess_command() {
        let got = plan(&[5, 6], true, &[&TRI, &QUAD]);
        assert_eq!(got.dest, SmodelSurfPath::Pretess);
        assert_eq!(got.cmd_kind, SmodelCmdKind::Pretess);
        assert_eq!(got.consumed, 2);
        assert_eq!(got.cmd_bytes, 12);
        let alloc = got.pretess.unwrap();
        assert_eq!((alloc.first_index, alloc.index_count), (0, 9));
        assert_eq!(alloc.cmd, pretess_cmd(2, 5, 0));
    }

    #[test]
    fn the_pretess_range_starts_where_the_last_one_ended() {
        let got = smodel_cached_cmd_plan(
            SmodelSurfPath::Cached,
            &[7],
            true,
            false,
            24,
            1000,
            30,
            100,
            &[&TRI],
        );
        let alloc = got.pretess.unwrap();
        assert_eq!(alloc.first_index, 30);
        assert_eq!(alloc.cmd, pretess_cmd(1, 7, 30));
    }

    #[test]
    fn without_pretess_the_run_becomes_a_cached_command() {
        let got = plan(&[5, 6, 7], false, &[&TRI]);
        assert_eq!(got.dest, SmodelSurfPath::Cached);
        assert_eq!(got.cmd_kind, SmodelCmdKind::Cached);
        assert_eq!((got.consumed, got.cmd_bytes), (3, 12));
        assert_eq!(got.pretess, None);
    }

    #[test]
    fn a_lost_device_never_plans_pretess() {
        let got = smodel_cached_cmd_plan(
            SmodelSurfPath::Cached,
            &[5],
            true,
            true,
            0,
            1000,
            0,
            100,
            &[&TRI],
        );
        assert_eq!(got.cmd_kind, SmodelCmdKind::Cached);
    }

    #[test]
    fn pretess_falls_back_to_cached_when_its_ranges_do_not_fit() {
        // No runs at all, an empty run, a run of a partial triangle, and a
        // destination that is too full.
        let empty: &[u16] = &[];
        let partial: &[u16] = &[1, 2];
        let none: &[&[u16]] = &[];
        let one_empty: &[&[u16]] = &[empty];
        let one_partial: &[&[u16]] = &[partial];
        for runs in [none, one_empty, one_partial] {
            assert_eq!(plan(&[5], true, runs).cmd_kind, SmodelCmdKind::Cached);
        }
        let full = smodel_cached_cmd_plan(
            SmodelSurfPath::Cached,
            &[5],
            true,
            false,
            0,
            1000,
            95,
            100,
            &[&QUAD],
        );
        assert_eq!(full.cmd_kind, SmodelCmdKind::Cached);
        let exact = smodel_cached_cmd_plan(
            SmodelSurfPath::Cached,
            &[5],
            true,
            false,
            0,
            1000,
            94,
            100,
            &[&QUAD],
        );
        assert_eq!(exact.cmd_kind, SmodelCmdKind::Pretess);
        let wrapped = smodel_cached_cmd_plan(
            SmodelSurfPath::Cached,
            &[5],
            true,
            false,
            0,
            1000,
            u32::MAX,
            u32::MAX,
            &[&TRI],
        );
        assert_eq!(wrapped.cmd_kind, SmodelCmdKind::Cached);
    }

    #[test]
    fn pretess_falls_back_when_the_command_buffer_is_nearly_full() {
        let tight = smodel_cached_cmd_plan(
            SmodelSurfPath::Cached,
            &[5],
            true,
            false,
            0,
            11,
            0,
            100,
            &[&TRI],
        );
        assert_eq!(tight.cmd_kind, SmodelCmdKind::Cached);
        let fits = smodel_cached_cmd_plan(
            SmodelSurfPath::Cached,
            &[5],
            true,
            false,
            0,
            12,
            0,
            100,
            &[&TRI],
        );
        assert_eq!(fits.cmd_kind, SmodelCmdKind::Pretess);
    }

    #[test]
    fn nothing_is_consumed_when_neither_command_fits() {
        let got = smodel_cached_cmd_plan(
            SmodelSurfPath::Cached,
            &[5],
            false,
            false,
            0,
            7,
            0,
            100,
            &[],
        );
        assert_eq!(got, unchanged(SmodelSurfPath::Cached, 0));
        let got = smodel_cached_cmd_plan(
            SmodelSurfPath::Cached,
            &[5],
            false,
            false,
            0,
            8,
            0,
            100,
            &[],
        );
        assert_eq!(got.cmd_kind, SmodelCmdKind::Cached);
    }

    #[test]
    fn a_plan_covers_only_the_models_of_the_first_bank() {
        let got = plan(&[0x0fff, 0x1000, 0x1001], false, &[]);
        assert_eq!(got.consumed, 2);
    }

    #[test]
    fn copying_lays_the_runs_end_to_end() {
        let alloc = reserve_pretess(0, 100, 2, 100, &[&TRI, &QUAD], 1, 1).unwrap();
        let mut dest = [9u16; 14];
        assert!(smodel_pretess_indices_copy(
            &mut dest,
            alloc,
            &[&TRI, &QUAD]
        ));
        assert_eq!(dest, [9, 9, 0, 1, 2, 0, 1, 2, 2, 1, 3, 9, 9, 9]);
    }

    #[test]
    fn copying_fails_when_the_range_or_the_runs_do_not_match() {
        let alloc = reserve_pretess(0, 100, 4, 100, &[&QUAD], 1, 1).unwrap();
        assert!(!smodel_pretess_indices_copy(&mut [0; 9], alloc, &[&QUAD]));
        assert!(!smodel_pretess_indices_copy(&mut [0; 10], alloc, &[&TRI]));
        assert!(!smodel_pretess_indices_copy(
            &mut [0; 10],
            alloc,
            &[&QUAD, &TRI]
        ));
        assert!(smodel_pretess_indices_copy(
            &mut [0; 10],
            alloc,
            &[&TRI, &TRI]
        ));
    }
}
