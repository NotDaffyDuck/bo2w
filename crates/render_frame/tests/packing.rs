//! The shadow passes hand the backend a source packet of entry lists. These
//! tests pin what ends up in each list and in the packet header.

use render_frame::packing::{
    HOST_XMODEL_RIGID_TESS_INFO_PACKED_ARM, PackDraw, PackKind, pack_spot_shadow_frontend,
    pack_sun_shadow_frontend,
};
use render_frame::{
    GfxXModelRigidEntry, LIST_TOKEN, PackedFrontendLists, SmodelPretessRange, SRC_SMODEL_CACHED_BYTES,
    SRC_SMODEL_CACHED_PTR, SRC_SMODEL_PRETESS_BYTES, SRC_SMODEL_PRETESS_PTR, SRC_SMODEL_RIGID_COUNT,
    SRC_SMODEL_RIGID_PTR, SRC_SMODEL_SKINNED_BYTES, SRC_SMODEL_SKINNED_PTR, SRC_WORLD_COUNT,
    SRC_WORLD_PTR, SRC_XMODEL_RIGID_COUNT, SRC_XMODEL_RIGID_PTR,
};

/// Draw-surface key with the primary light in bits 44.., surface type in 52...
fn key(light: u64, surf_type: u64) -> u64 {
    (light << 44) | (surf_type << 52)
}

fn draw(kind: PackKind) -> PackDraw {
    PackDraw {
        key: key(5, 3),
        material_rank: 0x1234,
        kind,
    }
}

/// What a list entry's sort key should be for `key(5, 3)` at rank 0x1234.
const SORT_KEY: u32 = 0x1234 | (5 << 16) | (3 << 24);

fn pack(draws: &[PackDraw]) -> PackedFrontendLists {
    let world_ranges = [(100, 9), (200, 2), (300, 12)];
    let smodel_ranges = [(10, 6), (50, 1), (60, 30)];
    let xmodel_ranges = [(7, 3), (8, 0)];
    let run_surfs = [9, 2, 0, 2, 1];
    pack_sun_shadow_frontend(draws, &run_surfs, &world_ranges, 777, &smodel_ranges, &xmodel_ranges)
}

#[test]
fn an_empty_draw_list_leaves_the_packet_blank() {
    let packed = pack(&[]);
    assert!(packed.src.iter().all(|&dword| dword == 0));
    assert_eq!(packed.work_entry_n(), 0);
    assert_eq!(packed.skipped_other, 0);
    assert_eq!(packed.skipped_empty_ib, 0);
}

#[test]
fn a_world_surface_becomes_a_triangle_list_entry() {
    let packed = pack(&[draw(PackKind::World { surf: 0, run: 1, run_off: 0 })]);
    assert_eq!(packed.world.len(), 1);
    let entry = packed.world[0];
    assert_eq!(entry.sort_key, SORT_KEY);
    assert_eq!(entry.tri_count, 3);
    assert_eq!(entry.base_index, 100);
    assert_eq!(entry.first_vertex, 0);
    assert_eq!(entry.vertex_count, 777);
    assert_eq!(packed.world_draw_indices, [0]);
    assert_eq!(packed.src[SRC_WORLD_PTR], LIST_TOKEN);
    assert_eq!(packed.src[SRC_WORLD_COUNT], 1);
}

#[test]
fn the_sort_key_keeps_fifteen_bits_of_rank() {
    let mut d = draw(PackKind::World { surf: 0, run: 1, run_off: 0 });
    d.material_rank = 0xffff_ffff;
    let packed = pack(&[d]);
    assert_eq!(packed.world[0].sort_key, 0x7fff | (5 << 16) | (3 << 24));
}

