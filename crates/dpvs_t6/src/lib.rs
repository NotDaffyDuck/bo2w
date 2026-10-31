//! Black Ops II's DPVS (dynamic potentially visible set): which cells of the map the camera
//! sees through the portals between them, and the clip planes each visible cell is seen
//! through. Built from BO2's GfxWorld layouts (GfxCell 48 bytes, GfxPortal 92, DpvsPlane 20,
//! GfxWorldDpvsPlanes 16) and the order of steps of R_AddWorldSurfacesPortalWalk,
//! R_VisitPortalsForCell, R_NextQueuedPortal and R_DpvsBoxIsVisible.
//!
//! What BO2 adds over the MW2 walk: a portal the eye stands in (within 0.125 of its plane) is
//! stepped through at once instead of queued; a portal reached twice keeps the smaller
//! recursion depth and forgets its queued parent when the parents differ; every visit of a
//! cell is kept with its own plane set (cached cells), not only the first; force-invisible
//! cells; occluders.
#![no_std]
#![forbid(unsafe_code)]

extern crate alloc;

mod box_tree;
mod world_faces;
mod cache;
mod cell;
mod shadow_cells;
mod portal_cut;
mod cut;
mod sort_bits;
mod debris_bit;
mod hull;
mod surf_key;
mod plane;
mod stage_puppet;
mod stage_actor;
mod scene_skin;
mod world_shadow;
mod prop_shadow;
mod cell_bits;
mod portal_crawl;
mod mesh_detail;

