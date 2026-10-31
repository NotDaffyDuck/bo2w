//! The static model cache: it hands out blocks of the cached vertex buffer to
//! static-model surfaces and takes them back when they go unused.
//!
//! The allocator's whole state sits in one [`SmcAllocator`] the host keeps; a
//! [`StaticMeshResidencyCache`] is the short-lived view that pairs it with the host's
//! table of models. See [`crate::smc_layout`] for how the buffer is cut into
//! banks, trees, leaves and blocks.
//!
//! A block is named by its first leaf and has a size class. Free blocks of
//! each class wait in a per-bank list; used blocks wait in another, so they
//! can be swept. When a class has nothing free it splits a block of the next
//! class up, and when the top class of a level has nothing free the bank's
//! least recently used tree is emptied and carved afresh. A freed block merges
//! with its twin whenever the twin is free too.

use crate::smc_layout::{
    LEVEL_FREE, BANK_COUNT, PROP_CLASS_COUNT, CACHE_IDLE_FRAMES, LEAF_COUNT, SMC_LEAVES_PER_TREE,
    LINK_COUNT, PATCH_FACET_MAX, PATCH_VERTEX_MAX, TREE_COUNT, TREES_PER_BANK,
    class_child, class_level, class_parent, class_stride, class_vertices, leaf_base_vert,
    level_blocks, level_leaf_range, size_class_for_vertex_count,
};
use crate::prop_light::STATIC_MODEL_CACHE_DETAIL_LEVELS;

/// What the host records for one static model: the cache index its block holds
/// at each cached level of detail, zero where it holds none.
pub type SmcCacheRow = [u16; STATIC_MODEL_CACHE_DETAIL_LEVELS];

/// "No tree" at either end of a bank's tree list.
const NO_TREE: u16 = u16::MAX;

/// The owner half of a tag nobody holds.
const NOBODY: u32 = 0xffff;

/// What a leaf says about the block that starts there, packed in one word: the
/// holding model in the low half, its level of detail in the third byte and the
/// block's size class in the top byte. A free block has [`NOBODY`] as owner.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Tag(u32);

impl Tag {
    /// A leaf that no block has started on yet.
    const UNSET: Self = Self(u32::MAX);

    fn pack(owner: u32, class: usize) -> Self {
        Self(owner | (class as u32) << 24)
    }

    fn free(class: usize) -> Self {
        Self::pack(NOBODY, class)
    }

    fn held(model: u16, lod: u8, class: usize) -> Self {
        Self::pack(u32::from_le_bytes([0, 0, lod, 0]) | u32::from(model), class)
    }

    fn model(self) -> u16 {
        (self.0 & 0xffff) as u16
    }

    fn lod(self) -> u8 {
        self.0.to_le_bytes()[2]
    }

    fn class(self) -> usize {
        usize::from(self.0.to_le_bytes()[3])
    }

    fn is_free(self) -> bool {
        self.model() == NOBODY as u16
    }
}

/// The block that starts at a leaf.
#[derive(Clone, Copy, Debug)]
struct Slot {
    tag: Tag,
    /// The block's first vertex.
    first_vert: u32,
    /// The frame the block was last used in.
    last_used: i32,
    /// How many vertices the surface in the block really uses.
    filled_vertices: u32,
}

/// A node's neighbours in the free or used list it sits on.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Link {
    next: u16,
    prev: u16,
}

/// A tree's place in its bank's least-recently-used list, and what it is
/// carved for.
#[derive(Clone, Copy, Debug, Default)]
struct Tree {
    /// The neighbour used less recently than this tree.
    older: u16,
    /// The neighbour used more recently than this tree.
    newer: u16,
    /// The level the tree is carved for, or [`LEVEL_FREE`].
    level: i32,
    /// The frame the tree was last used in.
    touched_frame: i32,
}

/// The two ends of a bank's tree list.
#[derive(Clone, Copy, Debug, Default)]
struct TreeEnds {
    oldest: u16,
    newest: u16,
}

/// What a bank holds back for one size class.
#[derive(Clone, Copy, Debug, Default)]
struct ClassBook {
    /// The frame the class's used blocks were last swept in.
    swept: i32,
    /// How many more blocks may be handed out; negative means no limit.
    allowed: i32,
}

/// What asking the cache for a surface came to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CachePropFacet {
    /// The surface already had a block.
    Hit { cache_slot_no: u16 },
    /// The surface got a new block; the host must now fill it.
    Miss {
        cache_slot_no: u16,
        base_vertex_index: u32,
        vertex_points: u32,
    },
    /// The cache could not or would not take the surface.
    Declined,
}

/// Everything the block allocator remembers between calls.
pub struct PropCacheBlocks {
    leaves: Vec<Slot>,
    /// One node per leaf, then a free-list and a used-list head for every
    /// bank and class.
    links: Vec<Link>,
    trees: Vec<Tree>,
    ends: Vec<TreeEnds>,
    /// Per bank and class.
    books: Vec<ClassBook>,
    /// The frame being built.
    pub frame: i32,
    /// Surfaces given new blocks this frame.
    pub patched_surface_total: u32,
    /// Vertices given new blocks this frame.
    pub patched_vertex_total: u32,
}

impl Default for PropCacheBlocks {
    fn default() -> Self {
        Self::new()
    }
}

