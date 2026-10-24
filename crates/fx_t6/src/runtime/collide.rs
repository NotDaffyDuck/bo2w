//! Element collision: how a move is cut into steps, which surfaces it hits,
//! and how an element bounces.

use crate::runtime::origin::sample_float_range;
use crate::runtime::random::{FX_RAND_CH_REFLECTION, random_table_f32};
use crate::runtime::vec::vec3_length_sq;

/// How long one collision step is, in milliseconds.
pub const FX_COLLIDE_SUBSTEP_MS: i32 = 0x32;

/// The surface types an element collides with.
pub const FX_TRACE_MASK: u32 = 0x811;

/// The surface types, for an element that also collides with item clip.
pub const FX_TRACE_MASK_ITEM_CLIP: u32 = 0xc11;

/// An element must have been moving faster than this (squared) to spawn an
/// impact effect or leave a mark.
const IMPACT_MIN_SPEED_SQ: f32 = 1.0;

/// A surface whose normal points up by more than this counts as ground.
const GROUND_NORMAL_Z: f32 = 0.7;

/// One step of a move, as a pair of times.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FxCollideSubstep {
    pub msec_start: i32,
    pub msec_end: i32,
}

/// The steps of a move, in order.
#[derive(Clone, Debug)]
pub struct FxCollideSubstepIter {
    next_start: i32,
    end: i32,
}

impl Iterator for FxCollideSubstepIter {
    type Item = FxCollideSubstep;

    fn next(&mut self) -> Option<FxCollideSubstep> {
        if self.next_start >= self.end {
            return None;
        }
        let msec_start = self.next_start;
        let cut = msec_start.wrapping_add(FX_COLLIDE_SUBSTEP_MS);
        let msec_end = if cut < self.end { cut } else { self.end };
        self.next_start = msec_end;
        Some(FxCollideSubstep {
            msec_start,
            msec_end,
        })
    }
}

/// A move from `msec_start` to `msec_end` cut into steps of
/// [`FX_COLLIDE_SUBSTEP_MS`]; the last step is whatever is left. Empty when
/// the move does not go forward in time.
pub fn collide_substep_schedule(msec_start: i32, msec_end: i32) -> FxCollideSubstepIter {
    FxCollideSubstepIter {
        next_start: msec_start,
        end: msec_end,
    }
}

/// The surface mask to trace with.
pub const fn trace_mask(use_item_clip: bool) -> u32 {
    if use_item_clip {
        FX_TRACE_MASK_ITEM_CLIP
    } else {
        FX_TRACE_MASK
    }
}

/// Whether an element that was going this fast (squared) when it hit
/// something is fast enough to spawn an impact effect.
pub fn impact_child_speed_allows(pre_impact_speed_sq: f32) -> bool {
    pre_impact_speed_sq > IMPACT_MIN_SPEED_SQ
}

/// How much of its speed an element keeps when it bounces, rolled from its
/// authored range.
pub fn sample_reflection_factor(base: f32, amplitude: f32, seed: u32) -> f32 {
    sample_float_range(
        base,
        amplitude,
        random_table_f32(seed, FX_RAND_CH_REFLECTION),
    )
}

/// Whether an element that hit a surface with this normal has come to rest
/// on the ground, leaving a mark: the surface is ground and the element is
/// too slow to bounce on.
pub fn collide_marks_at_rest(scaled_vel: [f32; 3], normal_z: f32) -> bool {
    normal_z > GROUND_NORMAL_Z && !impact_child_speed_allows(vec3_length_sq(scaled_vel))
}

/// How much to add to an element's base velocity so it bounces off a
/// surface: the pre-impact velocity, scaled by the reflection factor and
/// mirrored in the surface, minus the pre-impact velocity.
pub fn collision_reflect_base_vel_delta(
    pre_impact_vel: [f32; 3],
    normal: [f32; 3],
    reflection_factor: f32,
) -> [f32; 3] {
    let scaled = pre_impact_vel.map(|v| v * reflection_factor);
    let into_surface = scaled[0] * normal[0] + scaled[1] * normal[1] + scaled[2] * normal[2];
    let mirror = -2.0 * into_surface;
    let bounced = [
        scaled[0] + mirror * normal[0],
        scaled[1] + mirror * normal[1],
        scaled[2] + mirror * normal[2],
    ];
    [
        bounced[0] - pre_impact_vel[0],
        bounced[1] - pre_impact_vel[1],
        bounced[2] - pre_impact_vel[2],
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn steps(start: i32, end: i32) -> Vec<(i32, i32)> {
        collide_substep_schedule(start, end)
            .map(|s| (s.msec_start, s.msec_end))
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
        assert_eq!(trace_mask(false), FX_TRACE_MASK);
        assert_eq!(trace_mask(true), FX_TRACE_MASK_ITEM_CLIP);
        assert_eq!(FX_TRACE_MASK_ITEM_CLIP & FX_TRACE_MASK, FX_TRACE_MASK);
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
