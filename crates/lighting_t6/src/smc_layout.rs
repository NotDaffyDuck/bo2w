//! The shape of the static model cache: how big it is, how it is carved into
//! blocks, and where each block's vertices live.
//!
//! The cache is one big vertex buffer split into six banks. Each bank holds 32
//! *trees* of 128 *leaves*, and a leaf is the smallest unit the cache hands
//! out. A tree is carved for one *level*: a family of block sizes ("size
//! classes") where each class is exactly two blocks of the class below it. A
//! block is named by its first leaf.

use crate::prop_light::STATIC_MODEL_CACHE_DETAIL_LEVELS;

/// Banks the buffer is split into.
pub const BANK_COUNT: usize = 6;
/// Size classes a block can have.
pub const PROP_CLASS_COUNT: usize = 25;
/// Trees in one bank.
pub(crate) const TREES_PER_BANK: usize = 32;
/// Trees in the whole cache.
pub const TREE_COUNT: usize = BANK_COUNT * TREES_PER_BANK;
/// Leaves in one tree.
pub(crate) const SMC_LEAVES_PER_TREE: usize = 128;
/// Leaves in the whole cache.
pub const LEAF_COUNT: usize = TREE_COUNT * SMC_LEAVES_PER_TREE;
/// Entries in the block link arrays: one per leaf, then a free-list head and a
/// used-list head for every bank and class.
pub const LINK_COUNT: usize = LEAF_COUNT + 2 * BANK_COUNT * PROP_CLASS_COUNT;

/// Vertices a tree owns in the buffer.
pub(crate) const SMC_TREE_VERTS: u32 = 0x800;
/// Vertices in one bank.
pub const BANK_VERTEX_CAP: u32 = SMC_TREE_VERTS * TREES_PER_BANK as u32;
/// Vertices in the whole buffer.
pub(crate) const SMC_VB_VERTS: u32 = BANK_VERTEX_CAP * BANK_COUNT as u32;
/// Bytes per cached vertex.
pub(crate) const CACHE_VERTEX_BYTES: u32 = 0x20;
/// Bytes in one bank's vertex buffer.
pub const PROP_CACHE_BANK_VB_BYTES: u32 = BANK_VERTEX_CAP * CACHE_VERTEX_BYTES;
/// Bytes in the whole vertex buffer.
pub const SMC_VERTEX_BYTES: u32 = SMC_VB_VERTS * CACHE_VERTEX_BYTES;
/// Index slots in the whole index buffer: four per vertex.
pub const PROP_CACHE_INDEX_U16_TALLY: usize = SMC_VB_VERTS as usize * 4;

/// Lock flag for writing a patch into the buffer: the lock promises not to
/// disturb what the GPU may still be reading.
pub(crate) const SMC_PATCH_LOCK_NOOVERWRITE: u32 = 0x1000;
/// Surfaces that may be patched into the buffer in one frame.
pub(crate) const PATCH_FACET_MAX: u32 = 0x100;
/// Vertices that may be patched into the buffer in one frame.
pub(crate) const PATCH_VERTEX_MAX: u32 = 0x2000;
/// Frames a block or tree must go unused before it can be taken over.
pub(crate) const CACHE_IDLE_FRAMES: i32 = 4;

/// Vertices in a block of each size class.
const CLASS_VERTS: [u32; PROP_CLASS_COUNT] = [
    16, 21, 28, 32, 36, 42, 51, 56, 64, 73, 85, 102, 113, 128, 146, 170, 204, 227, 256, 292, 341,
    409, 512, 682, 1024,
];

/// "No such class" in the parent and child tables.
pub(crate) const NO_CLASS: u8 = 0xff;

/// The classes of each level, smallest first. A class's parent is the next one
/// in its level and its child is the one before.
const LEVEL_CLASSES: [&[u8]; 5] = [
    &[0, 3, 8, 13, 18, 22, 24],
    &[1, 5, 10, 15, 20, 23],
    &[6, 11, 16, 21],
    &[4, 9, 14, 19],
    &[2, 7, 12, 17],
];