/// Index of a bank and class among the per-class tables.
fn book_of(bank: usize, class: usize) -> usize {
    bank * PROP_CLASS_COUNT + class
}

/// The list head standing for the free blocks of a bank and class. The heads
/// follow the leaf nodes, a free and a used head per bank and class.
fn vacant_list_front(bank_no: usize, size_class: usize) -> u16 {
    (LEAF_COUNT + 2 * book_of(bank_no, size_class)) as u16
}

/// The list head standing for the used blocks of a bank and class.
fn used_head(bank_no: usize, size_class: usize) -> u16 {
    1 + vacant_list_front(bank_no, size_class)
}

/// The leaf a cache index names: cache indexes count leaves from one.
fn leaf_of(cache_slot_no: u16) -> Option<usize> {
    let leaf = usize::from(cache_slot_no).checked_sub(1)?;
    (leaf < LEAF_COUNT).then_some(leaf)
}

fn tree_of(leaf: usize) -> usize {
    leaf / SMC_LEAVES_PER_TREE
}

fn bank_of(leaf: usize) -> usize {
    leaf / (SMC_LEAVES_PER_TREE * TREES_PER_BANK)
}

/// A neighbour in a bank's tree list, as stored: [`NO_TREE`] past either end.
fn tree_ref(tree: Option<usize>) -> u16 {
    tree.map_or(NO_TREE, |t| t as u16)
}

impl PropCacheBlocks {
    /// An empty allocator, starting at frame 0.
    pub fn new() -> Self {
        let blank = Slot { tag: Tag::UNSET, first_vert: 0, last_used: 0, filled_vertices: 0 };
        let mut made = Self {
            leaves: vec![blank; LEAF_COUNT],
            links: vec![Link::default(); LINK_COUNT],
            trees: vec![Tree::default(); TREE_COUNT],
            ends: vec![TreeEnds::default(); BANK_COUNT],
            books: vec![ClassBook::default(); BANK_COUNT * PROP_CLASS_COUNT],
            frame: 0,
            patched_surface_total: 0,
            patched_vertex_total: 0,
        };
        made.wipe(0);
        made
    }

    /// Forgets every block. Everything counts as already idle, so the first
    /// requests may carve trees at once.
    pub fn wipe(&mut self, frame_no: i32) {
        let idle = frame_no.wrapping_sub(CACHE_IDLE_FRAMES);
        (self.frame, self.patched_surface_total, self.patched_vertex_total) = (frame_no, 0, 0);
        let blank = Slot { tag: Tag::UNSET, first_vert: u32::MAX, last_used: idle, filled_vertices: 0 };
        self.leaves.iter_mut().for_each(|slot| *slot = blank);
        self.links.iter_mut().for_each(|link| *link = Link::default());
        self.books.iter_mut().for_each(|book| *book = ClassBook { swept: idle, allowed: -1 });

        // Each bank_no's trees start in index order, its first tree the oldest.
        for (bank_no, trees) in self.trees.chunks_mut(TREES_PER_BANK).enumerate().take(BANK_COUNT) {
            let first_tree = bank_no * TREES_PER_BANK;
            let last = trees.len() - 1;
            for (at, tree) in trees.iter_mut().enumerate() {
                *tree = Tree {
                    older: tree_ref(at.checked_sub(1).map(|a| first_tree + a)),
                    newer: tree_ref((at < last).then_some(first_tree + at + 1)),
                    level: LEVEL_FREE,
                    touched_frame: idle,
                };
            }
            self.ends[bank_no] = TreeEnds { oldest: first_tree as u16, newest: (first_tree + last) as u16 };
        }
        // Every list head starts as an empty ring around itself.
        for head in LEAF_COUNT..LEAF_COUNT + 2 * BANK_COUNT * PROP_CLASS_COUNT {
            let me = head as u16;
            self.links[head] = Link { next: me, prev: me };
        }
    }

    /// Starts the next frame: the patch counts begin again.
    pub fn begin_frame(&mut self) {
        self.frame = self.frame.wrapping_add(1);
        self.patched_surface_total = 0;
        self.patched_vertex_total = 0;
    }

    /// Whether something last used at `since` has gone unused long enough to
    /// be taken over.
    fn idle_since(&self, since: i32) -> bool {
        let gap = self.frame.wrapping_sub(since) as u32;
        gap >= CACHE_IDLE_FRAMES as u32
    }

    fn first_after(&self, node: u16) -> u16 {
        self.links[usize::from(node)].next
    }

    fn is_empty(&self, head: u16) -> bool {
        self.first_after(head) == head
    }

    /// Joins `prev` and `next` directly, dropping whatever sat between.
    fn join(&mut self, prev: u16, next: u16) {
        self.links[usize::from(prev)].next = next;
        self.links[usize::from(next)].prev = prev;
    }

    fn detach(&mut self, link: u16) {
        let around = self.links[usize::from(link)];
        self.join(around.prev, around.next);
    }

    /// Puts a leaf between two neighbouring nodes of a list.
    fn splice(&mut self, before: u16, link: u16, after: u16) {
        debug_assert!(usize::from(link) < LEAF_COUNT, "only leaves go on block lists");
        self.join(before, link);
        self.join(link, after);
    }

