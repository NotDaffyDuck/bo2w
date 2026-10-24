//! Sorting static-model surfaces into draw buckets.
//!
//! While a frame is assembled, every visible static-model surface lands in one
//! of sixteen short lists: four kinds for each of four levels of detail. A
//! bucket number is `lod * 4 + kind`. The lists are drained later, one draw
//! group per list.

/// Buckets per level of detail.
pub const SMODEL_BUCKET_STRIDE: i32 = 4;
/// Kind: drawn rigid, with one transform for the whole model.
pub const SMODEL_BUCKET_RIGID: i32 = 0;
/// Kind: drawn skinned, vertex by vertex.
pub const SMODEL_BUCKET_SKINNED: i32 = 1;
/// Kind: drawn from the static model cache.
pub const SMODEL_BUCKET_CACHED: i32 = 2;
/// Kind: the cached surface was re-tessellated into the frame's own buffer.
pub const SMODEL_BUCKET_PRETESS: i32 = 3;

/// How many lists there are.
pub const SMODEL_BUCKET_LIST_N: usize = 16;
/// How many entries one list holds before it must be flushed.
pub const SMODEL_BUCKET_CAP: usize = 0x80;

/// How a static-model surface is drawn.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SmodelSurfPath {
    Rigid,
    Skinned,
    Cached,
    Pretess,
}

/// The surface-type number the lit-technique tables use for a draw path.
///
/// BO2's own table has only rigid and skinned static models; the cached and
/// pre-tessellated kinds borrow the numbers the lit-technique rows were
/// authored against.
#[must_use]
pub const fn smodel_surf_type(path: SmodelSurfPath) -> u8 {
    match path {
        SmodelSurfPath::Rigid => 2,
        SmodelSurfPath::Pretess => 3,
        SmodelSurfPath::Cached => 4,
        SmodelSurfPath::Skinned => 5,
    }
}

/// The path a bucket's surfaces start out on. The pre-tessellated kind is only
/// ever a destination, so it has no source.
fn bucket_source_path(bucket: usize) -> Option<SmodelSurfPath> {
    match bucket % SMODEL_BUCKET_STRIDE as usize {
        0 => Some(SmodelSurfPath::Rigid),
        1 => Some(SmodelSurfPath::Skinned),
        2 => Some(SmodelSurfPath::Cached),
        _ => None,
    }
}

/// Where a surface ends up: a cached surface that was re-tessellated moves to
/// the pre-tessellated path, anything else stays where it was.
#[must_use]
pub fn smodel_dest_path(source: SmodelSurfPath, cached_pretess_succeeded: bool) -> SmodelSurfPath {
    match source {
        SmodelSurfPath::Cached if cached_pretess_succeeded => SmodelSurfPath::Pretess,
        other => other,
    }
}

/// A level of detail is rigid when none of its surfaces carry skin weights.
#[must_use]
pub fn smodel_lod_is_rigid(skin_flags: &[u8]) -> bool {
    skin_flags.iter().all(|&flag| flag == 0)
}

/// The bucket for one surface of one level of detail.
///
/// It goes to the cache bucket when the cache is on, the instance has lighting,
/// the level of detail may be cached and the surface already holds a cache
/// index. Otherwise it is skinned or rigid depending on the model.
#[must_use]
pub fn add_static_model_surf_to_bucket(
    lod: i32,
    smc_enable: bool,
    draw_inst_lighting_nonzero: bool,
    lod_smc_flag: u8,
    cache_index: u16,
    lod_is_rigid: bool,
) -> i32 {
    let kind = if smc_enable && draw_inst_lighting_nonzero && lod_smc_flag != 0 && cache_index != 0
    {
        SMODEL_BUCKET_CACHED
    } else if lod_is_rigid {
        SMODEL_BUCKET_RIGID
    } else {
        SMODEL_BUCKET_SKINNED
    };
    lod * SMODEL_BUCKET_STRIDE + kind
}

