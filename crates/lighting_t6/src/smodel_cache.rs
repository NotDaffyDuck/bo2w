//! The static model cache: it hands out blocks of the cached vertex buffer to
//! static-model surfaces and takes them back when they go unused.
//!
//! Every piece of state lives in slices the host owns, so a [`StaticModelCache`]
//! can be rebuilt over the same slices at any time. See [`crate::smc_layout`]
//! for how the buffer is cut into banks, trees, leaves and blocks.
//!
//! A block is named by its first leaf and has a size class. Free blocks of
//! each class wait in a per-bank list; used blocks wait in another, so they
//! can be swept. When a class has nothing free it splits a block of the next
//! class up, and when the top class of a level has nothing free the bank's
//! least recently used tree is emptied and carved afresh. A freed block merges
//! with its twin whenever the twin is free too.

use crate::smc_layout::{
    LEVEL_FREE, SMC_BANK_N, SMC_CLASS_N, SMC_IDLE_FRAMES, SMC_LEAF_N, SMC_LEAVES_PER_TREE,
    SMC_LINK_N, SMC_PATCH_SURF_MAX, SMC_PATCH_VERT_MAX, SMC_TREE_N, SMC_TREES_PER_BANK,
    class_child, class_level, class_parent, class_stride, class_verts, leaf_base_vert,
    level_blocks, level_leaf_range, smc_size_class_for_verts,
};
use crate::smodel_lighting::SMC_CACHE_INDEX_LODS;

/// "No tree" in a bank's tree list.
const TREE_NONE: u16 = u16::MAX;

/// Owner value of a block nobody holds.
const FREE_OWNER: u32 = 0xffff;

/// A tree's place in its bank's least-recently-used list, and what it is
/// carved for.
#[derive(Clone, Copy, Debug, Default)]
pub struct SmcTree {
    /// The neighbour used less recently than this tree.
    older: u16,
    /// The neighbour used more recently than this tree.
    newer: u16,
    /// The level the tree is carved for, or [`LEVEL_FREE`].
    level: i32,
    /// The frame the tree was last used in.
    frame: i32,
}

/// Why a cache could not be built over the slices it was given.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SmcCacheError {
    ShortArray,
}

/// The per-leaf slices.
pub struct SmcLeafStorage<'a> {
    leaf_word0: &'a mut [u32],
    leaf_base: &'a mut [u32],
    leaf_frame: &'a mut [i32],
    cache_index: &'a mut [u16],
    leaf_verts: &'a mut [u32],
}

impl<'a> SmcLeafStorage<'a> {
    /// `leaf_word0`: owner, level of detail and size class of each leaf's
    /// block. `leaf_base`: the block's first vertex. `leaf_frame`: when the
    /// block was last used. `cache_index`: one entry per static model and level
    /// of detail, naming the leaf (plus one) that holds it. `leaf_verts`: how
    /// many vertices the surface in the block really uses.
    pub fn new(
        leaf_word0: &'a mut [u32],
        leaf_base: &'a mut [u32],
        leaf_frame: &'a mut [i32],
        cache_index: &'a mut [u16],
        leaf_verts: &'a mut [u32],
    ) -> Result<Self, SmcCacheError> {
        let leaf_sized = [
            leaf_word0.len(),
            leaf_base.len(),
            leaf_frame.len(),
            leaf_verts.len(),
        ]
        .iter()
        .all(|&len| len >= SMC_LEAF_N);
        if !leaf_sized || cache_index.len() % SMC_CACHE_INDEX_LODS != 0 {
            return Err(SmcCacheError::ShortArray);
        }
        Ok(Self {
            leaf_word0,
            leaf_base,
            leaf_frame,
            cache_index,
            leaf_verts,
        })
    }
}

/// The slices the block allocator keeps its lists in.
pub struct SmcAllocatorStorage<'a> {
    link_next: &'a mut [u16],
    link_prev: &'a mut [u16],
    tree_newest: &'a mut [u16],
    tree_oldest: &'a mut [u16],
    level_sweep: &'a mut [i32],
    budget: &'a mut [i32],
    trees: &'a mut [SmcTree],
}

impl<'a> SmcAllocatorStorage<'a> {
    /// `link_next` and `link_prev` link the blocks of each free and used list;
    /// `tree_newest` and `tree_oldest` hold each bank's tree list ends;
    /// `level_sweep` holds the frame each bank and class was last swept in;
    /// `budget` holds how many blocks each bank and class may still hand out
    /// (negative means no limit).
    pub fn new(
        link_next: &'a mut [u16],
        link_prev: &'a mut [u16],
        tree_newest: &'a mut [u16],
        tree_oldest: &'a mut [u16],
        level_sweep: &'a mut [i32],
        budget: &'a mut [i32],
        trees: &'a mut [SmcTree],
    ) -> Result<Self, SmcCacheError> {
        let sized = link_next.len() >= SMC_LINK_N
            && link_prev.len() >= SMC_LINK_N
            && trees.len() >= SMC_TREE_N
            && tree_newest.len() >= SMC_BANK_N
            && tree_oldest.len() >= SMC_BANK_N
            && level_sweep.len() >= SMC_BANK_N * SMC_CLASS_N
            && budget.len() >= SMC_BANK_N * SMC_CLASS_N;
        if !sized {
            return Err(SmcCacheError::ShortArray);
        }
        Ok(Self {
            link_next,
            link_prev,
            tree_newest,
            tree_oldest,
            level_sweep,
            budget,
            trees,
        })
    }
}

