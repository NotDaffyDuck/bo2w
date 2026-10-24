//! The per-frame budget for code-built meshes (vertices, indices, argument rows).

use render_frame::{
    GfxMeshData, reserve_code_mesh, reserve_code_mesh_indices, reserve_code_mesh_verts,
    shrink_code_mesh_verts,
};

#[test]
fn a_fresh_budget_is_empty_with_the_retail_caps() {
    let mesh = GfxMeshData::default();
    assert_eq!(mesh, GfxMeshData::code_mesh());
    assert_eq!((mesh.vert_used, mesh.index_used, mesh.args_used), (0, 0, 0));
    assert_eq!((mesh.vert_cap, mesh.index_cap, mesh.args_cap), (0x4000, 0x6000, 0x100));
    assert_eq!((mesh.warn_verts, mesh.warn_inds, mesh.warn_args), (0x24, 0x23, 0x25));
}

#[test]
fn vertex_reservations_hand_out_consecutive_bases() {
    let mut mesh = GfxMeshData::default();
    assert_eq!(reserve_code_mesh_verts(&mut mesh, 10), Some(0));
    assert_eq!(reserve_code_mesh_verts(&mut mesh, 6), Some(10));
    assert_eq!(mesh.vert_used, 16);
}

#[test]
fn vertex_reservations_stop_exactly_at_the_cap() {
    let mut mesh = GfxMeshData::default();
    assert_eq!(reserve_code_mesh_verts(&mut mesh, 0x4000), Some(0));
    assert_eq!(reserve_code_mesh_verts(&mut mesh, 1), None);
    assert_eq!(mesh.vert_used, 0x4000);
    let mut mesh = GfxMeshData::default();
    assert_eq!(reserve_code_mesh_verts(&mut mesh, 0x4001), None);
    assert_eq!(mesh.vert_used, 0);
}

#[test]
fn index_reservations_stop_at_their_cap() {
    let mut mesh = GfxMeshData::default();
    assert_eq!(reserve_code_mesh_indices(&mut mesh, 0x5fff), Some(0));
    assert_eq!(reserve_code_mesh_indices(&mut mesh, 1), Some(0x5fff));
    assert_eq!(reserve_code_mesh_indices(&mut mesh, 1), None);
    assert_eq!(mesh.index_used, 0x6000);
}

#[test]
fn shrinking_gives_vertices_back() {
    let mut mesh = GfxMeshData::default();
    reserve_code_mesh_verts(&mut mesh, 30);
    shrink_code_mesh_verts(&mut mesh, 12);
    assert_eq!(mesh.vert_used, 18);
    assert_eq!(reserve_code_mesh_verts(&mut mesh, 1), Some(18));
}

#[test]
fn a_combined_reservation_takes_all_three_or_nothing() {
    let mut mesh = GfxMeshData::default();
    assert_eq!(reserve_code_mesh(&mut mesh, 4, 6, 2), Some((0, 0, Some(0))));
    assert_eq!(reserve_code_mesh(&mut mesh, 3, 3, 1), Some((4, 6, Some(2))));
    assert_eq!((mesh.vert_used, mesh.index_used, mesh.args_used), (7, 9, 3));

    // Too many arguments: nothing moves.
    assert_eq!(reserve_code_mesh(&mut mesh, 1, 1, 0x100), None);
    assert_eq!((mesh.vert_used, mesh.index_used, mesh.args_used), (7, 9, 3));
    // Too many indices.
    assert_eq!(reserve_code_mesh(&mut mesh, 1, 0x6000, 0), None);
    assert_eq!((mesh.vert_used, mesh.index_used, mesh.args_used), (7, 9, 3));
}

#[test]
fn asking_for_no_arguments_skips_the_argument_budget() {
    let mut mesh = GfxMeshData { args_used: 0x100, ..GfxMeshData::default() };
    assert_eq!(reserve_code_mesh(&mut mesh, 2, 3, 0), Some((0, 0, None)));
    assert_eq!(mesh.args_used, 0x100);
}
