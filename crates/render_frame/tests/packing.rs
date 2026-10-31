//! The shadow passes hand the backend a source packet of entry lists. These
//! tests pin what ends up in each list and in the packet header.

use render_frame::packing::{
    RIGID_MODEL_TESS_FLAGS_PACKED_BRANCH, PackableDrawCall, BundleKind, pack_cone_shadow_frontend,
    pack_daylight_shadow_frontend,
};
use render_frame::{
    RigidModelBatchRecord, DRAWLIST_MARKER, CompactFrontendLists, PropPrebuiltRange, SOURCE_SMODEL_CACHED_BYTES,
    SOURCE_SMODEL_CACHED_PTR, SOURCE_SMODEL_PRETESS_BYTES, SOURCE_SMODEL_PRETESS_PTR, SOURCE_SMODEL_RIGID_COUNT,
    SOURCE_SMODEL_RIGID_PTR, SOURCE_SMODEL_SKINNED_BYTES, SOURCE_SMODEL_SKINNED_PTR, SOURCE_WORLD_COUNT,
    SOURCE_WORLD_PTR, SOURCE_XMODEL_RIGID_COUNT, SOURCE_XMODEL_RIGID_PTR,
};

/// Draw-surface key with the primary light in bits 44.., surface type in 52...
fn key(light: u64, surf_type: u64) -> u64 {
    (light << 44) | (surf_type << 52)
}

fn draw(kind: BundleKind) -> PackableDrawCall {
    PackableDrawCall {
        key: key(5, 3),
        material_order_no: 0x1234,
        kind,
    }
}

/// What a list entry's sort key should be for `key(5, 3)` at rank 0x1234.
const SORT_KEY: u32 = 0x1234 | (5 << 16) | (3 << 24);

fn pack(draws: &[PackableDrawCall]) -> CompactFrontendLists {
    let level_ranges = [(100, 9), (200, 2), (300, 12)];
    let prop_ranges = [(10, 6), (50, 1), (60, 30)];
    let figure_ranges = [(7, 3), (8, 0)];
    let run_surfs = [9, 2, 0, 2, 1];
    pack_daylight_shadow_frontend(draws, &run_surfs, &level_ranges, 777, &prop_ranges, &figure_ranges)
}

#[test]
fn an_empty_draw_list_leaves_the_packet_blank() {
    let packed = pack(&[]);
    assert!(packed.source_words.iter().all(|&dword| dword == 0));
    assert_eq!(packed.work_entry_tally(), 0);
    assert_eq!(packed.skipped_misc_total, 0);
    assert_eq!(packed.skipped_no_indices_total, 0);
}

#[test]
fn a_world_surface_becomes_a_triangle_list_entry() {
    let packed = pack(&[draw(BundleKind::World { surf: 0, run: 1, run_start_offset: 0 })]);
    assert_eq!(packed.world.len(), 1);
    let entry = packed.world[0];
    assert_eq!(entry.setup_sort, SORT_KEY);
    assert_eq!(entry.tri_tally, 3);
    assert_eq!(entry.index_start, 100);
    assert_eq!(entry.vertex_base, 0);
    assert_eq!(entry.vertex_count, 777);
    assert_eq!(packed.level_draw_indices, [0]);
    assert_eq!(packed.source_words[SOURCE_WORLD_PTR], DRAWLIST_MARKER);
    assert_eq!(packed.source_words[SOURCE_WORLD_COUNT], 1);
}

#[test]
fn the_sort_key_keeps_fifteen_bits_of_rank() {
    let mut d = draw(BundleKind::World { surf: 0, run: 1, run_start_offset: 0 });
    d.material_order_no = 0xffff_ffff;
    let packed = pack(&[d]);
    assert_eq!(packed.world[0].setup_sort, 0x7fff | (5 << 16) | (3 << 24));
}