/// What asking the cache for a surface came to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CacheStaticModelSurface {
    /// The surface already had a block.
    Hit { cache_index: u16 },
    /// The surface got a new block; the host must now fill it.
    Miss {
        cache_index: u16,
        base_vert_index: u32,
        verts: u32,
    },
    /// The cache could not or would not take the surface.
    Refused,
}

/// The static model cache over host-owned slices.
pub struct StaticModelCache<'a> {
    leaf_word0: &'a mut [u32],
    leaf_base: &'a mut [u32],
    leaf_frame: &'a mut [i32],
    cache_index: &'a mut [u16],
    leaf_verts: &'a mut [u32],
    link_next: &'a mut [u16],
    link_prev: &'a mut [u16],
    tree_newest: &'a mut [u16],
    tree_oldest: &'a mut [u16],
    level_sweep: &'a mut [i32],
    budget: &'a mut [i32],
    trees: &'a mut [SmcTree],
    /// The frame being built.
    pub frame: i32,
    /// Surfaces given new blocks this frame.
    pub patch_surfs: u32,
    /// Vertices given new blocks this frame.
    pub patch_verts: u32,
}

/// The word0 of a free block of this class.
fn free_tag(class: usize) -> u32 {
    (class as u32) << 24 | FREE_OWNER
}

/// The list head standing for the free blocks of a bank and class.
fn free_head(bank: usize, class: usize) -> u16 {
    (SMC_LEAF_N + (bank * SMC_CLASS_N + class) * 2) as u16
}

/// The list head standing for the used blocks of a bank and class.
fn used_head(bank: usize, class: usize) -> u16 {
    free_head(bank, class) + 1
}

/// The leaf a cache index names.
fn leaf_of(cache_index: u16) -> Option<usize> {
    let leaf = usize::from(cache_index.checked_sub(1)?);
    (leaf < SMC_LEAF_N).then_some(leaf)
}

fn tree_of(leaf: usize) -> usize {
    leaf / SMC_LEAVES_PER_TREE
}

fn bank_of(leaf: usize) -> usize {
    tree_of(leaf) / SMC_TREES_PER_BANK
}

impl<'a> StaticModelCache<'a> {
    pub fn new(leaves: SmcLeafStorage<'a>, allocator: SmcAllocatorStorage<'a>) -> Self {
        Self {
            leaf_word0: leaves.leaf_word0,
            leaf_base: leaves.leaf_base,
            leaf_frame: leaves.leaf_frame,
            cache_index: leaves.cache_index,
            leaf_verts: leaves.leaf_verts,
            link_next: allocator.link_next,
            link_prev: allocator.link_prev,
            tree_newest: allocator.tree_newest,
            tree_oldest: allocator.tree_oldest,
            level_sweep: allocator.level_sweep,
            budget: allocator.budget,
            trees: allocator.trees,
            frame: 0,
            patch_surfs: 0,
            patch_verts: 0,
        }
    }

    /// Empties the cache. Everything counts as already idle, so the first
    /// requests may carve trees at once.
    pub fn clear(&mut self, frame: i32) {
        self.frame = frame;
        self.patch_surfs = 0;
        self.patch_verts = 0;
        let idle = frame.wrapping_sub(SMC_IDLE_FRAMES);
        self.leaf_word0[..SMC_LEAF_N].fill(u32::MAX);
        self.leaf_base[..SMC_LEAF_N].fill(u32::MAX);
        self.leaf_frame[..SMC_LEAF_N].fill(idle);
        self.leaf_verts[..SMC_LEAF_N].fill(0);
        self.cache_index.fill(0);
        self.link_next[..SMC_LINK_N].fill(0);
        self.link_prev[..SMC_LINK_N].fill(0);
        self.budget[..SMC_BANK_N * SMC_CLASS_N].fill(-1);
        for bank in 0..SMC_BANK_N {
            // Until trees are used, the bank's first tree is the oldest.
            let first = bank * SMC_TREES_PER_BANK;
            self.tree_oldest[bank] = first as u16;
            self.tree_newest[bank] = (first + SMC_TREES_PER_BANK - 1) as u16;
            for tree in first..first + SMC_TREES_PER_BANK {
                self.trees[tree] = SmcTree {
                    older: if tree == first {
                        TREE_NONE
                    } else {
                        (tree - 1) as u16
                    },
                    newer: if tree + 1 == first + SMC_TREES_PER_BANK {
                        TREE_NONE
                    } else {
                        (tree + 1) as u16
                    },
                    level: LEVEL_FREE,
                    frame: idle,
                };
            }
            for class in 0..SMC_CLASS_N {
                for head in [free_head(bank, class), used_head(bank, class)] {
                    self.link_next[usize::from(head)] = head;
                    self.link_prev[usize::from(head)] = head;
                }
                self.level_sweep[bank * SMC_CLASS_N + class] = idle;
            }
        }
    }

