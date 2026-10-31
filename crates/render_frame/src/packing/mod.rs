//! Shadow passes draw the same instances as the colour pass, but through the
//! backend's list interface. This module turns a pass's ordered draws into
//! those lists.

mod emit;

use dpvs_t6::SortedDraw;

use self::emit::Packer;
use crate::src_lists::{
    DRAWLIST_MARKER, CompactFrontendLists, STATIC_FIXED_MESH_RECORD_BYTES, SOURCE_SMODEL_CACHED_BYTES,
    SOURCE_SMODEL_CACHED_PTR, SOURCE_SMODEL_PRETESS_BYTES, SOURCE_SMODEL_PRETESS_PTR, SOURCE_SMODEL_RIGID_COUNT,
    SOURCE_SMODEL_RIGID_PTR, SOURCE_SMODEL_SKINNED_BYTES, SOURCE_SMODEL_SKINNED_PTR, SOURCE_WORLD_COUNT,
    SOURCE_WORLD_PTR, SOURCE_XMODEL_RIGID_COUNT, SOURCE_XMODEL_RIGID_PTR, PropPrebuiltRange,
};

/// Tess-info byte stamped on every dynamic-model entry: shadow passes always
/// read the packed vertex arm.
pub const RIGID_MODEL_TESS_FLAGS_PACKED_BRANCH: u8 = 1;

/// What a draw is, as far as list packing cares.
#[derive(Clone, Copy, Debug)]
pub enum BundleKind {
    /// A world surface, or a run of `run` surfaces listed from `run_start_offset` in the
    /// shared run table.
    World { surf: u16, run: u16, run_start_offset: u32 },
    PropStatic { surface: u32, illum_handle: u32 },
    PropSkinned { surface: u32, illum_handle: u32 },
    PropCached { illum_handle: u32, write_target: PropPrebuiltRange },
    PropPretess { illum_handle: u32, write_target: PropPrebuiltRange },
    XModel { surface: u32, illum_handle: u32 },
    /// Not drawn by a shadow pass.
    Skip,
}

/// One draw of the pass, in draw order.
#[derive(Clone, Copy, Debug)]
pub struct PackableDrawCall {
    /// The draw-surface sort key.
    pub key: u64,
    pub material_order_no: u32,
    pub kind: BundleKind,
}

/// Packs a sun-shadow cascade's draws.
///
/// Each `*_ranges` table maps a surface to `(first index, index count)` in its
/// index pool. Surfaces that are not in their table, or that hold less than one
/// whole triangle, are counted in `skipped_no_indices_total` and left out.
pub fn pack_daylight_shadow_frontend(
    draw_calls: &[PackableDrawCall],
    level_run_facets: &[u16],
    level_ranges: &[(u32, u32)],
    level_verts: u32,
    prop_ranges: &[(u32, u32)],
    figure_ranges: &[(u32, u32)],
) -> CompactFrontendLists {
    let mut packer = Packer {
        lists: CompactFrontendLists::default(),
        run_surfs: level_run_facets,
        level_ranges,
        level_vert_total: level_verts,
        prop_ranges,
        figure_ranges,
    };
    for (draw_no, draw) in draw_calls.iter().enumerate() {
        packer.feed(draw_no as u32, draw);
    }
    let mut lists = packer.lists;
    announce_lists(&mut lists);
    lists
}

/// Spot shadows list their draws exactly as the sun does.
pub fn pack_cone_shadow_frontend(
    draws: &[PackableDrawCall],
    level_run_facets: &[u16],
    level_ranges: &[(u32, u32)],
    map_verts: u32,
    prop_ranges: &[(u32, u32)],
    figure_ranges: &[(u32, u32)],
) -> CompactFrontendLists {
    pack_daylight_shadow_frontend(
        draws,
        level_run_facets,
        level_ranges,
        map_verts,
        prop_ranges,
        figure_ranges,
    )
}

/// Sort key the backend sorts entries by: the low 15 bits of the material rank,
/// the primary light above them, the surface type on top.
fn setup_sort_bits(key: u64, material_order_no: u32) -> u32 {
    let surf = SortedDraw { packed: key };
    let light = u32::from(surf.key_light_index());
    let kind = u32::from(surf.kind());
    (material_order_no & 0x7fff) | light << 16 | kind << 24
}

/// Writes the header tokens for every non-empty list. Lists the backend reads
/// by element count get `count`; the span-style lists get a byte length.
fn announce_lists(lists: &mut CompactFrontendLists) {
    let counted = [
        (SOURCE_WORLD_PTR, SOURCE_WORLD_COUNT, lists.world.len()),
        (SOURCE_SMODEL_RIGID_PTR, SOURCE_SMODEL_RIGID_COUNT, lists.smodel.len()),
        (SOURCE_XMODEL_RIGID_PTR, SOURCE_XMODEL_RIGID_COUNT, lists.xmodel.len()),
    ];
    for (ptr, count, len) in counted {
        if len != 0 {
            lists.source_words[ptr] = DRAWLIST_MARKER;
            lists.source_words[count] = len as u32;
        }
    }
    let spans = [
        (SOURCE_SMODEL_CACHED_PTR, SOURCE_SMODEL_CACHED_BYTES, lists.smodel_cached.len()),
        (SOURCE_SMODEL_PRETESS_PTR, SOURCE_SMODEL_PRETESS_BYTES, lists.smodel_pretess.len()),
        (SOURCE_SMODEL_SKINNED_PTR, SOURCE_SMODEL_SKINNED_BYTES, lists.smodel_skinned.len()),
    ];
    for (ptr, bytes, len) in spans {
        if len != 0 {
            lists.source_words[ptr] = DRAWLIST_MARKER;
            lists.source_words[bytes] = (len * STATIC_FIXED_MESH_RECORD_BYTES) as u32;
        }
    }
}
