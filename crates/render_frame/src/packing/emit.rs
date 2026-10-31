//! The per-draw half of packing: which list a draw lands in, and what its
//! entry holds.

use super::{RIGID_MODEL_TESS_FLAGS_PACKED_BRANCH, PackableDrawCall, BundleKind, setup_sort_bits};
use crate::src_lists::{
    StaticModelRigidRecord, TriangleRunEntry, RigidModelBatchRecord, CompactFrontendLists,
    PropPrebuiltRange,
};

/// Packing state: the lists being built and the tables draws are resolved in.
pub(super) struct Packer<'a> {
    pub(super) lists: CompactFrontendLists,
    pub(super) run_surfs: &'a [u16],
    pub(super) level_ranges: &'a [(u32, u32)],
    pub(super) level_vert_total: u32,
    pub(super) prop_ranges: &'a [(u32, u32)],
    pub(super) figure_ranges: &'a [(u32, u32)],
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
    pub(super) fn feed(&mut self, draw_no: u32, call: &PackableDrawCall) {
        let sort = setup_sort_bits(call.key, call.material_order_no);
        match call.kind {
            BundleKind::World { surf, run, run_start_offset } => {
                let lead = [surf];
                let members = self.run_members(run, run_start_offset).unwrap_or(&lead);
                for &member in members {
                    self.world_surface(draw_no, sort, member);
                }
            }
            BundleKind::PropStatic { surface, illum_handle } => {
                let hit = drawable(self.prop_ranges, surface as usize);
                let lists = &mut self.lists;
                match hit {
                    Some((head_ix, triangle_list)) => {
                        lists.smodel.push(smodel_entry(sort, triangle_list, head_ix, illum_handle));
                        lists.prop_draw_indices.push(draw_no);
                    }
                    None => lists.skipped_no_indices_total += 1,
                }
            }
            BundleKind::PropSkinned { surface, illum_handle } => {
                let hit = drawable(self.prop_ranges, surface as usize);
                let lists = &mut self.lists;
                match hit {
                    Some((run_head, triangle_list)) => {
                        lists.smodel_skinned.push(smodel_entry(sort, triangle_list, run_head, illum_handle));
                        lists.prop_skinned_draw_indices.push(draw_no);
                    }
                    None => lists.skipped_no_indices_total += 1,
                }
            }
            BundleKind::PropPretess { illum_handle, write_target } => {
                if let Some(entry) = dest_entry(sort, illum_handle, write_target) {
                    self.lists.smodel_pretess.push(entry);
                    self.lists.prop_prebuilt_draw_indices.push(draw_no);
                }
            }
            BundleKind::PropCached { illum_handle, write_target } => {
                if let Some(entry) = dest_entry(sort, illum_handle, write_target) {
                    self.lists.smodel_cached.push(entry);
                    self.lists.prop_cached_draw_indices.push(draw_no);
                }
            }
            BundleKind::XModel { surface, illum_handle } => {
                match drawable(self.figure_ranges, surface as usize) {
                    Some((base_ix, triangle_list)) => {
                        self.lists.xmodel.push(RigidModelBatchRecord::new(
                            sort,
                            byte_offset(base_ix),
                            triangle_list,
                            RIGID_MODEL_TESS_FLAGS_PACKED_BRANCH,
                            illum_handle,
                        ));
                        self.lists.model_draw_ix.push(draw_no);
                    }
                    None => self.lists.skipped_no_indices_total += 1,
                }
            }
            BundleKind::Skip => self.lists.skipped_misc_total += 1,
        }
    }

    /// The surfaces a world draw stands for: the run table slice when the draw is
    /// a run and the whole slice exists, otherwise just the lead surface.
    fn run_members(&self, run: u16, run_start_offset: u32) -> Option<&'a [u16]> {
        if run <= 1 {
            return None;
        }
        let from = run_start_offset as usize;
        let to = from.saturating_add(usize::from(run));
        let slice = self.run_surfs.get(from..to)?;
        (slice.len() == usize::from(run)).then_some(slice)
    }

    fn world_surface(&mut self, call_no: u32, sort: u32, face_ix: u16) {
        let Some((first, triangle_list)) = drawable(self.level_ranges, usize::from(face_ix)) else {
            self.lists.skipped_no_indices_total += 1;
            return;
        };
        self.lists.world.push(TriangleRunEntry {
            setup_sort: sort,
            tri_tally: triangle_list,
            index_start: first,
            vertex_base: 0,
            vertex_count: self.level_vert_total,
        });
        self.lists.level_draw_indices.push(call_no);
    }
}

fn smodel_entry(sort: u32, triangle_list: u16, first: u32, illum_handle: u32) -> StaticModelRigidRecord {
    StaticModelRigidRecord {
        combined_key_bits: sort,
        tri_tally: triangle_list,
        index_data_start_bytes: byte_offset(first),
        illum_handle: illum_handle as u16,
    }
}

/// Entry for a model whose indices were already placed by the pretessellation
/// stage. A range too short for a triangle is dropped without a count: the stage
/// that sized it owns that diagnosis.
fn dest_entry(sort: u32, illum_handle: u32, write_target: PropPrebuiltRange) -> Option<StaticModelRigidRecord> {
    (write_target.count >= 3).then(|| {
        smodel_entry(sort, (write_target.count / 3) as u16, write_target.start, illum_handle)
    })
}