    fn link_first(&mut self, list_head: u16, link: u16) {
        let first = self.first_after(list_head);
        self.splice(list_head, link, first);
    }

    fn link_last(&mut self, list_head: u16, link: u16) {
        let last = self.links[usize::from(list_head)].prev;
        self.splice(last, link, list_head);
    }

    /// Makes `older` come right before `newer` in a bank's tree list; either
    /// may be [`NO_TREE`], which moves the list's end instead.
    fn chain_trees(&mut self, bank_no: usize, old_tree: u16, new_tree: u16) {
        match self.trees.get_mut(usize::from(old_tree)) {
            Some(tree) if old_tree != NO_TREE => tree.newer = new_tree,
            _ => self.ends[bank_no].oldest = new_tree,
        }
        match self.trees.get_mut(usize::from(new_tree)) {
            Some(tree) if new_tree != NO_TREE => tree.older = old_tree,
            _ => self.ends[bank_no].newest = old_tree,
        }
    }

    fn detach_subtree(&mut self, tree_no: usize, bank_no: usize) {
        let Tree { older, newer, .. } = self.trees[tree_no];
        self.chain_trees(bank_no, older, newer);
    }

    fn push_tree_newest(&mut self, tree_no: usize, bank_no: usize) {
        let newest = self.ends[bank_no].newest;
        self.chain_trees(bank_no, newest, tree_no as u16);
        self.chain_trees(bank_no, tree_no as u16, NO_TREE);
    }

    /// Marks a block used this frame and its tree the most recently used one in
    /// its bank.
    fn touch(&mut self, leaf: usize) {
        let now = self.frame;
        self.leaves[leaf].last_used = now;
        let tree = tree_of(leaf);
        if std::mem::replace(&mut self.trees[tree].touched_frame, now) == now {
            return;
        }
        let bank = bank_of(leaf);
        self.detach_subtree(tree, bank);
        self.push_tree_newest(tree, bank);
    }
}

/// The static model cache: an allocator paired with the host's model table.
pub struct StaticMeshResidencyCache<'a> {
    alloc: &'a mut PropCacheBlocks,
    table: &'a mut [SmcCacheRow],
}

impl<'a> StaticMeshResidencyCache<'a> {
    pub fn over(alloc: &'a mut PropCacheBlocks, table: &'a mut [SmcCacheRow]) -> Self {
        Self { alloc, table }
    }

    /// Empties the cache and the table with it.
    pub fn clear(&mut self, frame_no: i32) {
        self.table.iter_mut().for_each(|row| *row = [0; STATIC_MODEL_CACHE_DETAIL_LEVELS]);
        self.alloc.wipe(frame_no);
    }

    /// Finds or makes room for one static-model surface.
    ///
    /// A surface that already has a block (`cache_slot_no` nonzero) is marked
    /// used this frame. Otherwise a block is allocated from `bank`, of
    /// `size_band` (or the smallest class holding `vertex_points` when that is out of
    /// range), and recorded under `prop_index` and `lod`. Allocation is
    /// refused when the cache is `enabled == false`, when this frame's patch
    /// limits are spent, when the surface does not fit any class, or when
    /// nothing can be freed.
    #[allow(clippy::too_many_arguments)]
    pub fn cache_facet(
        &mut self,
        bank: u8,
        size_band: u8,
        prop_index: u16,
        lod: u8,
        cache_slot_no: u16,
        vertex_points: u32,
        on: bool,
    ) -> CachePropFacet {
        let granted = match (on, cache_slot_no) {
            (false, _) => None,
            (true, 0) => self.grant(bank, size_band, prop_index, lod, vertex_points),
            (true, held) => leaf_of(held).map(|leaf| {
                self.alloc.touch(leaf);
                CachePropFacet::Hit { cache_slot_no: held }
            }),
        };
        granted.unwrap_or(CachePropFacet::Declined)
    }

    /// A new block for a surface that has none, within this frame's patch limits.
    fn grant(&mut self, bank: u8, size_band: u8, prop_index: u16, lod: u8, vertex_points: u32) -> Option<CachePropFacet> {
        let class = match usize::from(size_band) {
            asked if asked < PROP_CLASS_COUNT => asked,
            _ => usize::from(size_class_for_vertex_count(vertex_points)?),
        };
        let block_verts = class_vertices(class);
        let room = self.alloc.patched_surface_total < PATCH_FACET_MAX
            && self.alloc.patched_vertex_total.saturating_add(block_verts) <= PATCH_VERTEX_MAX;
        if !room {
            return None;
        }
        let (row, column) = self.entry_pos(prop_index, lod)?;
        let cache_slot_no = self.allocate(usize::from(bank), class)?;
        self.table[row][column] = cache_slot_no;

        let leaf = usize::from(cache_slot_no) - 1;
        let slot = &mut self.alloc.leaves[leaf];
        (slot.tag, slot.filled_vertices) = (Tag::held(prop_index, lod, class), vertex_points);
        let base_vertex_index = slot.first_vert;
        self.alloc.patched_surface_total += 1;
        self.alloc.patched_vertex_total += block_verts;
        self.alloc.touch(leaf);
        Some(CachePropFacet::Miss { cache_slot_no, base_vertex_index, vertex_points: block_verts })
    }

