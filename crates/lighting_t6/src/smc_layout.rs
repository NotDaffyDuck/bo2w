//! The shape of the static model cache: how big it is, how it is carved into
//! blocks, and where each block's vertices live.
//!
//! The cache is one big vertex buffer split into six banks. Each bank holds 32
//! *trees* of 128 *leaves*, and a leaf is the smallest unit the cache hands
//! out. A tree is carved for one *level*: a family of block sizes ("size
//! classes") where each class is exactly two blocks of the class below it. A
//! block is named by its first leaf.

use crate::smodel_lighting::SMC_CACHE_INDEX_LODS;

/// Banks the buffer is split into.
pub const SMC_BANK_N: usize = 6;
/// Size classes a block can have.
pub const SMC_CLASS_N: usize = 25;
/// Trees in one bank.
pub(crate) const SMC_TREES_PER_BANK: usize = 32;
/// Trees in the whole cache.
pub const SMC_TREE_N: usize = SMC_BANK_N * SMC_TREES_PER_BANK;
/// Leaves in one tree.
pub(crate) const SMC_LEAVES_PER_TREE: usize = 128;
/// Leaves in the whole cache.
pub const SMC_LEAF_N: usize = SMC_TREE_N * SMC_LEAVES_PER_TREE;
/// Entries in the block link arrays: one per leaf, then a free-list head and a
/// used-list head for every bank and class.
pub const SMC_LINK_N: usize = SMC_LEAF_N + 2 * SMC_BANK_N * SMC_CLASS_N;

/// Vertices a tree owns in the buffer.
pub(crate) const SMC_TREE_VERTS: u32 = 0x800;
/// Vertices in one bank.
pub const SMC_BANK_VERTS: u32 = SMC_TREE_VERTS * SMC_TREES_PER_BANK as u32;
/// Vertices in the whole buffer.
pub(crate) const SMC_VB_VERTS: u32 = SMC_BANK_VERTS * SMC_BANK_N as u32;
/// Bytes per cached vertex.
pub(crate) const SMC_VERT_STRIDE: u32 = 0x20;
/// Bytes in one bank's vertex buffer.
pub const SMC_BANK_VB_BYTES: u32 = SMC_BANK_VERTS * SMC_VERT_STRIDE;
/// Bytes in the whole vertex buffer.
pub const SMC_VB_BYTES: u32 = SMC_VB_VERTS * SMC_VERT_STRIDE;
/// Index slots in the whole index buffer: four per vertex.
pub const SMC_INDEX_U16_N: usize = SMC_VB_VERTS as usize * 4;

/// Lock flag for writing a patch into the buffer: the lock promises not to
/// disturb what the GPU may still be reading.
pub(crate) const SMC_PATCH_LOCK_NOOVERWRITE: u32 = 0x1000;
/// Surfaces that may be patched into the buffer in one frame.
pub(crate) const SMC_PATCH_SURF_MAX: u32 = 0x100;
/// Vertices that may be patched into the buffer in one frame.
pub(crate) const SMC_PATCH_VERT_MAX: u32 = 0x2000;
/// Frames a block or tree must go unused before it can be taken over.
pub(crate) const SMC_IDLE_FRAMES: i32 = 4;