/// How many top-class blocks a carved tree holds, per level.
const LEVEL_BLOCKS: [usize; 5] = [2, 3, 5, 7, 9];
/// Leaves between the top-class blocks of each level.
const LEVEL_TOP_STRIDE: [u16; 5] = [64, 32, 8, 8, 8];
/// Vertices in each group of 16 leaves, per level.
const LEVEL_GROUP_VERTS: [u32; 5] = [0x100, 0x155, 0x332, 0x248, 0x1c6];

/// A tree that has not been carved for any level yet.
pub(crate) const LEVEL_FREE: i32 = 6;

#[derive(Clone, Copy)]
struct ClassGeom {
    level: u8,
    parent: u8,
    child: u8,
    /// Leaves between the two halves a parent splits into.
    stride: u16,
}

const CLASS_GEOM: [ClassGeom; PROP_CLASS_COUNT] = build_class_geom();

const fn build_class_geom() -> [ClassGeom; PROP_CLASS_COUNT] {
    let mut shape_data = [ClassGeom {
        level: 0,
        parent: NO_CLASS,
        child: NO_CLASS,
        stride: 0,
    }; PROP_CLASS_COUNT];
    let mut level = 0;
    while level < LEVEL_CLASSES.len() {
        let chain = LEVEL_CLASSES[level];
        let mut pos = 0;
        while pos < chain.len() {
            let class = chain[pos] as usize;
            shape_data[class] = ClassGeom {
                level: level as u8,
                parent: if pos + 1 < chain.len() {
                    chain[pos + 1]
                } else {
                    NO_CLASS
                },
                child: if pos > 0 { chain[pos - 1] } else { NO_CLASS },
                stride: LEVEL_TOP_STRIDE[level] >> (chain.len() - 1 - pos),
            };
            pos += 1;
        }
        level += 1;
    }
    shape_data
}

/// Vertices in a block of this class.
pub(crate) fn class_vertices(class: usize) -> u32 {
    CLASS_VERTS[class]
}

/// The smallest class that holds at least `vertex_points` vertices.
pub(crate) fn size_class_for_vertex_count(vertex_points: u32) -> Option<u8> {
    let too_small = CLASS_VERTS.iter().take_while(|&&n| n < vertex_points).count();
    (too_small < PROP_CLASS_COUNT).then_some(too_small as u8)
}

/// A class link from the geometry table, or nothing for [`NO_CLASS`].
fn class_link(link: u8) -> Option<usize> {
    match link {
        NO_CLASS => None,
        class => Some(class.into()),
    }
}

/// The class whose block this class's blocks are the halves of.
pub(crate) fn class_parent(class: usize) -> Option<usize> {
    class_link(CLASS_GEOM[class].parent)
}

/// The class this class's blocks split into.
pub(crate) fn class_child(class: usize) -> Option<usize> {
    class_link(CLASS_GEOM[class].child)
}

/// Leaves between the two halves of a split block of this class's size.
pub(crate) fn class_stride(class: usize) -> u16 {
    CLASS_GEOM[class].stride
}

/// The level a class belongs to.
pub(crate) fn class_level(class: usize) -> usize {
    usize::from(CLASS_GEOM[class].level)
}

/// How many top-class blocks a tree carved for this level holds.
pub(crate) fn level_blocks(level: usize) -> usize {
    LEVEL_BLOCKS[level]
}

/// How many leaves of a tree a carved level covers.
pub(crate) fn level_leaf_range(level: usize) -> u16 {
    LEVEL_BLOCKS[level] as u16 * LEVEL_TOP_STRIDE[level]
}

/// Vertices from the start of a 16-leaf group to the leaf at `slot` within it.
fn slot_offset(level: usize, slot: u32) -> u32 {
    match level {
        0 => slot * 16,
        1 => slot * 85 / 4,
        2 => slot * 409 / 8,
        3 => slot * 73 / 2,
        _ => (slot >> 2) * 227 / 2 + (slot & 3) * 28,
    }
}

