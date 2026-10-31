//! Black Ops II lighting: the light grid decoded as the game does, what lights a
//! moving thing at run time (grid lookup, sky volumes, sight checks), the lightmaps on the
//! CPU for marks and glass, reflection probe origins and the static models' own
//! grid lights. Installed once per map load by the asset lane.

mod grid_tint;
mod point_route;
mod light_reach;
mod transient_lamps;
mod grid_entry;
mod tile_unpack;
mod shade_bits;
mod bloom_view;
mod grid;
mod sample_ok;
mod probe_light;
mod tech_rows;
mod illum_lookup;
mod model_illum_cache;
mod corner_pick;
mod cube_probe;
mod run_rows;
mod runtime;
mod sample;
mod stage_light;
mod smc_layout;
mod prop_alloc;
mod smodel_bucket;
mod prop_cache;
mod prop_cmds;
mod prop_light;
mod prop_skin;
mod cone_shadow;
mod cone_pick;
mod spot_shadow_view;
mod held_view_origin;

pub use grid_tint::{
    TINT_PAYLOAD_LEN, PackedCellTint, sum_cell_tints,
    pack_cell_tints,
};
pub use point_route::{
    GRID_SAMPLE_VACANT_SUN_BAND, GridProbeEmptyGuard, GridProbeFallbackBranch,
    point_primary_light, pick_point_route,
    tint_table_start, fallback_tint_index,
    grid_quantized_mix_factors,
};
pub use light_reach::{
    PROP_KEY_LIGHT_LINK_FARTHEST_SQUARED, LightVolumeSpan, LightVolumeHull, LightVolumeHullList,
    SecondaryLightWalkLog, LampReach, cull_volume_for_main_light,
    mover_relink_dist_sq, hulls_reject_box, illum_query_half_size,
    secondary_main_light_walk_trace,
};
pub use transient_lamps::{
    TRANSIENT_LAMP_GPU_CEILING, TRANSIENT_LAMP_INITIAL_CAP, BriefLamp, forward_scene_dynamic_lights_to_renderer,
    brief_lamp_hits_box,
};
pub use grid_entry::GridCellEntry;
pub use tile_unpack::{
    GRID_PACKED_DIRECTION_CELLS, MODEL_LIGHT_ATLAS_LAYERS, MODEL_LIGHT_ATLAS_TEXELS_ACROSS,
    ATLAS_INV_WIDTH, ILLUM_TILE_LEN, ILLUM_TILE_SIDE,
    MODEL_LIGHT_VOLUME_DEPTH_SPAN, ModelIllumAtlasDims, ModelIlluminationCoords3d, ModelIlluminationLookupFactor,
    IllumTileNo, TEX_SLOT_LAMP_FALLOFF, TEX_SLOT_ILLUM_ATLAS,
    widen_shell_into_tile, illum_atlas_dims,
    light_atlas_coords_for_handle, light_atlas_reciprocal_height,
    atlas_lookup_scale, blit_illum_tile,
};
pub use shade_bits::{
    unpack_model_light_reading, shaded_base_color, shaded_pixel_colour, sunlit_shading,
    light_tile_closest_direction,
};
pub use bloom_view::{
    ENGINE_CONST_BLOOM_COMPOSE, ENGINE_CONST_BLOOM_PREPARE, BloomShaderConstants, GlowSettings,
    UNSHADED_MODE_INITIAL, BLOOM_INITIAL_ON, BLOOM_CUSTOM_SETTINGS_INITIAL, select_glow_view_info,
    set_glow_info, using_glow,
};
pub use grid::*;
pub use sample_ok::{GRID_LINE_OF_SIGHT_CONTENT_BITS, sample_counts};
pub use probe_light::probe_ambient_from_bytes;
pub use tech_rows::{
    LIT_DRAW_PATH_FIRST, UNSHADOWED_SUN_ROWS, UNSHADOWED_LOCAL_ROWS,
    SHADOWED_SUN_ROWS, SHADOWED_CONE_ROWS, PLAIN_LIT_ROW, TECHNIQUE_DEBUG,
    TECHNIQUE_EMISSIVE, TECHNIQUE_LIGHT_OMNI, TECHNIQUE_LIGHT_SPOT, TECHNIQUE_LIGHT_SPOT_SHADOW,
    TECHNIQUE_SUN_SHADOW_CASTER, TECHNIQUE_UNLIT, additional_light_tech_type, slot_is_lit_substitute,
    lit_technique_kind,
};
pub use illum_lookup::{
    GridLookupTintAccumulator, GridProbeCornerSample, GridProbeBlendFactors,
    grid_sum_corner_contributions, grid_remap_main_light,
};
pub use model_illum_cache::{
    MODEL_LIGHT_VACANCY_BITSET_COUNT, IllumSlotGrant, ModelLightCacheBundle,
    IllumBlobFault, free_texel_word, illum_word_from_bytes,
    lighting_word_reflection_probe, light_info_to_scene_light_slot,
    illum_free_word_count, illum_entry_slot,
    toggle_debris_model_illum_frame,
};
pub use corner_pick::{
    IlluminationVoxelRef, GridCornerChoice, GridDominantLightChoice, GridPickRunLengthSpan,
    axis_mix, corner_wants_trace, corner_keeps_weight,
    corner_mix, fallback_cell_entry,
    grid_pick_combine_cell_quartets, grid_choose_main_light_from_corners,
    packed_row_cells, sun_band_light, cell_under,
};
pub use cube_probe::{
    nearest_cube_in_cells, nearest_cube_probe,
};
pub use run_rows::{
    ROW_NOT_SET, ROW_PREAMBLE_LEN, RunQuad, RunWalker,
    RowPreamble, on_last_column, column_before_quad,
    grid_vacant_span_quad, grid_compressed_row_seek_column,
    row_holds_cell, filled_quad, solid_run_floor,
};
pub use runtime::*;
pub use sample::*;
pub use stage_light::{
    PointLampRefusal, ENGINE_CONST_LAMP_DIFFUSE, ENGINE_CONST_LAMP_FALLOFF_ORIGIN,
    ENGINE_CONST_LAMP_ORIGIN, ENGINE_CONST_LAMP_GLOSS, ENGINE_CONST_LAMP_AIM,
    ENGINE_CONST_LAMP_CONE_TERMS, LAMP_KIND_PARALLEL, POINT_LAMP, CONE_LAMP,
    GFX_LIGHT_TYPE_SPOT_ROUND, GFX_LIGHT_TYPE_SPOT_SQUARE, LampRecord, COLOR_GAIN_INITIAL,
    ConeLightTuningVars, add_omni_light_to_scene_allows, gamma_encoded_to_linear,
    sun_point_at_infinity, is_cone_lamp, lamp_diffuse_rgba, lamp_gloss_rgba, pack_point_lamp,
    spot_light_pack,
};
pub use smc_layout::{
    BANK_COUNT, PROP_CACHE_BANK_VB_BYTES, BANK_VERTEX_CAP, PROP_CLASS_COUNT, PROP_CACHE_INDEX_U16_TALLY, LEAF_COUNT,
    LINK_COUNT, TREE_COUNT, SMC_VERTEX_BYTES, PropIndexBakeFault, StaticModelLodCacheLayout, PropCachePatchLock,
    bake_prop_indices, lock_prop_cache_patch, prop_lod_cache_plan,
    smc_stream_source_byte_offset,
};
pub use prop_alloc::{
    StaleIllumAction, SModelIllumAlloc, SModelIllumCounters, StaticModelLightingBlob,
    PropLightBlobFault, stale_illum_action, prop_illum_word_count,
};
pub use smodel_bucket::{
    STATIC_BUCKET_KIND_CACHED, STATIC_MODEL_BUCKET_CAPACITY, PROP_BUCKET_LIST_COUNT, STATIC_BUCKET_KIND_PRESPLIT,
    STATIC_BUCKET_KIND_FIXED_MESH, STATIC_BUCKET_KIND_BONE_DRIVEN, STATIC_BUCKET_KIND_SPAN, StaticModelBucketInsertResult,
    StaticModelSpentBucket, PropFacetBucketLists, PropFacetPath, add_static_model_surf_to_bucket,
    static_bucket_lists_drain, smodel_bucket_mask, smodel_bucket_store_payload, smodel_dest_path,
    prop_lod_rigid, push_prop_facet, prop_facet_casts_sun, prop_facet_kind,
};
pub use prop_cache::{CachePropFacet, PropCacheBlocks, SmcCacheRow, StaticMeshResidencyCache};
pub use prop_cmds::{
    PropCmdReplay, StaticModelCmdVariety, PropPretessGrant, plan_cached_prop_cmds,
    copy_presplit_indices, same_bank_run,
};
pub use prop_light::{
    MODEL_ILLUMINATION_WARN_SLOT_GRANT_FAILED, STATIC_MODEL_CACHE_DETAIL_LEVELS, STATIC_LIGHT_OVERUSE_WARN_LIMIT,
    PROP_FLAG_NO_CAST_SHADOW, model_illumination_handle_for_entry,
};
pub use prop_skin::{
    CachedVertexIllum, PropSkinFault, SkinnedCacheFacet, unit_axis_transform,
    skin_cached_prop_order, skin_cached_static_model_cmd_matrix,
};
pub use cone_shadow::{
    GFX_SPOT_SHADOW_RT, SM_SPOT_QUALITY_DEFAULT, SPOT_SHADOW_CONFIG_ENTITY_LIMIT,
    SPOT_SHADOW_ENTITY_MARK_CAPACITY, SPOT_SHADOW_ORIGIN_TRACKED_LIMIT,
    SPOT_SHADOW_RELINK_MOVE_DIST_SQUARED, SPOT_SHADOW_ANIMATED_VISIBILITY_SPAN,
    SPOT_SHADOW_STATIC_VISIBILITY_SPAN, ConeLightOccluderFeed, ConeShadowFillFault,
    ConeViewSlot, ConeSlotPlan, cone_pose_ready,
    mover_brush_admitted, debris_lit_bit,
    cone_origin_slot, cone_relink_due,
    populate_spot_shadow_occupancy, populate_spot_animated_visibility, populate_spot_static_visibility,
    cone_shadow_prune_static_ids, cone_shadow_link_debris_solid_vis, cone_shadow_link_entity,
    cone_lists_ready, lamp_lit_bit,
    spot_shadow_main_light_visibility_words, plan_cone_slots, spot_shadow_static_model_casts,
    claim_cone_slot, cone_world_facets, split_moving_ids,
    cone_shadow_stitch_stage_model_indices, spot_shadow_tile, cone_shadow_walk_casters,
    model_casts_cone,
};
pub use cone_pick::{
    SUN_SHADOW_ENABLED_BY_DEFAULT, LAMP_LUMA_MIX, CONE_TILE_COUNT, SpotShadowSelector,
    ConeShadowChooseTunables, SpotShadowSubmission, SpotShadowSubmittedSlot, SpotShadowFrontStage,
    ConePickMemory, ConeCasterOffer, unmark_lights_to_sun,
    sun_off_frees_lights, cone_pick_enabled, mark_light_used, pick_cone_shadows,
    submit_spot_shadow_frontend,
};
pub use held_view_origin::held_view_light_point;