    /// Gives a surface's block back. Indexes that are zero, out of range or
    /// already free are ignored.
    pub fn uncache_index(&mut self, slot_no: u16) {
        let Some(leaf) = leaf_of(slot_no) else {
            return;
        };
        let class = self.alloc.leaves[leaf].tag.class();
        if class < PROP_CLASS_COUNT && !self.alloc.leaves[leaf].tag.is_free() {
            self.release_block(leaf);
            self.free_merging(bank_of(leaf), class, leaf);
        }
    }

    /// Where the table records a model's level of detail.
    fn entry_pos(&self, prop_index: u16, lod: u8) -> Option<(usize, usize)> {
        let row = usize::from(prop_index);
        let column = usize::from(lod);
        let inside = row < self.table.len() && column < STATIC_MODEL_CACHE_DETAIL_LEVELS;
        inside.then_some((row, column))
    }

    /// Takes a free block of the class out of the bank, making one if needed,
    /// and returns its cache index.
    fn allocate(&mut self, bank_no: usize, size_class: usize) -> Option<u16> {
        if bank_no >= BANK_COUNT || size_class >= PROP_CLASS_COUNT {
            return None;
        }
        let book = book_of(bank_no, size_class);
        let allowed = self.alloc.books[book].allowed;
        let has_free = allowed != 0
            && (!self.alloc.is_empty(vacant_list_front(bank_no, size_class)) || self.make_free_block(bank_no, size_class));
        if !has_free {
            return None;
        }
        if allowed > 0 {
            self.alloc.books[book].allowed = allowed - 1;
        }
        let block = self.alloc.first_after(vacant_list_front(bank_no, size_class));
        self.alloc.detach(block);
        self.alloc.link_first(used_head(bank_no, size_class), block);
        let first_vert = leaf_base_vert(class_level(size_class), u32::from(block));
        self.alloc.leaves[usize::from(block)].first_vert = first_vert;
        Some(block + 1)
    }

    /// Makes sure the class has a free block: by reclaiming idle blocks, by
    /// splitting a block of the class above, or, for the top class of a level,
    /// by carving a tree.
    fn make_free_block(&mut self, bank_no: usize, size_class: usize) -> bool {
        self.sweep_idle_blocks(bank_no, size_class);
        let head = vacant_list_front(bank_no, size_class);
        if !self.alloc.is_empty(head) {
            return true;
        }
        match class_parent(size_class) {
            None => self.carve_tree(bank_no, size_class),
            Some(parent) => self.split_from(bank_no, parent, size_class),
        }
    }

    /// Splits a free block of `parent` into its two `class` halves; the low
    /// half ends up first on the free list, so it is handed out first.
    fn split_from(&mut self, bank_no: usize, parent_no: usize, size_class: usize) -> bool {
        let parent_head = vacant_list_front(bank_no, parent_no);
        let available = !self.alloc.is_empty(parent_head) || self.make_free_block(bank_no, parent_no);
        if !available {
            return false;
        }
        let low = self.alloc.first_after(parent_head);
        self.alloc.detach(low);
        let high = low + class_stride(size_class);
        debug_assert!(usize::from(high) < LEAF_COUNT);
        let head = vacant_list_front(bank_no, size_class);
        for half in [high, low] {
            self.alloc.leaves[usize::from(half)].tag = Tag::free(size_class);
            self.alloc.link_first(head, half);
        }
        true
    }

    /// Frees the class's used blocks that have gone idle, smaller classes
    /// first. A class is swept at most once in a few frames.
    fn sweep_idle_blocks(&mut self, bank_no: usize, size_class: usize) {
        let book = book_of(bank_no, size_class);
        if !self.alloc.idle_since(self.alloc.books[book].swept) {
            return;
        }
        self.alloc.books[book].swept = self.alloc.frame;
        if let Some(child) = class_child(size_class) {
            self.sweep_idle_blocks(bank_no, child);
        }
        let head = used_head(bank_no, size_class);
        let mut at = self.alloc.first_after(head);
        // A healthy list is shorter than the leaf count; the bound only keeps a
        // damaged one from spinning.
        let mut budget = LEAF_COUNT;
        while at != head && budget > 0 {
            budget -= 1;
            let following = self.alloc.first_after(at);
            let idle = self.alloc.idle_since(self.alloc.leaves[usize::from(at)].last_used);
            if idle {
                self.uncache_index(at + 1);
            }
            at = following;
        }
    }

    /// Empties the bank's least recently used tree, if it has been idle long
    /// enough, and carves it for the level of `class` (the top class of its
    /// level): the level's blocks go on the class's free list.
    fn carve_tree(&mut self, bank_no: usize, size_class: usize) -> bool {
        let Some(tree) = Some(self.alloc.ends[bank_no].oldest).filter(|&t| t != NO_TREE).map(usize::from) else {
            return false;
        };
        if !self.alloc.idle_since(self.alloc.trees[tree].touched_frame) {
            return false;
        }
        self.alloc.detach_subtree(tree, bank_no);
        self.empty_tree(tree);
        let level = class_level(size_class);
        self.alloc.trees[tree].level = level as i32;
        let stride = usize::from(class_stride(size_class));
        let head = vacant_list_front(bank_no, size_class);
        let blocks = (0..level_blocks(level)).map(|n| tree * SMC_LEAVES_PER_TREE + n * stride);
        for leaf in blocks {
            self.alloc.leaves[leaf].tag = Tag::free(size_class);
            self.alloc.link_last(head, leaf as u16);
        }
        self.alloc.trees[tree].touched_frame = self.alloc.frame;
        self.alloc.push_tree_newest(tree, bank_no);
        true
    }