/// The sixteen bucket lists and how full each one is.
#[derive(Clone, Copy, Debug)]
pub struct SmodelSurfBucketLists {
    pub ids: [[u16; SMODEL_BUCKET_CAP]; SMODEL_BUCKET_LIST_N],
    pub count: [i32; SMODEL_BUCKET_LIST_N],
}

impl Default for SmodelSurfBucketLists {
    fn default() -> Self {
        Self {
            ids: [[0; SMODEL_BUCKET_CAP]; SMODEL_BUCKET_LIST_N],
            count: [0; SMODEL_BUCKET_LIST_N],
        }
    }
}

/// What a push did to its list.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SmodelBucketPush {
    /// Stored; there is still room.
    Stored,
    /// Stored, and the list is now full and should be flushed.
    Full,
}

/// What a bucket's list holds: the cache index for the cache kind (the cache
/// owns the vertices), the static model's own index for every other kind.
#[must_use]
pub fn smodel_bucket_store_payload(bucket: i32, smodel_index: u16, cache_index: u16) -> u16 {
    if bucket & (SMODEL_BUCKET_STRIDE - 1) == SMODEL_BUCKET_CACHED {
        cache_index
    } else {
        smodel_index
    }
}

/// Appends to a bucket's list. `None` when the bucket does not exist or its
/// list was already full.
#[must_use]
pub fn smodel_surf_bucket_push(
    lists: &mut SmodelSurfBucketLists,
    bucket: i32,
    payload: u16,
) -> Option<SmodelBucketPush> {
    let bucket = usize::try_from(bucket)
        .ok()
        .filter(|&b| b < SMODEL_BUCKET_LIST_N)?;
    let used = usize::try_from(lists.count[bucket])
        .ok()
        .filter(|&n| n < SMODEL_BUCKET_CAP)?;
    lists.ids[bucket][used] = payload;
    lists.count[bucket] += 1;
    let full = used + 1 == SMODEL_BUCKET_CAP;
    Some(if full {
        SmodelBucketPush::Full
    } else {
        SmodelBucketPush::Stored
    })
}

/// One bucket's worth of surfaces, copied out of the lists.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SmodelConsumedBucket {
    pub bucket: u8,
    pub lod: u8,
    pub source: SmodelSurfPath,
    pub count: u16,
    pub ids: [u16; SMODEL_BUCKET_CAP],
}

/// A bucket's bit in an active-bucket mask; bucket 0 is the top bit.
#[must_use]
pub const fn smodel_bucket_mask(bucket: u8) -> Option<u32> {
    if (bucket as usize) < SMODEL_BUCKET_LIST_N {
        Some(0x8000_0000 >> bucket)
    } else {
        None
    }
}

/// Drains the lists named by `active_mask` into `out`, in bucket order, and
/// returns how many were written. Empty lists and the pre-tessellated kind are
/// skipped; each drained list is reset.
pub fn smodel_bucket_lists_consume(
    lists: &mut SmodelSurfBucketLists,
    active_mask: u32,
    out: &mut [Option<SmodelConsumedBucket>; SMODEL_BUCKET_LIST_N],
) -> usize {
    out.fill(None);
    let mut written = 0;
    for bucket in 0..SMODEL_BUCKET_LIST_N {
        if active_mask & (0x8000_0000 >> bucket) == 0 {
            continue;
        }
        let Some(source) = bucket_source_path(bucket) else {
            continue;
        };
        let Some(count) = usize::try_from(lists.count[bucket])
            .ok()
            .filter(|n| (1..=SMODEL_BUCKET_CAP).contains(n))
        else {
            continue;
        };
        let mut ids = [0u16; SMODEL_BUCKET_CAP];
        ids[..count].copy_from_slice(&lists.ids[bucket][..count]);
        out[written] = Some(SmodelConsumedBucket {
            bucket: bucket as u8,
            lod: (bucket / SMODEL_BUCKET_STRIDE as usize) as u8,
            source,
            count: count as u16,
            ids,
        });
        written += 1;
        lists.count[bucket] = 0;
    }
    written
}

