//! Black Ops II lighting: the light grid decoded as the game does, what lights a
//! moving thing at run time (grid lookup, sky volumes, sight checks), the lightmaps on the
//! CPU for marks and glass, reflection probe origins and the static models' own
//! grid lights. Installed once per map load by the asset lane.

mod accumulate;
mod atapoint;
mod cull;
mod dlight;
mod entry;
mod expand;
mod fragment;
mod glow;
mod grid;
mod isvalid;
mod light_probe;
mod lit_tech;
mod lookup;
mod model_lighting_cache;
mod pick;
mod reflection_probe;
mod row_rle;
mod runtime;
mod sample;
mod scene_light;
mod smc_layout;
mod smodel_alloc;
mod smodel_bucket;
mod smodel_cache;
mod smodel_cmd;
mod smodel_lighting;
mod smodel_skin;
mod spot_shadow;
mod spot_shadow_choose;
mod spot_shadow_view;
mod viewmodel_origin;

pub use accumulate::{
    LIGHT_GRID_COLORS_BYTE_COUNT, LightGridCompressedColor, light_grid_accumulate_colors,
    light_grid_compress_colors,
};
pub use atapoint::{
    LIGHT_GRID_ATPOINT_EMPTY_PRIMARY, LightGridAtPointEmptyGate, LightGridAtPointPath,
    light_grid_atapoint_return_primary, light_grid_atapoint_select_path,
    light_grid_colors_byte_offset, light_grid_default_colors_index,
    light_grid_fixed_point_blend_weights,
};
pub use cull::{
    DYN_ENT_PRIMARY_LIGHT_LINK_DIST2_INIT, LightRegionAxis, LightRegionHull, LightRegionHulls,
    NonSunPrimaryWalkTrace, PrimaryLightCull, cull_box_from_primary_light,
    dyn_ent_primary_light_link_dist2, light_region_culls_box, lighting_query_box_half,
    non_sun_primary_light_walk_trace,
};
pub use dlight::{
    R_DLIGHT_BACKEND_MAX, R_DLIGHT_LIMIT_DEFAULT, SceneDlight, append_scene_dlights_to_backend,
    dlight_hits_aabb,
};
pub use entry::GfxLightGridEntry;
pub use expand::{
    LIGHT_GRID_COMPRESS_DIRS, MODEL_LIGHTING_ATLAS_DEPTH, MODEL_LIGHTING_ATLAS_WIDTH,
    MODEL_LIGHTING_INV_ATLAS_WIDTH, MODEL_LIGHTING_TILE_BYTES, MODEL_LIGHTING_TILE_DIM,
    MODEL_LIGHTING_VOLUME_W, ModelLightingAtlasDims, ModelLightingCoords, ModelLightingLookupScale,
    ModelLightingTileIndex, TEXTURE_SRC_CODE_LIGHT_ATTENUATION, TEXTURE_SRC_CODE_MODEL_LIGHTING,
    light_grid_expand_shell_to_tile_rgba, model_lighting_atlas_dims,
    model_lighting_coords_from_handle, model_lighting_inv_image_height,
    model_lighting_lookup_scale, model_lighting_write_tile_to_atlas,
};
pub use fragment::{
    decode_model_lighting_sample, lit_albedo, lit_fragment_color, lit_sun_lighting,
    model_lighting_local_tile_nearest_index,
};
pub use glow::{
    CONST_SRC_CODE_GLOW_APPLY, CONST_SRC_CODE_GLOW_SETUP, GlowBloomConsts, GlowViewInfo,
    R_FULLBRIGHT_DEFAULT, R_GLOW_DEFAULT, R_GLOW_USE_TWEAKS_DEFAULT, select_glow_view_info,
    set_glow_info, using_glow,
};
pub use grid::*;
pub use isvalid::{LIGHT_GRID_SIGHT_CONTENT_MASK, light_grid_isvalid_sample};
pub use light_probe::light_probe_ambient_from_packed;
pub use lit_tech::{
    GFX_DRAW_METHOD_LIT_BEGIN, LIT_TECH_NO_SHADOW_DIR_SLOTS, LIT_TECH_NO_SHADOW_LOCAL_SLOTS,
    LIT_TECH_SHADOW_DIR_SLOTS, LIT_TECH_SHADOW_SPOT_SLOTS, LIT_TECH_STANDARD_ROW, TECHNIQUE_DEBUG,
    TECHNIQUE_EMISSIVE, TECHNIQUE_LIGHT_OMNI, TECHNIQUE_LIGHT_SPOT, TECHNIQUE_LIGHT_SPOT_SHADOW,
    TECHNIQUE_SUN_SHADOW_CASTER, TECHNIQUE_UNLIT, additional_light_tech_type, is_lit_remap_slot,
    lit_tech_type,
};
pub use lookup::{
    LightGridLookupColorAccum, LightGridLookupCorner, LightGridLookupWeights,
    light_grid_lookup_accumulate_corners, light_grid_lookup_remap_primary,
};
pub use model_lighting_cache::{
    MODEL_LIGHTING_PIXEL_FREE_BITS_BUFFERS, ModelLightingCacheAlloc, ModelLightingCacheGlob,
    ModelLightingCacheGlobError, dyn_pixel_free_bits_index, lighting_info_from_bytes,
    lighting_info_reflection_probe_index, lighting_info_scene_light_index,
    model_lighting_cache_free_bits_words, model_lighting_cache_slot,
    toggle_dyn_model_lighting_frame,
};
pub use pick::{
    LightGridCell, LightGridPickCorner, LightGridPickPrimary, LightGridPickRowRleCells,
    light_grid_axis_lerps, light_grid_corner_needs_trace, light_grid_corner_weight_keeps_entry,
    light_grid_corner_weights, light_grid_pick_default_grid_entry,
    light_grid_pick_merge_entry_quads, light_grid_pick_primary_from_corners,
    light_grid_pick_row_rle_cells, light_grid_primary_light_sun_band, light_grid_sample_cell,
};
pub use reflection_probe::{
    nearest_reflection_probe_in_cell_list, nearest_reflection_probe_skip_0,
};
pub use row_rle::{
    LIGHT_GRID_ROW_ABSENT, LIGHT_GRID_ROW_HEADER_SIZE, LightGridEntryQuad, LightGridRleCursor,
    LightGridRowHeader, light_grid_at_row_last_column, light_grid_column_before_row_start_quad,
    light_grid_empty_run_entry_quad, light_grid_rle_advance_to_column,
    light_grid_row_contains_cell, light_grid_solid_run_entry_quad, light_grid_solid_run_z_base,
};
pub use runtime::*;
pub use sample::*;
pub use scene_light::{
    AddOmniLightRefuse, CONST_SRC_CODE_LIGHT_DIFFUSE, CONST_SRC_CODE_LIGHT_FALLOFF_PLACEMENT,
    CONST_SRC_CODE_LIGHT_POSITION, CONST_SRC_CODE_LIGHT_SPECULAR, CONST_SRC_CODE_LIGHT_SPOTDIR,
    CONST_SRC_CODE_LIGHT_SPOTFACTORS, GFX_LIGHT_TYPE_DIR, GFX_LIGHT_TYPE_OMNI, GFX_LIGHT_TYPE_SPOT,
    GFX_LIGHT_TYPE_SPOT_ROUND, GFX_LIGHT_TYPE_SPOT_SQUARE, GfxLightPack, R_COLOR_SCALE_DEFAULT,
    ShadowableLightPack, SpotLightConeDvars, add_omni_light_to_scene_allows, color_srgb_to_linear,
    dir_light_position, gfx_light_is_spot, light_diffuse, light_specular, omni_light_pack,
    pack_shadowable_light, spot_light_pack,
};
pub use smc_layout::{
    SMC_BANK_N, SMC_BANK_VB_BYTES, SMC_BANK_VERTS, SMC_CLASS_N, SMC_INDEX_U16_N, SMC_LEAF_N,
    SMC_LINK_N, SMC_TREE_N, SMC_VB_BYTES, SmcIndexBakeError, SmcLodCacheSpec, SmcPatchLock,
    cache_static_model_indices, patch_static_model_cache_lock, smc_lod_cache_spec,
    smc_stream_source_byte_offset,
};
pub use smodel_alloc::{
    SModelDirtyLightingAction, SModelLightingAlloc, SModelLightingCounters, SModelLightingGlob,
    SModelLightingGlobError, dirty_smodel_lighting_action, smodel_lighting_bits_words,
};
pub use smodel_bucket::{
    SMODEL_BUCKET_CACHED, SMODEL_BUCKET_CAP, SMODEL_BUCKET_LIST_N, SMODEL_BUCKET_PRETESS,
    SMODEL_BUCKET_RIGID, SMODEL_BUCKET_SKINNED, SMODEL_BUCKET_STRIDE, SmodelBucketPush,
    SmodelConsumedBucket, SmodelSurfBucketLists, SmodelSurfPath, add_static_model_surf_to_bucket,
    smodel_bucket_lists_consume, smodel_bucket_mask, smodel_bucket_store_payload, smodel_dest_path,
    smodel_lod_is_rigid, smodel_surf_bucket_push, smodel_surf_sun_shadow_emits, smodel_surf_type,
};
pub use smodel_cache::{
    CacheStaticModelSurface, SmcAllocatorStorage, SmcCacheError, SmcLeafStorage, SmcTree,
    StaticModelCache,
};
pub use smodel_cmd::{
    SmodelCachedCmdPlan, SmodelCmdKind, SmodelPretessAlloc, smodel_cached_cmd_plan,
    smodel_pretess_indices_copy, smodel_same_bank_count,
};
pub use smodel_lighting::{
    MODEL_LIGHTING_WARN_CACHE_ALLOC_FAILED, SMC_CACHE_INDEX_LODS, SMODEL_LIGHTING_WARN_TOO_MUCH,
    STATIC_MODEL_FLAG_NO_CAST_SHADOW, model_lighting_handle_from_entry,
};
pub use smodel_skin::{
    SmcCachedVertLighting, SmcSkinError, SmcSkinSurface, setup_transform_unit_vec,
    skin_cached_static_model_cmd, skin_cached_static_model_cmd_matrix,
};
pub use spot_shadow::{
    GFX_SPOT_SHADOW_RT, SM_SPOT_QUALITY_DEFAULT, SPOT_SHADOW_CFG_INDEX_ENTS,
    SPOT_SHADOW_ENT_MARK_LEN, SPOT_SHADOW_ENTITY_ORIGIN_TRACK_ENTS,
    SPOT_SHADOW_ENTITY_RELINK_THRESH_SQ, SPOT_SHADOW_SCENE_DOBJ_VIS_STRIDE,
    SPOT_SHADOW_SCENE_MODEL_VIS_STRIDE, SpotShadowCasterIn, SpotShadowOccupancyError,
    SpotShadowSceneSlot, SpotShadowSlotPlan, spot_shadow_175d0_pose_ok,
    spot_shadow_dyn_brush_tess_admits, spot_shadow_dyn_brush_vis_bit_index,
    spot_shadow_entity_origin_track_index, spot_shadow_entity_should_relink,
    spot_shadow_fill_occupancy, spot_shadow_fill_scene_dobj_vis, spot_shadow_fill_scene_model_vis,
    spot_shadow_filter_smodel_ids, spot_shadow_link_dyn_brush_vis, spot_shadow_link_entity,
    spot_shadow_packed_lists_ready, spot_shadow_primary_vis_bit,
    spot_shadow_primary_vis_word_count, spot_shadow_slot_plan, spot_shadow_smodel_casts,
    spot_shadow_take_slot, spot_shadow_tess_bsp_surfs, spot_shadow_tess_scene_dobj_indices,
    spot_shadow_tess_scene_model_indices, spot_shadow_tile, spot_shadow_walk_casters,
    spot_shadow_xmodel_is_caster,
};
pub use spot_shadow_choose::{
    SM_SUN_ENABLE_DEFAULT, SPOT_SHADOW_SCORE_LUMA, SPOT_SHADOW_SM_LIGHT_CAP, SpotShadowChoose,
    SpotShadowChooseDvars, SpotShadowEmit, SpotShadowEmittedSlot, SpotShadowFrontend,
    SpotShadowHistory, SpotShadowHistoryEntry, SpotShadowableLight, clear_used_bits_1_through_sun,
    generate_clears_used_bits_through_sun, generate_runs_choose, set_used_bit, spot_shadow_choose,
    spot_shadow_emit_frontend,
};
pub use viewmodel_origin::viewmodel_lighting_origin;
