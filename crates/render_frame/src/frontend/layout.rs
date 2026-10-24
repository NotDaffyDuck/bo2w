//! Shape of the source packet.
//!
//! The packet is a flat run of dwords. Each list the backend walks is announced
//! by a token in a pointer slot plus a count (or a byte span) in its neighbour;
//! the slots sit at fixed byte offsets, written here as `dword(offset)`.

use super::{GfxSmodelRigidEntry, GfxTrianglesListEntry, GfxXModelRigidEntry};

const fn dword(byte_offset: usize) -> usize {
    byte_offset >> 2
}

/// Length of the packet, in dwords.
pub const FRONTEND_DRAW_LISTS_DWORDS: usize = dword(0x74);

/// Stored in a pointer slot to say "this list is present".
pub const LIST_TOKEN: u32 = 1;

pub const SRC_WORLD_PTR: usize = dword(0x08);
pub const SRC_WORLD_COUNT: usize = dword(0x0c);

pub const SRC_XMODEL_RIGID_PTR: usize = dword(0x50);
pub const SRC_XMODEL_RIGID_COUNT: usize = dword(0x54);

pub const SRC_SMODEL_RIGID_PTR: usize = dword(0x58);
pub const SRC_SMODEL_RIGID_COUNT: usize = dword(0x5c);

pub const SRC_SMODEL_SKINNED_BYTES: usize = dword(0x1c);
pub const SRC_SMODEL_SKINNED_PTR: usize = dword(0x20);

pub const SRC_SMODEL_CACHED_BYTES: usize = dword(0x28);
pub const SRC_SMODEL_CACHED_PTR: usize = dword(0x2c);

pub const SRC_SMODEL_PRETESS_BYTES: usize = dword(0x34);
pub const SRC_SMODEL_PRETESS_PTR: usize = dword(0x38);

/// One shadow pass's worth of entries, with the header that announces them.
///
/// Every `*_draw_indices` vector runs parallel to the entry list of the same
/// stem: element `i` is the position, in the pass's draw order, of the draw
/// that produced entry `i`.
#[derive(Clone, Debug, Default)]
pub struct PackedFrontendLists {
    pub src: [u32; FRONTEND_DRAW_LISTS_DWORDS],
    pub world: Vec<GfxTrianglesListEntry>,
    pub world_draw_indices: Vec<u32>,
    pub smodel: Vec<GfxSmodelRigidEntry>,
    pub smodel_draw_indices: Vec<u32>,
    pub xmodel: Vec<GfxXModelRigidEntry>,
    pub xmodel_draw_indices: Vec<u32>,
    pub smodel_cached: Vec<GfxSmodelRigidEntry>,
    pub smodel_cached_draw_indices: Vec<u32>,
    pub smodel_pretess: Vec<GfxSmodelRigidEntry>,
    pub smodel_pretess_draw_indices: Vec<u32>,
    pub smodel_skinned: Vec<GfxSmodelRigidEntry>,
    pub smodel_skinned_draw_indices: Vec<u32>,
    /// Draws that never reach a list (kinds a shadow pass does not draw).
    pub skipped_other: u32,
    /// Draws dropped because their surface has no whole triangle to draw.
    pub skipped_empty_ib: u32,
}

impl PackedFrontendLists {
    /// How many entries the backend will walk, over every list.
    #[must_use]
    pub fn work_entry_n(&self) -> u32 {
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