#[test]
fn world_surfaces_without_a_triangle_are_counted_not_listed() {
    let packed = pack(&[
        draw(BundleKind::World { surf: 1, run: 1, run_start_offset: 0 }),
        draw(BundleKind::World { surf: 40, run: 1, run_start_offset: 0 }),
    ]);
    assert!(packed.world.is_empty());
    assert!(packed.level_draw_indices.is_empty());
    assert_eq!(packed.skipped_no_indices_total, 2);
    assert_eq!(packed.source_words[SOURCE_WORLD_PTR], 0);
}

#[test]
fn a_world_run_lists_every_surface_under_one_draw_index() {
    // Draw 1 is a run of three surfaces starting at offset 1: 2, 0, 2.
    let packed = pack(&[
        draw(BundleKind::World { surf: 0, run: 1, run_start_offset: 0 }),
        draw(BundleKind::World { surf: 9, run: 3, run_start_offset: 1 }),
    ]);
    let bases: Vec<u32> = packed.world.iter().map(|e| e.index_start).collect();
    assert_eq!(bases, [100, 300, 100, 300]);
    assert_eq!(packed.level_draw_indices, [0, 1, 1, 1]);
    assert_eq!(packed.source_words[SOURCE_WORLD_COUNT], 4);
}

#[test]
fn a_run_past_the_surface_list_falls_back_to_the_lead_surface() {
    let packed = pack(&[draw(BundleKind::World { surf: 2, run: 4, run_start_offset: 3 })]);
    assert_eq!(packed.world.len(), 1);
    assert_eq!(packed.world[0].index_start, 300);
    assert_eq!(packed.world[0].tri_tally, 4);
}

#[test]
fn a_rigid_static_model_lists_byte_offsets() {
    let packed = pack(&[draw(BundleKind::PropStatic { surface: 0, illum_handle: 0x1_0007 })]);
    assert_eq!(packed.smodel.len(), 1);
    let entry = packed.smodel[0];
    assert_eq!(entry.combined_key_bits, SORT_KEY);
    assert_eq!(entry.tri_tally, 2);
    assert_eq!(entry.index_data_start_bytes, 20);
    assert_eq!(entry.illum_handle, 7);
    assert_eq!(packed.prop_draw_indices, [0]);
    assert_eq!(packed.source_words[SOURCE_SMODEL_RIGID_PTR], DRAWLIST_MARKER);
    assert_eq!(packed.source_words[SOURCE_SMODEL_RIGID_COUNT], 1);
}

#[test]
fn skinned_static_models_have_their_own_list_and_byte_span() {
    let packed = pack(&[
        draw(BundleKind::PropSkinned { surface: 2, illum_handle: 3 }),
        draw(BundleKind::PropSkinned { surface: 0, illum_handle: 4 }),
        draw(BundleKind::PropSkinned { surface: 1, illum_handle: 4 }),
    ]);
    assert_eq!(packed.smodel_skinned.len(), 2);
    assert_eq!(packed.prop_skinned_draw_indices, [0, 1]);
    assert_eq!(packed.skipped_no_indices_total, 1);
    assert!(packed.smodel.is_empty());
    assert_eq!(packed.source_words[SOURCE_SMODEL_SKINNED_PTR], DRAWLIST_MARKER);
    assert_eq!(packed.source_words[SOURCE_SMODEL_SKINNED_BYTES], 2 * 16);
}

#[test]
fn pretessellated_and_cached_models_use_their_destination_ranges() {
    let packed = pack(&[
        draw(BundleKind::PropPretess {
            illum_handle: 9,
            write_target: PropPrebuiltRange { start: 4, count: 9 },
        }),
        draw(BundleKind::PropCached {
            illum_handle: 8,
            write_target: PropPrebuiltRange { start: 40, count: 3 },
        }),
        draw(BundleKind::PropCached {
            illum_handle: 8,
            write_target: PropPrebuiltRange { start: 90, count: 2 },
        }),
    ]);
    assert_eq!(packed.smodel_pretess.len(), 1);
    assert_eq!(packed.smodel_pretess[0].tri_tally, 3);
    assert_eq!(packed.smodel_pretess[0].index_data_start_bytes, 8);
    assert_eq!(packed.smodel_pretess[0].illum_handle, 9);
    assert_eq!(packed.prop_prebuilt_draw_indices, [0]);
    // The short cached range is dropped without being counted.
    assert_eq!(packed.smodel_cached.len(), 1);
    assert_eq!(packed.smodel_cached[0].index_data_start_bytes, 80);
    assert_eq!(packed.prop_cached_draw_indices, [1]);
    assert_eq!(packed.skipped_no_indices_total, 0);
    assert_eq!(packed.source_words[SOURCE_SMODEL_PRETESS_PTR], DRAWLIST_MARKER);
    assert_eq!(packed.source_words[SOURCE_SMODEL_PRETESS_BYTES], 16);
    assert_eq!(packed.source_words[SOURCE_SMODEL_CACHED_PTR], DRAWLIST_MARKER);
    assert_eq!(packed.source_words[SOURCE_SMODEL_CACHED_BYTES], 16);
}

