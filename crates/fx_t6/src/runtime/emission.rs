//! Elements that emit other effects as they move: where along the path the
//! emissions fall.
//!
//! An element emits once for every stretch of travel between its authored
//! minimum and maximum spacing. The part of a stretch left over when an
//! update ends is carried to the next one as a single byte, "the residual":
//! 0 to 255 of the way across the longest spacing.

use crate::runtime::dice::random_table_f32;
use super::dice::DICE_LANE_EMIT_GAP;

/// The element flag that makes an emitted effect take the emitter's
/// orientation axes.
pub const PARTICLE_EMISSION_FACES_BASIS: i32 = 0x8000;

/// How many emissions one update can produce.
const SPAWN_CAP: usize = 32;

/// One residual step as a fraction of the longest spacing.
const RESIDUAL_STEP: f32 = 1.0 / 256.0;

/// Added before rounding a packed residual, so values that land exactly
/// between two steps fall to the upper one.
const RESIDUAL_ROUND_BIAS: f64 = 9.313_225_746_154_785e-10;

/// One emission: how far along the update's path it falls and when.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct EmissionBirth {
    /// Fraction of the way from the path's start to its end.
    pub mix: f32,
    /// The time of the emission, in milliseconds.
    pub born_at_ms: i32,
}

/// The emissions of one update, and the residual to carry on.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EmissionTimetable {
    /// The residual after this update.
    pub fresh_carry: u8,
    /// How many of `spawns` are in use.
    pub births: usize,
    /// The emissions, in order; only the first `spawn_count` are set.
    pub spawns: [EmissionBirth; SPAWN_CAP],
}

impl EmissionTimetable {
    /// The emissions that happen.
    pub fn spawns(&self) -> &[EmissionBirth] {
        self.spawns.split_at(self.births).0
    }
}

/// The shortest and longest travel between emissions for one element: the
/// authored spacing rolled for this element, plus the authored variance.
pub fn emit_dist_range(
    gap_min: f32,
    gap_span: f32,
    var_min: f32,
    var_span: f32,
    elem_salt: u32,
) -> (f32, f32) {
    let roll = random_table_f32(elem_salt, DICE_LANE_EMIT_GAP);
    let shortest = gap_min + gap_span * roll + var_min;
    (shortest, shortest + var_span)
}

/// The residual byte for `residual_dist` units of unfinished travel.
fn pack_residual(residual_dist: f32, widest_emission_step: f32) -> u8 {
    // A spacing that is not positive packs to 0 (NaN would too, via the cast).
    match widest_emission_step {
        longest if longest > 0.0 => {
            let steps = f64::from(residual_dist) * 256.0 / f64::from(longest) + RESIDUAL_ROUND_BIAS;
            libm::round(steps).max(0.0).min(255.0) as u8
        }
        _ => 0,
    }
}

/// Where the first emission's travel starts: a residual's worth of travel
/// already behind the update's start.
fn residual_start_dist(residual: u8, widest_emission_step: f32) -> f32 {
    -(f32::from(residual) * widest_emission_step * RESIDUAL_STEP)
}

/// The emissions an element makes while it travels from `start_point` to
/// `closing_spot` between `refresh_window_open_ms` and `refresh_window_close_ms`.
///
/// `next_gap` supplies a number from 0 to 1 for each emission: how
/// far, between the element's two spacings, to wait before the next one.
#[allow(clippy::too_many_arguments)]
pub fn process_emitting_schedule(
    emission_carry: u8,
    start_point: [f32; 3],
    closing_spot: [f32; 3],
    refresh_window_open_ms: i32,
    refresh_window_close_ms: i32,
    gap_each: f32,
    widest_emission_step: f32,
    mut next_gap: impl FnMut() -> f32,
) -> EmissionTimetable {
    let mut spawns = [EmissionBirth::default(); SPAWN_CAP];
    let mut used = 0;
    let travelled = norm3(sub3(closing_spot, start_point));
    // Standing still, or no spacing: nothing is emitted and the residual stays.
    let idle = travelled * travelled <= 1e-12 || widest_emission_step <= 0.0;
    let fresh_carry = if idle {
        emission_carry
    } else {
        let spread = (widest_emission_step - gap_each).max(0.0);
        let msec_span = refresh_window_close_ms.saturating_sub(refresh_window_open_ms).max(0) as f32;
        let mut walk = Walk::new(residual_start_dist(emission_carry, widest_emission_step));
        loop {
            walk.move_on(next_gap() * spread + gap_each);
            // The step after the last stored one is still taken, and decides nothing.
            if used == SPAWN_CAP || walk.at > travelled {
                break;
            }
            let lerp = walk.at / travelled;
            let into_update = libm::floorf(msec_span * lerp) as i32;
            spawns[used] = EmissionBirth { mix: lerp, born_at_ms: refresh_window_open_ms.wrapping_add(into_update) };
            used += 1;
        }
        pack_residual(travelled - walk.behind, widest_emission_step)
    };
    EmissionTimetable { fresh_carry, births: used, spawns }
}

/// Where along the path the last emission fell and where the next one lands.
struct Walk {
    behind: f32,
    at: f32,
}

impl Walk {
    fn new(start: f32) -> Self {
        Walk {
            behind: start,
            at: start,
        }
    }

    /// Moves on by `step`; a spot before the path's start is held at the start.
    fn move_on(&mut self, step: f32) {
        self.behind = self.at;
        let next = step + self.at;
        self.at = if next < 0.0 { 0.0 } else { next };
    }
}

fn sub3(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    core::array::from_fn(|k| a[k] - b[k])
}

fn norm3(v: [f32; 3]) -> f32 {
    libm::sqrtf(v.iter().map(|c| c * c).sum())
}