/// The first vertex of the block that starts at `leaf`, for a tree carved for
/// `level`.
pub(crate) fn leaf_base_vert(level: usize, leaf: u32) -> u32 {
    ((leaf >> 4) & 7) * LEVEL_GROUP_VERTS[level]
        + slot_offset(level, leaf & 0xf)
        + (leaf >> 7) * SMC_TREE_VERTS
}

/// Where the cache looks for a level of detail: which cache slot it uses, the
/// bank its vertices go in and the size class of its blocks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StaticModelLodCacheLayout {
    pub detail_slot: u8,
    pub bank_no: u8,
    pub size_band: u8,
}

/// Reads a level of detail's cache description. `lod_info` is its four bytes:
/// slot, bank base, bank adjust mask and size class. `None` when the level of
/// detail opts out of the cache (a zero bank base) or names a slot, bank or
/// class that does not exist.
pub fn prop_lod_cache_plan(lod_info: [u8; 4], draw_inst_flags: u8) -> Option<StaticModelLodCacheLayout> {
    let [detail_slot, bank_base, bank_mask, size_band] = lod_info;
    if bank_base == 0
        || usize::from(detail_slot) >= STATIC_MODEL_CACHE_DETAIL_LEVELS
        || usize::from(size_band) >= PROP_CLASS_COUNT
    {
        return None;
    }
    // Bank bases count from one; the instance flags may move it along.
    let adjust = bank_mask & draw_inst_flags & 7;
    let bank = usize::from(bank_base) + usize::from(adjust) - 1;
    let bank = u8::try_from(bank).ok().filter(|&b| usize::from(b) < BANK_COUNT)?;
    Some(StaticModelLodCacheLayout { detail_slot, bank_no: bank, size_band })
}

/// The byte offset, inside its bank's buffer, of the start of the 4096-leaf
/// run that holds a cache index's leaf. `None` for "no index" and for an index
/// past the last leaf.
pub fn smc_stream_source_byte_offset(cache_slot_no: u16) -> Option<u32> {
    let leaf = u32::from(cache_slot_no.checked_sub(1)?);
    ((leaf as usize) < LEAF_COUNT).then(|| (leaf & !0xfff) << 9)
}

/// Why an index bake could not run.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PropIndexBakeFault {
    /// The source has fewer indices than the triangles need.
    InputTooShort,
    /// The destination ends before the baked indices would.
    ShortDest,
    /// The destination slot does not fit in 32 bits.
    SlotsExhausted,
}

/// Copies a surface's triangle indices into the cache's index buffer, moved to
/// the vertices its block holds, and returns the slot they start at.
///
/// Triangles are baked two at a time, so an odd last triangle is left out.
pub fn bake_prop_indices(
    write_target: &mut [u16],
    base_vertex_index: u32,
    figure_part_base_index: u32,
    figure_part_vertex_offset: u16,
    tri_tally: u16,
    source_indices: &[u16],
) -> Result<u32, PropIndexBakeFault> {
    use PropIndexBakeFault::{ShortDest, InputTooShort, SlotsExhausted};
    // Whole pairs of triangles, three indices each.
    let pairs = usize::from(tri_tally / 2);
    let source = source_indices.get(..pairs * 6).ok_or(InputTooShort)?;
    let index_slot = |vertex_points: u32, triangle_list: u32| vertex_points.checked_mul(4)?.checked_add(triangle_list.checked_mul(3)?);
    let first = index_slot(base_vertex_index, figure_part_base_index).ok_or(SlotsExhausted)?;
    let window = write_target.get_mut(first as usize..).and_then(|rest| rest.get_mut(..source.len())).ok_or(ShortDest)?;
    // Indices are rebased onto the block's vertices, in 16 bits.
    let rebase = (base_vertex_index as u16).wrapping_add(figure_part_vertex_offset);
    window.iter_mut().zip(source).for_each(|(slot, &index)| *slot = rebase.wrapping_add(index));
    Ok(first)
}