#[test]
fn world_surfaces_without_a_triangle_are_counted_not_listed() {
    let packed = pack(&[
        draw(PackKind::World { surf: 1, run: 1, run_off: 0 }),
        draw(PackKind::World { surf: 40, run: 1, run_off: 0 }),
    ]);
    assert!(packed.world.is_empty());
    assert!(packed.world_draw_indices.is_empty());
    assert_eq!(packed.skipped_empty_ib, 2);
    assert_eq!(packed.src[SRC_WORLD_PTR], 0);
}

#[test]
fn a_world_run_lists_every_surface_under_one_draw_index() {
    // Draw 1 is a run of three surfaces starting at offset 1: 2, 0, 2.
    let packed = pack(&[
        draw(PackKind::World { surf: 0, run: 1, run_off: 0 }),
        draw(PackKind::World { surf: 9, run: 3, run_off: 1 }),
    ]);
    let bases: Vec<u32> = packed.world.iter().map(|e| e.base_index).collect();
    assert_eq!(bases, [100, 300, 100, 300]);
    assert_eq!(packed.world_draw_indices, [0, 1, 1, 1]);
    assert_eq!(packed.src[SRC_WORLD_COUNT], 4);
}

#[test]
fn a_run_past_the_surface_list_falls_back_to_the_lead_surface() {
    let packed = pack(&[draw(PackKind::World { surf: 2, run: 4, run_off: 3 })]);
    assert_eq!(packed.world.len(), 1);
    assert_eq!(packed.world[0].base_index, 300);
    assert_eq!(packed.world[0].tri_count, 4);
}

#[test]
fn a_rigid_static_model_lists_byte_offsets() {
    let packed = pack(&[draw(PackKind::SmodelRigid { surface: 0, lighting_handle: 0x1_0007 })]);
    assert_eq!(packed.smodel.len(), 1);
    let entry = packed.smodel[0];
    assert_eq!(entry.packed_key, SORT_KEY);
    assert_eq!(entry.tri_count, 2);
    assert_eq!(entry.index_byte_offset, 20);
    assert_eq!(entry.lighting_handle, 7);
    assert_eq!(packed.smodel_draw_indices, [0]);
    assert_eq!(packed.src[SRC_SMODEL_RIGID_PTR], LIST_TOKEN);
    assert_eq!(packed.src[SRC_SMODEL_RIGID_COUNT], 1);
}

#[test]
fn skinned_static_models_have_their_own_list_and_byte_span() {
    let packed = pack(&[
        draw(PackKind::SmodelSkinned { surface: 2, lighting_handle: 3 }),
        draw(PackKind::SmodelSkinned { surface: 0, lighting_handle: 4 }),
        draw(PackKind::SmodelSkinned { surface: 1, lighting_handle: 4 }),
    ]);
    assert_eq!(packed.smodel_skinned.len(), 2);
    assert_eq!(packed.smodel_skinned_draw_indices, [0, 1]);
    assert_eq!(packed.skipped_empty_ib, 1);
    assert!(packed.smodel.is_empty());
    assert_eq!(packed.src[SRC_SMODEL_SKINNED_PTR], LIST_TOKEN);
    assert_eq!(packed.src[SRC_SMODEL_SKINNED_BYTES], 2 * 16);
}

#[test]
fn pretessellated_and_cached_models_use_their_destination_ranges() {
    let packed = pack(&[
        draw(PackKind::SmodelPretess {
            lighting_handle: 9,
            dest: SmodelPretessRange { start: 4, count: 9 },
        }),
        draw(PackKind::SmodelCached {
            lighting_handle: 8,
            dest: SmodelPretessRange { start: 40, count: 3 },
        }),
        draw(PackKind::SmodelCached {
            lighting_handle: 8,
            dest: SmodelPretessRange { start: 90, count: 2 },
        }),
    ]);
    assert_eq!(packed.smodel_pretess.len(), 1);
    assert_eq!(packed.smodel_pretess[0].tri_count, 3);
    assert_eq!(packed.smodel_pretess[0].index_byte_offset, 8);
    assert_eq!(packed.smodel_pretess[0].lighting_handle, 9);
    assert_eq!(packed.smodel_pretess_draw_indices, [0]);
    // The short cached range is dropped without being counted.
    assert_eq!(packed.smodel_cached.len(), 1);
    assert_eq!(packed.smodel_cached[0].index_byte_offset, 80);
    assert_eq!(packed.smodel_cached_draw_indices, [1]);
    assert_eq!(packed.skipped_empty_ib, 0);
    assert_eq!(packed.src[SRC_SMODEL_PRETESS_PTR], LIST_TOKEN);
    assert_eq!(packed.src[SRC_SMODEL_PRETESS_BYTES], 16);
    assert_eq!(packed.src[SRC_SMODEL_CACHED_PTR], LIST_TOKEN);
    assert_eq!(packed.src[SRC_SMODEL_CACHED_BYTES], 16);
}