/// The point `mix` of the way from `begin` to `to_pt`.
pub fn emit_lerp_origin(begin: [f32; 3], to_pt: [f32; 3], mix: f32) -> [f32; 3] {
    core::array::from_fn(|k| begin[k] + (to_pt[k] - begin[k]) * mix)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A straight run of 100 units along x over 100 ms starting at 1000.
    fn run(residual: u8, base: f32, max: f32, mut spacing: impl FnMut() -> f32) -> EmissionTimetable {
        process_emitting_schedule(
            residual,
            [0.0; 3],
            [100.0, 0.0, 0.0],
            1000,
            1100,
            base,
            max,
            &mut spacing,
        )
    }

    #[test]
    fn the_distance_range_is_the_roll_plus_the_variance() {
        for seed in 0..16 {
            let (short, long) = emit_dist_range(10.0, 4.0, 1.0, 2.5, seed);
            assert!((11.0..=15.0).contains(&short), "{short}");
            assert_eq!(long, short + 2.5);
        }
    }

    #[test]
    fn an_even_spacing_emits_at_every_step_along_the_path() {
        let got = run(0, 10.0, 10.0, || 0.0);
        assert_eq!(got.births, 10);
        assert_eq!(got.spawns()[0].mix, 0.1);
        assert_eq!(got.spawns()[9].mix, 1.0);
        assert_eq!(got.spawns()[9].born_at_ms, 1100);
        assert_eq!(got.fresh_carry, 0);
    }

    #[test]
    fn the_leftover_travel_is_carried_as_a_residual() {
        // Emissions at 30 and 60 and 90; 10 units are left over, a third of
        // the way across the 30-unit spacing.
        let got = run(0, 30.0, 30.0, || 0.0);
        assert_eq!(got.births, 3);
        assert_eq!(got.fresh_carry, 85);
    }

    #[test]
    fn a_carried_residual_brings_the_first_emission_forward() {
        let fresh = run(0, 30.0, 30.0, || 0.0);
        let carried = run(128, 30.0, 30.0, || 0.0);
        assert_eq!(fresh.spawns()[0].mix, 0.3);
        // Half a spacing was already travelled: the first emission is 15 in.
        assert_eq!(carried.spawns()[0].mix, 0.15);
    }

    #[test]
    fn no_travel_means_no_emissions_and_the_residual_is_kept() {
        let got = process_emitting_schedule(77, [5.0; 3], [5.0; 3], 0, 100, 10.0, 10.0, || 0.0);
        assert_eq!(got.births, 0);
        assert_eq!(got.fresh_carry, 77);
    }

    #[test]
    fn a_spacing_of_nothing_emits_nothing() {
        let got = run(9, 0.0, 0.0, || 0.0);
        assert_eq!(got.births, 0);
        assert_eq!(got.fresh_carry, 9);
    }

    #[test]
    fn one_update_never_emits_more_than_the_cap() {
        let got = run(0, 1.0, 1.0, || 0.0);
        assert_eq!(got.births, SPAWN_CAP);
    }

    #[test]
    fn the_spacing_roll_stretches_the_gap_up_to_the_longest() {
        let short = run(0, 10.0, 20.0, || 0.0);
        let long = run(0, 10.0, 20.0, || 1.0);
        assert_eq!(short.births, 10);
        assert_eq!(long.births, 5);
    }

    #[test]
    fn residuals_pack_to_steps_of_a_256th_and_stop_at_255() {
        assert_eq!(pack_residual(0.0, 10.0), 0);
        assert_eq!(pack_residual(5.0, 10.0), 128);
        assert_eq!(pack_residual(10.0, 10.0), 255);
        assert_eq!(pack_residual(-3.0, 10.0), 0);
        assert_eq!(pack_residual(5.0, 0.0), 0);
    }

    #[test]
    fn a_residual_unpacks_to_travel_already_behind() {
        assert_eq!(residual_start_dist(128, 10.0), -5.0);
        assert_eq!(residual_start_dist(0, 10.0), 0.0);
    }

    #[test]
    fn the_origin_slides_along_the_path() {
        assert_eq!(
            emit_lerp_origin([0.0, 10.0, 0.0], [10.0, 20.0, -4.0], 0.5),
            [5.0, 15.0, -2.0]
        );
    }

    #[test]
    fn the_spacing_is_asked_once_more_after_the_cap_and_the_residual_follows_the_last_emission() {
        let mut asked = 0;
        let schedule = run(0, 1.0, 1.0, || {
            asked += 1;
            0.0
        });
        assert_eq!(schedule.births, SPAWN_CAP);
        assert_eq!(asked, SPAWN_CAP + 1);
        // 32 steps of 1.0 from 0 leave 100 - 32 = 68 behind; packed against a max of 1.0.
        assert_eq!(schedule.fresh_carry, 255);
    }

    #[test]
    fn the_spacing_is_asked_until_a_step_leaves_the_path() {
        let mut asked = 0;
        let schedule = run(0, 30.0, 30.0, || {
            asked += 1;
            0.5
        });
        assert_eq!(schedule.births, 3);
        assert_eq!(asked, 4);
        let lerps: Vec<f32> = schedule.spawns().iter().map(|s| s.mix).collect();
        assert_eq!(lerps, [0.3, 0.6, 0.9]);
    }

    #[test]
    fn a_path_that_dips_behind_the_start_is_clamped_to_it() {
        let schedule = run(255, 1.0, 10.0, || 0.0);
        assert!(schedule.spawns().iter().all(|s| s.mix >= 0.0));
        assert_eq!(schedule.spawns()[0].mix, 0.0);
    }
}
