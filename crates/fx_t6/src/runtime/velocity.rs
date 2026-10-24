//! An element's velocity at an age: its base velocity plus whichever velocity
//! graphs it carries.

use crate::runtime::flags::{elem_uses_vel_local, elem_uses_vel_world};
use crate::runtime::integrate::{FxElemVec3Range, sample_vel_graph_at_age};

/// The graphs are authored per millisecond; the result is per second.
const PER_SECOND: f32 = 1000.0;

/// The fraction of the life gone, held between zero and one. A life with no
/// length is always at the start.
fn age_fraction(age_msec: f32, life_ms: f32) -> f32 {
    if life_ms <= 0.0 {
        return 0.0;
    }
    let fraction = age_msec / life_ms;
    if fraction < 0.0 {
        0.0
    } else if fraction > 1.0 {
        1.0
    } else {
        fraction
    }
}

/// The velocity `age_msec` into a life of `life_ms`. A graph needs at least
/// two samples to count. The world graph is added as it is; the local graph
/// is turned into world axes with `orient_axis` first.
#[allow(clippy::too_many_arguments)]
pub fn get_velocity_at_time(
    flags: i32,
    base_vel: [f32; 3],
    age_msec: f32,
    life_ms: f32,
    local_samples: &[FxElemVec3Range],
    world_samples: &[FxElemVec3Range],
    orient_axis: [[f32; 3]; 3],
    seed: u32,
) -> [f32; 3] {
    let age = age_fraction(age_msec, life_ms);
    let mut velocity = base_vel;

    if elem_uses_vel_world(flags) && world_samples.len() >= 2 {
        let world = sample_vel_graph_at_age(world_samples, age, seed);
        for axis in 0..3 {
            velocity[axis] += world[axis] * PER_SECOND;
        }
    }
    if elem_uses_vel_local(flags) && local_samples.len() >= 2 {
        let local = sample_vel_graph_at_age(local_samples, age, seed);
        for axis in 0..3 {
            let turned = local[0] * orient_axis[0][axis]
                + local[1] * orient_axis[1][axis]
                + local[2] * orient_axis[2][axis];
            velocity[axis] += turned * PER_SECOND;
        }
    }
    velocity
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::flags::{FX_ELEM_VEL_LOCAL, FX_ELEM_VEL_WORLD};

    const IDENTITY: [[f32; 3]; 3] = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
    const TURNED: [[f32; 3]; 3] = [[0.0, 1.0, 0.0], [-1.0, 0.0, 0.0], [0.0, 0.0, 1.0]];

    fn constant(x: f32, y: f32, z: f32) -> [FxElemVec3Range; 2] {
        let sample = FxElemVec3Range {
            base: [x, y, z],
            amplitude: [0.0; 3],
        };
        [sample, sample]
    }

    #[test]
    fn with_no_graph_flags_the_base_velocity_stands() {
        let got = get_velocity_at_time(
            0,
            [1.0, 2.0, 3.0],
            5.0,
            10.0,
            &constant(1.0, 1.0, 1.0),
            &constant(1.0, 1.0, 1.0),
            IDENTITY,
            0,
        );
        assert_eq!(got, [1.0, 2.0, 3.0]);
    }

    #[test]
    fn the_world_graph_adds_per_second() {
        let got = get_velocity_at_time(
            FX_ELEM_VEL_WORLD,
            [1.0, 0.0, 0.0],
            5.0,
            10.0,
            &[],
            &constant(0.5, 0.0, -0.25),
            TURNED,
            0,
        );
        assert_eq!(got, [501.0, 0.0, -250.0]);
    }

    #[test]
    fn the_local_graph_is_turned_into_world_axes() {
        let got = get_velocity_at_time(
            FX_ELEM_VEL_LOCAL,
            [0.0; 3],
            5.0,
            10.0,
            &constant(1.0, 0.0, 0.0),
            &[],
            TURNED,
            0,
        );
        assert_eq!(got, [0.0, 1000.0, 0.0]);
    }

    #[test]
    fn a_graph_with_one_sample_is_ignored() {
        let one = [FxElemVec3Range {
            base: [9.0; 3],
            amplitude: [0.0; 3],
        }];
        let got = get_velocity_at_time(
            FX_ELEM_VEL_WORLD | FX_ELEM_VEL_LOCAL,
            [1.0, 1.0, 1.0],
            0.0,
            10.0,
            &one,
            &one,
            IDENTITY,
            0,
        );
        assert_eq!(got, [1.0, 1.0, 1.0]);
    }

    #[test]
    fn the_age_is_held_inside_the_life() {
        assert_eq!(age_fraction(-5.0, 10.0), 0.0);
        assert_eq!(age_fraction(50.0, 10.0), 1.0);
        assert_eq!(age_fraction(2.5, 10.0), 0.25);
        assert_eq!(age_fraction(2.5, 0.0), 0.0);
    }
}