/// A region of the vertex buffer a patch is written into.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PropCachePatchLock {
    pub start_byte: u32,
    pub payload_extent_bytes: u32,
    pub map_mode_flags: u32,
}

impl PropCachePatchLock {
    /// Whether a patch of `patch_len` bytes fits this lock exactly, and the lock
    /// is the kind that leaves the rest of the buffer alone.
    pub fn fits_byte_length(self, patch_len: usize) -> bool {
        let leaves_rest_alone = self.map_mode_flags == SMC_PATCH_LOCK_NOOVERWRITE;
        leaves_rest_alone && usize::try_from(self.payload_extent_bytes).is_ok_and(|n| n == patch_len)
    }
}

/// The lock for patching a block of `class_vertices` vertices that starts at
/// `base_vertex_index`; `None` when it would reach past the end of the buffer.
pub fn lock_prop_cache_patch(
    base_vertex_index: u32,
    class_vertices: u32,
) -> Option<PropCachePatchLock> {
    let bytes_of = |vertex_points: u32| vertex_points.checked_mul(CACHE_VERTEX_BYTES);
    let lock = PropCachePatchLock {
        start_byte: bytes_of(base_vertex_index)?,
        payload_extent_bytes: bytes_of(class_vertices)?,
        map_mode_flags: SMC_PATCH_LOCK_NOOVERWRITE,
    };
    let end = lock.start_byte.checked_add(lock.payload_extent_bytes)?;
    (end <= SMC_VERTEX_BYTES).then_some(lock)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_buffer_sizes_agree_with_each_other() {
        assert_eq!(TREE_COUNT, 192);
        assert_eq!(LEAF_COUNT, 0x6000);
        assert_eq!(LINK_COUNT, 0x612c);
        assert_eq!(BANK_VERTEX_CAP, 0x10000);
        assert_eq!(SMC_VB_VERTS, 393_216);
        assert_eq!(PROP_CACHE_BANK_VB_BYTES, 0x20_0000);
        assert_eq!(SMC_VERTEX_BYTES, 0xc0_0000);
        assert_eq!(PROP_CACHE_INDEX_U16_TALLY, 393_216 * 4);
    }

    #[test]
    fn classes_grow_and_a_parent_is_two_children() {
        assert!(CLASS_VERTS.windows(2).all(|pair| pair[0] < pair[1]));
        for class in 0..PROP_CLASS_COUNT {
            if let Some(parent) = class_parent(class) {
                assert_eq!(class_child(parent), Some(class));
                let (small, big) = (class_vertices(class), class_vertices(parent));
                assert!(
                    big == 2 * small || big == 2 * small + 1,
                    "{class} -> {parent}"
                );
                assert_eq!(class_stride(parent), 2 * class_stride(class));
                assert_eq!(class_level(parent), class_level(class));
            }
            if let Some(child) = class_child(class) {
                assert_eq!(class_parent(child), Some(class));
            }
        }
    }

    #[test]
    fn every_class_is_in_exactly_one_level() {
        let mut seen = [0u8; PROP_CLASS_COUNT];
        for chain in LEVEL_CLASSES {
            for &class in chain {
                seen[usize::from(class)] += 1;
            }
        }
        assert_eq!(seen, [1; PROP_CLASS_COUNT]);
    }

    #[test]
    fn the_lowest_class_of_every_level_is_one_leaf_per_block() {
        for chain in LEVEL_CLASSES {
            let lowest = usize::from(chain[0]);
            assert_eq!(class_child(lowest), None);
            assert_eq!(class_stride(lowest), 1);
        }
        assert_eq!(class_stride(24), 64);
        assert_eq!(class_stride(23), 32);
        assert_eq!(class_stride(21), 8);
    }

    #[test]
    fn each_level_covers_the_leaves_it_should() {
        let ranges: Vec<u16> = (0..5).map(level_leaf_range).collect();
        assert_eq!(ranges, [0x80, 0x60, 0x28, 0x38, 0x48]);
        for range in ranges {
            assert!(usize::from(range) <= SMC_LEAVES_PER_TREE);
        }
    }

    #[test]
    fn the_smallest_class_that_fits_is_picked() {
        assert_eq!(size_class_for_vertex_count(0), Some(0));
        assert_eq!(size_class_for_vertex_count(16), Some(0));
        assert_eq!(size_class_for_vertex_count(17), Some(1));
        assert_eq!(size_class_for_vertex_count(1024), Some(24));
        assert_eq!(size_class_for_vertex_count(1025), None);
    }

    #[test]
    fn blocks_stay_inside_their_tree_and_never_overlap() {
        for level in 0..5 {
            for &class in LEVEL_CLASSES[level] {
                let class = usize::from(class);
                let stride = u32::from(class_stride(class));
                let blocks = u32::from(level_leaf_range(level)) / stride;
                let mut next_free = 0;
                for block in 0..blocks {
                    let base = leaf_base_vert(level, block * stride);
                    assert!(
                        base >= next_free,
                        "level {level} class {class} block {block}"
                    );
                    next_free = base + class_vertices(class);
                    assert!(next_free <= SMC_TREE_VERTS, "level {level} class {class}");
                }
            }
        }
    }

    #[test]
    fn a_split_block_gives_its_vertices_to_its_halves() {
        for class in 0..PROP_CLASS_COUNT {
            let Some(parent) = class_parent(class) else {
                continue;
            };
            let level = class_level(class);
            let stride = u32::from(class_stride(class));
            let parents = u32::from(level_leaf_range(level)) / u32::from(class_stride(parent));
            for block in 0..parents {
                let leaf = block * u32::from(class_stride(parent));
                let whole = leaf_base_vert(level, leaf);
                let high = leaf_base_vert(level, leaf + stride);
                assert!(
                    high >= whole + class_vertices(class),
                    "class {class} block {block}"
                );
                assert!(
                    high + class_vertices(class) <= whole + class_vertices(parent),
                    "class {class} block {block}"
                );
            }
        }
    }

    #[test]
    fn a_leaf_in_a_later_tree_sits_one_window_further_on() {
        for level in 0..5 {
            assert_eq!(
                leaf_base_vert(level, 128 + 3) - leaf_base_vert(level, 3),
                SMC_TREE_VERTS
            );
        }
        assert_eq!(leaf_base_vert(0, 0), 0);
        assert_eq!(leaf_base_vert(0, 16), 0x100);
        assert_eq!(leaf_base_vert(1, 1), 21);
    }

    #[test]
    fn a_lod_spec_picks_the_bank_from_the_base_and_the_flags() {
        let spec = prop_lod_cache_plan([2, 1, 0, 7], 0xff).unwrap();
        assert_eq!(
            spec,
            StaticModelLodCacheLayout {
                detail_slot: 2,
                bank_no: 0,
                size_band: 7
            }
        );
        let adjusted = prop_lod_cache_plan([0, 2, 0b101, 0], 0b100).unwrap();
        assert_eq!(adjusted.bank_no, 5);
        let masked = prop_lod_cache_plan([0, 2, 0b101, 0], 0b010).unwrap();
        assert_eq!(masked.bank_no, 1);
    }

    #[test]
    fn a_lod_spec_refuses_what_the_cache_cannot_hold() {
        assert_eq!(prop_lod_cache_plan([0, 0, 0, 0], 0xff), None);
        assert_eq!(prop_lod_cache_plan([4, 1, 0, 0], 0xff), None);
        assert_eq!(prop_lod_cache_plan([0, 1, 0, 25], 0xff), None);
        assert_eq!(prop_lod_cache_plan([0, 7, 0, 0], 0), None);
        assert!(prop_lod_cache_plan([3, 6, 0, 24], 0).is_some());
    }

    #[test]
    fn a_stream_offset_names_the_run_a_leaf_lives_in() {
        assert_eq!(smc_stream_source_byte_offset(0), None);
        assert_eq!(smc_stream_source_byte_offset(1), Some(0));
        assert_eq!(smc_stream_source_byte_offset(0x1000), Some(0));
        assert_eq!(smc_stream_source_byte_offset(0x1001), Some(0x1000 << 9));
        assert_eq!(
            smc_stream_source_byte_offset(LEAF_COUNT as u16),
            Some(0x5000 << 9)
        );
        assert_eq!(smc_stream_source_byte_offset(LEAF_COUNT as u16 + 1), None);
    }

    #[test]
    fn baking_moves_indices_to_the_block_and_returns_the_slot() {
        let mut write_target = vec![0u16; 64];
        let src = [0u16, 1, 2, 2, 1, 3, 9, 9, 9];
        let slot = bake_prop_indices(&mut write_target, 2, 3, 5, 3, &src).unwrap();
        assert_eq!(slot, 2 * 4 + 3 * 3);
        // One pair of triangles is baked: six indices, each moved by 2 + 5.
        assert_eq!(&write_target[17..23], &[7, 8, 9, 9, 8, 10]);
        assert_eq!(write_target[16], 0);
        assert_eq!(write_target[23], 0);
    }

    #[test]
    fn baking_wraps_the_shift_like_the_buffer_does() {
        let mut write_target = vec![0u16; 32];
        bake_prop_indices(&mut write_target, 0, 0, 3, 2, &[65_534; 6]).unwrap();
        assert_eq!(&write_target[..6], &[1; 6]);
    }

    #[test]
    fn baking_checks_both_ends_before_writing() {
        let mut write_target = vec![7u16; 10];
        assert_eq!(
            bake_prop_indices(&mut write_target, 0, 0, 0, 4, &[0; 11]),
            Err(PropIndexBakeFault::InputTooShort)
        );
        assert_eq!(
            bake_prop_indices(&mut write_target, 2, 0, 0, 2, &[0; 6]),
            Err(PropIndexBakeFault::ShortDest)
        );
        assert_eq!(
            bake_prop_indices(&mut write_target, u32::MAX, 0, 0, 2, &[0; 6]),
            Err(PropIndexBakeFault::SlotsExhausted)
        );
        assert_eq!(
            bake_prop_indices(&mut write_target, 0, u32::MAX, 0, 2, &[0; 6]),
            Err(PropIndexBakeFault::SlotsExhausted)
        );
        assert_eq!(write_target, vec![7u16; 10]);
    }

    #[test]
    fn baking_an_empty_surface_still_reports_its_slot() {
        let mut write_target = vec![0u16; 4];
        assert_eq!(
            bake_prop_indices(&mut write_target, 1, 0, 0, 1, &[]),
            Ok(4)
        );
    }

    #[test]
    fn a_patch_lock_covers_the_block_in_bytes() {
        let lock = lock_prop_cache_patch(10, 32).unwrap();
        assert_eq!(lock.start_byte, 320);
        assert_eq!(lock.payload_extent_bytes, 1024);
        assert_eq!(lock.map_mode_flags, 0x1000);
        assert!(lock.fits_byte_length(1024));
        assert!(!lock.fits_byte_length(1023));
        assert!(
            !PropCachePatchLock {
                map_mode_flags: 0,
                ..lock
            }
            .fits_byte_length(1024)
        );
    }

    #[test]
    fn a_patch_lock_may_not_leave_the_buffer() {
        assert!(lock_prop_cache_patch(SMC_VB_VERTS - 16, 16).is_some());
        assert_eq!(lock_prop_cache_patch(SMC_VB_VERTS - 16, 17), None);
        assert_eq!(lock_prop_cache_patch(u32::MAX, 1), None);
        assert_eq!(lock_prop_cache_patch(0, u32::MAX), None);
    }
}
