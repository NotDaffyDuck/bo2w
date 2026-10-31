//! Velocity graphs: a few samples spread evenly over an element's life, each
//! a base plus a random share of an amplitude.

use crate::runtime::dice::random_table_f32;

/// One sample of a velocity graph.
#[derive(Clone, Copy, Debug, Default)]
#[repr(C)]
pub struct Vec3Spread {
    pub base: [f32; 3],
    pub amplitude: [f32; 3],
}

impl Vec3Spread {
    /// The sample's value for this seed: each axis takes its own random
    /// channel.
    fn drawn(self, salt: u32) -> [f32; 3] {
        let mut channel = 0u32;
        self.base.map(|base| {
            let value = base + self.amplitude[channel as usize] * random_table_f32(salt, channel);
            channel += 1;
            value
        })
    }
}

fn clamp_unit(x: f32) -> f32 {
    x.clamp(0.0, 1.0)
}

/// The graph's value at `age01` (0 at birth, 1 at the end of life), blended
/// between the two samples either side of it.
pub(super) fn sample_vel_graph_at_age(
    keys: &[Vec3Spread],
    age01: f32,
    seed: u32,
) -> [f32; 3] {
    let Some(last) = keys.len().checked_sub(1) else {
        panic!("a velocity graph has at least one sample");
    };
    let along = clamp_unit(age01) * last as f32;
    let lower = (libm::floorf(along) as usize).min(last);
    let upper = (lower + 1).min(last);
    let blend = (upper != lower)
        .then(|| along - lower as f32)
        .unwrap_or(0.0);
    let (a, b) = (keys[lower].drawn(seed), keys[upper].drawn(seed));
    core::array::from_fn(|k| a[k] + (b[k] - a[k]) * blend)
}

/// How far the graph's velocity carries an element between two ages: the
/// area under the graph, taken segment by segment as trapezoids. Returns zero
/// for a graph with fewer than two samples or a life with no length.
pub fn integrate_velocity_graph(
    keys: &[Vec3Spread],
    age_from: f32,
    age_to: f32,
    lifespan_ms: f32,
    seed: u32,
) -> [f32; 3] {
    let zero = [0.0f32; 3];
    let start = clamp_unit(age_from);
    let end = clamp_unit(age_to).max(start);
    // Needs a segment, a life, and a forward span (written as "not <=" so a
    // NaN carries on, as it always has).
    let refused = keys.len() < 2 || lifespan_ms <= 0.0 || end <= start;
    if refused {
        return zero;
    }

    let segments = keys.len() - 1;
    let ms_per_unit_age = lifespan_ms * segments as f32;
    let first = (libm::floorf(start * segments as f32) as usize).min(segments - 1);
    let one_past_last = (libm::ceilf(end * segments as f32) as usize).clamp(first + 1, segments);
    // One trapezoid per graph segment the span touches.
    (first..one_past_last)
        .map(|segment| {
            (
                start.max(segment as f32 / segments as f32),
                end.min((segment + 1) as f32 / segments as f32),
            )
        })
        .filter(|(from, to)| to > from)
        .fold(zero, |total, (from, to)| {
            let at_from = sample_vel_graph_at_age(keys, from, seed);
            let at_to = sample_vel_graph_at_age(keys, to, seed);
            let half_width = 0.5 * (to - from) * ms_per_unit_age;
            core::array::from_fn(|k| total[k] + (at_from[k] + at_to[k]) * half_width)
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flat(speed: f32) -> [Vec3Spread; 2] {
        let sample = Vec3Spread {
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
            Vec3Spread {
                base: [0.0; 3],
                amplitude: [0.0; 3],
            },
            Vec3Spread {
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
            Vec3Spread {
                base: [1.0; 3],
                amplitude: [2.0; 3],
            },
            Vec3Spread {
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
        let one = [Vec3Spread::default()];
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
            Vec3Spread {
                base: [0.0; 3],
                amplitude: [0.0; 3],
            },
            Vec3Spread {
                base: [2.0, 0.0, 0.0],
                amplitude: [0.0; 3],
            },
            Vec3Spread {
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
