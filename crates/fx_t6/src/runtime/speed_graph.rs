//! An element's velocity at an age: its base velocity plus whichever velocity
//! graphs it carries.

use crate::runtime::mode_bits::{elem_uses_vel_local, elem_uses_vel_world};
use crate::runtime::step_motion::Vec3Spread;
use crate::runtime::step_motion::sample_vel_graph_at_age;

/// The graphs are authored per millisecond; the result is per second.
const PER_SECOND: f32 = 1000.0;

/// The fraction of the life gone, held between zero and one. A life with no
/// length is always at the start.
fn age_fraction(lived_ms: f32, lifespan_ms: f32) -> f32 {
    if lifespan_ms <= 0.0 {
        return 0.0;
    }
    let fraction = lived_ms / lifespan_ms;
    if fraction < 0.0 {
        0.0
    } else if fraction > 1.0 {
        1.0
    } else {
        fraction
    }
}

/// The velocity `lived_ms` into a life of `lifespan_ms`. A graph needs at least
/// two samples to count. The world graph is added as it is; the local graph
/// is turned into world axes with `facing_turn` first.
#[allow(clippy::too_many_arguments)]
pub fn get_velocity_at_time(
    flags: i32,
    launch_speed_vec: [f32; 3],
    lived_ms: f32,
    lifespan_ms: f32,
    local_keys: &[Vec3Spread],
    world_keys: &[Vec3Spread],
    facing_turn: [[f32; 3]; 3],
    seed: u32,
) -> [f32; 3] {
    let age = age_fraction(lived_ms, lifespan_ms);
    let mut velocity = launch_speed_vec;

    if elem_uses_vel_world(flags) && world_keys.len() >= 2 {
        let world = sample_vel_graph_at_age(world_keys, age, seed);
        for axis in 0_usize..3 {
            velocity[axis] += world[axis] * PER_SECOND;
        }
    }
    if elem_uses_vel_local(flags) && local_keys.len() >= 2 {
        let local = sample_vel_graph_at_age(local_keys, age, seed);
        for axis in 0_usize..3 {
            let turned = local[0] * facing_turn[0][axis]
                + local[1] * facing_turn[1][axis]
                + local[2] * facing_turn[2][axis];
            velocity[axis] += turned * PER_SECOND;
        }
    }
    velocity
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::mode_bits::ELEM_SPEED_OWN_AXES;
    use crate::runtime::mode_bits::ELEM_SPEED_WORLD_AXES;

    const IDENTITY: [[f32; 3]; 3] = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
    const TURNED: [[f32; 3]; 3] = [[0.0, 1.0, 0.0], [-1.0, 0.0, 0.0], [0.0, 0.0, 1.0]];

    fn constant(x: f32, y: f32, z: f32) -> [Vec3Spread; 2] {
        let sample = Vec3Spread {
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
            ELEM_SPEED_WORLD_AXES,
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
            ELEM_SPEED_OWN_AXES,
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
        let one = [Vec3Spread {
            base: [9.0; 3],
            amplitude: [0.0; 3],
        }];
        let got = get_velocity_at_time(
            ELEM_SPEED_WORLD_AXES | ELEM_SPEED_OWN_AXES,
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