    /// Finds or makes room for one static-model surface.
    ///
    /// A surface that already has a block (`cache_index` nonzero) is marked
    /// used this frame. Otherwise a block is allocated from `bank`, of
    /// `size_class` (or the smallest class holding `verts` when that is out of
    /// range), and recorded under `smodel_index` and `lod`. Allocation is
    /// refused when the cache is `enabled == false`, when this frame's patch
    /// limits are spent, when the surface does not fit any class, or when
    /// nothing can be freed.
    #[allow(clippy::too_many_arguments)]
    pub fn cache_surface(
        &mut self,
        bank: u8,
        size_class: u8,
        smodel_index: u16,
        lod: u8,
        cache_index: u16,
        verts: u32,
        enabled: bool,
    ) -> CacheStaticModelSurface {
        use CacheStaticModelSurface::{Hit, Miss, Refused};
        if !enabled {
            return Refused;
        }
        if cache_index != 0 {
            return if self.touch(cache_index) {
                Hit { cache_index }
            } else {
                Refused
            };
        }
        let class = if usize::from(size_class) < SMC_CLASS_N {
            usize::from(size_class)
        } else if let Some(class) = smc_size_class_for_verts(verts) {
            usize::from(class)
        } else {
            return Refused;
        };
        let block_verts = class_verts(class);
        if self.patch_surfs >= SMC_PATCH_SURF_MAX
            || self.patch_verts.saturating_add(block_verts) > SMC_PATCH_VERT_MAX
        {
            return Refused;
        }
        let Some(slot) = self.cache_index_slot(smodel_index, lod) else {
            return Refused;
        };
        let Some(new_index) = self.allocate(usize::from(bank), class) else {
            return Refused;
        };
        let leaf = usize::from(new_index - 1);
        self.leaf_word0[leaf] =
            u32::from(smodel_index) | u32::from(lod) << 16 | (class as u32) << 24;
        self.leaf_verts[leaf] = verts;
        self.cache_index[slot] = new_index;
        self.patch_surfs += 1;
        self.patch_verts += block_verts;
        self.touch(new_index);
        Miss {
            cache_index: new_index,
            base_vert_index: self.leaf_base[leaf],
            verts: block_verts,
        }
    }

    /// Gives a surface's block back. Indexes that are zero, out of range or
    /// already free are ignored.
    pub fn uncache_index(&mut self, cache_index: u16) {
        let Some(leaf) = leaf_of(cache_index) else {
            return;
        };
        let word0 = self.leaf_word0[leaf];
        let class = (word0 >> 24) as usize;
        if word0 & 0xffff == FREE_OWNER || class >= SMC_CLASS_N {
            return;
        }
        self.release_block(leaf);
        self.merge_free(bank_of(leaf), class, leaf);
    }

    /// Marks a block used this frame and its tree the most recently used one in
    /// its bank. `false` when the index names no leaf.
    fn touch(&mut self, cache_index: u16) -> bool {
        let Some(leaf) = leaf_of(cache_index) else {
            return false;
        };
        self.leaf_frame[leaf] = self.frame;
        let tree = tree_of(leaf);
        if self.trees[tree].frame != self.frame {
            self.trees[tree].frame = self.frame;
            let bank = bank_of(leaf);
            self.unlink_tree(tree, bank);
            self.push_tree_newest(tree, bank);
        }
        true
    }

    /// Whether something last used at `since` has gone unused long enough to
    /// be taken over.
    fn idle_since(&self, since: i32) -> bool {
        self.frame.wrapping_sub(since) as u32 >= SMC_IDLE_FRAMES as u32
    }

    /// Where a model's level of detail records its cache index.
    fn cache_index_slot(&self, smodel_index: u16, lod: u8) -> Option<usize> {
        let slot = usize::from(smodel_index) * SMC_CACHE_INDEX_LODS + usize::from(lod);
        (usize::from(lod) < SMC_CACHE_INDEX_LODS && slot < self.cache_index.len()).then_some(slot)
    }

    /// Takes a free block of the class out of the bank, making one if needed,
    /// and returns its cache index.
    fn allocate(&mut self, bank: usize, class: usize) -> Option<u16> {
        if bank >= SMC_BANK_N || class >= SMC_CLASS_N {
            return None;
        }
        let budget = self.budget[bank * SMC_CLASS_N + class];
        if budget == 0 {
            return None;
        }
        let head = free_head(bank, class);
        if self.link_next[usize::from(head)] == head && !self.make_free_block(bank, class) {
            return None;
        }
        let block = self.link_next[usize::from(head)];
        if budget > 0 {
            self.budget[bank * SMC_CLASS_N + class] -= 1;
        }
        self.unlink(block);
        self.push_front(used_head(bank, class), block);
        let leaf = usize::from(block);
        self.leaf_base[leaf] = leaf_base_vert(class_level(class), u32::from(block));
        Some(block + 1)
    }

    /// Makes sure the class has a free block: by reclaiming idle blocks, by
    /// splitting a block of the class above, or, for the top class of a level,
    /// by carving a tree.
    fn make_free_block(&mut self, bank: usize, class: usize) -> bool {
        self.sweep_idle_blocks(bank, class);
        let head = free_head(bank, class);
        if self.link_next[usize::from(head)] != head {
            return true;
        }
        let Some(parent) = class_parent(class) else {
            return self.carve_tree(bank, class);
        };
        let parent_head = free_head(bank, parent);
        if self.link_next[usize::from(parent_head)] == parent_head
            && !self.make_free_block(bank, parent)
        {
            return false;
        }
        let low = self.link_next[usize::from(parent_head)];
        self.unlink(low);
        let high = low + class_stride(class);
        debug_assert!(usize::from(high) < SMC_LEAF_N);
        self.leaf_word0[usize::from(low)] = free_tag(class);
        self.leaf_word0[usize::from(high)] = free_tag(class);
        // The low half goes in front, so it is handed out first.
        self.push_front(head, high);
        self.push_front(head, low);
        true
    }