    /// Takes every block of a carved tree off its lists, uncaching the surfaces
    /// that used them, and marks the tree uncarved.
    fn empty_tree(&mut self, tree: usize) {
        let was = std::mem::replace(&mut self.alloc.trees[tree].level, LEVEL_FREE);
        let level = match usize::try_from(was) {
            Ok(level) if level < LEVEL_FREE as usize => level,
            _ => return,
        };
        let start = tree * SMC_LEAVES_PER_TREE;
        let stop = start + usize::from(level_leaf_range(level));
        let mut at = start;
        while at < stop {
            let tag = self.alloc.leaves[at].tag;
            let class = tag.class();
            if class >= PROP_CLASS_COUNT {
                debug_assert!(false, "carved tree {tree} has a leaf without a class: {at}");
                return;
            }
            match tag.is_free() {
                true => self.alloc.detach(at as u16),
                false => self.release_block(at),
            }
            at += usize::from(class_stride(class));
        }
    }

    /// Drops the surface in a used block: its cache index is cleared and the
    /// block is tagged free and taken off the used list. It is on no list
    /// afterwards.
    fn release_block(&mut self, leaf: usize) {
        let slot = &mut self.alloc.leaves[leaf];
        let held = slot.tag;
        (slot.tag, slot.filled_vertices) = (Tag::free(held.class()), 0);
        if let Some((row, column)) = self.entry_pos(held.model(), held.lod()) {
            self.table[row][column] = 0;
        }
        self.alloc.detach(leaf as u16);
    }

    /// Puts a block that is on no list onto its class's free list, first
    /// merging it with its twin for as long as the twin is free as well.
    fn free_merging(&mut self, bank_no: usize, size_class: usize, leaf_no: usize) {
        let (mut size_class, mut leaf_no) = (size_class, leaf_no);
        loop {
            let Some(parent) = class_parent(size_class) else { break };
            let stride = usize::from(class_stride(size_class));
            let twin = leaf_no ^ stride;
            let twin_free = self.alloc.leaves.get(twin).is_some_and(|slot| slot.tag == Tag::free(size_class));
            if !twin_free {
                break;
            }
            self.alloc.detach(twin as u16);
            (size_class, leaf_no) = (parent, leaf_no & !stride);
            self.alloc.leaves[leaf_no].tag = Tag::free(size_class);
        }
        self.alloc.link_first(vacant_list_front(bank_no, size_class), leaf_no as u16);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::smc_layout::BANK_VERTEX_CAP;

    /// An allocator and a model table, owned together so tests can look inside.
    struct Host {
        alloc: PropCacheBlocks,
        table: Vec<SmcCacheRow>,
    }

    impl Host {
        fn cache(&mut self) -> StaticMeshResidencyCache<'_> {
            StaticMeshResidencyCache::over(&mut self.alloc, &mut self.table)
        }
    }

    fn started(models: usize, frame: i32) -> Host {
        let mut host = Host {
            alloc: PropCacheBlocks::new(),
            table: vec![[0; STATIC_MODEL_CACHE_DETAIL_LEVELS]; models],
        };
        host.cache().clear(frame);
        host
    }

    /// Asks for a surface in a fresh slot and a fresh patch budget.
    fn add(
        cache: &mut StaticMeshResidencyCache<'_>,
        bank: u8,
        class: u8,
        model: u16,
        lod: u8,
    ) -> CachePropFacet {
        cache.alloc.patched_surface_total = 0;
        cache.alloc.patched_vertex_total = 0;
        cache.cache_facet(
            bank,
            class,
            model,
            lod,
            0,
            class_vertices(usize::from(class)),
            true,
        )
    }

    fn miss(result: CachePropFacet) -> (u16, u32, u32) {
        match result {
            CachePropFacet::Miss {
                cache_slot_no,
                base_vertex_index,
                vertex_points,
            } => (cache_slot_no, base_vertex_index, vertex_points),
            other => panic!("expected a miss, got {other:?}"),
        }
    }

    /// The nodes on the list behind `head`, front to back.
    fn walk(alloc: &PropCacheBlocks, head: u16) -> Vec<u16> {
        let mut out = Vec::new();
        let mut node = alloc.first_after(head);
        while node != head {
            out.push(node);
            node = alloc.first_after(node);
            assert!(out.len() <= LEAF_COUNT, "list does not close");
        }
        out
    }

    fn free_list(blocks: &PropCacheBlocks, bank_no: usize, size_class: usize) -> Vec<u16> {
        walk(blocks, vacant_list_front(bank_no, size_class))
    }

    fn used_list(blocks: &PropCacheBlocks, bank_no: usize, size_class: usize) -> Vec<u16> {
        walk(blocks, used_head(bank_no, size_class))
    }

