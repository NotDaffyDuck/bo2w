//! Decoded views of an element def and an effect def: the byte-packed ranges
//! read out into plain numbers.

/// An element def with its ranges decoded.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ParticleRecipeLens {
    pub flags: i32,
    pub birth_corner_a: i32,
    pub birth_corner_b: i32,
    pub birth_span_floor: f32,
    pub birth_span_swing: f32,
    pub appear_blend_reach: [f32; 2],
    pub vanish_blend_reach: [f32; 2],
    pub birth_view_reject_reach: f32,
    pub birth_wait_floor_ms: i32,
    pub birth_wait_swing_ms: i32,
    pub lifespan_window_floor: i32,
    pub lifespan_window_swing: i32,
    pub birth_point: [[f32; 2]; 3],
    pub birth_ring_reach_floor: f32,
    pub birth_ring_reach_swing: f32,
    pub birth_lift_floor: f32,
    pub birth_lift_swing: f32,
    pub birth_orientation: [[f32; 2]; 3],
    pub twirl_rate: [[f32; 2]; 3],
    pub birth_spin_angle: [f32; 2],
    pub fall_pull_floor: f32,
    pub fall_pull_swing: f32,
    pub mirror_strength: [f32; 2],
    pub bounce_box_bottom: [f32; 3],
    pub bounce_box_top: [f32; 3],
    pub shape_kind: u8,
    pub look_variant_total: u8,
    pub speed_keyframe_total: u8,
    pub look_keyframe_total: u8,
    pub shading_blend: u8,
    pub collide_with_pickups: u8,
    /// The line element (type 4), carried as a tail that runs ahead of its
    /// part instead of behind it.
    pub line: bool,
    pub paint_rank: u8,
    pub emission_spacing: [f32; 2],
    pub emission_spacing_spread: [f32; 2],
}

/// An effect def's header with its flags.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EmitterRecipeLens {
    pub flags: i32,
    pub loop_lifetime_millis: i32,
    pub repeat_total: i32,
    pub single_play_total: i32,
    pub spawn_total: i32,
}