#[test]
fn a_dynamic_model_entry_carries_the_packed_arm() {
    let packed = pack(&[
        draw(PackKind::XModel { surface: 0, lighting_handle: 0x2_0001 }),
        draw(PackKind::XModel { surface: 1, lighting_handle: 1 }),
        draw(PackKind::XModel { surface: 5, lighting_handle: 1 }),
    ]);
    assert_eq!(packed.xmodel.len(), 1);
    assert_eq!(
        packed.xmodel[0],
        GfxXModelRigidEntry::new(SORT_KEY, 14, 1, HOST_XMODEL_RIGID_TESS_INFO_PACKED_ARM, 0x2_0001)
    );
    assert_eq!(packed.xmodel_draw_indices, [0]);
    assert_eq!(packed.skipped_empty_ib, 2);
    assert_eq!(packed.src[SRC_XMODEL_RIGID_PTR], LIST_TOKEN);
    assert_eq!(packed.src[SRC_XMODEL_RIGID_COUNT], 1);
}

#[test]
fn skipped_draws_keep_their_slot_in_the_draw_indices() {
    let packed = pack(&[
        draw(PackKind::Skip),
        draw(PackKind::World { surf: 0, run: 1, run_off: 0 }),
        draw(PackKind::Skip),
        draw(PackKind::World { surf: 2, run: 1, run_off: 0 }),
    ]);
    assert_eq!(packed.skipped_other, 2);
    assert_eq!(packed.world_draw_indices, [1, 3]);
}

#[test]
fn work_entry_n_sums_every_entry_list() {
    let packed = pack(&[
        draw(PackKind::World { surf: 0, run: 1, run_off: 0 }),
        draw(PackKind::SmodelRigid { surface: 0, lighting_handle: 0 }),
        draw(PackKind::SmodelSkinned { surface: 0, lighting_handle: 0 }),
        draw(PackKind::XModel { surface: 0, lighting_handle: 0 }),
        draw(PackKind::SmodelPretess { lighting_handle: 0, dest: SmodelPretessRange { start: 0, count: 3 } }),
        draw(PackKind::SmodelCached { lighting_handle: 0, dest: SmodelPretessRange { start: 0, count: 3 } }),
    ]);
    assert_eq!(packed.work_entry_n(), 6);
}

#[test]
fn spot_shadow_packing_matches_sun_shadow_packing() {
    let draws = [
        draw(PackKind::World { surf: 0, run: 1, run_off: 0 }),
        draw(PackKind::XModel { surface: 0, lighting_handle: 1 }),
        draw(PackKind::Skip),
    ];
    let world_ranges = [(100, 9)];
    let ranges = [(10, 6)];
    let sun = pack_sun_shadow_frontend(&draws, &[], &world_ranges, 4, &ranges, &ranges);
    let spot = pack_spot_shadow_frontend(&draws, &[], &world_ranges, 4, &ranges, &ranges);
    assert_eq!(sun.src, spot.src);
    assert_eq!(sun.world, spot.world);
    assert_eq!(sun.xmodel, spot.xmodel);
    assert_eq!(sun.skipped_other, spot.skipped_other);
}
