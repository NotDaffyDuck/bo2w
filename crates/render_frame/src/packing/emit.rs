//! The per-draw half of packing: which list a draw lands in, and what its
//! entry holds.

use super::{HOST_XMODEL_RIGID_TESS_INFO_PACKED_ARM, PackDraw, PackKind, backend_setup_key};
use crate::frontend::{
    GfxSmodelRigidEntry, GfxTrianglesListEntry, GfxXModelRigidEntry, PackedFrontendLists,
    SmodelPretessRange,
};

/// Packing state: the lists being built and the tables draws are resolved in.
pub(super) struct Packer<'a> {
    pub(super) lists: PackedFrontendLists,
    pub(super) run_surfs: &'a [u16],
    pub(super) world_ranges: &'a [(u32, u32)],
    pub(super) world_vertex_count: u32,
    pub(super) smodel_ranges: &'a [(u32, u32)],
    pub(super) xmodel_ranges: &'a [(u32, u32)],
}

/// First index and whole-triangle count of `table[at]`, when it holds one.
fn drawable(table: &[(u32, u32)], at: usize) -> Option<(u32, u16)> {
    let &(first, indices) = table.get(at)?;
    (indices >= 3).then_some((first, (indices / 3) as u16))
}

/// Byte offset of an index in the 16-bit pool.
fn byte_offset(first_index: u32) -> u32 {
    first_index.saturating_mul(2)
}

impl<'a> Packer<'a> {
    pub(super) fn feed(&mut self, ordinal: u32, draw: &PackDraw) {
        let sort = backend_setup_key(draw.key, draw.material_rank);
        match draw.kind {
            PackKind::World { surf, run, run_off } => {
                let lead = [surf];
                let members = self.run_members(run, run_off).unwrap_or(&lead);
                for &member in members {
                    self.world_surface(ordinal, sort, member);
                }
            }
            PackKind::SmodelRigid { surface, lighting_handle } => {
                let hit = drawable(self.smodel_ranges, surface as usize);
                let lists = &mut self.lists;
                match hit {
                    Some((first, tris)) => {
                        lists.smodel.push(smodel_entry(sort, tris, first, lighting_handle));
                        lists.smodel_draw_indices.push(ordinal);
                    }
                    None => lists.skipped_empty_ib += 1,
                }
            }
            PackKind::SmodelSkinned { surface, lighting_handle } => {
                let hit = drawable(self.smodel_ranges, surface as usize);
                let lists = &mut self.lists;
                match hit {
                    Some((first, tris)) => {
                        lists.smodel_skinned.push(smodel_entry(sort, tris, first, lighting_handle));
                        lists.smodel_skinned_draw_indices.push(ordinal);
                    }
                    None => lists.skipped_empty_ib += 1,
                }
            }
            PackKind::SmodelPretess { lighting_handle, dest } => {
                if let Some(entry) = dest_entry(sort, lighting_handle, dest) {
                    self.lists.smodel_pretess.push(entry);
                    self.lists.smodel_pretess_draw_indices.push(ordinal);
                }
            }
            PackKind::SmodelCached { lighting_handle, dest } => {
                if let Some(entry) = dest_entry(sort, lighting_handle, dest) {
                    self.lists.smodel_cached.push(entry);
                    self.lists.smodel_cached_draw_indices.push(ordinal);
                }
            }
            PackKind::XModel { surface, lighting_handle } => {
                match drawable(self.xmodel_ranges, surface as usize) {
                    Some((first, tris)) => {
                        self.lists.xmodel.push(GfxXModelRigidEntry::new(
                            sort,
                            byte_offset(first),
                            tris,
                            HOST_XMODEL_RIGID_TESS_INFO_PACKED_ARM,
                            lighting_handle,
                        ));
                        self.lists.xmodel_draw_indices.push(ordinal);
                    }
                    None => self.lists.skipped_empty_ib += 1,
                }
            }
            PackKind::Skip => self.lists.skipped_other += 1,
        }
    }

    /// The surfaces a world draw stands for: the run table slice when the draw is
    /// a run and the whole slice exists, otherwise just the lead surface.
    fn run_members(&self, run: u16, run_off: u32) -> Option<&'a [u16]> {
        if run <= 1 {
            return None;
        }
        let from = run_off as usize;
        let to = from.saturating_add(usize::from(run));
        let slice = self.run_surfs.get(from..to)?;
        (slice.len() == usize::from(run)).then_some(slice)
    }

    fn world_surface(&mut self, ordinal: u32, sort: u32, surf: u16) {
        let Some((first, tris)) = drawable(self.world_ranges, usize::from(surf)) else {
            self.lists.skipped_empty_ib += 1;
            return;
        };
        self.lists.world.push(GfxTrianglesListEntry {
            sort_key: sort,
            tri_count: tris,
            base_index: first,
            first_vertex: 0,
            vertex_count: self.world_vertex_count,
        });
        self.lists.world_draw_indices.push(ordinal);
    }
}

fn smodel_entry(sort: u32, tris: u16, first: u32, lighting_handle: u32) -> GfxSmodelRigidEntry {
    GfxSmodelRigidEntry {
        packed_key: sort,
        tri_count: tris,
        index_byte_offset: byte_offset(first),
        lighting_handle: lighting_handle as u16,
    }
}

/// Entry for a model whose indices were already placed by the pretessellation
/// stage. A range too short for a triangle is dropped without a count: the stage
/// that sized it owns that diagnosis.
fn dest_entry(sort: u32, lighting_handle: u32, dest: SmodelPretessRange) -> Option<GfxSmodelRigidEntry> {
    (dest.count >= 3).then(|| {
        smodel_entry(sort, (dest.count / 3) as u16, dest.start, lighting_handle)
    })
}