    #[test]
    fn a_cleared_cache_holds_nothing() {
        let host = started(8, 100);
        let alloc = &host.alloc;
        assert!(alloc.leaves.iter().all(|leaf| leaf.tag == Tag::UNSET));
        assert!(alloc.leaves.iter().all(|leaf| leaf.last_used == 96));
        assert!(alloc.books.iter().all(|book| book.allowed == -1));
        assert!(alloc.books.iter().all(|book| book.swept == 96));
        for bank in 0..BANK_COUNT {
            for class in 0..PROP_CLASS_COUNT {
                assert!(free_list(alloc, bank, class).is_empty());
                assert!(used_list(alloc, bank, class).is_empty());
            }
            assert_eq!(alloc.ends[bank].oldest, (bank * 32) as u16);
            assert_eq!(alloc.ends[bank].newest, (bank * 32 + 31) as u16);
        }
    }

    #[test]
    fn clearing_forgets_the_table_too() {
        let mut host = started(4, 10);
        miss(add(&mut host.cache(), 0, 3, 2, 1));
        assert_ne!(host.table[2][1], 0);
        host.cache().clear(11);
        assert!(host.table.iter().flatten().all(|&index| index == 0));
    }

    #[test]
    fn the_first_block_of_a_bank_is_its_first_vertices() {
        let mut host = started(4, 10);
        let mut cache = host.cache();
        let (first, base, vertex_points) = miss(add(&mut cache, 0, 0, 1, 0));
        assert_eq!((first, base, vertex_points), (1, 0, 16));
        let (second, base, vertex_points) = miss(add(&mut cache, 0, 0, 1, 1));
        assert_eq!((second, base, vertex_points), (2, 16, 16));
        assert_eq!(cache.table[1][..2], [1, 2]);
    }

    #[test]
    fn each_bank_hands_out_its_own_vertices() {
        let mut host = started(BANK_COUNT, 10);
        let mut cache = host.cache();
        for bank in 0..BANK_COUNT as u8 {
            let (_, base, _) = miss(add(&mut cache, bank, 24, bank.into(), 0));
            assert_eq!(base, u32::from(bank) * BANK_VERTEX_CAP);
        }
    }

    #[test]
    fn a_new_block_records_its_owner() {
        let mut host = started(8, 10);
        let mut cache = host.cache();
        let (index, ..) = miss(add(&mut cache, 2, 5, 6, 3));
        let leaf = cache.alloc.leaves[usize::from(index - 1)];
        assert_eq!(leaf.tag, Tag::held(6, 3, 5));
        assert_eq!((leaf.filled_vertices, leaf.last_used), (42, 10));
        assert_eq!(cache.table[6][3], index);
        assert_eq!(
            (cache.alloc.patched_surface_total, cache.alloc.patched_vertex_total),
            (1, 42)
        );
    }

    #[test]
    fn a_tag_reads_back_what_it_was_made_from() {
        let tag = Tag::held(0x1234, 3, 17);
        assert_eq!((tag.model(), tag.lod(), tag.class()), (0x1234, 3, 17));
        assert!(!tag.is_free());
        assert!(Tag::free(9).is_free());
        assert_eq!(Tag::free(9).class(), 9);
    }

    #[test]
    fn asking_again_with_the_index_is_a_hit() {
        let mut host = started(4, 10);
        let mut cache = host.cache();
        let (index, ..) = miss(add(&mut cache, 0, 8, 0, 0));
        cache.alloc.frame = 12;
        assert_eq!(
            cache.cache_facet(0, 8, 0, 0, index, 64, true),
            CachePropFacet::Hit { cache_slot_no: index }
        );
        assert_eq!(cache.alloc.leaves[usize::from(index - 1)].last_used, 12);
    }

    #[test]
    fn a_disabled_cache_refuses_everything() {
        let mut host = started(4, 10);
        let mut cache = host.cache();
        let (index, ..) = miss(add(&mut cache, 0, 8, 0, 0));
        assert_eq!(
            cache.cache_facet(0, 8, 1, 0, 0, 64, false),
            CachePropFacet::Declined
        );
        assert_eq!(
            cache.cache_facet(0, 8, 0, 0, index, 64, false),
            CachePropFacet::Declined
        );
    }

    #[test]
    fn a_hit_on_an_index_that_names_no_leaf_is_refused() {
        let mut host = started(4, 10);
        let mut cache = host.cache();
        let past = LEAF_COUNT as u16 + 1;
        assert_eq!(
            cache.cache_facet(0, 8, 0, 0, past, 64, true),
            CachePropFacet::Declined
        );
    }

    #[test]
    fn the_class_comes_from_the_vertex_count_when_the_given_one_is_out_of_range() {
        let mut host = started(4, 10);
        let mut cache = host.cache();
        let result = cache.cache_facet(0, 200, 0, 0, 0, 100, true);
        let (_, _, vertex_points) = miss(result);
        assert_eq!(vertex_points, 102);
        cache.alloc.patched_surface_total = 0;
        assert_eq!(
            cache.cache_facet(0, 200, 1, 0, 0, 1025, true),
            CachePropFacet::Declined
        );
    }

