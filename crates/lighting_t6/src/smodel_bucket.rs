//! Sorting static-model surfaces into draw buckets.
//!
//! While a frame is assembled, every visible static-model surface lands in one
//! of sixteen short lists: four kinds for each of four levels of detail. A
//! bucket number is `lod * 4 + kind`. The lists are drained later, one draw
//! group per list.

/// Buckets per level of detail.
pub const STATIC_BUCKET_KIND_SPAN: i32 = 4;
/// Kind: drawn rigid, with one transform for the whole model.
pub const STATIC_BUCKET_KIND_FIXED_MESH: i32 = 0;
/// Kind: drawn skinned, vertex by vertex.
pub const STATIC_BUCKET_KIND_BONE_DRIVEN: i32 = 1;
/// Kind: drawn from the static model cache.
pub const STATIC_BUCKET_KIND_CACHED: i32 = 2;
/// Kind: the cached surface was re-tessellated into the frame's own buffer.
pub const STATIC_BUCKET_KIND_PRESPLIT: i32 = 3;

/// How many lists there are.
pub const PROP_BUCKET_LIST_COUNT: usize = 16;
/// How many entries one list holds before it must be flushed.
pub const STATIC_MODEL_BUCKET_CAPACITY: usize = 0x80;

/// How a static-model surface is drawn.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PropFacetPath {
    Rigid,
    Skinned,
    Cached,
    Pretess,
}

/// The paths in bucket-kind order: kind `n` of a level of detail is `KIND_PATHS[n]`.
const KIND_PATHS: [PropFacetPath; 4] =
    [PropFacetPath::Rigid, PropFacetPath::Skinned, PropFacetPath::Cached, PropFacetPath::Pretess];

/// The top bit of an active-bucket mask: bucket 0.
const FIRST_BUCKET_BIT: u32 = 1 << 31;

/// The surface-type number the lit-technique tables use for a draw path.
///
/// BO2's own table has only rigid and skinned static models; the cached and
/// pre-tessellated kinds borrow the numbers the lit-technique rows were
/// authored against.
#[must_use]
pub const fn prop_facet_kind(path: PropFacetPath) -> u8 {
    // Indexed by kind: rigid, skinned, cached, pretess.
    const SURF_TYPES: [u8; 4] = [2, 5, 4, 3];
    SURF_TYPES[path as usize]
}

/// The path a bucket's surfaces start out on. The pre-tessellated kind is only
/// ever a destination, so it has no source.
fn bucket_source_path(bucket: usize) -> Option<PropFacetPath> {
    let path = KIND_PATHS[bucket % KIND_PATHS.len()];
    (path != PropFacetPath::Pretess).then_some(path)
}

/// Where a surface ends up: a cached surface that was re-tessellated moves to
/// the pre-tessellated path, anything else stays where it was.
#[must_use]
pub fn smodel_dest_path(source: PropFacetPath, cached_pretess_succeeded: bool) -> PropFacetPath {
    let moves = cached_pretess_succeeded && source == PropFacetPath::Cached;
    if moves { PropFacetPath::Pretess } else { source }
}

/// A level of detail is rigid when none of its surfaces carry skin weights.
#[must_use]
pub fn prop_lod_rigid(skin_flags: &[u8]) -> bool {
    !skin_flags.iter().any(|&flag| flag != 0)
}

/// The bucket for one surface of one level of detail.
///
/// It goes to the cache bucket when the cache is on, the instance has lighting,
/// the level of detail may be cached and the surface already holds a cache
/// index. Otherwise it is skinned or rigid depending on the model.
#[must_use]
pub fn add_static_model_surf_to_bucket(
    lod: i32,
    prop_cache_enable: bool,
    inst_lit: bool,
    lod_smc_flag: u8,
    cache_slot_no: u16,
    rigid_detail: bool,
) -> i32 {
    let cacheable = [prop_cache_enable, inst_lit, lod_smc_flag != 0, cache_slot_no != 0];
    let kind = match (cacheable.iter().all(|&ok| ok), rigid_detail) {
        (true, _) => STATIC_BUCKET_KIND_CACHED,
        (false, true) => STATIC_BUCKET_KIND_FIXED_MESH,
        (false, false) => STATIC_BUCKET_KIND_BONE_DRIVEN,
    };
    kind + STATIC_BUCKET_KIND_SPAN * lod
}