    /// Frees the class's used blocks that have gone idle, smaller classes
    /// first. A class is swept at most once in a few frames.
    fn sweep_idle_blocks(&mut self, bank: usize, class: usize) {
        let swept = bank * SMC_CLASS_N + class;
        if !self.idle_since(self.level_sweep[swept]) {
            return;
        }
        self.level_sweep[swept] = self.frame;
        if let Some(child) = class_child(class) {
            self.sweep_idle_blocks(bank, child);
        }
        let head = used_head(bank, class);
        let mut block = self.link_next[usize::from(head)];
        // A healthy list is shorter than the leaf count; the bound only keeps a
        // damaged one from spinning.
        for _ in 0..SMC_LEAF_N {
            if block == head {
                break;
            }
            let next = self.link_next[usize::from(block)];
            if self.idle_since(self.leaf_frame[usize::from(block)]) {
                self.uncache_index(block + 1);
            }
            block = next;
        }
    }

    /// Empties the bank's least recently used tree, if it has been idle long
    /// enough, and carves it for the level of `class` (the top class of its
    /// level): the level's blocks go on the class's free list.
    fn carve_tree(&mut self, bank: usize, class: usize) -> bool {
        let oldest = self.tree_oldest[bank];
        if oldest == TREE_NONE {
            return false;
        }
        let tree = usize::from(oldest);
        if !self.idle_since(self.trees[tree].frame) {
            return false;
        }
        self.unlink_tree(tree, bank);
        self.empty_tree(tree);
        let level = class_level(class);
        self.trees[tree].level = level as i32;
        let first = tree * SMC_LEAVES_PER_TREE;
        let stride = usize::from(class_stride(class));
        let head = free_head(bank, class);
        for block in 0..level_blocks(level) {
            let leaf = first + block * stride;
            self.leaf_word0[leaf] = free_tag(class);
            self.push_back(head, leaf as u16);
        }
        self.trees[tree].frame = self.frame;
        self.push_tree_newest(tree, bank);
        true
    }

    /// Takes every block of a carved tree off its lists, uncaching the surfaces
    /// that used them, and marks the tree uncarved.
    fn empty_tree(&mut self, tree: usize) {
        let carved = usize::try_from(self.trees[tree].level)
            .ok()
            .filter(|&l| l < LEVEL_FREE as usize);
        self.trees[tree].level = LEVEL_FREE;
        let Some(level) = carved else {
            return;
        };
        let first = tree * SMC_LEAVES_PER_TREE;
        let end = first + usize::from(level_leaf_range(level));
        let mut leaf = first;
        while leaf < end {
            let word0 = self.leaf_word0[leaf];
            let class = (word0 >> 24) as usize;
            if class >= SMC_CLASS_N {
                debug_assert!(
                    false,
                    "carved tree {tree} has a leaf without a class: {leaf}"
                );
                break;
            }
            if word0 & 0xffff == FREE_OWNER {
                self.unlink(leaf as u16);
            } else {
                self.release_block(leaf);
            }
            leaf += usize::from(class_stride(class));
        }
    }

    /// Drops the surface in a used block: its cache index is cleared and the
    /// block is tagged free and taken off the used list. It is on no list
    /// afterwards.
    fn release_block(&mut self, leaf: usize) {
        let word0 = self.leaf_word0[leaf];
        let smodel_index = word0 as u16;
        let lod = (word0 >> 16) as u8;
        let class = (word0 >> 24) as usize;
        self.leaf_verts[leaf] = 0;
        if let Some(slot) = self.cache_index_slot(smodel_index, lod) {
            self.cache_index[slot] = 0;
        }
        self.leaf_word0[leaf] = free_tag(class);
        self.unlink(leaf as u16);
    }

    /// Puts a block that is on no list onto its class's free list, first
    /// merging it with its twin for as long as the twin is free as well.
    fn merge_free(&mut self, bank: usize, mut class: usize, mut leaf: usize) {
        while let Some(parent) = class_parent(class) {
            let stride = usize::from(class_stride(class));
            let twin = leaf ^ stride;
            if twin >= SMC_LEAF_N || self.leaf_word0[twin] != free_tag(class) {
                break;
            }
            self.unlink(twin as u16);
            class = parent;
            leaf &= !stride;
            self.leaf_word0[leaf] = free_tag(class);
        }
        self.push_front(free_head(bank, class), leaf as u16);
    }

    fn unlink(&mut self, node: u16) {
        let next = self.link_next[usize::from(node)];
        let prev = self.link_prev[usize::from(node)];
        self.link_prev[usize::from(next)] = prev;
        self.link_next[usize::from(prev)] = next;
    }

    fn push_front(&mut self, head: u16, node: u16) {
        debug_assert!(
            usize::from(node) < SMC_LEAF_N,
            "only leaves go on block lists"
        );
        let first = self.link_next[usize::from(head)];
        self.link_next[usize::from(node)] = first;
        self.link_prev[usize::from(node)] = head;
        self.link_prev[usize::from(first)] = node;
        self.link_next[usize::from(head)] = node;
    }

    fn push_back(&mut self, head: u16, node: u16) {
        debug_assert!(
            usize::from(node) < SMC_LEAF_N,
            "only leaves go on block lists"
        );
        let last = self.link_prev[usize::from(head)];
        self.link_prev[usize::from(node)] = last;
        self.link_next[usize::from(node)] = head;
        self.link_next[usize::from(last)] = node;
        self.link_prev[usize::from(head)] = node;
    }

    fn unlink_tree(&mut self, tree: usize, bank: usize) {
        let SmcTree { older, newer, .. } = self.trees[tree];
        if older == TREE_NONE {
            self.tree_oldest[bank] = newer;
        } else {
            self.trees[usize::from(older)].newer = newer;
        }
        if newer == TREE_NONE {
            self.tree_newest[bank] = older;
        } else {
            self.trees[usize::from(newer)].older = older;
        }
    }

