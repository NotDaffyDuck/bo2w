//! Elements that emit other effects as they move: where along the path the
//! emissions fall.
//!
//! An element emits once for every stretch of travel between its authored
//! minimum and maximum spacing. The part of a stretch left over when an
//! update ends is carried to the next one as a single byte, "the residual":
//! 0 to 255 of the way across the longest spacing.

use crate::runtime::random::{FX_RAND_CH_EMIT_DIST, random_table_f32};

/// The element flag that makes an emitted effect take the emitter's
/// orientation axes.
pub const FX_ELEM_EMIT_ORIENT_AXIS: i32 = 0x8000;

/// How many emissions one update can produce.
const SPAWN_CAP: usize = 32;

/// One residual step as a fraction of the longest spacing.
const RESIDUAL_STEP: f32 = 1.0 / 256.0;

/// Added before rounding a packed residual, so values that land exactly
/// between two steps fall to the upper one.
const RESIDUAL_ROUND_BIAS: f64 = 9.313_225_746_154_785e-10;

/// One emission: how far along the update's path it falls and when.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FxEmitSpawn {
    /// Fraction of the way from the path's start to its end.
    pub lerp: f32,
    /// The time of the emission, in milliseconds.
    pub msec_at_spawn: i32,
}

/// The emissions of one update, and the residual to carry on.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FxEmitSchedule {
    /// The residual after this update.
    pub new_residual: u8,
    /// How many of `spawns` are in use.
    pub spawn_count: usize,
    /// The emissions, in order; only the first `spawn_count` are set.
    pub spawns: [FxEmitSpawn; SPAWN_CAP],
}

impl FxEmitSchedule {
    /// The emissions that happen.
    pub fn spawns(&self) -> &[FxEmitSpawn] {
        &self.spawns[..self.spawn_count]
    }
}

/// The shortest and longest travel between emissions for one element: the
/// authored spacing rolled for this element, plus the authored variance.
pub fn emit_dist_range(
    emit_dist_base: f32,
    emit_dist_amp: f32,
    emit_var_base: f32,
    emit_var_amp: f32,
    elem_random_seed: u32,
) -> (f32, f32) {
    let roll = random_table_f32(elem_random_seed, FX_RAND_CH_EMIT_DIST);
    let shortest = emit_dist_base + emit_dist_amp * roll + emit_var_base;
    (shortest, shortest + emit_var_amp)
}

/// The residual byte for `residual_dist` units of unfinished travel.
fn pack_residual(residual_dist: f32, max_dist_per_emit: f32) -> u8 {
    if max_dist_per_emit <= 0.0 {
        return 0;
    }
    let steps =
        f64::from(residual_dist) * 256.0 / f64::from(max_dist_per_emit) + RESIDUAL_ROUND_BIAS;
    libm::round(steps).clamp(0.0, 255.0) as u8
}

/// Where the first emission's travel starts: a residual's worth of travel
/// already behind the update's start.
fn residual_start_dist(residual: u8, max_dist_per_emit: f32) -> f32 {
    -(f32::from(residual) * max_dist_per_emit * RESIDUAL_STEP)
}

