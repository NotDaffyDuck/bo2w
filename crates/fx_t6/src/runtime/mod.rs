//! The effects runtime: the rules of the effects system as free functions and
//! small types, which the `fx` crate drives.
//!
//! It reads BO2's element flag bits and keeps BO2's pool of 2048 elements.
//! Each module covers one rule: how an element is placed, sampled, moved,
//! culled or drawn, how a trail is cut, how an effect is bolted to a bone.

mod angles;
mod rest_state;
mod atlas;
mod attach;
mod bounce_hit;
mod cull;
mod draw;
mod fx_template;
mod elem;
mod elem_template;
mod emission;
mod mode_bits;
mod fall_accel;
mod hit_table;
mod step_motion;
mod lifespan;
mod probe_tint;
mod orient;
mod axis_frame;
mod birth_spot;
mod point_cloud;
mod bank;
mod quat;
mod dice;
mod axis_turn;
mod angle_graph;
mod sort;
mod spawn;
mod camera_quad;
mod state_word;
mod tail_quads;
mod streak;
mod trail_template;
mod trail_live;
mod vec;
mod speed_graph;
mod view;
mod sight_block;
mod look;

pub use angles::get_elem_angles_axis;
pub use angles::mat3_mul;
pub use rest_state::get_at_rest_fraction;
pub use atlas::BillboardSheetCell;
pub use atlas::sprite_atlas_uv;
pub use attach::{
    BoneOrientationRefuse, ATTACH_JOINT_BIT_OFFSET, NO_SPARE_ATTACH, NO_ATTACH_TICKET,
    ATTACH_ROOT_NO_TURN, ATTACH_RECORD_LIMIT, ATTACH_HELD_GUN_SKELETON_START,
    JointFacingPath, EmitterAttachRefresh, begin_iterating_over_effects_exclusive,
    bolt_alloc, bolt_bone, bolt_centity_teleport_for_compare, bolt_compose_orientation, bolt_dobj,
    bolt_handle_is_none, bolt_init_next_index, bolt_init_parent_orientation, bolt_mark_lost,
    bolt_pack, bolt_spawn_teleport_bit, end_iterating_over_effects, end_iterating_runs_gc,
    get_bone_orientation_route, quat_mul, stop_effect_has_owned, stop_effect_non_recursive_allows,
    update_effect_bolt,
};
pub use bounce_hit::{
    BOUNCE_SLICE_MS, PARTICLE_HIT_FILTER, PARTICLE_HIT_FILTER_PICKUPS, ParticleBounceSlice,
    collide_marks_at_rest, collide_substep_schedule, collision_reflect_base_vel_delta,
    impact_child_speed_allows, sample_reflection_factor, trace_mask,
};
pub use cull::cull_cloud;
pub use cull::cull_elem_for_spawn_allows;
pub use cull::cull_elem_light;
pub use cull::elem_light_color_bgr;
pub use draw::ParticleShape;
pub use draw::draw_elem_handler_present;
pub use fx_template::EffectBlueprint;
pub use elem::PARTICLE_NEVER_SETTLED;
pub use elem_template::ElemBlueprint;
pub use emission::{
    PARTICLE_EMISSION_FACES_BASIS, EmissionTimetable, EmissionBirth, emit_dist_range, emit_lerp_origin,
    process_emitting_schedule,
};
pub use mode_bits::{
    elem_runs_relative_to_offset, elem_skips_position_update, elem_update_has_velocity_graph,
    elem_uses_collision, elem_uses_vel_local, elem_uses_vel_world,
};
pub use mode_bits::ELEM_ENDS_ON_TOUCH;
pub use mode_bits::PARTICLE_PATH_BITS;
pub use mode_bits::FX_ELEM_RUN_RELATIVE_TO_EFFECT;
pub use mode_bits::FX_ELEM_RUN_RELATIVE_TO_OFFSET;
pub use mode_bits::FX_ELEM_RUN_RELATIVE_TO_OFFSET_EFFECT_NOW;
pub use mode_bits::FX_ELEM_RUN_RELATIVE_TO_SPAWN;
pub use mode_bits::ELEM_RUNNER_ROLLS_AT_START;
pub use mode_bits::ELEM_CULL_AT_BIRTH;
pub use mode_bits::ELEM_COLLIDES_WITH_WORLD;
pub use mode_bits::ELEM_SPEED_OWN_AXES;
pub use mode_bits::ELEM_SPEED_WORLD_AXES;
pub use mode_bits::elem_dies_on_touch;
pub use mode_bits::elem_run_mode;
pub use fall_accel::elem_gravity_accel_z_sampled;
pub use hit_table::{
    HIT_ON_WAY_OUT_BIT, BODY_HIT_KIND_TOTAL, SOLID_HIT_KIND_TOTAL,
    impact_table_row, material_kind_number,
};
pub use hit_table::MATERIAL_KIND_BODY;
pub use hit_table::ImpactRow;
pub use hit_table::ImpactSheet;
pub use hit_table::flesh_effect_index;
pub use hit_table::flesh_hit_flags;
pub use step_motion::Vec3Spread;
pub use step_motion::integrate_velocity_graph;
pub use lifespan::{
    sample_life_span_msec, trail_elem_base_vel_z_pack, trail_elem_keep, trail_elem_norm_ages,
};
pub use probe_tint::{
    apply_lighting_frac_bgra, effect_def_needs_lighting_sample, elem_uses_lighting_frac,
};
pub use orient::orientation_pos_from_world;
pub use orient::orientation_pos_to_world;
pub use axis_frame::EffectFacingBasis;
pub use axis_frame::FacingBirthKnobs;
pub use axis_frame::EffectFacing;
pub use axis_frame::get_orientation;
pub use birth_spot::{
    BirthDisplacementStyle, elem_spawn_offset_mode, spawn_origin_world, world_delta_to_local,
};
pub use point_cloud::{
    SPRAY_SHADER_KNOB_A, SPRAY_SHADER_KNOB_B, SWARM_SHADER_TINT,
    SWARM_SHADER_BASIS_ROW, EMBER_SHADER_TINT, SWARM_IS_EMBERS,
    SWARM_LATTICE_ACROSS, SWARM_LATTICE_DEEP, SWARM_LATTICE_TALL,
    SWARM_TRIANGLE_SLOTS_EACH_SPECK, SWARM_RECT_TRIANGLE_SLOTS,
    SWARM_STAMP_SPECKS, SWARM_CORNER_UVS, SWARM_CORNERS_EACH_SPECK,
    EMBER_TRIANGLE_SLOTS_EACH_SPECK, EMBER_CORNERS_EACH_SPECK,
    POINT_UV_CORNER_BYTES, VisualParticleCloud, PointUvCorner, CLASSIC_DICE_START,
    particle_cloud_cell_radius_sq, particle_cloud_cell_verts, particle_cloud_cell_xyz,
    particle_cloud_color_const, particle_cloud_compare_cell_radius, particle_cloud_draw_cell_count,
    particle_cloud_draw_counts, particle_cloud_matrix_diag, particle_cloud_particle_id,
    particle_fountain_parm0, particle_spark_cell_indices, particle_spark_cell_verts,
};
pub use point_cloud::build_cloud;
pub use point_cloud::point_uv_corner_bytes;
pub use point_cloud::classic_dice_unit;
pub use point_cloud::particle_cloud_cell_indices;
pub use bank::{
    EMITTER_TICKET_WHEEL_LEN, EMITTER_BANK_LIMIT, EMITTER_SLOT_BYTES,
    DICE_TABLE_WRAP, BIRTH_UNATTACHED, CONE_GLOW_LIMIT, STREAK_JOINT_BANK_LIMIT,
    STREAK_JOINT_RECORD_BYTES, STREAK_BANK_LIMIT, STREAK_RECORD_BYTES,
    PARTICLE_ALERT_THRESHOLD, effect_handle_for_slot, elem_handle_from_ptr_delta,
};
pub use bank::PARTICLE_BANK_LIMIT;
pub use bank::PARTICLE_LIVE_RECORD_BYTES;
pub use bank::FX_ENTITYNUM_WORLD;
pub use bank::START_UNATTACHED;
pub use bank::trail_piece_ticket_for_slot;
pub use bank::trail_ticket_for_slot;
pub use quat::axis_to_quat;
pub use quat::quat_normalize;
pub use quat::unit_quat_to_axis;
pub use dice::{
    DICE_LANE_TINT, DICE_LANE_WAIT, DICE_LANE_LIFESPAN, DICE_LANE_BURST_TOTAL,
    DICE_LANE_GROWTH, DICE_LANE_FIRST_EXTENT, DICE_TABLE_REALS, effect_random_seed_from_msec,
};
pub use dice::elem_random_seed;
pub use dice::elem_visual_index;
pub use dice::random_table_f32;
pub use dice::random_table_u16;
pub use dice::trail_random_seed;
pub use axis_turn::impact_mark_axis;
pub use axis_turn::randomly_rotate_axis;
pub use axis_turn::runner_rand_rot_degrees;
pub use angle_graph::clamp_elem_rotation_time;
pub use sort::existing_elem_sorts_before_new;
pub use sort::sort_dist_to_cam_sq;
pub use spawn::{
    PARTICLE_SHAPE_STREAK, looping_catchup_begin, looping_spawn_schedule, sample_oneshot_spawn_count,
    spawn_effect_status,
};
pub use camera_quad::BILLBOARD_TRIANGLE_SLOTS;
pub use camera_quad::BILLBOARD_CORNER_OFFSETS;
pub use camera_quad::sprite_quad_indices;
pub use state_word::{
    EMITTER_REPEAT_QUEUED_BIT, FX_STATUS_REF_COUNT_LOW_MASK, EMITTER_HOLDER_TALLY_BITS,
    status_is_unique_done,
};
pub use tail_quads::tail_anchor_origin;
pub use tail_quads::tail_sprite_axes;
pub use tail_quads::tail_sprite_full_extent;
pub use streak::{
    pack_code_mesh_vertex, trail_compute_u, trail_emit_index_quad, trail_emit_segment_verts,
    trail_index_quad_tris, trail_pack_normal, trail_pack_texcoord, trail_uncompress_basis,
};
pub use streak::TRAIL_TANGENT_WORD;
pub use streak::StreakLaidCorner;
pub use streak::StreakPiecePaintStatus;
pub use streak::compress_basis_from_axis;
pub use trail_template::StreakBlueprint;
pub use trail_template::StreakCorner;
pub use trail_live::{
    StreakBreak, trail_split_interpolant_msec, trail_split_interpolant_t, trail_split_lerp_axis,
    trail_split_lerp_origin, trail_split_skips_update, trail_split_window,
};
pub use vec::{
    effect_orient_arc, perpendicular_vector, vec3_distance, vec3_length_sq, vec3_normalize,
    contact_facing_basis,
};
pub use speed_graph::get_velocity_at_time;
pub use view::EmitterRecipeLens;
pub use view::ParticleRecipeLens;
pub use sight_block::{
    SightOccluderStore, SightOccluderEntry, evaluate_distance_fade, vis_blocker_add_prepared,
    vis_blocker_generate_verts,
};
pub use look::{
    elem_norm_time, evaluate_color_bgra, evaluate_rotation_total, evaluate_scale, evaluate_size0,
    evaluate_size1,
};