    #[test]
    fn a_surface_with_no_place_to_record_its_index_is_refused_untouched() {
        let mut host = started(2, 10);
        let mut cache = host.cache();
        assert_eq!(
            cache.cache_facet(0, 3, 2, 0, 0, 32, true),
            CachePropFacet::Declined
        );
        assert_eq!(
            cache.cache_facet(0, 3, 0, 4, 0, 32, true),
            CachePropFacet::Declined
        );
        assert_eq!(
            (cache.alloc.patched_surface_total, cache.alloc.patched_vertex_total),
            (0, 0)
        );
        assert!(cache.alloc.leaves.iter().all(|leaf| leaf.tag == Tag::UNSET));
    }

    #[test]
    fn a_bank_that_does_not_exist_gives_nothing() {
        let mut host = started(2, 10);
        let mut cache = host.cache();
        assert_eq!(
            cache.cache_facet(6, 3, 0, 0, 0, 32, true),
            CachePropFacet::Declined
        );
    }

    #[test]
    fn a_frame_takes_at_most_so_many_surfaces() {
        let mut host = started(400, 10);
        let mut cache = host.cache();
        for model in 0..PATCH_FACET_MAX as u16 {
            let result = cache.cache_facet(0, 0, model, 0, 0, 16, true);
            assert!(
                matches!(result, CachePropFacet::Miss { .. }),
                "surface {model}"
            );
        }
        assert_eq!(
            cache.cache_facet(0, 0, 300, 0, 0, 16, true),
            CachePropFacet::Declined
        );
    }

    #[test]
    fn a_frame_takes_at_most_so_many_vertices() {
        let mut host = started(40, 10);
        let mut cache = host.cache();
        for model in 0..8 {
            miss(cache.cache_facet(0, 24, model, 0, 0, 1024, true));
        }
        assert_eq!(cache.alloc.patched_vertex_total, PATCH_VERTEX_MAX);
        assert_eq!(
            cache.cache_facet(0, 0, 20, 0, 0, 16, true),
            CachePropFacet::Declined
        );
    }

    #[test]
    fn a_new_frame_starts_the_patch_counts_again() {
        let mut host = started(4, 10);
        miss(host.cache().cache_facet(0, 3, 0, 0, 0, 32, true));
        host.alloc.begin_frame();
        assert_eq!(host.alloc.frame, 11);
        assert_eq!((host.alloc.patched_surface_total, host.alloc.patched_vertex_total), (0, 0));
    }

    #[test]
    fn a_spent_budget_refuses_that_class_in_that_bank() {
        let mut host = started(8, 10);
        let mut cache = host.cache();
        let book = 2 * PROP_CLASS_COUNT + 7;
        cache.alloc.books[book].allowed = 1;
        miss(add(&mut cache, 2, 7, 0, 0));
        assert_eq!(cache.alloc.books[book].allowed, 0);
        assert_eq!(
            add(&mut cache, 2, 7, 1, 0),
            CachePropFacet::Declined
        );
        miss(add(&mut cache, 2, 8, 1, 0));
        miss(add(&mut cache, 3, 7, 1, 1));
    }