/// The emissions an element makes while it travels from `origin_begin` to
/// `origin_end` between `msec_update_begin` and `msec_update_end`.
///
/// `next_spacing01` supplies a number from 0 to 1 for each emission: how
/// far, between the element's two spacings, to wait before the next one.
#[allow(clippy::too_many_arguments)]
pub fn process_emitting_schedule(
    emit_residual: u8,
    origin_begin: [f32; 3],
    origin_end: [f32; 3],
    msec_update_begin: i32,
    msec_update_end: i32,
    base_dist_per_emit: f32,
    max_dist_per_emit: f32,
    mut next_spacing01: impl FnMut() -> f32,
) -> FxEmitSchedule {
    let mut schedule = FxEmitSchedule {
        new_residual: emit_residual,
        spawn_count: 0,
        spawns: [FxEmitSpawn {
            lerp: 0.0,
            msec_at_spawn: 0,
        }; SPAWN_CAP],
    };
    let delta = [
        origin_end[0] - origin_begin[0],
        origin_end[1] - origin_begin[1],
        origin_end[2] - origin_begin[2],
    ];
    let travelled_sq = delta[0] * delta[0] + delta[1] * delta[1] + delta[2] * delta[2];
    if travelled_sq <= 1e-12 || max_dist_per_emit <= 0.0 {
        return schedule;
    }
    let travelled = libm::sqrtf(travelled_sq);
    let msec_span = msec_update_end.saturating_sub(msec_update_begin).max(0) as f32;
    let spacing_spread = (max_dist_per_emit - base_dist_per_emit).max(0.0);

    let mut next_at = residual_start_dist(emit_residual, max_dist_per_emit);
    let mut last_at;
    loop {
        last_at = next_at;
        next_at = next_spacing01() * spacing_spread + base_dist_per_emit + next_at;
        if next_at > travelled {
            break;
        }
        if next_at < 0.0 {
            next_at = 0.0;
        }
        let lerp = next_at / travelled;
        let msec_at_spawn = msec_update_begin.wrapping_add(libm::floorf(msec_span * lerp) as i32);
        if schedule.spawn_count >= SPAWN_CAP {
            break;
        }
        schedule.spawns[schedule.spawn_count] = FxEmitSpawn {
            lerp,
            msec_at_spawn,
        };
        schedule.spawn_count += 1;
    }

    schedule.new_residual = pack_residual(travelled - last_at, max_dist_per_emit);
    schedule
}

/// The point `lerp` of the way from `begin` to `end`.
pub fn emit_lerp_origin(begin: [f32; 3], end: [f32; 3], lerp: f32) -> [f32; 3] {
    [
        begin[0] + (end[0] - begin[0]) * lerp,
        begin[1] + (end[1] - begin[1]) * lerp,
        begin[2] + (end[2] - begin[2]) * lerp,
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A straight run of 100 units along x over 100 ms starting at 1000.
    fn run(residual: u8, base: f32, max: f32, mut spacing: impl FnMut() -> f32) -> FxEmitSchedule {
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
        assert_eq!(got.spawn_count, 10);
        assert_eq!(got.spawns()[0].lerp, 0.1);
        assert_eq!(got.spawns()[9].lerp, 1.0);
        assert_eq!(got.spawns()[9].msec_at_spawn, 1100);
        assert_eq!(got.new_residual, 0);
    }

    #[test]
    fn the_leftover_travel_is_carried_as_a_residual() {
        // Emissions at 30 and 60 and 90; 10 units are left over, a third of
        // the way across the 30-unit spacing.
        let got = run(0, 30.0, 30.0, || 0.0);
        assert_eq!(got.spawn_count, 3);
        assert_eq!(got.new_residual, 85);
    }

    #[test]
    fn a_carried_residual_brings_the_first_emission_forward() {
        let fresh = run(0, 30.0, 30.0, || 0.0);
        let carried = run(128, 30.0, 30.0, || 0.0);
        assert_eq!(fresh.spawns()[0].lerp, 0.3);
        // Half a spacing was already travelled: the first emission is 15 in.
        assert_eq!(carried.spawns()[0].lerp, 0.15);
    }

    #[test]
    fn no_travel_means_no_emissions_and_the_residual_is_kept() {
        let got = process_emitting_schedule(77, [5.0; 3], [5.0; 3], 0, 100, 10.0, 10.0, || 0.0);
        assert_eq!(got.spawn_count, 0);
        assert_eq!(got.new_residual, 77);
    }

    #[test]
    fn a_spacing_of_nothing_emits_nothing() {
        let got = run(9, 0.0, 0.0, || 0.0);
        assert_eq!(got.spawn_count, 0);
        assert_eq!(got.new_residual, 9);
    }

    #[test]
    fn one_update_never_emits_more_than_the_cap() {
        let got = run(0, 1.0, 1.0, || 0.0);
        assert_eq!(got.spawn_count, SPAWN_CAP);
    }

    #[test]
    fn the_spacing_roll_stretches_the_gap_up_to_the_longest() {
        let short = run(0, 10.0, 20.0, || 0.0);
        let long = run(0, 10.0, 20.0, || 1.0);
        assert_eq!(short.spawn_count, 10);
        assert_eq!(long.spawn_count, 5);
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
}