/// The sixteen bucket lists and how full each one is.
#[derive(Clone, Copy, Debug)]
pub struct PropFacetBucketLists {
    pub members: [[u16; STATIC_MODEL_BUCKET_CAPACITY]; PROP_BUCKET_LIST_COUNT],
    pub filled: [i32; PROP_BUCKET_LIST_COUNT],
}

impl Default for PropFacetBucketLists {
    fn default() -> Self {
        let empty = [0u16; STATIC_MODEL_BUCKET_CAPACITY];
        Self { filled: [0; PROP_BUCKET_LIST_COUNT], members: [empty; PROP_BUCKET_LIST_COUNT] }
    }
}

/// What a push did to its list.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StaticModelBucketInsertResult {
    /// Stored; there is still room.
    Stored,
    /// Stored, and the list is now full and should be flushed.
    Full,
}

/// What a bucket's list holds: the cache index for the cache kind (the cache
/// owns the vertices), the static model's own index for every other kind.
#[must_use]
pub fn smodel_bucket_store_payload(bucket: i32, prop_index: u16, cache_slot_no: u16) -> u16 {
    let kind = bucket.rem_euclid(STATIC_BUCKET_KIND_SPAN);
    match kind {
        STATIC_BUCKET_KIND_CACHED => cache_slot_no,
        _ => prop_index,
    }
}

/// Appends to a bucket's list. `None` when the bucket does not exist or its
/// list was already full.
#[must_use]
pub fn push_prop_facet(
    lists: &mut PropFacetBucketLists,
    bin_no: i32,
    member: u16,
) -> Option<StaticModelBucketInsertResult> {
    let list = usize::try_from(bin_no).ok()?;
    let filled = lists.filled.get_mut(list)?;
    let slot = lists.members[list].get_mut(usize::try_from(*filled).ok()?)?;
    *slot = member;
    *filled += 1;
    let now_full = *filled as usize == STATIC_MODEL_BUCKET_CAPACITY;
    Some(if now_full { StaticModelBucketInsertResult::Full } else { StaticModelBucketInsertResult::Stored })
}

/// One bucket's worth of surfaces, copied out of the lists.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StaticModelSpentBucket {
    pub bin: u8,
    pub lod: u8,
    pub source: PropFacetPath,
    pub count: u16,
    pub members: [u16; STATIC_MODEL_BUCKET_CAPACITY],
}

/// A bucket's bit in an active-bucket mask; bucket 0 is the top bit.
#[must_use]
pub const fn smodel_bucket_mask(bucket: u8) -> Option<u32> {
    match bucket as usize {
        b if b < PROP_BUCKET_LIST_COUNT => Some(FIRST_BUCKET_BIT >> b),
        _ => None,
    }
}