    fn push_tree_newest(&mut self, tree: usize, bank: usize) {
        let previous = self.tree_newest[bank];
        self.trees[tree].newer = TREE_NONE;
        self.trees[tree].older = previous;
        if previous == TREE_NONE {
            self.tree_oldest[bank] = tree as u16;
        } else {
            self.trees[usize::from(previous)].newer = tree as u16;
        }
        self.tree_newest[bank] = tree as u16;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::smc_layout::SMC_BANK_VERTS;

    /// Everything a cache needs, owned in one place so tests can look inside.
    struct Host {
        leaf_word0: Vec<u32>,
        leaf_base: Vec<u32>,
        leaf_frame: Vec<i32>,
        cache_index: Vec<u16>,
        leaf_verts: Vec<u32>,
        link_next: Vec<u16>,
        link_prev: Vec<u16>,
        tree_newest: Vec<u16>,
        tree_oldest: Vec<u16>,
        level_sweep: Vec<i32>,
        budget: Vec<i32>,
        trees: Vec<SmcTree>,
        /// The frame a cache made from this host starts in.
        frame: i32,
    }

    impl Host {
        fn new(models: usize) -> Self {
            Self {
                leaf_word0: vec![0; SMC_LEAF_N],
                leaf_base: vec![0; SMC_LEAF_N],
                leaf_frame: vec![0; SMC_LEAF_N],
                cache_index: vec![0; models * SMC_CACHE_INDEX_LODS],
                leaf_verts: vec![0; SMC_LEAF_N],
                link_next: vec![0; SMC_LINK_N],
                link_prev: vec![0; SMC_LINK_N],
                tree_newest: vec![0; SMC_BANK_N],
                tree_oldest: vec![0; SMC_BANK_N],
                level_sweep: vec![0; SMC_BANK_N * SMC_CLASS_N],
                budget: vec![0; SMC_BANK_N * SMC_CLASS_N],
                trees: vec![SmcTree::default(); SMC_TREE_N],
                frame: 0,
            }
        }

        fn cache(&mut self) -> StaticModelCache<'_> {
            let leaves = SmcLeafStorage::new(
                &mut self.leaf_word0,
                &mut self.leaf_base,
                &mut self.leaf_frame,
                &mut self.cache_index,
                &mut self.leaf_verts,
            )
            .unwrap();
            let allocator = SmcAllocatorStorage::new(
                &mut self.link_next,
                &mut self.link_prev,
                &mut self.tree_newest,
                &mut self.tree_oldest,
                &mut self.level_sweep,
                &mut self.budget,
                &mut self.trees,
            )
            .unwrap();
            let mut cache = StaticModelCache::new(leaves, allocator);
            cache.frame = self.frame;
            cache
        }
    }

    fn started(models: usize, frame: i32) -> Host {
        let mut host = Host::new(models);
        host.frame = frame;
        host.cache().clear(frame);
        host
    }

    /// Asks for a surface in a fresh slot and a fresh patch budget.
    fn add(
        cache: &mut StaticModelCache<'_>,
        bank: u8,
        class: u8,
        model: u16,
        lod: u8,
    ) -> CacheStaticModelSurface {
        cache.patch_surfs = 0;
        cache.patch_verts = 0;
        cache.cache_surface(
            bank,
            class,
            model,
            lod,
            0,
            class_verts(usize::from(class)),
            true,
        )
    }

    fn miss(result: CacheStaticModelSurface) -> (u16, u32, u32) {
        match result {
            CacheStaticModelSurface::Miss {
                cache_index,
                base_vert_index,
                verts,
            } => (cache_index, base_vert_index, verts),
            other => panic!("expected a miss, got {other:?}"),
        }
    }

    fn free_list(host: &Host, bank: usize, class: usize) -> Vec<u16> {
        let head = free_head(bank, class);
        let mut out = Vec::new();
        let mut node = host.link_next[usize::from(head)];
        while node != head {
            out.push(node);
            node = host.link_next[usize::from(node)];
            assert!(out.len() <= SMC_LEAF_N, "free list does not close");
        }
        out
    }

    fn used_list(host: &Host, bank: usize, class: usize) -> Vec<u16> {
        let head = used_head(bank, class);
        let mut out = Vec::new();
        let mut node = host.link_next[usize::from(head)];
        while node != head {
            out.push(node);
            node = host.link_next[usize::from(node)];
            assert!(out.len() <= SMC_LEAF_N, "used list does not close");
        }
        out
    }

    #[test]
    fn storage_must_be_big_enough() {
        let mut host = Host::new(4);
        let short = &mut host.leaf_word0[..SMC_LEAF_N - 1];
        assert_eq!(
            SmcLeafStorage::new(
                short,
                &mut host.leaf_base,
                &mut host.leaf_frame,
                &mut host.cache_index,
                &mut host.leaf_verts
            )
            .err(),
            Some(SmcCacheError::ShortArray)
        );
        let mut host = Host::new(4);
        host.cache_index.push(0);
        assert!(
            SmcLeafStorage::new(
                &mut host.leaf_word0,
                &mut host.leaf_base,
                &mut host.leaf_frame,
                &mut host.cache_index,
                &mut host.leaf_verts
            )
            .is_err()
        );
        let mut host = Host::new(4);
        let few = &mut host.trees[..SMC_TREE_N - 1];
        assert!(
            SmcAllocatorStorage::new(
                &mut host.link_next,
                &mut host.link_prev,
                &mut host.tree_newest,
                &mut host.tree_oldest,
                &mut host.level_sweep,
                &mut host.budget,
                few
            )
            .is_err()
        );
    }

