//! Decoded views of an element def and an effect def: the byte-packed ranges
//! read out into plain numbers.

/// An element def with its ranges decoded.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FxElemDefView {
    pub flags: i32,
    pub spawn_a: i32,
    pub spawn_b: i32,
    pub spawn_range_base: f32,
    pub spawn_range_amplitude: f32,
    pub fade_in_range: [f32; 2],
    pub fade_out_range: [f32; 2],
    pub spawn_frustum_cull_radius: f32,
    pub spawn_delay_msec_base: i32,
    pub spawn_delay_msec_amplitude: i32,
    pub life_span_msec_base: i32,
    pub life_span_msec_amplitude: i32,
    pub spawn_origin: [[f32; 2]; 3],
    pub spawn_offset_radius_base: f32,
    pub spawn_offset_radius_amplitude: f32,
    pub spawn_offset_height_base: f32,
    pub spawn_offset_height_amplitude: f32,
    pub spawn_angles: [[f32; 2]; 3],
    pub angular_velocity: [[f32; 2]; 3],
    pub initial_rotation: [f32; 2],
    pub gravity_base: f32,
    pub gravity_amplitude: f32,
    pub reflection_factor: [f32; 2],
    pub coll_mins: [f32; 3],
    pub coll_maxs: [f32; 3],
    pub elem_type: u8,
    pub visual_count: u8,
    pub vel_interval_count: u8,
    pub vis_state_interval_count: u8,
    pub lighting_frac: u8,
    pub use_item_clip: u8,
    /// The line element (type 4), carried as a tail that runs ahead of its
    /// part instead of behind it.
    pub line: bool,
    pub sort_order: u8,
    pub emit_dist: [f32; 2],
    pub emit_dist_variance: [f32; 2],
}

/// An effect def's header with its flags.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FxEffectDefView {
    pub flags: i32,
    pub msec_looping_life: i32,
    pub looping_count: i32,
    pub one_shot_count: i32,
    pub emission_count: i32,
}