/// Drains the lists named by `live_bins` into `out`, in bucket order, and
/// returns how many were written. Empty lists and the pre-tessellated kind are
/// skipped; each drained list is reset.
pub fn static_bucket_lists_drain(
    lists: &mut PropFacetBucketLists,
    live_bins: u32,
    out: &mut [Option<StaticModelSpentBucket>; PROP_BUCKET_LIST_COUNT],
) -> usize {
    let stride = STATIC_BUCKET_KIND_SPAN as usize;
    let mut taken = Vec::with_capacity(PROP_BUCKET_LIST_COUNT);
    for bin_no in (0..PROP_BUCKET_LIST_COUNT).filter(|&b| live_bins & (FIRST_BUCKET_BIT >> b) != 0) {
        let filled = lists.filled[bin_no];
        let (Some(source), Ok(len @ 1..=STATIC_MODEL_BUCKET_CAPACITY)) = (bucket_source_path(bin_no), usize::try_from(filled)) else {
            continue;
        };
        let mut members = [0u16; STATIC_MODEL_BUCKET_CAPACITY];
        members[..len].copy_from_slice(&lists.members[bin_no][..len]);
        lists.filled[bin_no] = 0;
        taken.push(StaticModelSpentBucket {
            bin: bin_no as u8,
            lod: (bin_no / stride) as u8,
            source,
            count: len as u16,
            members,
        });
    }
    let written = taken.len();
    let mut fresh = taken.into_iter().map(Some).chain(std::iter::repeat(None));
    out.iter_mut().for_each(|place| *place = fresh.next().flatten());
    written
}