    #[test]
    fn a_cleared_cache_holds_nothing() {
        let host = started(8, 100);
        assert!(host.leaf_word0.iter().all(|&w| w == u32::MAX));
        assert!(host.leaf_frame.iter().all(|&f| f == 96));
        assert!(host.budget.iter().all(|&b| b == -1));
        assert!(host.level_sweep.iter().all(|&s| s == 96));
        for bank in 0..SMC_BANK_N {
            for class in 0..SMC_CLASS_N {
                assert!(free_list(&host, bank, class).is_empty());
                assert!(used_list(&host, bank, class).is_empty());
            }
            assert_eq!(host.tree_oldest[bank], (bank * 32) as u16);
            assert_eq!(host.tree_newest[bank], (bank * 32 + 31) as u16);
        }
    }

    #[test]
    fn clearing_forgets_the_cache_indices_too() {
        let mut host = started(4, 10);
        {
            let mut cache = host.cache();
            miss(add(&mut cache, 0, 3, 2, 1));
        }
        assert_ne!(host.cache_index[2 * 4 + 1], 0);
        host.cache().clear(11);
        assert!(host.cache_index.iter().all(|&i| i == 0));
    }

    #[test]
    fn the_first_block_of_a_bank_is_its_first_vertices() {
        let mut host = started(4, 10);
        let mut cache = host.cache();
        let (first, base, verts) = miss(add(&mut cache, 0, 0, 1, 0));
        assert_eq!((first, base, verts), (1, 0, 16));
        let (second, base, verts) = miss(add(&mut cache, 0, 0, 1, 1));
        assert_eq!((second, base, verts), (2, 16, 16));
        assert_eq!(cache.cache_index[4], 1);
        assert_eq!(cache.cache_index[5], 2);
    }

    #[test]
    fn each_bank_hands_out_its_own_vertices() {
        let mut host = started(SMC_BANK_N, 10);
        let mut cache = host.cache();
        for bank in 0..SMC_BANK_N as u8 {
            let (_, base, _) = miss(add(&mut cache, bank, 24, bank.into(), 0));
            assert_eq!(base, u32::from(bank) * SMC_BANK_VERTS);
        }
    }

    #[test]
    fn a_new_block_records_its_owner() {
        let mut host = started(8, 10);
        let mut cache = host.cache();
        let (index, ..) = miss(add(&mut cache, 2, 5, 6, 3));
        let leaf = usize::from(index - 1);
        assert_eq!(cache.leaf_word0[leaf], 6 | 3 << 16 | 5 << 24);
        assert_eq!(cache.leaf_verts[leaf], 42);
        assert_eq!(cache.leaf_frame[leaf], 10);
        assert_eq!(cache.cache_index[6 * 4 + 3], index);
        assert_eq!((cache.patch_surfs, cache.patch_verts), (1, 42));
    }

    #[test]
    fn asking_again_with_the_index_is_a_hit() {
        let mut host = started(4, 10);
        let mut cache = host.cache();
        let (index, ..) = miss(add(&mut cache, 0, 8, 0, 0));
        cache.frame = 12;
        assert_eq!(
            cache.cache_surface(0, 8, 0, 0, index, 64, true),
            CacheStaticModelSurface::Hit { cache_index: index }
        );
        assert_eq!(cache.leaf_frame[usize::from(index - 1)], 12);
    }

    #[test]
    fn a_disabled_cache_refuses_everything() {
        let mut host = started(4, 10);
        let mut cache = host.cache();
        let (index, ..) = miss(add(&mut cache, 0, 8, 0, 0));
        assert_eq!(
            cache.cache_surface(0, 8, 1, 0, 0, 64, false),
            CacheStaticModelSurface::Refused
        );
        assert_eq!(
            cache.cache_surface(0, 8, 0, 0, index, 64, false),
            CacheStaticModelSurface::Refused
        );
    }

    #[test]
    fn a_hit_on_an_index_that_names_no_leaf_is_refused() {
        let mut host = started(4, 10);
        let mut cache = host.cache();
        let past = SMC_LEAF_N as u16 + 1;
        assert_eq!(
            cache.cache_surface(0, 8, 0, 0, past, 64, true),
            CacheStaticModelSurface::Refused
        );
    }

    #[test]
    fn the_class_comes_from_the_vertex_count_when_the_given_one_is_out_of_range() {
        let mut host = started(4, 10);
        let mut cache = host.cache();
        let result = cache.cache_surface(0, 200, 0, 0, 0, 100, true);
        let (_, _, verts) = miss(result);
        assert_eq!(verts, 102);
        cache.patch_surfs = 0;
        assert_eq!(
            cache.cache_surface(0, 200, 1, 0, 0, 1025, true),
            CacheStaticModelSurface::Refused
        );
    }

    #[test]
    fn a_surface_with_no_place_to_record_its_index_is_refused_untouched() {
        let mut host = started(2, 10);
        let mut cache = host.cache();
        assert_eq!(
            cache.cache_surface(0, 3, 2, 0, 0, 32, true),
            CacheStaticModelSurface::Refused
        );
        assert_eq!(
            cache.cache_surface(0, 3, 0, 4, 0, 32, true),
            CacheStaticModelSurface::Refused
        );
        assert_eq!((cache.patch_surfs, cache.patch_verts), (0, 0));
        assert!(cache.leaf_word0.iter().all(|&w| w == u32::MAX));
    }