/// Whether a surface's material casts into the sun shadow: either of the top
/// two bits of its shadow byte.
#[must_use]
pub const fn smodel_surf_sun_shadow_emits(material_shadow_byte: u8) -> bool {
    material_shadow_byte & 0xc0 != 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_path_has_its_own_surface_type() {
        assert_eq!(smodel_surf_type(SmodelSurfPath::Rigid), 2);
        assert_eq!(smodel_surf_type(SmodelSurfPath::Pretess), 3);
        assert_eq!(smodel_surf_type(SmodelSurfPath::Cached), 4);
        assert_eq!(smodel_surf_type(SmodelSurfPath::Skinned), 5);
    }

    #[test]
    fn only_a_retessellated_cached_surface_changes_path() {
        use SmodelSurfPath::*;
        assert_eq!(smodel_dest_path(Cached, true), Pretess);
        assert_eq!(smodel_dest_path(Cached, false), Cached);
        for path in [Rigid, Skinned, Pretess] {
            assert_eq!(smodel_dest_path(path, true), path);
            assert_eq!(smodel_dest_path(path, false), path);
        }
    }

    #[test]
    fn a_lod_is_rigid_only_when_no_surface_is_skinned() {
        assert!(smodel_lod_is_rigid(&[]));
        assert!(smodel_lod_is_rigid(&[0, 0, 0]));
        assert!(!smodel_lod_is_rigid(&[0, 1, 0]));
    }

    #[test]
    fn a_cached_surface_needs_every_condition() {
        let bucket = |enable, lit, flag, index, rigid| {
            add_static_model_surf_to_bucket(2, enable, lit, flag, index, rigid)
        };
        assert_eq!(bucket(true, true, 1, 9, true), 2 * 4 + SMODEL_BUCKET_CACHED);
        assert_eq!(bucket(false, true, 1, 9, true), 2 * 4 + SMODEL_BUCKET_RIGID);
        assert_eq!(bucket(true, false, 1, 9, true), 2 * 4 + SMODEL_BUCKET_RIGID);
        assert_eq!(bucket(true, true, 0, 9, true), 2 * 4 + SMODEL_BUCKET_RIGID);
        assert_eq!(bucket(true, true, 1, 0, true), 2 * 4 + SMODEL_BUCKET_RIGID);
    }

    #[test]
    fn an_uncached_surface_is_skinned_unless_the_lod_is_rigid() {
        assert_eq!(
            add_static_model_surf_to_bucket(1, false, false, 0, 0, false),
            4 + SMODEL_BUCKET_SKINNED
        );
        assert_eq!(
            add_static_model_surf_to_bucket(3, false, false, 0, 0, true),
            12 + SMODEL_BUCKET_RIGID
        );
    }

    #[test]
    fn the_cache_bucket_stores_the_cache_index_and_the_rest_the_model() {
        assert_eq!(smodel_bucket_store_payload(2, 70, 900), 900);
        assert_eq!(smodel_bucket_store_payload(6, 70, 900), 900);
        assert_eq!(smodel_bucket_store_payload(0, 70, 900), 70);
        assert_eq!(smodel_bucket_store_payload(1, 70, 900), 70);
        assert_eq!(smodel_bucket_store_payload(3, 70, 900), 70);
    }

    #[test]
    fn pushing_fills_a_list_until_it_reports_full() {
        let mut lists = SmodelSurfBucketLists::default();
        for i in 0..SMODEL_BUCKET_CAP - 1 {
            assert_eq!(
                smodel_surf_bucket_push(&mut lists, 5, i as u16),
                Some(SmodelBucketPush::Stored)
            );
        }
        assert_eq!(
            smodel_surf_bucket_push(&mut lists, 5, 999),
            Some(SmodelBucketPush::Full)
        );
        assert_eq!(lists.count[5], SMODEL_BUCKET_CAP as i32);
        assert_eq!(lists.ids[5][SMODEL_BUCKET_CAP - 1], 999);
        assert_eq!(smodel_surf_bucket_push(&mut lists, 5, 1), None);
        assert_eq!(lists.count[5], SMODEL_BUCKET_CAP as i32);
    }

    #[test]
    fn a_bucket_that_does_not_exist_takes_nothing() {
        let mut lists = SmodelSurfBucketLists::default();
        assert_eq!(smodel_surf_bucket_push(&mut lists, -1, 1), None);
        assert_eq!(smodel_surf_bucket_push(&mut lists, 16, 1), None);
        assert_eq!(lists.count, [0; SMODEL_BUCKET_LIST_N]);
    }

    #[test]
    fn bucket_masks_run_from_the_top_bit() {
        assert_eq!(smodel_bucket_mask(0), Some(0x8000_0000));
        assert_eq!(smodel_bucket_mask(15), Some(0x0001_0000));
        assert_eq!(smodel_bucket_mask(16), None);
    }

    #[test]
    fn consuming_copies_active_lists_in_order_and_resets_them() {
        let mut lists = SmodelSurfBucketLists::default();
        smodel_surf_bucket_push(&mut lists, 9, 41).unwrap();
        smodel_surf_bucket_push(&mut lists, 9, 42).unwrap();
        smodel_surf_bucket_push(&mut lists, 2, 7).unwrap();
        let mask = smodel_bucket_mask(2).unwrap() | smodel_bucket_mask(9).unwrap();
        let mut out = [None; SMODEL_BUCKET_LIST_N];
        assert_eq!(smodel_bucket_lists_consume(&mut lists, mask, &mut out), 2);

        let first = out[0].unwrap();
        assert_eq!((first.bucket, first.lod, first.count), (2, 0, 1));
        assert_eq!(first.source, SmodelSurfPath::Cached);
        assert_eq!(first.ids[0], 7);

        let second = out[1].unwrap();
        assert_eq!((second.bucket, second.lod, second.count), (9, 2, 2));
        assert_eq!(second.source, SmodelSurfPath::Skinned);
        assert_eq!(&second.ids[..2], &[41, 42]);

        assert!(out[2..].iter().all(Option::is_none));
        assert_eq!(lists.count[2], 0);
        assert_eq!(lists.count[9], 0);
    }

    #[test]
    fn consuming_leaves_inactive_and_pretess_lists_alone() {
        let mut lists = SmodelSurfBucketLists::default();
        smodel_surf_bucket_push(&mut lists, 3, 1).unwrap();
        smodel_surf_bucket_push(&mut lists, 4, 2).unwrap();
        let mut out = [None; SMODEL_BUCKET_LIST_N];
        let everything = u32::from(u16::MAX) << 16;
        assert_eq!(
            smodel_bucket_lists_consume(
                &mut lists,
                everything & !smodel_bucket_mask(4).unwrap(),
                &mut out
            ),
            0
        );
        assert_eq!(lists.count[3], 1);
        assert_eq!(lists.count[4], 1);
        assert_eq!(
            smodel_bucket_lists_consume(&mut lists, everything, &mut out),
            1
        );
        assert_eq!(out[0].unwrap().bucket, 4);
        assert_eq!(lists.count[3], 1);
    }

    #[test]
    fn consuming_clears_what_the_output_held_before() {
        let mut lists = SmodelSurfBucketLists::default();
        let mut out = [None; SMODEL_BUCKET_LIST_N];
        smodel_surf_bucket_push(&mut lists, 0, 5).unwrap();
        smodel_bucket_lists_consume(&mut lists, 0xffff_0000, &mut out);
        assert!(out[0].is_some());
        assert_eq!(
            smodel_bucket_lists_consume(&mut lists, 0xffff_0000, &mut out),
            0
        );
        assert!(out.iter().all(Option::is_none));
    }

    #[test]
    fn the_sun_shadow_needs_one_of_the_top_two_bits() {
        assert!(smodel_surf_sun_shadow_emits(0x40));
        assert!(smodel_surf_sun_shadow_emits(0x80));
        assert!(smodel_surf_sun_shadow_emits(0xc0));
        assert!(!smodel_surf_sun_shadow_emits(0x3f));
        assert!(!smodel_surf_sun_shadow_emits(0));
    }
}
