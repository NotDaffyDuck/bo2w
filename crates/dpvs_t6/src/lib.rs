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

mod aabb;
mod bsp_drawsurf;
mod cache;
mod cell;
mod cell_caster;
mod chop;
mod clip;
mod drawsurf;
mod dynent;
mod hull;
mod material_key;
mod plane;
mod scene_dobj;
mod scene_ent;
mod scene_skin;
mod sun_shadow_bsp;
mod sun_shadow_smodel;
mod vis;
mod walk;
mod xmodel_lod;

pub use aabb::{
    AABB_TREE_SIZE, AabbCullStats, AabbTree, AabbTreeCull, Bounds, DpvsVisData,
    add_aabb_tree_surfaces_in_frustum, bounds_culled, bounds_from_origin_axis,
};
pub use bsp_drawsurf::{
    BSP_DRAW_SURF_RUN_MAX, BspDrawSurfCensus, BspDrawSurfRun, add_bsp_draw_surfs_camera,
};
pub use cache::{CachedCell, CellClip, PLANE_POOL_MAX};
pub use cell::{CPlane, Cell, DpvsPlanes, DpvsWorld, Portal, cell_for_point};
pub use cell_caster::{
    cell_caster_matrix_words, cell_caster_row_words, generate_shadow_map_caster_cells,
    or_caster_rows_for_visible,
};
pub use chop::{CHOP_EPSILON, chop_portal, chop_winding};
pub use clip::{
    Bevels, CLIP_PLANE_EPSILON, CLIP_SIDE_LIMIT, MAX_CLIP_PLANES, PROJECT_W_MIN, add_bevel_planes,
    portal_clip_planes,
};
pub use drawsurf::{
    DRAW_SURF_BSP_CAMERA_EMISSIVE_OPAQUE, DRAW_SURF_BSP_CAMERA_EMISSIVE_TRANS,
    DRAW_SURF_BSP_CAMERA_LIGHT_MAP, DRAW_SURF_BSP_CAMERA_LIT, DRAW_SURF_BSP_CAMERA_LIT_TRANS,
    DRAW_SURF_STAGE_COUNT, GfxDrawSurf, GfxDrawSurfFields, SF_BMODEL, SF_CODE_MESH,
    SF_GLASS_MESH, SF_MARK_MESH, SF_PARTICLE_CLOUD, SF_ROPE_MESH,
    SF_STATICMODEL_RIGID, SF_STATICMODEL_RIGID_NO_SUN_SHADOW, SF_STATICMODEL_SKINNED,
    SF_TRIANGLES, SF_TRIANGLES_NO_SUN_SHADOW, SF_XMODEL_RIGID, SF_XMODEL_RIGID_NO_SUN_SHADOW,
    SF_XMODEL_RIGID_SKINNED, SF_XMODEL_SKINNED, SF_XMODEL_SKINNED_NO_SUN_SHADOW,
    material_rebind_count, pack, pack_code_mesh_draw_surf, pack_glass_mesh_draw_surf,
    pack_mark_mesh_draw_surf, pack_particle_cloud_draw_surf, pack_rope_mesh_draw_surf,
    pack_xmodel_rigid_draw_surf, pack_xmodel_rigid_skinned_draw_surf, sort_keys,
    stage_capacity, unpack, with_dlight_mask, with_object_id, with_prepass,
    with_primary_light_index, with_reflection_probe_index, with_surf_type,
};
pub use dynent::{
    add_dyn_ent_to_cell, cell_dyn_ent_words, dyn_brush_scene_list_admits, dyn_ent_cell_bits_len,
    dyn_ent_client_word_count, dyn_ent_in_cell, dyn_ent_primary_light_vis_word_count,
    filter_dyn_ent_into_cells, link_dyn_ent_primary_light_bit, unfilter_dyn_ent_from_cells,
    unlink_dyn_ent_from_primary_lights, unlink_dyn_ent_primary_light_bit_index,
    write_dyn_brush_vis,
};
pub use hull::{HULL_POINTS_MAX, HullPoints, convex_hull, hull_point, hull_uv};
pub use material_key::{
    MATERIAL_GAME_FLAG_CASTS_SHADOW, MaterialPrepassInput, SURF_FLAG_CASTS_SHADOW,
    SURF_FLAG_CASTS_SUN_SHADOW, SURF_FLAG_NO_COLOR, SURF_FLAG_NO_DRAW,
    SURF_FLAG_PRIMARY_LIGHT_CONFLICT, SURF_FLAG_QUANTIZED, SURF_FLAG_SKY, SurfaceCasters,
    TECHNIQUE_FLAG_PREPASS_3, bake_material_draw_surf_key, fill_surface_materials,
    material_prepass, material_sort_key_row, stored_world_surface_key, surface_casters,
    world_surface_material,
};
pub use plane::{
    box_culled, box_culled_by, plane_dist, positive_vertex, view_plane, winding_behind_plane,
};
pub use scene_dobj::{
    DObjAnimMat, GFX_CFG_ENT_COUNT, SCENE_DOBJ_CULL_BOUNDED, SCENE_DOBJ_CULL_BOUNDING,
    SCENE_DOBJ_CULL_DONE, SCENE_DOBJ_CULL_IDLE, SCENE_DOBJ_CULL_SKIN_PENDING,
    mark_scene_ent_visible, scene_dobj_bounds_pending, scene_dobj_cull_begin,
    scene_dobj_initial_bounds, scene_ent_child_planes, scene_ent_is_visible,
    scene_ent_sphere_culled,
};
pub use scene_ent::{
    SCENE_ENT_CELL_ROW_WORDS, SCENE_ENT_CELL_VIEW_BANKS, add_scene_ent_to_cell,
    filter_bmodel_into_cells, filter_scene_ent_into_cells, scene_ent_box_reaches_cell,
    scene_ent_cell_bits_len, scene_ent_cell_row, scene_ent_cell_row_second_pass,
    scene_ent_cell_walk_bits, scene_ent_cells, unfilter_scene_ent_from_cells_view0,
};
pub use scene_skin::{
    PreSkinModel, PreSkinSummary, PreSkinSurface, SCENE_DOBJ_CULL_LOD_COUNT,
    SKIN_BLEND_WEIGHT_SCALE, SKIN_FRAME_BYTES, SKIN_SURFACE_BYTES, SKIN_SURFACE_SKIP_TAG,
    SKIN_VERT_LIST_BYTES, SceneEntSkinEntry, pre_skin_scene_ent, scene_dobj_flag_skinned,
    scene_dobj_skin_begin, scene_dobj_skinned_surface_count, scene_ent_surface_hidden,
    shift_part_bits, skin_surface_bytes, skin_transform_point, skin_weighted_point,
};
pub use sun_shadow_bsp::add_bsp_draw_surfs_range_sun_shadow;
pub use sun_shadow_smodel::{
    SMODEL_FLAG_DRAW_LV, SMODEL_FLAG_NO_CAST_SHADOW, SUN_PRIMARY_LIGHT, SmodelShadowInst,
    SunShadowLodParms, add_smodel_range_sun_shadow, smodel_cull_dist_hides,
    smodel_shadow_culled,
};
pub use walk::{
    EYE_EPSILON, OCCLUDER_MAX, OCCLUDER_PLANES, POP_EYE_LIMIT, QUEUED_PORTAL_MAX,
    START_PLANE_LIMIT, WalkParams, WalkScratch, WalkStats, visit_portals,
    visit_portals_no_frustum,
};
pub use vis::{
    MsbBitIter, VisBits, bit_get, bit_set, msb_get, msb_iter, msb_set, words_for_bits,
};
pub use xmodel_lod::{
    XMODEL_MAX_LODS, lod_adjusted_dist, lod_base_dist, smodel_lod_dist, xmodel_get_lod_for_dist,
};