    #[test]
    fn a_bank_that_does_not_exist_gives_nothing() {
        let mut host = started(2, 10);
        let mut cache = host.cache();
        assert_eq!(
            cache.cache_surface(6, 3, 0, 0, 0, 32, true),
            CacheStaticModelSurface::Refused
        );
    }

    #[test]
    fn a_frame_takes_at_most_so_many_surfaces() {
        let mut host = started(400, 10);
        let mut cache = host.cache();
        for model in 0..SMC_PATCH_SURF_MAX as u16 {
            let result = cache.cache_surface(0, 0, model, 0, 0, 16, true);
            assert!(
                matches!(result, CacheStaticModelSurface::Miss { .. }),
                "surface {model}"
            );
        }
        assert_eq!(
            cache.cache_surface(0, 0, 300, 0, 0, 16, true),
            CacheStaticModelSurface::Refused
        );
    }

    #[test]
    fn a_frame_takes_at_most_so_many_vertices() {
        let mut host = started(40, 10);
        let mut cache = host.cache();
        for model in 0..8 {
            miss(cache.cache_surface(0, 24, model, 0, 0, 1024, true));
        }
        assert_eq!(cache.patch_verts, SMC_PATCH_VERT_MAX);
        assert_eq!(
            cache.cache_surface(0, 0, 20, 0, 0, 16, true),
            CacheStaticModelSurface::Refused
        );
    }

    #[test]
    fn a_spent_budget_refuses_that_class_in_that_bank() {
        let mut host = started(8, 10);
        let mut cache = host.cache();
        cache.budget[2 * SMC_CLASS_N + 7] = 1;
        miss(add(&mut cache, 2, 7, 0, 0));
        assert_eq!(cache.budget[2 * SMC_CLASS_N + 7], 0);
        assert_eq!(
            add(&mut cache, 2, 7, 1, 0),
            CacheStaticModelSurface::Refused
        );
        miss(add(&mut cache, 2, 8, 1, 0));
        miss(add(&mut cache, 3, 7, 1, 1));
    }

    #[test]
    fn live_blocks_never_share_vertices() {
        let mut host = started(2000, 10);
        let mut cache = host.cache();
        let mut live = Vec::new();
        let mut seed = 12345u32;
        for model in 0..1500u16 {
            seed = seed.wrapping_mul(1_103_515_245).wrapping_add(12345);
            let bank = ((seed >> 8) % 3) as u8;
            let class = ((seed >> 16) % SMC_CLASS_N as u32) as u8;
            if let CacheStaticModelSurface::Miss {
                base_vert_index,
                verts,
                ..
            } = add(&mut cache, bank, class, model, 0)
            {
                let window = u32::from(bank) * SMC_BANK_VERTS;
                assert!(base_vert_index >= window);
                assert!(base_vert_index + verts <= window + SMC_BANK_VERTS);
                live.push((base_vert_index, base_vert_index + verts));
            }
        }
        assert!(live.len() > 100);
        live.sort_unstable();
        for pair in live.windows(2) {
            assert!(pair[0].1 <= pair[1].0, "{pair:?} overlap");
        }
    }

    #[test]
    fn freeing_both_halves_puts_the_whole_block_back() {
        let mut host = started(4, 10);
        let (first, second) = {
            let mut cache = host.cache();
            let (first, ..) = miss(add(&mut cache, 0, 0, 0, 0));
            let (second, ..) = miss(add(&mut cache, 0, 0, 1, 0));
            (first, second)
        };
        assert_eq!((first, second), (1, 2));
        host.cache().uncache_index(first);
        assert_eq!(free_list(&host, 0, 0), vec![0]);
        host.cache().uncache_index(second);
        // The two smallest blocks merge, and so on all the way up: one whole
        // 1024-vertex block is free again and nothing smaller is.
        assert_eq!(free_list(&host, 0, 24), vec![0, 64]);
        for class in 0..24 {
            assert!(free_list(&host, 0, class).is_empty(), "class {class}");
        }
        assert!(host.cache_index.iter().all(|&i| i == 0));
        assert_eq!(host.leaf_word0[0], free_tag(24));
    }

    #[test]
    fn freeing_one_half_leaves_the_other_where_it_was() {
        let mut host = started(4, 10);
        let mut cache = host.cache();
        let (first, ..) = miss(add(&mut cache, 0, 0, 0, 0));
        miss(add(&mut cache, 0, 0, 1, 0));
        cache.uncache_index(first);
        assert_eq!(cache.leaf_word0[0], free_tag(0));
        assert_eq!(cache.leaf_verts[0], 0);
        assert_eq!(cache.cache_index[0], 0);
        assert_ne!(cache.cache_index[4], 0);
    }

    #[test]
    fn freeing_nothing_in_particular_does_nothing() {
        let mut host = started(4, 10);
        let mut cache = host.cache();
        let (index, ..) = miss(add(&mut cache, 0, 4, 0, 0));
        cache.uncache_index(0);
        cache.uncache_index(SMC_LEAF_N as u16 + 1);
        cache.uncache_index(u16::MAX);
        cache.uncache_index(index);
        let free_before = cache.link_next.to_vec();
        cache.uncache_index(index);
        assert_eq!(cache.link_next.to_vec(), free_before);
        assert_eq!(cache.cache_index[0], 0);
    }