#[test]
fn a_dynamic_model_entry_carries_the_packed_arm() {
    let packed = pack(&[
        draw(BundleKind::XModel { surface: 0, illum_handle: 0x2_0001 }),
        draw(BundleKind::XModel { surface: 1, illum_handle: 1 }),
        draw(BundleKind::XModel { surface: 5, illum_handle: 1 }),
    ]);
    assert_eq!(packed.xmodel.len(), 1);
    assert_eq!(
        packed.xmodel[0],
        RigidModelBatchRecord::new(SORT_KEY, 14, 1, RIGID_MODEL_TESS_FLAGS_PACKED_BRANCH, 0x2_0001)
    );
    assert_eq!(packed.model_draw_ix, [0]);
    assert_eq!(packed.skipped_no_indices_total, 2);
    assert_eq!(packed.source_words[SOURCE_XMODEL_RIGID_PTR], DRAWLIST_MARKER);
    assert_eq!(packed.source_words[SOURCE_XMODEL_RIGID_COUNT], 1);
}

#[test]
fn skipped_draws_keep_their_slot_in_the_draw_indices() {
    let packed = pack(&[
        draw(BundleKind::Skip),
        draw(BundleKind::World { surf: 0, run: 1, run_start_offset: 0 }),
        draw(BundleKind::Skip),
        draw(BundleKind::World { surf: 2, run: 1, run_start_offset: 0 }),
    ]);
    assert_eq!(packed.skipped_misc_total, 2);
    assert_eq!(packed.level_draw_indices, [1, 3]);
}

#[test]
fn work_entry_n_sums_every_entry_list() {
    let packed = pack(&[
        draw(BundleKind::World { surf: 0, run: 1, run_start_offset: 0 }),
        draw(BundleKind::PropStatic { surface: 0, illum_handle: 0 }),
        draw(BundleKind::PropSkinned { surface: 0, illum_handle: 0 }),
        draw(BundleKind::XModel { surface: 0, illum_handle: 0 }),
        draw(BundleKind::PropPretess { illum_handle: 0, write_target: PropPrebuiltRange { start: 0, count: 3 } }),
        draw(BundleKind::PropCached { illum_handle: 0, write_target: PropPrebuiltRange { start: 0, count: 3 } }),
    ]);
    assert_eq!(packed.work_entry_tally(), 6);
}

#[test]
fn spot_shadow_packing_matches_sun_shadow_packing() {
    let draws = [
        draw(BundleKind::World { surf: 0, run: 1, run_start_offset: 0 }),
        draw(BundleKind::XModel { surface: 0, illum_handle: 1 }),
        draw(BundleKind::Skip),
    ];
    let level_ranges = [(100, 9)];
    let ranges = [(10, 6)];
    let sun = pack_daylight_shadow_frontend(&draws, &[], &level_ranges, 4, &ranges, &ranges);
    let spot = pack_cone_shadow_frontend(&draws, &[], &level_ranges, 4, &ranges, &ranges);
    assert_eq!(sun.source_words, spot.source_words);
    assert_eq!(sun.world, spot.world);
    assert_eq!(sun.xmodel, spot.xmodel);
    assert_eq!(sun.skipped_misc_total, spot.skipped_misc_total);
}
