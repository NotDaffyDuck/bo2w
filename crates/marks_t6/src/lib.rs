//! Impact marks: the bullet holes and scorches an effect leaves on what it hits.
//!
//! A mark is a box around the hit. Each triangle of a surface the box touches is cut down to
//! the part inside the box; the pieces wait in staging, then move into the mark pool, where
//! they stay until they are drawn and a newer mark pushes them out. Sizes are BO2's: 256
//! marks, 1024 triangle groups and 1536 point groups per local client, and at most 8 marks in
//! one grid cell.

#![no_std]
#![forbid(unsafe_code)]

mod fan_clip;
mod context;
mod grid;
mod hit_rules;
mod mark;
mod decal_mesh;
mod mark_store;
mod receiver;
mod staging;
mod world;

pub use fan_clip::{
    DecalEmitDenial, WorldDecalVertex, MarkClipPlaneSet, DECAL_CLIP_POLYGON_VERTEX_CEILING,
    mark_chop_world_triangle_points, mark_emit_brush_fragment, mark_fragment_clip_planes,
    mark_is_triangle_rejected,
};
pub use context::{
    GfxMarkContext, MARK_MODEL_DOBJ_MODEL_MASK, MARK_MODEL_TYPE_ENTITY_BRUSH,
    MARK_MODEL_TYPE_ENTITY_MODEL, MARK_MODEL_TYPE_MASK, MARK_MODEL_TYPE_STATIC_MODEL,
    MARK_MODEL_TYPE_WORLD_BRUSH, MARK_MODEL_VIEWMODEL_BIT,
};
pub use grid::{
    FX_MARKS_GRID_BUCKETS, FX_MARKS_GRID_CELL_LIMIT, FX_MARKS_GRID_CELL_SCALE,
    FX_MARKS_GRID_UPDATE_MSEC, FxMarksGrid, FxMarksGridCell, marks_grid_cell_from_position,
    marks_grid_hash_from_cell,
};
pub use hit_rules::{
    FX_MARKS_RANGE_DEFAULT, DecalClipTarget, impact_mark_material,
    impact_mark_models_generate, impact_mark_outer_gate, impact_mark_skip_world_from_stored_bolt,
    impact_mark_within_range,
};
pub use mark::{
    DECAL_INITIAL_CLAIMED, DecalClaimDenial, DecalAllocationAsk, FxMarkAlphaFade,
    BuiltDecalRecord, alloc_and_construct_mark, lru_mark_pick,
};
pub use decal_mesh::{
    DECAL_MESH_VERTEX_BYTES, DecalMeshAllowance, DecalMeshDenial, add_mark_mesh_draw_surf,
    generate_mark_verts_begin, pack_mark_model_vertex, pack_mark_world_vertex,
};
pub use mark_store::{
    DECAL_HANDLE_ABSENT, DECAL_POOL_CAPACITY, POINT_CHUNK_CHAIN_TERMINATOR, DECAL_POINT_BLOCK_CAPACITY,
    POINT_CHUNK_SUCCESSOR_ABSENT, TRI_CHUNK_CHAIN_TERMINATOR, DECAL_TRIANGLE_BLOCK_CAPACITY, TRI_CHUNK_SUCCESSOR_ABSENT,
    DecalPointChunk, DecalTriangleChunk, copy_mark_points, copy_mark_tris, init_mark_next_handle,
    init_point_next_slot, init_tri_next_slot, mark_contexts_equal, mark_point_groups_for_count,
    mark_tri_groups_for_staging, marks_active_limit,
};
pub use receiver::{DecalAdmit, DecalSurfaceGate, mark_allow, mark_include_in_world_clip};
pub use staging::{
    EffectStainStagingPoint, EffectStainStagingTri, DECAL_FRAGMENT_VERTEX_CEILING, DECAL_FRAGMENT_TRIANGLE_CEILING,
};
pub use world::{StainLevelStaging, mark_sphere_hits_bounds, mark_stage_world_surfaces};

/// The lightmap number of a surface that has no lightmap (models, and marks on them).
pub const SURFACE_BAKED_LIGHT_ABSENT: u8 = 0x1f;
