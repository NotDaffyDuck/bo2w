//! Velocity graphs: a few samples spread evenly over an element's life, each
//! a base plus a random share of an amplitude.

use crate::runtime::random::random_table_f32;

/// One sample of a velocity graph.
#[derive(Clone, Copy, Debug, Default)]
#[repr(C)]
pub struct FxElemVec3Range {
    pub base: [f32; 3],
    pub amplitude: [f32; 3],
}

impl FxElemVec3Range {
    /// The sample's value for this seed: each axis takes its own random
    /// channel.
    fn sampled(self, seed: u32) -> [f32; 3] {
        core::array::from_fn(|axis| {
            self.base[axis] + self.amplitude[axis] * random_table_f32(seed, axis as u32)
        })
    }
}

fn clamp_unit(x: f32) -> f32 {
    if x < 0.0 {
        0.0
    } else if x > 1.0 {
        1.0
    } else {
        x
    }
}

/// The graph's value at `age01` (0 at birth, 1 at the end of life), blended
/// between the two samples either side of it.
pub(super) fn sample_vel_graph_at_age(
    samples: &[FxElemVec3Range],
    age01: f32,
    seed: u32,
) -> [f32; 3] {
    let last = samples.len() - 1;
    let along = clamp_unit(age01) * last as f32;
    let lower = (libm::floorf(along) as usize).min(last);
    let upper = (lower + 1).min(last);
    let blend = if lower == upper {
        0.0
    } else {
        along - lower as f32
    };
    let a = samples[lower].sampled(seed);
    let b = samples[upper].sampled(seed);
    [
        a[0] + (b[0] - a[0]) * blend,
        a[1] + (b[1] - a[1]) * blend,
        a[2] + (b[2] - a[2]) * blend,
    ]
}

/// How far the graph's velocity carries an element between two ages: the
/// area under the graph, taken segment by segment as trapezoids. Returns zero
/// for a graph with fewer than two samples or a life with no length.
pub fn integrate_velocity_graph(
    samples: &[FxElemVec3Range],
    start_age01: f32,
    end_age01: f32,
    life_ms: f32,
    seed: u32,
) -> [f32; 3] {
    let mut distance = [0.0f32; 3];
    if samples.len() < 2 || life_ms <= 0.0 {
        return distance;
    }
    let start = clamp_unit(start_age01);
    let end = clamp_unit(end_age01).max(start);
    if end <= start {
        return distance;
    }

    let segments = samples.len() - 1;
    let ms_per_unit_age = life_ms * segments as f32;
    let first = (libm::floorf(start * segments as f32) as usize).min(segments - 1);
    let one_past_last = (libm::ceilf(end * segments as f32) as usize).clamp(first + 1, segments);
    for segment in first..one_past_last {
        let from = start.max(segment as f32 / segments as f32);
        let to = end.min((segment + 1) as f32 / segments as f32);
        if to <= from {
            continue;
        }
        let at_from = sample_vel_graph_at_age(samples, from, seed);
        let at_to = sample_vel_graph_at_age(samples, to, seed);
        let half_width = 0.5 * (to - from) * ms_per_unit_age;
        for axis in 0..3 {
            distance[axis] += (at_from[axis] + at_to[axis]) * half_width;
        }
    }
    distance
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flat(speed: f32) -> [FxElemVec3Range; 2] {
        let sample = FxElemVec3Range {
            base: [speed, 0.0, -speed],
            amplitude: [0.0; 3],
        };
        [sample, sample]
    }

    #[test]
    fn a_flat_graph_reads_the_same_everywhere() {
        let graph = flat(3.0);
        assert_eq!(sample_vel_graph_at_age(&graph, 0.0, 1), [3.0, 0.0, -3.0]);
        assert_eq!(sample_vel_graph_at_age(&graph, 0.7, 1), [3.0, 0.0, -3.0]);
        assert_eq!(sample_vel_graph_at_age(&graph, 5.0, 1), [3.0, 0.0, -3.0]);
    }

    #[test]
    fn a_ramp_is_blended_between_its_samples() {
        let graph = [
            FxElemVec3Range {
                base: [0.0; 3],
                amplitude: [0.0; 3],
            },
            FxElemVec3Range {
                base: [10.0, 0.0, 0.0],
                amplitude: [0.0; 3],
            },
        ];
        assert_eq!(sample_vel_graph_at_age(&graph, 0.5, 0), [5.0, 0.0, 0.0]);
        assert_eq!(sample_vel_graph_at_age(&graph, -1.0, 0), [0.0; 3]);
    }

    #[test]
    fn a_sample_amplitude_takes_the_random_table_per_axis() {
        let graph = [
            FxElemVec3Range {
                base: [1.0; 3],
                amplitude: [2.0; 3],
            },
            FxElemVec3Range {
                base: [1.0; 3],
                amplitude: [2.0; 3],
            },
        ];
        let got = sample_vel_graph_at_age(&graph, 0.0, 9);
        for (axis, value) in got.iter().enumerate() {
            assert_eq!(*value, 1.0 + 2.0 * random_table_f32(9, axis as u32));
        }
    }

    #[test]
    fn a_flat_graph_covers_speed_times_time() {
        // Speeds are per millisecond: 2 ms of life at speed 3 covers 6.
        let got = integrate_velocity_graph(&flat(3.0), 0.0, 1.0, 2.0, 0);
        assert_eq!(got, [6.0, 0.0, -6.0]);
    }

    #[test]
    fn a_partial_span_covers_its_share() {
        let got = integrate_velocity_graph(&flat(1.0), 0.25, 0.75, 8.0, 0);
        assert_eq!(got, [4.0, 0.0, -4.0]);
    }

    #[test]
    fn short_graphs_empty_spans_and_dead_lives_cover_nothing() {
        let one = [FxElemVec3Range::default()];
        assert_eq!(integrate_velocity_graph(&one, 0.0, 1.0, 1.0, 0), [0.0; 3]);
        assert_eq!(
            integrate_velocity_graph(&flat(1.0), 0.5, 0.5, 1.0, 0),
            [0.0; 3]
        );
        assert_eq!(
            integrate_velocity_graph(&flat(1.0), 0.0, 1.0, 0.0, 0),
            [0.0; 3]
        );
        assert_eq!(
            integrate_velocity_graph(&flat(1.0), 0.9, 0.1, 1.0, 0),
            [0.0; 3]
        );
    }

    #[test]
    fn a_two_segment_graph_adds_the_trapezoids() {
        let graph = [
            FxElemVec3Range {
                base: [0.0; 3],
                amplitude: [0.0; 3],
            },
            FxElemVec3Range {
                base: [2.0, 0.0, 0.0],
                amplitude: [0.0; 3],
            },
            FxElemVec3Range {
                base: [0.0; 3],
                amplitude: [0.0; 3],
            },
        ];
        // Two segments of life 1 ms each is 2 ms of age; each trapezoid is
        // (0 + 2) / 2 * 1 ms = 1.
        let got = integrate_velocity_graph(&graph, 0.0, 1.0, 1.0, 0);
        assert!((got[0] - 2.0).abs() < 1e-6, "{got:?}");
    }
}
