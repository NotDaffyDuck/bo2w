//! Element collision: how a move is cut into steps, which surfaces it hits,
//! and how an element bounces.

use crate::runtime::birth_spot::sample_float_range;
use crate::runtime::dice::random_table_f32;
use crate::runtime::vec::vec3_length_sq;
use super::dice::DICE_LANE_BOUNCE;

/// How long one collision step is, in milliseconds.
pub const BOUNCE_SLICE_MS: i32 = 0x32;

/// The surface types an element collides with.
pub const PARTICLE_HIT_FILTER: u32 = 0x811;

/// The surface types, for an element that also collides with item clip.
pub const PARTICLE_HIT_FILTER_PICKUPS: u32 = 0xc11;

/// An element must have been moving faster than this (squared) to spawn an
/// impact effect or leave a mark.
const IMPACT_MIN_SPEED_SQ: f32 = 1.0;

/// A surface whose normal points up by more than this counts as ground.
const GROUND_NORMAL_Z: f32 = 0.7;

/// One step of a move, as a pair of times.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ParticleBounceSlice {
    pub window_begin_ms: i32,
    pub window_close_ms: i32,
}

/// The steps of a move, in order.
#[derive(Clone, Debug)]
pub struct ParticleBounceSliceWalker {
    next_start: i32,
    end: i32,
}

impl Iterator for ParticleBounceSliceWalker {
    type Item = ParticleBounceSlice;

    fn next(&mut self) -> Option<ParticleBounceSlice> {
        let window_begin_ms = self.next_start;
        if window_begin_ms >= self.end {
            return None;
        }
        self.next_start = match window_begin_ms.wrapping_add(BOUNCE_SLICE_MS) {
            cut if cut < self.end => cut,
            _ => self.end,
        };
        Some(ParticleBounceSlice {
            window_begin_ms,
            window_close_ms: self.next_start,
        })
    }
}

/// A move from `window_begin_ms` to `window_close_ms` cut into steps of
/// [`BOUNCE_SLICE_MS`]; the last step is whatever is left. Empty when
/// the move does not go forward in time.
pub fn collide_substep_schedule(window_begin_ms: i32, window_close_ms: i32) -> ParticleBounceSliceWalker {
    ParticleBounceSliceWalker {
        next_start: window_begin_ms,
        end: window_close_ms,
    }
}

/// The surface mask to trace with.
pub const fn trace_mask(collide_with_pickups: bool) -> u32 {
    [PARTICLE_HIT_FILTER, PARTICLE_HIT_FILTER_PICKUPS][collide_with_pickups as usize]
}

/// Whether an element that was going this fast (squared) when it hit
/// something is fast enough to spawn an impact effect.
pub fn impact_child_speed_allows(pre_impact_speed_sq: f32) -> bool {
    pre_impact_speed_sq > IMPACT_MIN_SPEED_SQ
}

/// How much of its speed an element keeps when it bounces, rolled from its
/// authored range.
pub fn sample_reflection_factor(base: f32, spread: f32, salt: u32) -> f32 {
    sample_float_range(
        base,
        spread,
        random_table_f32(salt, DICE_LANE_BOUNCE),
    )
}

/// Whether an element that hit a surface with this normal has come to rest
/// on the ground, leaving a mark: the surface is ground and the element is
/// too slow to bounce on.
pub fn collide_marks_at_rest(scaled_vel: [f32; 3], up_part: f32) -> bool {
    up_part > GROUND_NORMAL_Z && !impact_child_speed_allows(vec3_length_sq(scaled_vel))
}

/// How much to add to an element's base velocity so it bounces off a
/// surface: the pre-impact velocity, scaled by the reflection factor and
/// mirrored in the surface, minus the pre-impact velocity.
pub fn collision_reflect_base_vel_delta(
    incoming_vel: [f32; 3],
    face_dir: [f32; 3],
    mirror_strength: f32,
) -> [f32; 3] {
    let scaled = incoming_vel.map(|v| v * mirror_strength);
    let mirror = -2.0
        * scaled
            .iter()
            .zip(&face_dir)
            .fold(0.0, |sum, (v, n)| sum + v * n);
    core::array::from_fn(|k| scaled[k] + mirror * face_dir[k] - incoming_vel[k])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn steps(start: i32, end: i32) -> Vec<(i32, i32)> {
        collide_substep_schedule(start, end)
            .map(|s| (s.window_begin_ms, s.window_close_ms))
            .collect()
    }

    #[test]
    fn a_short_move_is_one_step() {
        assert_eq!(steps(100, 130), [(100, 130)]);
    }

    #[test]
    fn a_long_move_is_cut_every_fifty_milliseconds() {
        assert_eq!(steps(0, 120), [(0, 50), (50, 100), (100, 120)]);
    }

    #[test]
    fn a_move_that_ends_on_a_cut_has_no_empty_step() {
        assert_eq!(steps(0, 100), [(0, 50), (50, 100)]);
        assert_eq!(steps(0, 50), [(0, 50)]);
    }

    #[test]
    fn a_move_that_does_not_go_forward_has_no_steps() {
        assert!(steps(100, 100).is_empty());
        assert!(steps(100, 40).is_empty());
    }

    #[test]
    fn item_clip_widens_the_trace_mask() {
        assert_eq!(trace_mask(false), PARTICLE_HIT_FILTER);
        assert_eq!(trace_mask(true), PARTICLE_HIT_FILTER_PICKUPS);
        assert_eq!(PARTICLE_HIT_FILTER_PICKUPS & PARTICLE_HIT_FILTER, PARTICLE_HIT_FILTER);
    }

    #[test]
    fn only_a_fast_element_spawns_an_impact_effect() {
        assert!(impact_child_speed_allows(1.5));
        assert!(!impact_child_speed_allows(1.0));
        assert!(!impact_child_speed_allows(0.0));
    }

    #[test]
    fn a_slow_element_on_flat_ground_comes_to_rest() {
        assert!(collide_marks_at_rest([0.0, 0.0, -0.5], 1.0));
        assert!(!collide_marks_at_rest([0.0, 0.0, -5.0], 1.0));
        assert!(!collide_marks_at_rest([0.0, 0.0, -0.5], 0.5));
        assert!(!collide_marks_at_rest([0.0, 0.0, -0.5], 0.7));
    }

    #[test]
    fn a_perfect_bounce_off_a_floor_flips_the_vertical_speed() {
        let delta = collision_reflect_base_vel_delta([3.0, 0.0, -10.0], [0.0, 0.0, 1.0], 1.0);
        assert_eq!(delta, [0.0, 0.0, 20.0]);
    }

    #[test]
    fn a_weaker_bounce_keeps_less_of_the_speed() {
        let delta = collision_reflect_base_vel_delta([0.0, 0.0, -10.0], [0.0, 0.0, 1.0], 0.5);
        // Arrives at -10, leaves at +5: the base velocity gains 15.
        assert_eq!(delta, [0.0, 0.0, 15.0]);
    }

    #[test]
    fn a_zero_reflection_factor_stops_the_element_dead() {
        let delta = collision_reflect_base_vel_delta([2.0, -3.0, -4.0], [0.0, 0.0, 1.0], 0.0);
        assert_eq!(delta, [-2.0, 3.0, 4.0]);
    }

    #[test]
    fn the_reflection_factor_stays_in_its_authored_range() {
        for seed in 0..32 {
            let f = sample_reflection_factor(0.25, 0.5, seed);
            assert!((0.25..=0.75).contains(&f), "{f}");
        }
    }
}