    #[test]
    fn blocks_idle_for_a_few_frames_make_way_for_new_ones() {
        let mut host = started(100, 10);
        let mut cache = host.cache();
        // 32 trees of two 1024-vertex blocks fill the whole bank.
        for model in 0..64 {
            miss(add(&mut cache, 0, 24, model, 0));
        }
        assert_eq!(
            add(&mut cache, 0, 24, 64, 0),
            CacheStaticModelSurface::Refused
        );
        cache.frame = 13;
        assert_eq!(
            add(&mut cache, 0, 24, 64, 0),
            CacheStaticModelSurface::Refused
        );
        cache.frame = 14;
        let (_, base, _) = miss(add(&mut cache, 0, 24, 64, 0));
        assert!(base < SMC_BANK_VERTS);
        // Only the blocks that were not needed again were taken back.
        let held = (0..64).filter(|&m| cache.cache_index[m * 4] != 0).count();
        assert_eq!(held, 0);
        assert_ne!(cache.cache_index[64 * 4], 0);
    }

    #[test]
    fn a_block_in_use_every_frame_is_never_taken_back() {
        let mut host = started(100, 10);
        let mut cache = host.cache();
        let (kept, ..) = miss(add(&mut cache, 0, 24, 0, 0));
        for model in 1..64 {
            miss(add(&mut cache, 0, 24, model, 0));
        }
        for frame in 11..30 {
            cache.frame = frame;
            assert_eq!(
                cache.cache_surface(0, 24, 0, 0, kept, 1024, true),
                CacheStaticModelSurface::Hit { cache_index: kept }
            );
            let result = add(&mut cache, 0, 24, 64, 0);
            if let CacheStaticModelSurface::Miss { cache_index, .. } = result {
                cache.uncache_index(cache_index);
            }
            assert_eq!(cache.cache_index[0], kept, "frame {frame}");
        }
    }

    #[test]
    fn an_idle_tree_is_recarved_for_a_different_level() {
        let mut host = started(100, 10);
        let mut cache = host.cache();
        for model in 0..64 {
            miss(add(&mut cache, 0, 24, model, 0));
        }
        cache.frame = 20;
        // Class 23 belongs to another level, so a tree has to be emptied.
        let (index, base, verts) = miss(add(&mut cache, 0, 23, 70, 0));
        assert_eq!(verts, 682);
        assert_eq!(base % 0x800, 0);
        let tree = usize::from(index - 1) / SMC_LEAVES_PER_TREE;
        assert_eq!(cache.trees[tree].level, 1);
        // The models that lived in that tree lost their blocks; the rest kept
        // theirs.
        let lost = (0..64).filter(|&m| cache.cache_index[m * 4] == 0).count();
        assert_eq!(lost, 2);
    }

    #[test]
    fn the_least_recently_used_tree_is_carved_first() {
        let mut host = started(100, 10);
        let mut cache = host.cache();
        assert_eq!(cache.tree_oldest[0], 0);
        miss(add(&mut cache, 0, 24, 0, 0));
        // The first tree was carved and is now the newest one.
        assert_eq!(cache.tree_newest[0], 0);
        assert_eq!(cache.tree_oldest[0], 1);
        miss(add(&mut cache, 0, 24, 1, 0));
        miss(add(&mut cache, 0, 24, 2, 0));
        assert_eq!(cache.tree_newest[0], 1);
        assert_eq!(cache.tree_oldest[0], 2);
    }

    #[test]
    fn touching_a_block_makes_its_tree_the_newest_in_its_bank() {
        let mut host = started(100, 10);
        let mut cache = host.cache();
        let (first, ..) = miss(add(&mut cache, 0, 24, 0, 0));
        miss(add(&mut cache, 0, 24, 1, 0));
        miss(add(&mut cache, 0, 24, 2, 0));
        assert_eq!(cache.tree_newest[0], 1);
        cache.frame = 11;
        cache.cache_surface(0, 24, 0, 0, first, 1024, true);
        assert_eq!(cache.tree_newest[0], 0);
        let mut order = Vec::new();
        let mut tree = cache.tree_newest[0];
        while tree != TREE_NONE {
            order.push(tree);
            tree = cache.trees[usize::from(tree)].older;
        }
        assert_eq!(order.len(), SMC_TREES_PER_BANK);
        assert_eq!(&order[..3], &[0, 1, 31]);
        assert_eq!(order.last(), Some(&2));
    }

    #[test]
    fn a_block_list_keeps_its_links_consistent_through_churn() {
        let mut host = started(600, 10);
        {
            let mut cache = host.cache();
            let mut live: Vec<u16> = Vec::new();
            let mut seed = 99u32;
            for step in 0..800u32 {
                seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                cache.frame = 10 + (step / 40) as i32;
                if seed >> 28 < 10 || live.is_empty() {
                    let class = ((seed >> 8) % 12) as u8;
                    let model = (step % 600) as u16;
                    if cache.cache_index[usize::from(model) * 4] != 0 {
                        continue;
                    }
                    if let CacheStaticModelSurface::Miss { cache_index, .. } =
                        add(&mut cache, 0, class, model, 0)
                    {
                        live.push(cache_index);
                    }
                } else {
                    let at = (seed >> 4) as usize % live.len();
                    cache.uncache_index(live.swap_remove(at));
                }
            }
        }
        // Every list closes on itself and agrees in both directions.
        for class in 0..SMC_CLASS_N {
            for list in [free_list(&host, 0, class), used_list(&host, 0, class)] {
                let mut back = host.link_prev[usize::from(free_head(0, class))];
                let _ = &mut back;
                for &node in &list {
                    let next = host.link_next[usize::from(node)];
                    assert_eq!(host.link_prev[usize::from(next)], node);
                }
            }
        }
    }
}
