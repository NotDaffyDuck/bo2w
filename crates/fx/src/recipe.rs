//! What the host needs to know about one effect definition and its parts.

use fx_t6::runtime::PARTICLE_SHAPE_STREAK;

/// One part of an effect: how it spawns, how long it lives, how it moves.
#[derive(Clone, Copy, Debug, Default)]
pub struct ParticleRecipeSummary {
    pub part_shape: u8,
    pub bit_set: i32,

    // Spawn schedule: counts, delay and life as base plus random amplitude.
    pub birth_corner_a: i32,
    pub birth_corner_b: i32,
    pub wait_floor: i32,
    pub wait_spread: i32,
    pub lifespan_floor: i32,
    pub lifespan_spread: i32,
    pub look_variant_total: u8,
    pub look_keyframe_total: u8,
    pub paint_rank: u8,

    // Where it spawns.
    pub birth_span_floor: f32,
    pub birth_span_spread: f32,
    pub birth_point: [[f32; 2]; 3],
    pub birth_ring_reach_floor: f32,
    pub birth_ring_reach_spread: f32,
    pub birth_lift_floor: f32,
    pub birth_lift_spread: f32,
    pub birth_orientation: [[f32; 2]; 3],
    pub twirl_rate: [[f32; 2]; 3],

    // Child effects it can start.
    pub spawns_on_contact: bool,
    pub spawns_on_finish: bool,
    pub emitted_once: bool,

    // Emission spacing for trails and emitters.
    pub reciprocal_split_distance: f32,
    pub reciprocal_split_arc_length: f32,
    pub reciprocal_split_duration: f32,

    // Motion and collision.
    pub fall_pull_floor: f32,
    pub fall_pull_spread: f32,
    pub mirror_floor: f32,
    pub mirror_spread: f32,
    pub bounce_box_bottom: [f32; 3],
    pub bounce_box_top: [f32; 3],
    pub collide_with_pickups: u8,
}

impl ParticleRecipeSummary {
    /// A part that starts a child effect keeps its effect alive while the
    /// child can still appear.
    pub fn held_open_by_offspring(self) -> bool {
        [self.spawns_on_contact, self.spawns_on_finish, self.emitted_once].contains(&true)
    }

    /// Streak parts are laid by their trail; every other shape is born as a particle.
    pub fn births_as_particle(self) -> bool {
        self.part_shape != PARTICLE_SHAPE_STREAK
    }
}

/// An effect definition: its looping life and its parts, looping first, then
/// one-shot, then emission.
#[derive(Clone, Copy, Debug)]
pub struct EmitterRecipeSummary<'a> {
    pub loop_lifetime_millis: i32,
    pub repeat_total: i32,
    pub single_play_total: i32,
    pub spawn_total: i32,
    pub elems: &'a [ParticleRecipeSummary],
}

impl EmitterRecipeSummary<'_> {
    pub fn part_def_count(self) -> i32 {
        // Looping, one-shot and emission parts, summed without wrapping.
        [self.repeat_total, self.single_play_total, self.spawn_total]
            .into_iter()
            .fold(0, i32::saturating_add)
    }
}
