//! Shape of the source packet.
//!
//! The packet is a flat run of dwords. Each list the backend walks is announced
//! by a token in a pointer slot plus a count (or a byte span) in its neighbour;
//! the slots sit at fixed byte offsets, written here as `dword(offset)`.

use super::{StaticModelRigidRecord, TriangleRunEntry, RigidModelBatchRecord};

const fn dword(byte_offset: usize) -> usize {
    byte_offset >> 2
}

/// Length of the packet, in dwords.
pub const DRAWLIST_BLOCK_WORDS: usize = dword(0x74);

/// Stored in a pointer slot to say "this list is present".
pub const DRAWLIST_MARKER: u32 = 1;

pub const SOURCE_WORLD_PTR: usize = dword(0x08);
pub const SOURCE_WORLD_COUNT: usize = dword(0x0c);

pub const SOURCE_XMODEL_RIGID_PTR: usize = dword(0x50);
pub const SOURCE_XMODEL_RIGID_COUNT: usize = dword(0x54);

pub const SOURCE_SMODEL_RIGID_PTR: usize = dword(0x58);
pub const SOURCE_SMODEL_RIGID_COUNT: usize = dword(0x5c);

pub const SOURCE_SMODEL_SKINNED_BYTES: usize = dword(0x1c);
pub const SOURCE_SMODEL_SKINNED_PTR: usize = dword(0x20);

pub const SOURCE_SMODEL_CACHED_BYTES: usize = dword(0x28);
pub const SOURCE_SMODEL_CACHED_PTR: usize = dword(0x2c);

pub const SOURCE_SMODEL_PRETESS_BYTES: usize = dword(0x34);
pub const SOURCE_SMODEL_PRETESS_PTR: usize = dword(0x38);

/// One shadow pass's worth of entries, with the header that announces them.
///
/// Every `*_draw_indices` vector runs parallel to the entry list of the same
/// stem: element `i` is the position, in the pass's draw order, of the draw
/// that produced entry `i`.
#[derive(Clone, Debug, Default)]
pub struct CompactFrontendLists {
    pub source_words: [u32; DRAWLIST_BLOCK_WORDS],
    pub world: Vec<TriangleRunEntry>,
    pub level_draw_indices: Vec<u32>,
    pub smodel: Vec<StaticModelRigidRecord>,
    pub prop_draw_indices: Vec<u32>,
    pub xmodel: Vec<RigidModelBatchRecord>,
    pub model_draw_ix: Vec<u32>,
    pub smodel_cached: Vec<StaticModelRigidRecord>,
    pub prop_cached_draw_indices: Vec<u32>,
    pub smodel_pretess: Vec<StaticModelRigidRecord>,
    pub prop_prebuilt_draw_indices: Vec<u32>,
    pub smodel_skinned: Vec<StaticModelRigidRecord>,
    pub prop_skinned_draw_indices: Vec<u32>,
    /// Draws that never reach a list (kinds a shadow pass does not draw).
    pub skipped_misc_total: u32,
    /// Draws dropped because their surface has no whole triangle to draw.
    pub skipped_no_indices_total: u32,
}

impl CompactFrontendLists {
    /// How many entries the backend will walk, over every list.
    #[must_use]
    pub fn work_entry_tally(&self) -> u32 {
        let lens = [
            self.world.len(),
            self.smodel.len(),
            self.xmodel.len(),
            self.smodel_cached.len(),
            self.smodel_pretess.len(),
            self.smodel_skinned.len(),
        ];
        lens.iter().sum::<usize>() as u32
    }
}
