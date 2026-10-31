//! Three-component arithmetic the offset needs.

pub(super) fn length_sq(v: [f32; 3]) -> f32 {
    v[0] * v[0] + v[1] * v[1] + v[2] * v[2]
}

pub(super) fn scaled_by(v: [f32; 3], by: f32) -> [f32; 3] {
    [v[0] * by, v[1] * by, v[2] * by]
}

pub(super) fn add(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

pub(super) fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

/// What is left of a miss `decay_ms` long, `elapsed` ms after it began: 1 at the start, 0 at the end.
pub(super) fn remaining(decay_ms: f32, clock_millis: i32, began_ms: i32) -> f32 {
    let elapsed = clock_millis.wrapping_sub(began_ms) as f32;
    ((decay_ms - elapsed) / decay_ms).max(0.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arithmetic_is_per_component() {
        assert_eq!(add([1.0, 2.0, 3.0], [10.0, 20.0, 30.0]), [11.0, 22.0, 33.0]);
        assert_eq!(sub([10.0, 20.0, 30.0], [1.0, 2.0, 3.0]), [9.0, 18.0, 27.0]);
        assert_eq!(scaled_by([1.0, 2.0, 3.0], 2.0), [2.0, 4.0, 6.0]);
        assert_eq!(length_sq([1.0, 2.0, 2.0]), 9.0);
    }

    #[test]
    fn a_miss_fades_linearly_to_nothing() {
        assert_eq!(remaining(100.0, 1000, 1000), 1.0);
        assert_eq!(remaining(100.0, 1050, 1000), 0.5);
        assert_eq!(remaining(100.0, 1100, 1000), 0.0);
        assert_eq!(remaining(100.0, 1500, 1000), 0.0, "long after: not negative");
    }

    #[test]
    fn the_clock_may_have_wrapped_since_the_miss_began() {
        assert_eq!(remaining(100.0, i32::MIN + 25, i32::MAX - 24), 0.5);
    }
}
