//! Shadow passes draw the same instances as the colour pass, but through the
//! backend's list interface. This module turns a pass's ordered draws into
//! those lists.

mod emit;

use dpvs_t6::GfxDrawSurf;

use self::emit::Packer;
use crate::frontend::{
    LIST_TOKEN, PackedFrontendLists, SMODEL_RIGID_ENTRY_STRIDE, SRC_SMODEL_CACHED_BYTES,
    SRC_SMODEL_CACHED_PTR, SRC_SMODEL_PRETESS_BYTES, SRC_SMODEL_PRETESS_PTR, SRC_SMODEL_RIGID_COUNT,
    SRC_SMODEL_RIGID_PTR, SRC_SMODEL_SKINNED_BYTES, SRC_SMODEL_SKINNED_PTR, SRC_WORLD_COUNT,
    SRC_WORLD_PTR, SRC_XMODEL_RIGID_COUNT, SRC_XMODEL_RIGID_PTR, SmodelPretessRange,
};

/// Tess-info byte stamped on every dynamic-model entry: shadow passes always
/// read the packed vertex arm.
pub const HOST_XMODEL_RIGID_TESS_INFO_PACKED_ARM: u8 = 1;

/// What a draw is, as far as list packing cares.
#[derive(Clone, Copy, Debug)]
pub enum PackKind {
    /// A world surface, or a run of `run` surfaces listed from `run_off` in the
    /// shared run table.
    World { surf: u16, run: u16, run_off: u32 },
    SmodelRigid { surface: u32, lighting_handle: u32 },
    SmodelSkinned { surface: u32, lighting_handle: u32 },
    SmodelCached { lighting_handle: u32, dest: SmodelPretessRange },
    SmodelPretess { lighting_handle: u32, dest: SmodelPretessRange },
    XModel { surface: u32, lighting_handle: u32 },
    /// Not drawn by a shadow pass.
    Skip,
}

/// One draw of the pass, in draw order.
#[derive(Clone, Copy, Debug)]
pub struct PackDraw {
    /// The draw-surface sort key.
    pub key: u64,
    pub material_rank: u32,
    pub kind: PackKind,
}

/// Packs a sun-shadow cascade's draws.
///
/// Each `*_ranges` table maps a surface to `(first index, index count)` in its
/// index pool. Surfaces that are not in their table, or that hold less than one
/// whole triangle, are counted in `skipped_empty_ib` and left out.
pub fn pack_sun_shadow_frontend(
    draws: &[PackDraw],
    world_run_surfs: &[u16],
    world_ranges: &[(u32, u32)],
    world_vertex_count: u32,
    smodel_ranges: &[(u32, u32)],
    xmodel_ranges: &[(u32, u32)],
) -> PackedFrontendLists {
    let mut packer = Packer {
        lists: PackedFrontendLists::default(),
        run_surfs: world_run_surfs,
        world_ranges,
        world_vertex_count,
        smodel_ranges,
        xmodel_ranges,
    };
    for (ordinal, draw) in draws.iter().enumerate() {
        packer.feed(ordinal as u32, draw);
    }
    let mut lists = packer.lists;
    announce_lists(&mut lists);
    lists
}

/// Spot shadows list their draws exactly as the sun does.
pub fn pack_spot_shadow_frontend(
    draws: &[PackDraw],
    world_run_surfs: &[u16],
    world_ranges: &[(u32, u32)],
    world_vertex_count: u32,
    smodel_ranges: &[(u32, u32)],
    xmodel_ranges: &[(u32, u32)],
) -> PackedFrontendLists {
    pack_sun_shadow_frontend(
        draws,
        world_run_surfs,
        world_ranges,
        world_vertex_count,
        smodel_ranges,
        xmodel_ranges,
    )
}

/// Sort key the backend sorts entries by: the low 15 bits of the material rank,
/// the primary light above them, the surface type on top.
fn backend_setup_key(key: u64, material_rank: u32) -> u32 {
    let surf = GfxDrawSurf { packed: key };
    let light = u32::from(surf.primary_light_index());
    let kind = u32::from(surf.surf_type());
    (material_rank & 0x7fff) | light << 16 | kind << 24
}

/// Writes the header tokens for every non-empty list. Lists the backend reads
/// by element count get `count`; the span-style lists get a byte length.
fn announce_lists(lists: &mut PackedFrontendLists) {
    let counted = [
        (SRC_WORLD_PTR, SRC_WORLD_COUNT, lists.world.len()),
        (SRC_SMODEL_RIGID_PTR, SRC_SMODEL_RIGID_COUNT, lists.smodel.len()),
        (SRC_XMODEL_RIGID_PTR, SRC_XMODEL_RIGID_COUNT, lists.xmodel.len()),
    ];
    for (ptr, count, len) in counted {
        if len != 0 {
            lists.src[ptr] = LIST_TOKEN;
            lists.src[count] = len as u32;
        }
    }
    let spans = [
        (SRC_SMODEL_CACHED_PTR, SRC_SMODEL_CACHED_BYTES, lists.smodel_cached.len()),
        (SRC_SMODEL_PRETESS_PTR, SRC_SMODEL_PRETESS_BYTES, lists.smodel_pretess.len()),
        (SRC_SMODEL_SKINNED_PTR, SRC_SMODEL_SKINNED_BYTES, lists.smodel_skinned.len()),
    ];
    for (ptr, bytes, len) in spans {
        if len != 0 {
            lists.src[ptr] = LIST_TOKEN;
            lists.src[bytes] = (len * SMODEL_RIGID_ENTRY_STRIDE) as u32;
        }
    }
}
