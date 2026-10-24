//! What the host needs to know about one effect definition and its parts.

/// One part of an effect: how it spawns, how long it lives, how it moves.
#[derive(Clone, Copy, Debug, Default)]
pub struct FxElemDefInfo {
    pub elem_type: u8,
    pub flags: i32,

    // Spawn schedule: counts, delay and life as base plus random amplitude.
    pub spawn_a: i32,
    pub spawn_b: i32,
    pub delay_base: i32,
    pub delay_amp: i32,
    pub life_base: i32,
    pub life_amp: i32,
    pub visual_count: u8,
    pub vis_state_interval_count: u8,
    pub sort_order: u8,

    // Where it spawns.
    pub spawn_range_base: f32,
    pub spawn_range_amp: f32,
    pub spawn_origin: [[f32; 2]; 3],
    pub spawn_offset_radius_base: f32,
    pub spawn_offset_radius_amp: f32,
    pub spawn_offset_height_base: f32,
    pub spawn_offset_height_amp: f32,
    pub spawn_angles: [[f32; 2]; 3],
    pub angular_velocity: [[f32; 2]; 3],

    // Child effects it can start.
    pub has_effect_on_impact: bool,
    pub has_effect_on_death: bool,
    pub has_effect_emitted: bool,

    // Emission spacing for trails and emitters.
    pub inv_split_dist: f32,
    pub inv_split_arc_dist: f32,
    pub inv_split_time: f32,

    // Motion and collision.
    pub gravity_base: f32,
    pub gravity_amp: f32,
    pub reflection_base: f32,
    pub reflection_amp: f32,
    pub coll_mins: [f32; 3],
    pub coll_maxs: [f32; 3],
    pub use_item_clip: u8,
}

impl FxElemDefInfo {
    /// A part that starts a child effect keeps its effect alive while the
    /// child can still appear.
    pub fn keep_alive_by_child(self) -> bool {
        self.has_effect_on_impact || self.has_effect_on_death || self.has_effect_emitted
    }
}

/// An effect definition: its looping life and its parts, looping first, then
/// one-shot, then emission.
#[derive(Clone, Copy, Debug)]
pub struct FxEffectDefInfo<'a> {
    pub msec_looping_life: i32,
    pub looping_count: i32,
    pub one_shot_count: i32,
    pub emission_count: i32,
    pub elems: &'a [FxElemDefInfo],
}

impl FxEffectDefInfo<'_> {
    pub fn total_elem_defs(self) -> i32 {
        self.looping_count
            .saturating_add(self.one_shot_count)
            .saturating_add(self.emission_count)
    }
}
