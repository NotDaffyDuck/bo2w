//! Impact marks: the bullet holes and scorches an effect leaves on what it hits.
//!
//! A mark is a box around the hit. Each triangle of a surface the box touches is cut down to
//! the part inside the box; the pieces wait in staging, then move into the mark pool, where
//! they stay until they are drawn and a newer mark pushes them out. Sizes are BO2's: 256
//! marks, 1024 triangle groups and 1536 point groups per local client, and at most 8 marks in
//! one grid cell.

#![no_std]
#![forbid(unsafe_code)]

mod chop;
mod context;
mod grid;
mod impact;
mod mark;
mod mesh;
mod pool;
mod receiver;
mod staging;
mod world;

pub use chop::{
    FxMarkEmitRefuse, FxWorldMarkPoint, MarkClipPlaneSet, R_MARK_CHOP_MAX_POINTS,
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
pub use impact::{
    FX_MARKS_RANGE_DEFAULT, MarkFragmentsAgainst, impact_mark_material,
    impact_mark_models_generate, impact_mark_outer_gate, impact_mark_skip_world_from_stored_bolt,
    impact_mark_within_range,
};
pub use mark::{
    FX_MARKS_INIT_ALLOCED_COUNT, FxAllocMarkRefuse, FxAllocMarkRequest, FxMarkAlphaFade,
    FxMarkConstructed, alloc_and_construct_mark, lru_mark_pick,
};
pub use mesh::{
    GFX_MARK_MESH_VERTEX_STRIDE, GfxMarkMeshBudget, GfxMarkMeshRefuse, add_mark_mesh_draw_surf,
    generate_mark_verts_begin, pack_mark_model_vertex, pack_mark_world_vertex,
};
pub use pool::{
    FX_MARK_HANDLE_NONE, FX_MARKS_LIMIT, FX_POINT_GROUP_CHAIN_NONE, FX_POINT_GROUP_LIMIT,
    FX_POINT_GROUP_NEXT_NONE, FX_TRI_GROUP_CHAIN_NONE, FX_TRI_GROUP_LIMIT, FX_TRI_GROUP_NEXT_NONE,
    FxPointGroup, FxTriGroup, copy_mark_points, copy_mark_tris, init_mark_next_handle,
    init_point_next_slot, init_tri_next_slot, mark_contexts_equal, mark_point_groups_for_count,
    mark_tri_groups_for_staging, marks_active_limit,
};
pub use receiver::{FxMarkAllow, MarkReceiver, mark_allow, mark_include_in_world_clip};
pub use staging::{
    FxMarkStagingPoint, FxMarkStagingTri, R_MARK_FRAGMENTS_MAX_POINTS, R_MARK_FRAGMENTS_MAX_TRIS,
};
pub use world::{MarkWorldStaging, mark_sphere_hits_bounds, mark_stage_world_surfaces};

/// The lightmap number of a surface that has no lightmap (models, and marks on them).
pub const GFX_SURFACE_LIGHTMAP_NONE: u8 = 0x1f;