    #[test]
    fn live_blocks_never_share_vertices() {
        let mut host = started(2000, 10);
        let mut cache = host.cache();
        let mut alive = Vec::new();
        let mut seed = 12345u32;
        for model in 0..1500u16 {
            seed = seed.wrapping_mul(1_103_515_245).wrapping_add(12345);
            let bank = ((seed >> 8) % 3) as u8;
            let class = ((seed >> 16) % PROP_CLASS_COUNT as u32) as u8;
            if let CachePropFacet::Miss {
                base_vertex_index,
                vertex_points,
                ..
            } = add(&mut cache, bank, class, model, 0)
            {
                let window = u32::from(bank) * BANK_VERTEX_CAP;
                assert!(base_vertex_index >= window);
                assert!(base_vertex_index + vertex_points <= window + BANK_VERTEX_CAP);
                alive.push((base_vertex_index, base_vertex_index + vertex_points));
            }
        }
        assert!(alive.len() > 100);
        alive.sort_unstable();
        for pair in alive.windows(2) {
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
        assert_eq!(free_list(&host.alloc, 0, 0), vec![0]);
        host.cache().uncache_index(second);
        // The two smallest blocks merge, and so on all the way up: one whole
        // 1024-vertex block is free again and nothing smaller is.
        assert_eq!(free_list(&host.alloc, 0, 24), vec![0, 64]);
        for class in 0..24 {
            assert!(
                free_list(&host.alloc, 0, class).is_empty(),
                "class {class}"
            );
        }
        assert!(host.table.iter().flatten().all(|&index| index == 0));
        assert_eq!(host.alloc.leaves[0].tag, Tag::free(24));
    }

    #[test]
    fn freeing_one_half_leaves_the_other_where_it_was() {
        let mut host = started(4, 10);
        let mut cache = host.cache();
        let (first, ..) = miss(add(&mut cache, 0, 0, 0, 0));
        miss(add(&mut cache, 0, 0, 1, 0));
        cache.uncache_index(first);
        assert_eq!(cache.alloc.leaves[0].tag, Tag::free(0));
        assert_eq!(cache.alloc.leaves[0].filled_vertices, 0);
        assert_eq!(cache.table[0][0], 0);
        assert_ne!(cache.table[1][0], 0);
    }

    #[test]
    fn freeing_nothing_in_particular_does_nothing() {
        let mut host = started(4, 10);
        let mut cache = host.cache();
        let (index, ..) = miss(add(&mut cache, 0, 4, 0, 0));
        cache.uncache_index(0);
        cache.uncache_index(LEAF_COUNT as u16 + 1);
        cache.uncache_index(u16::MAX);
        cache.uncache_index(index);
        let lists_before = cache.alloc.links.clone();
        cache.uncache_index(index);
        assert_eq!(cache.alloc.links, lists_before);
        assert_eq!(cache.table[0][0], 0);
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
            CachePropFacet::Declined
        );
        cache.alloc.frame = 13;
        assert_eq!(
            add(&mut cache, 0, 24, 64, 0),
            CachePropFacet::Declined
        );
        cache.alloc.frame = 14;
        let (_, base, _) = miss(add(&mut cache, 0, 24, 64, 0));
        assert!(base < BANK_VERTEX_CAP);
        // Only the blocks that were not needed again were taken back.
        let held = cache.table[..64].iter().filter(|row| row[0] != 0).count();
        assert_eq!(held, 0);
        assert_ne!(cache.table[64][0], 0);
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
            cache.alloc.frame = frame;
            assert_eq!(
                cache.cache_facet(0, 24, 0, 0, kept, 1024, true),
                CachePropFacet::Hit { cache_slot_no: kept }
            );
            let result = add(&mut cache, 0, 24, 64, 0);
            if let CachePropFacet::Miss { cache_slot_no, .. } = result {
                cache.uncache_index(cache_slot_no);
            }
            assert_eq!(cache.table[0][0], kept, "frame {frame}");
        }
    }

    #[test]
    fn an_idle_tree_is_recarved_for_a_different_level() {
        let mut host = started(100, 10);
        let mut cache = host.cache();
        for model in 0..64 {
            miss(add(&mut cache, 0, 24, model, 0));
        }
        cache.alloc.frame = 20;
        // Class 23 belongs to another level, so a tree has to be emptied.
        let (index, base, vertex_points) = miss(add(&mut cache, 0, 23, 70, 0));
        assert_eq!(vertex_points, 682);
        assert_eq!(base % 0x800, 0);
        let tree = usize::from(index - 1) / SMC_LEAVES_PER_TREE;
        assert_eq!(cache.alloc.trees[tree].level, 1);
        // The models that lived in that tree lost their blocks; the rest kept
        // theirs.
        let lost = cache.table[..64].iter().filter(|row| row[0] == 0).count();
        assert_eq!(lost, 2);
    }

    #[test]
    fn the_least_recently_used_tree_is_carved_first() {
        let mut host = started(100, 10);
        let mut cache = host.cache();
        assert_eq!(cache.alloc.ends[0].oldest, 0);
        miss(add(&mut cache, 0, 24, 0, 0));
        // The first tree was carved and is now the newest one.
        assert_eq!(cache.alloc.ends[0].newest, 0);
        assert_eq!(cache.alloc.ends[0].oldest, 1);
        miss(add(&mut cache, 0, 24, 1, 0));
        miss(add(&mut cache, 0, 24, 2, 0));
        assert_eq!(cache.alloc.ends[0].newest, 1);
        assert_eq!(cache.alloc.ends[0].oldest, 2);
    }

    #[test]
    fn touching_a_block_makes_its_tree_the_newest_in_its_bank() {
        let mut host = started(100, 10);
        let mut cache = host.cache();
        let (first, ..) = miss(add(&mut cache, 0, 24, 0, 0));
        miss(add(&mut cache, 0, 24, 1, 0));
        miss(add(&mut cache, 0, 24, 2, 0));
        assert_eq!(cache.alloc.ends[0].newest, 1);
        cache.alloc.frame = 11;
        cache.cache_facet(0, 24, 0, 0, first, 1024, true);
        assert_eq!(cache.alloc.ends[0].newest, 0);
        let mut sequence = Vec::new();
        let mut tree = cache.alloc.ends[0].newest;
        while tree != NO_TREE {
            sequence.push(tree);
            tree = cache.alloc.trees[usize::from(tree)].older;
        }
        assert_eq!(sequence.len(), TREES_PER_BANK);
        assert_eq!(&sequence[..3], &[0, 1, 31]);
        assert_eq!(sequence.last(), Some(&2));
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
                cache.alloc.frame = 10 + (step / 40) as i32;
                if seed >> 28 < 10 || live.is_empty() {
                    let class = ((seed >> 8) % 12) as u8;
                    let model = (step % 600) as u16;
                    if cache.table[usize::from(model)][0] != 0 {
                        continue;
                    }
                    if let CachePropFacet::Miss { cache_slot_no, .. } =
                        add(&mut cache, 0, class, model, 0)
                    {
                        live.push(cache_slot_no);
                    }
                } else {
                    let at = (seed >> 4) as usize % live.len();
                    cache.uncache_index(live.swap_remove(at));
                }
            }
        }
        // Every list closes on itself and agrees in both directions.
        let alloc = &host.alloc;
        for class in 0..PROP_CLASS_COUNT {
            for head in [vacant_list_front(0, class), used_head(0, class)] {
                for node in walk(alloc, head) {
                    let next = alloc.links[usize::from(node)].next;
                    assert_eq!(alloc.links[usize::from(next)].prev, node);
                }
            }
        }
    }
}
