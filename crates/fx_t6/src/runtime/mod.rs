//! The effects runtime: the rules of the effects system as free functions and
//! small types, which the `fx` crate drives.
//!
//! It reads BO2's element flag bits and keeps BO2's pool of 2048 elements.
//! Each module covers one rule: how an element is placed, sampled, moved,
//! culled or drawn, how a trail is cut, how an effect is bolted to a bone.

mod angles;
mod at_rest;
mod atlas;
mod bolt;
mod collide;
mod cull;
mod draw;
mod effect_def;
mod elem;
mod elem_def;
mod emit;
mod flags;
mod gravity;
mod impact;
mod integrate;
mod life;
mod lighting;
mod orient;
mod orientation;
mod origin;
mod particle_cloud;
mod pool;
mod quat;
mod random;
mod rotate_axis;
mod rotation;
mod sort;
mod spawn;
mod sprite_quad;
mod status;
mod tail_draw;
mod trail;
mod trail_def;
mod trail_runtime;
mod vec;
mod velocity;
mod view;
mod vis_blocker;
mod visual;

pub use angles::{get_elem_angles_axis, mat3_mul};
pub use at_rest::get_at_rest_fraction;
pub use atlas::{FxSpriteAtlasUv, sprite_atlas_uv};
pub use bolt::{
    BoneOrientationRefuse, FX_BOLT_BONE_SHIFT, FX_BOLT_FREE_NONE, FX_BOLT_HANDLE_NONE,
    FX_BOLT_PARENT_IDENTITY_QUAT, FX_BOLT_RECORD_CAPACITY, FX_BOLT_VIEWMODEL_DOBJ_BASE,
    FxGetBoneOrientationRoute, FxUpdateEffectBolt, begin_iterating_over_effects_exclusive,
    bolt_alloc, bolt_bone, bolt_centity_teleport_for_compare, bolt_compose_orientation, bolt_dobj,
    bolt_handle_is_none, bolt_init_next_index, bolt_init_parent_orientation, bolt_mark_lost,
    bolt_pack, bolt_spawn_teleport_bit, end_iterating_over_effects, end_iterating_runs_gc,
    get_bone_orientation_route, quat_mul, stop_effect_has_owned, stop_effect_non_recursive_allows,
    update_effect_bolt,
};
pub use collide::{
    FX_COLLIDE_SUBSTEP_MS, FX_TRACE_MASK, FX_TRACE_MASK_ITEM_CLIP, FxCollideSubstep,
    collide_marks_at_rest, collide_substep_schedule, collision_reflect_base_vel_delta,
    impact_child_speed_allows, sample_reflection_factor, trace_mask,
};
pub use cull::{cull_cloud, cull_elem_for_spawn_allows, cull_elem_light, elem_light_color_bgr};
pub use draw::{FxElemType, draw_elem_handler_present};
pub use effect_def::FxEffectDef;
pub use elem::FX_ELEM_AT_REST_NONE;
pub use elem_def::FxElemDef;
pub use emit::{
    FX_ELEM_EMIT_ORIENT_AXIS, FxEmitSchedule, FxEmitSpawn, emit_dist_range, emit_lerp_origin,
    process_emitting_schedule,
};
pub use flags::{
    FX_ELEM_DIE_ON_TOUCH, FX_ELEM_RUN_MASK, FX_ELEM_RUN_RELATIVE_TO_EFFECT,
    FX_ELEM_RUN_RELATIVE_TO_OFFSET, FX_ELEM_RUN_RELATIVE_TO_OFFSET_EFFECT_NOW,
    FX_ELEM_RUN_RELATIVE_TO_SPAWN, FX_ELEM_RUNNER_USES_RAND_ROT, FX_ELEM_SPAWN_FRUSTUM_CULL,
    FX_ELEM_USE_COLLISION, FX_ELEM_VEL_LOCAL, FX_ELEM_VEL_WORLD, elem_dies_on_touch, elem_run_mode,
    elem_runs_relative_to_offset, elem_skips_position_update, elem_update_has_velocity_graph,
    elem_uses_collision, elem_uses_vel_local, elem_uses_vel_world,
};
pub use gravity::elem_gravity_accel_z_sampled;
pub use impact::{
    FX_IMPACT_EXIT_SURFACE_FLAG, FX_IMPACT_FLESH_COUNT, FX_IMPACT_NONFLESH_COUNT,
    FX_SURF_TYPE_FLESH, FxImpactEntry, FxImpactTable, flesh_effect_index, flesh_hit_flags,
    impact_table_row, surface_type_index,
};
pub use integrate::{FxElemVec3Range, integrate_velocity_graph};
pub use life::{
    sample_life_span_msec, trail_elem_base_vel_z_pack, trail_elem_keep, trail_elem_norm_ages,
};
pub use lighting::{
    apply_lighting_frac_bgra, effect_def_needs_lighting_sample, elem_uses_lighting_frac,
};
pub use orient::{orientation_pos_from_world, orientation_pos_to_world};
pub use orientation::{FxOrientFrame, FxOrientSpawnParams, FxOrientation, get_orientation};
pub use origin::{
    FxSpawnOffsetMode, elem_spawn_offset_mode, spawn_origin_world, world_delta_to_local,
};
pub use particle_cloud::{
    FX_CODE_FOUNTAIN_PARM0, FX_CODE_FOUNTAIN_PARM1, FX_CODE_PARTICLE_CLOUD_COLOR,
    FX_CODE_PARTICLE_CLOUD_MATRIX0, FX_CODE_SPARK_COLOR0, FX_PARTICLE_CLOUD_FLAG_SPARK,
    FX_PARTICLE_CLOUD_GRID_X, FX_PARTICLE_CLOUD_GRID_Y, FX_PARTICLE_CLOUD_GRID_Z,
    FX_PARTICLE_CLOUD_INDICES_PER_CELL, FX_PARTICLE_CLOUD_QUAD_INDICES,
    FX_PARTICLE_CLOUD_TEMPLATE_CELLS, FX_PARTICLE_CLOUD_UV, FX_PARTICLE_CLOUD_VERTS_PER_CELL,
    FX_PARTICLE_SPARK_INDICES_PER_CELL, FX_PARTICLE_SPARK_VERTS_PER_CELL,
    GFX_POS_TEX_VERTEX_STRIDE, GfxParticleCloud, GfxPosTexVertex, MSVCRT_HOLDRAND_DEFAULT,
    build_cloud, gfx_pos_tex_vertex_bytes, msvcrt_rand01, particle_cloud_cell_indices,
    particle_cloud_cell_radius_sq, particle_cloud_cell_verts, particle_cloud_cell_xyz,
    particle_cloud_color_const, particle_cloud_compare_cell_radius, particle_cloud_draw_cell_count,
    particle_cloud_draw_counts, particle_cloud_matrix_diag, particle_cloud_particle_id,
    particle_fountain_parm0, particle_spark_cell_indices, particle_spark_cell_verts,
};
pub use pool::{
    FX_EFFECT_HANDLE_RING_SIZE, FX_EFFECT_POOL_CAPACITY, FX_EFFECT_SLOT_SIZE,
    FX_ELEM_POOL_CAPACITY, FX_ELEM_RUNTIME_STRIDE, FX_ENTITYNUM_WORLD, FX_PLAY_BOLT_NONE,
    FX_RAND_TABLE_MOD, FX_SPAWN_BOLT_NONE, FX_SPOT_LIGHT_LIMIT, FX_TRAIL_ELEM_POOL_CAPACITY,
    FX_TRAIL_ELEM_RUNTIME_STRIDE, FX_TRAIL_POOL_CAPACITY, FX_TRAIL_RUNTIME_STRIDE,
    FX_WARN_ELEM_LIMIT, effect_handle_for_slot, elem_handle_from_ptr_delta,
    trail_elem_handle_for_slot, trail_handle_for_slot,
};
pub use quat::{axis_to_quat, quat_normalize, unit_quat_to_axis};
pub use random::{
    FX_RAND_CH_COLOR, FX_RAND_CH_DELAY, FX_RAND_CH_LIFE, FX_RAND_CH_ONESHOT_COUNT,
    FX_RAND_CH_SCALE, FX_RAND_CH_SIZE0, FX_RANDOM_TABLE_FLOATS, effect_random_seed_from_msec,
    elem_random_seed, elem_visual_index, random_table_f32, random_table_u16, trail_random_seed,
};
pub use rotate_axis::{impact_mark_axis, randomly_rotate_axis, runner_rand_rot_degrees};
pub use rotation::clamp_elem_rotation_time;
pub use sort::{existing_elem_sorts_before_new, sort_dist_to_cam_sq};
pub use spawn::{
    FX_ELEM_TYPE_TRAIL, looping_catchup_begin, looping_spawn_schedule, sample_oneshot_spawn_count,
    spawn_effect_status,
};
pub use sprite_quad::{FX_SPRITE_QUAD_INDICES, FX_SPRITE_QUAD_LOCAL_XY, sprite_quad_indices};
pub use status::{
    FX_STATUS_HAS_PENDING_LOOP_ELEMS, FX_STATUS_REF_COUNT_LOW_MASK, FX_STATUS_REF_COUNT_MASK,
    status_is_unique_done,
};
pub use tail_draw::{tail_anchor_origin, tail_sprite_axes, tail_sprite_full_extent};
pub use trail::{
    FX_TRAIL_TANGENT_PACKED, FxTrailEmittedVert, FxTrailSegmentDrawState, compress_basis_from_axis,
    pack_code_mesh_vertex, trail_compute_u, trail_emit_index_quad, trail_emit_segment_verts,
    trail_index_quad_tris, trail_pack_normal, trail_pack_texcoord, trail_uncompress_basis,
};
pub use trail_def::{FxTrailDef, FxTrailVertex};
pub use trail_runtime::{
    FxTrailSplit, trail_split_interpolant_msec, trail_split_interpolant_t, trail_split_lerp_axis,
    trail_split_lerp_origin, trail_split_skips_update, trail_split_window,
};
pub use vec::{
    effect_orient_arc, perpendicular_vector, vec3_distance, vec3_length_sq, vec3_normalize,
    vector_vectors,
};
pub use velocity::get_velocity_at_time;
pub use view::{FxEffectDefView, FxElemDefView};
pub use vis_blocker::{
    FxVisBlockerBuf, FxVisBlockerRec, evaluate_distance_fade, vis_blocker_add_prepared,
    vis_blocker_generate_verts,
};
pub use visual::{
    elem_norm_time, evaluate_color_bgra, evaluate_rotation_total, evaluate_scale, evaluate_size0,
    evaluate_size1,
};