pub use box_tree::{
    AABB_TREE_SIZE, AabbHideStats, AabbTree, BoxTreeVisibilityWalk, Bounds, VisibilityResultSet,
    admit_box_tree_surfaces_inside_frustum, box_outside_planes, box_around_oriented_frame,
};
pub use world_faces::{
    BSP_DRAW_SURF_RUN_MAX, LevelTreeFacetDrawCensus, WorldFaceRun, queue_world_faces_for_view,
};
pub use cache::{CachedCell, CellClip, PLANE_POOL_MAX};
pub use cell::{PlaneEquation, Cell, VisibilityCellPlanes, DpvsWorld, Portal, cell_holding};
pub use shadow_cells::{
    cell_caster_grid_word_total, cell_caster_bitset_width, collect_shadow_caster_cells,
    merge_caster_rows_of_visible,
};
pub use portal_cut::{CHOP_EPSILON, slice_by_portal, chop_winding};
pub use cut::{
    Bevels, CLIP_PLANE_EPSILON, CLIP_SIDE_LIMIT, CLIP_PLANE_CEILING, PROJECT_W_MIN, append_edge_bevel_planes,
    portal_frustum_planes,
};
pub use sort_bits::{
    DRAW_SURF_BSP_CAMERA_EMISSIVE_OPAQUE, DRAW_SURF_BSP_CAMERA_EMISSIVE_TRANS,
    DRAW_SURF_BSP_CAMERA_LIGHT_MAP, DRAW_SURF_BSP_CAMERA_LIT, DRAW_SURF_BSP_CAMERA_LIT_TRANS,
    DRAW_SURF_STAGE_COUNT, SortedDraw, SortedDrawParts, SF_BMODEL, KIND_CODE_MESH,
    SURFACE_KIND_GLASS_MESH, SURFACE_KIND_DECAL_MESH, SURFACE_KIND_PARTICLE_CLOUD, SF_ROPE_MESH,
    SF_STATICMODEL_RIGID, SF_STATICMODEL_RIGID_NO_SUN_SHADOW, SF_STATICMODEL_SKINNED,
    SF_TRIANGLES, SF_TRIANGLES_NO_SUN_SHADOW, SURFACE_KIND_XMODEL_RIGID, SF_XMODEL_RIGID_NO_SUN_SHADOW,
    SURFACE_KIND_MODEL_RIGID_BONED, SF_XMODEL_SKINNED, SF_XMODEL_SKINNED_NO_SUN_SHADOW,
    material_switch_count, pack, encode_engine_mesh_sort_key, encode_window_pane_sort_key,
    pack_stain_mesh_facet_draw, encode_particle_cloud_sort_key, pack_rope_mesh_draw_surf,
    pack_rigid_model_surface, encode_model_bone_driven_sort_key, order_draws,
    stage_capacity, unpack, with_dlight_mask, set_object_tag, with_prepass,
    with_key_light, with_env_probe_index, with_surf_type,
};
pub use debris_bit::{
    mark_debris_in_cell, debris_words_of_cell, debris_brush_listed, debris_cell_mask_len,
    debris_viewer_words, debris_marked_in_cell, moving_entity_main_light_visibility_words,
    filter_debris_into_rooms, attach_dynamic_prop_key_light_bit, unfilter_dynamic_ent_from_cells,
    detach_moving_entity_from_main_lights, detach_dynamic_prop_key_light_slot,
    write_debris_solid_vis,
};
pub use hull::{HULL_POINTS_MAX, HullPoints, convex_hull, hull_point, hull_uv};
pub use surf_key::{
    MATERIAL_GAME_FLAG_CASTS_SHADOW, MaterialPrepassInput, SURF_FLAG_CASTS_SHADOW,
    SURF_FLAG_CASTS_SUN_SHADOW, SURF_FLAG_NO_COLOR, SURF_FLAG_NO_DRAW,
    SURF_FLAG_PRIMARY_LIGHT_CONFLICT, SURF_FLAG_QUANTIZED, SURF_FLAG_SKY, SurfaceCasters,
    TECHNIQUE_FLAG_PREPASS_3, material_order_key, populate_surface_material_table,
    prepass_flag_of, material_sort_group, stored_world_surface_key, surface_casters,
    static_surface_material,
};
pub use plane::{
    box_culled, box_culled_by, plane_dist, positive_vertex, facing_plane, winding_behind_plane,
};
pub use stage_puppet::{
    BonePose, VISUAL_CFG_ACTOR_TALLY, SCENE_DOBJ_CULL_BOUNDED, SCENE_DOBJ_CULL_BOUNDING,
    SCENE_DOBJ_CULL_DONE, SCENE_DOBJ_CULL_IDLE, SCENE_DOBJ_CULL_SKIN_PENDING,
    stain_stage_actor_visible, scene_dobj_bounds_pending, scene_dobj_cull_begin,
    animated_object_starting_box, scene_ent_child_planes, entity_flagged_visible,
    scene_ent_sphere_culled,
};
pub use stage_actor::{
    ENTITY_CELL_BITSET_WORDS, ENTITY_CELL_VISIBILITY_SETS, mark_actor_in_cell,
    sort_brush_model_into_cells, sort_scene_entity_into_cells, actor_box_touches_cell,
    entity_cell_bitset_length, entity_cell_bitset_row, entity_cell_row_later_pass,
    actor_cell_mask, scene_ent_cells, clear_actor_cells_main_view,
};
pub use scene_skin::{
    PreposedModelInput, PreSkinningDigest, PreposedSurfaceInput, SCENE_DOBJ_CULL_LOD_COUNT,
    SKIN_WEIGHT_UNIT, SKIN_FRAME_BYTES, SKIN_SURFACE_BYTES, SKIN_SURFACE_SKIP_TAG,
    SKIN_VERT_LIST_BYTES, EntitySkinSlotRecord, bake_scene_entity_bones_early, scene_dobj_flag_skinned,
    scene_dobj_skin_begin, scene_dobj_skinned_surface_count, scene_surface_is_hidden,
    shift_part_bits, skin_surface_bytes, skin_transform_point, skin_weighted_point,
};
pub use world_shadow::add_bsp_draw_surfs_range_sun_shadow;
pub use prop_shadow::{
    SMODEL_FLAG_DRAW_LV, SMODEL_FLAG_NO_CAST_SHADOW, SUN_PRIMARY_LIGHT, SmodelShadowInst,
    SunShadowLodParms, queue_static_range_sun_shadow, prop_beyond_cull_range,
    smodel_shadow_culled,
};
pub use portal_crawl::{
    EYE_EPSILON, OCCLUDER_MAX, OCCLUDER_PLANES, POP_EYE_LIMIT, QUEUED_PORTAL_MAX,
    START_PLANE_LIMIT, WalkParams, TraversalScratchpad, TraversalTally, visit_portals,
    crawl_portals_unbounded,
};
pub use cell_bits::{
    HighBitFirstBitWalker, VisibilityBitView, bit_get, bit_set, top_bit_get, msb_iter, top_bit_set, word_span,
};
pub use mesh_detail::{
    XMODEL_MAX_LODS, lod_adjusted_dist, lod_base_dist, smodel_lod_dist, pick_detail_tier_by_range,
};