/// Whether a surface's material casts into the sun shadow: either of the top
/// two bits of its shadow byte.
#[must_use]
pub const fn prop_facet_casts_sun(material_shadow_byte: u8) -> bool {
    material_shadow_byte >> 6 != 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_path_has_its_own_surface_type() {
        let types = KIND_PATHS.map(prop_facet_kind);
        assert_eq!(types, [2, 5, 4, 3]);
    }

    #[test]
    fn only_a_retessellated_cached_surface_changes_path() {
        use PropFacetPath::*;
        assert_eq!(smodel_dest_path(Cached, true), Pretess);
        assert_eq!(smodel_dest_path(Cached, false), Cached);
        for (path, done) in [Rigid, Skinned, Pretess].into_iter().flat_map(|p| [(p, true), (p, false)]) {
            assert_eq!(smodel_dest_path(path, done), path);
        }
    }

    #[test]
    fn a_lod_is_rigid_only_when_no_surface_is_skinned() {
        assert!(prop_lod_rigid(&[]) && prop_lod_rigid(&[0, 0, 0]));
        assert!(!prop_lod_rigid(&[0, 1, 0]));
    }

    #[test]
    fn a_cached_surface_needs_every_condition() {
        let lod2 = |enable, lit, flag, index| add_static_model_surf_to_bucket(2, enable, lit, flag, index, true);
        assert_eq!(lod2(true, true, 1, 9), 8 + STATIC_BUCKET_KIND_CACHED);
        let missing_one = [lod2(false, true, 1, 9), lod2(true, false, 1, 9), lod2(true, true, 0, 9), lod2(true, true, 1, 0)];
        assert_eq!(missing_one, [8 + STATIC_BUCKET_KIND_FIXED_MESH; 4]);
    }

    #[test]
    fn an_uncached_surface_is_skinned_unless_the_lod_is_rigid() {
        assert_eq!(add_static_model_surf_to_bucket(1, false, false, 0, 0, false), 4 + STATIC_BUCKET_KIND_BONE_DRIVEN);
        assert_eq!(add_static_model_surf_to_bucket(3, false, false, 0, 0, true), 12 + STATIC_BUCKET_KIND_FIXED_MESH);
    }

    #[test]
    fn the_cache_bucket_stores_the_cache_index_and_the_rest_the_model() {
        let stored = [2, 6, 0, 1, 3].map(|bucket| smodel_bucket_store_payload(bucket, 70, 900));
        assert_eq!(stored, [900, 900, 70, 70, 70]);
    }

    #[test]
    fn pushing_fills_a_list_until_it_reports_full() {
        let mut lists = PropFacetBucketLists::default();
        for i in 0..STATIC_MODEL_BUCKET_CAPACITY as u16 - 1 {
            assert_eq!(push_prop_facet(&mut lists, 5, i), Some(StaticModelBucketInsertResult::Stored));
        }
        assert_eq!(push_prop_facet(&mut lists, 5, 999), Some(StaticModelBucketInsertResult::Full));
        assert_eq!(lists.filled[5], STATIC_MODEL_BUCKET_CAPACITY as i32);
        assert_eq!(lists.members[5][STATIC_MODEL_BUCKET_CAPACITY - 1], 999);
        assert_eq!(push_prop_facet(&mut lists, 5, 1), None);
        assert_eq!(lists.filled[5], STATIC_MODEL_BUCKET_CAPACITY as i32);
    }

    #[test]
    fn a_bucket_that_does_not_exist_takes_nothing() {
        let mut lists = PropFacetBucketLists::default();
        assert_eq!([-1, 16].map(|b| push_prop_facet(&mut lists, b, 1)), [None, None]);
        assert_eq!(lists.filled, [0; PROP_BUCKET_LIST_COUNT]);
    }

    #[test]
    fn bucket_masks_run_from_the_top_bit() {
        assert_eq!([0, 15, 16].map(smodel_bucket_mask), [Some(0x8000_0000), Some(0x0001_0000), None]);
    }

    fn mask_of(buckets: &[u8]) -> u32 {
        buckets.iter().filter_map(|&b| smodel_bucket_mask(b)).fold(0, |m, bit| m | bit)
    }

    #[test]
    fn consuming_copies_active_lists_in_order_and_resets_them() {
        let mut lists = PropFacetBucketLists::default();
        for (list_no, ident) in [(9, 41), (9, 42), (2, 7)] {
            push_prop_facet(&mut lists, list_no, ident).unwrap();
        }
        let mut dest = [None; PROP_BUCKET_LIST_COUNT];
        assert_eq!(static_bucket_lists_drain(&mut lists, mask_of(&[2, 9]), &mut dest), 2);

        let first = dest[0].unwrap();
        assert_eq!((first.bin, first.lod, first.count, first.source), (2, 0, 1, PropFacetPath::Cached));
        assert_eq!(first.members[0], 7);

        let second = dest[1].unwrap();
        assert_eq!((second.bin, second.lod, second.count, second.source), (9, 2, 2, PropFacetPath::Skinned));
        assert_eq!(&second.members[..2], &[41, 42]);

        assert!(dest[2..].iter().all(Option::is_none));
        assert_eq!((lists.filled[2], lists.filled[9]), (0, 0));
    }

    #[test]
    fn consuming_leaves_inactive_and_pretess_lists_alone() {
        let mut lists = PropFacetBucketLists::default();
        push_prop_facet(&mut lists, 3, 1).unwrap();
        push_prop_facet(&mut lists, 4, 2).unwrap();
        let mut dest = [None; PROP_BUCKET_LIST_COUNT];
        let everything = u32::from(u16::MAX) << 16;
        let all_but_4 = everything & !mask_of(&[4]);
        assert_eq!(static_bucket_lists_drain(&mut lists, all_but_4, &mut dest), 0);
        assert_eq!((lists.filled[3], lists.filled[4]), (1, 1));
        assert_eq!(static_bucket_lists_drain(&mut lists, everything, &mut dest), 1);
        assert_eq!(dest[0].unwrap().bin, 4);
        assert_eq!(lists.filled[3], 1);
    }

    #[test]
    fn consuming_clears_what_the_output_held_before() {
        let mut lists = PropFacetBucketLists::default();
        let mut dest = [None; PROP_BUCKET_LIST_COUNT];
        push_prop_facet(&mut lists, 0, 5).unwrap();
        static_bucket_lists_drain(&mut lists, 0xffff_0000, &mut dest);
        assert!(dest[0].is_some());
        assert_eq!(static_bucket_lists_drain(&mut lists, 0xffff_0000, &mut dest), 0);
        assert!(dest.iter().all(Option::is_none));
    }

    #[test]
    fn the_sun_shadow_needs_one_of_the_top_two_bits() {
        assert_eq!([0x40, 0x80, 0xc0, 0x3f, 0].map(prop_facet_casts_sun), [true, true, true, false, false]);
    }
}