/// Vertices in a block of each size class.
const CLASS_VERTS: [u32; SMC_CLASS_N] = [
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

const CLASS_GEOM: [ClassGeom; SMC_CLASS_N] = build_class_geom();

const fn build_class_geom() -> [ClassGeom; SMC_CLASS_N] {
    let mut geom = [ClassGeom {
        level: 0,
        parent: NO_CLASS,
        child: NO_CLASS,
        stride: 0,
    }; SMC_CLASS_N];
    let mut level = 0;
    while level < LEVEL_CLASSES.len() {
        let chain = LEVEL_CLASSES[level];
        let mut pos = 0;
        while pos < chain.len() {
            let class = chain[pos] as usize;
            geom[class] = ClassGeom {
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
    geom
}

/// Vertices in a block of this class.
pub(crate) fn class_verts(class: usize) -> u32 {
    CLASS_VERTS[class]
}

/// The smallest class that holds at least `verts` vertices.
pub(crate) fn smc_size_class_for_verts(verts: u32) -> Option<u8> {
    CLASS_VERTS
        .iter()
        .position(|&n| n >= verts)
        .map(|class| class as u8)
}

/// The class whose block this class's blocks are the halves of.
pub(crate) fn class_parent(class: usize) -> Option<usize> {
    Some(CLASS_GEOM[class].parent)
        .filter(|&p| p != NO_CLASS)
        .map(usize::from)
}

/// The class this class's blocks split into.
pub(crate) fn class_child(class: usize) -> Option<usize> {
    Some(CLASS_GEOM[class].child)
        .filter(|&c| c != NO_CLASS)
        .map(usize::from)
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
pub struct SmcLodCacheSpec {
    pub lod_slot: u8,
    pub bank: u8,
    pub size_class: u8,
}

/// Reads a level of detail's cache description. `lod_info` is its four bytes:
/// slot, bank base, bank adjust mask and size class. `None` when the level of
/// detail opts out of the cache (a zero bank base) or names a slot, bank or
/// class that does not exist.
pub fn smc_lod_cache_spec(lod_info: [u8; 4], draw_inst_flags: u8) -> Option<SmcLodCacheSpec> {
    let [lod_slot, bank_base, bank_mask, size_class] = lod_info;
    if bank_base == 0
        || usize::from(lod_slot) >= SMC_CACHE_INDEX_LODS
        || usize::from(size_class) >= SMC_CLASS_N
    {
        return None;
    }
    let bank = u32::from(bank_base)
        .wrapping_add(u32::from(bank_mask & draw_inst_flags & 7))
        .wrapping_sub(1);
    (bank < SMC_BANK_N as u32).then_some(SmcLodCacheSpec {
        lod_slot,
        bank: bank as u8,
        size_class,
    })
}

/// The byte offset, inside its bank's buffer, of the start of the 4096-leaf
/// run that holds a cache index's leaf. `None` for "no index" and for an index
/// past the last leaf.
pub fn smc_stream_source_byte_offset(cache_index: u16) -> Option<u32> {
    let leaf = u32::from(cache_index.checked_sub(1)?);
    ((leaf as usize) < SMC_LEAF_N).then(|| (leaf & !0xfff) << 9)
}

/// Why an index bake could not run.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SmcIndexBakeError {
    /// The source has fewer indices than the triangles need.
    ShortSource,
    /// The destination ends before the baked indices would.
    ShortDest,
    /// The destination slot does not fit in 32 bits.
    SlotOverflow,
}

/// Copies a surface's triangle indices into the cache's index buffer, moved to
/// the vertices its block holds, and returns the slot they start at.
///
/// Triangles are baked two at a time, so an odd last triangle is left out.
pub fn cache_static_model_indices(
    dest: &mut [u16],
    base_vert_index: u32,
    xsurface_base_index: u32,
    xsurface_vert_offset: u16,
    tri_count: u16,
    src_indices: &[u16],
) -> Result<u32, SmcIndexBakeError> {
    let count = usize::from(tri_count >> 1) * 6;
    let src = src_indices
        .get(..count)
        .ok_or(SmcIndexBakeError::ShortSource)?;
    let slot = base_vert_index
        .checked_mul(4)
        .zip(xsurface_base_index.checked_mul(3))
        .and_then(|(verts, tris)| verts.checked_add(tris))
        .ok_or(SmcIndexBakeError::SlotOverflow)?;
    let start = slot as usize;
    let out = start
        .checked_add(count)
        .and_then(|end| dest.get_mut(start..end))
        .ok_or(SmcIndexBakeError::ShortDest)?;
    let shift = base_vert_index.wrapping_add(u32::from(xsurface_vert_offset)) as u16;
    for (out, &index) in out.iter_mut().zip(src) {
        *out = index.wrapping_add(shift);
    }
    Ok(slot)
}

/// A region of the vertex buffer a patch is written into.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SmcPatchLock {
    pub byte_offset: u32,
    pub byte_count: u32,
    pub lock_flags: u32,
}

impl SmcPatchLock {
    /// Whether a patch of `byte_len` bytes fits this lock exactly, and the lock
    /// is the kind that leaves the rest of the buffer alone.
    pub fn accepts_bytes(self, byte_len: usize) -> bool {
        byte_len == self.byte_count as usize && self.lock_flags == SMC_PATCH_LOCK_NOOVERWRITE
    }
}

/// The lock for patching a block of `class_verts` vertices that starts at
/// `base_vert_index`; `None` when it would reach past the end of the buffer.
pub fn patch_static_model_cache_lock(
    base_vert_index: u32,
    class_verts: u32,
) -> Option<SmcPatchLock> {
    let byte_offset = base_vert_index.checked_mul(SMC_VERT_STRIDE)?;
    let byte_count = class_verts.checked_mul(SMC_VERT_STRIDE)?;
    (byte_offset.checked_add(byte_count)? <= SMC_VB_BYTES).then_some(SmcPatchLock {
        byte_offset,
        byte_count,
        lock_flags: SMC_PATCH_LOCK_NOOVERWRITE,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_buffer_sizes_agree_with_each_other() {
        assert_eq!(SMC_TREE_N, 192);
        assert_eq!(SMC_LEAF_N, 0x6000);
        assert_eq!(SMC_LINK_N, 0x612c);
        assert_eq!(SMC_BANK_VERTS, 0x10000);
        assert_eq!(SMC_VB_VERTS, 393_216);
        assert_eq!(SMC_BANK_VB_BYTES, 0x20_0000);
        assert_eq!(SMC_VB_BYTES, 0xc0_0000);
        assert_eq!(SMC_INDEX_U16_N, 393_216 * 4);
    }

    #[test]
    fn classes_grow_and_a_parent_is_two_children() {
        assert!(CLASS_VERTS.windows(2).all(|pair| pair[0] < pair[1]));
        for class in 0..SMC_CLASS_N {
            if let Some(parent) = class_parent(class) {
                assert_eq!(class_child(parent), Some(class));
                let (small, big) = (class_verts(class), class_verts(parent));
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
        let mut seen = [0u8; SMC_CLASS_N];
        for chain in LEVEL_CLASSES {
            for &class in chain {
                seen[usize::from(class)] += 1;
            }
        }
        assert_eq!(seen, [1; SMC_CLASS_N]);
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
        assert_eq!(smc_size_class_for_verts(0), Some(0));
        assert_eq!(smc_size_class_for_verts(16), Some(0));
        assert_eq!(smc_size_class_for_verts(17), Some(1));
        assert_eq!(smc_size_class_for_verts(1024), Some(24));
        assert_eq!(smc_size_class_for_verts(1025), None);
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
                    next_free = base + class_verts(class);
                    assert!(next_free <= SMC_TREE_VERTS, "level {level} class {class}");
                }
            }
        }
    }

    #[test]
    fn a_split_block_gives_its_vertices_to_its_halves() {
        for class in 0..SMC_CLASS_N {
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
                    high >= whole + class_verts(class),
                    "class {class} block {block}"
                );
                assert!(
                    high + class_verts(class) <= whole + class_verts(parent),
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
        let spec = smc_lod_cache_spec([2, 1, 0, 7], 0xff).unwrap();
        assert_eq!(
            spec,
            SmcLodCacheSpec {
                lod_slot: 2,
                bank: 0,
                size_class: 7
            }
        );
        let adjusted = smc_lod_cache_spec([0, 2, 0b101, 0], 0b100).unwrap();
        assert_eq!(adjusted.bank, 5);
        let masked = smc_lod_cache_spec([0, 2, 0b101, 0], 0b010).unwrap();
        assert_eq!(masked.bank, 1);
    }

    #[test]
    fn a_lod_spec_refuses_what_the_cache_cannot_hold() {
        assert_eq!(smc_lod_cache_spec([0, 0, 0, 0], 0xff), None);
        assert_eq!(smc_lod_cache_spec([4, 1, 0, 0], 0xff), None);
        assert_eq!(smc_lod_cache_spec([0, 1, 0, 25], 0xff), None);
        assert_eq!(smc_lod_cache_spec([0, 7, 0, 0], 0), None);
        assert!(smc_lod_cache_spec([3, 6, 0, 24], 0).is_some());
    }

    #[test]
    fn a_stream_offset_names_the_run_a_leaf_lives_in() {
        assert_eq!(smc_stream_source_byte_offset(0), None);
        assert_eq!(smc_stream_source_byte_offset(1), Some(0));
        assert_eq!(smc_stream_source_byte_offset(0x1000), Some(0));
        assert_eq!(smc_stream_source_byte_offset(0x1001), Some(0x1000 << 9));
        assert_eq!(
            smc_stream_source_byte_offset(SMC_LEAF_N as u16),
            Some(0x5000 << 9)
        );
        assert_eq!(smc_stream_source_byte_offset(SMC_LEAF_N as u16 + 1), None);
    }

    #[test]
    fn baking_moves_indices_to_the_block_and_returns_the_slot() {
        let mut dest = vec![0u16; 64];
        let src = [0u16, 1, 2, 2, 1, 3, 9, 9, 9];
        let slot = cache_static_model_indices(&mut dest, 2, 3, 5, 3, &src).unwrap();
        assert_eq!(slot, 2 * 4 + 3 * 3);
        // One pair of triangles is baked: six indices, each moved by 2 + 5.
        assert_eq!(&dest[17..23], &[7, 8, 9, 9, 8, 10]);
        assert_eq!(dest[16], 0);
        assert_eq!(dest[23], 0);
    }

    #[test]
    fn baking_wraps_the_shift_like_the_buffer_does() {
        let mut dest = vec![0u16; 32];
        cache_static_model_indices(&mut dest, 0, 0, 3, 2, &[65_534; 6]).unwrap();
        assert_eq!(&dest[..6], &[1; 6]);
    }

    #[test]
    fn baking_checks_both_ends_before_writing() {
        let mut dest = vec![7u16; 10];
        assert_eq!(
            cache_static_model_indices(&mut dest, 0, 0, 0, 4, &[0; 11]),
            Err(SmcIndexBakeError::ShortSource)
        );
        assert_eq!(
            cache_static_model_indices(&mut dest, 2, 0, 0, 2, &[0; 6]),
            Err(SmcIndexBakeError::ShortDest)
        );
        assert_eq!(
            cache_static_model_indices(&mut dest, u32::MAX, 0, 0, 2, &[0; 6]),
            Err(SmcIndexBakeError::SlotOverflow)
        );
        assert_eq!(
            cache_static_model_indices(&mut dest, 0, u32::MAX, 0, 2, &[0; 6]),
            Err(SmcIndexBakeError::SlotOverflow)
        );
        assert_eq!(dest, vec![7u16; 10]);
    }

    #[test]
    fn baking_an_empty_surface_still_reports_its_slot() {
        let mut dest = vec![0u16; 4];
        assert_eq!(
            cache_static_model_indices(&mut dest, 1, 0, 0, 1, &[]),
            Ok(4)
        );
    }

    #[test]
    fn a_patch_lock_covers_the_block_in_bytes() {
        let lock = patch_static_model_cache_lock(10, 32).unwrap();
        assert_eq!(lock.byte_offset, 320);
        assert_eq!(lock.byte_count, 1024);
        assert_eq!(lock.lock_flags, 0x1000);
        assert!(lock.accepts_bytes(1024));
        assert!(!lock.accepts_bytes(1023));
        assert!(
            !SmcPatchLock {
                lock_flags: 0,
                ..lock
            }
            .accepts_bytes(1024)
        );
    }

    #[test]
    fn a_patch_lock_may_not_leave_the_buffer() {
        assert!(patch_static_model_cache_lock(SMC_VB_VERTS - 16, 16).is_some());
        assert_eq!(patch_static_model_cache_lock(SMC_VB_VERTS - 16, 17), None);
        assert_eq!(patch_static_model_cache_lock(u32::MAX, 1), None);
        assert_eq!(patch_static_model_cache_lock(0, u32::MAX), None);
    }
}
