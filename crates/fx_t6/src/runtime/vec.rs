//! The small vector helpers the effects code shares.

use core::f32::consts::TAU;

use crate::runtime::quat::axis_to_quat;

/// A vector this short (squared) counts as having no direction.
const TOO_SHORT_SQ: f32 = 1e-12;

/// The squared length of `v`.
pub fn vec3_length_sq(v: [f32; 3]) -> f32 {
    v[0] * v[0] + v[1] * v[1] + v[2] * v[2]
}

/// `v` scaled to length one, or all zeros when it is too short to have a
/// direction.
pub fn vec3_normalize(v: [f32; 3]) -> [f32; 3] {
    let length_sq = vec3_length_sq(v);
    if length_sq <= TOO_SHORT_SQ {
        return [0.0; 3];
    }
    let scale = 1.0 / libm::sqrtf(length_sq);
    [v[0] * scale, v[1] * scale, v[2] * scale]
}

/// `a` crossed with `b`.
pub(super) fn vec3_cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

/// How far apart two points are.
pub fn vec3_distance(from: [f32; 3], dst: [f32; 3]) -> f32 {
    libm::sqrtf(vec3_length_sq([dst[0] - from[0], dst[1] - from[1], dst[2] - from[2]]))
}

/// A unit vector at right angles to `v`: the one built from the axis `v`
/// points along least.
pub fn perpendicular_vector(w: [f32; 3]) -> [f32; 3] {
    let squares = [w[0] * w[0], w[1] * w[1], w[2] * w[2]];
    let mut weakest = usize::from(squares[1] < squares[0]);
    if squares[2] < squares[weakest] {
        weakest = 2;
    }
    let k = -w[weakest];
    let mut out = [k * w[0], k * w[1], k * w[2]];
    out[weakest] += 1.0;
    vec3_normalize(out)
}

/// A right-handed frame `[forward, right, up]` around `forward`. A forward
/// with no direction gives the identity frame.
pub fn contact_facing_basis(forward: [f32; 3]) -> [[f32; 3]; 3] {
    let f = vec3_normalize(forward);
    if vec3_length_sq(f) < TOO_SHORT_SQ {
        return [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
    }
    let up = perpendicular_vector(f);
    [f, vec3_cross(up, f), up]
}

/// The turn, in radians, between two frames, going whichever way round is
/// shorter.
pub fn effect_orient_arc(last_axis: [[f32; 3]; 3], cur_turn: [[f32; 3]; 3]) -> f32 {
    let last = axis_to_quat(last_axis);
    let now = axis_to_quat(cur_turn);
    let dot = last[0] * now[0] + last[1] * now[1] + last[2] * now[2] + last[3] * now[3];
    let angle = 2.0 * libm::acosf(dot.clamp(-1.0, 1.0));
    let other_way = TAU - angle;
    if angle <= other_way { angle } else { other_way }
}

#[cfg(test)]
mod tests {
    use super::*;

    const IDENTITY: [[f32; 3]; 3] = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];

    #[test]
    fn normalizing_gives_length_one_and_zero_stays_zero() {
        let n = vec3_normalize([3.0, 0.0, 4.0]);
        assert!((vec3_length_sq(n) - 1.0).abs() < 1e-6);
        assert_eq!(vec3_normalize([0.0; 3]), [0.0; 3]);
    }

    #[test]
    fn distance_is_straight_line() {
        assert_eq!(vec3_distance([1.0, 2.0, 3.0], [4.0, 6.0, 3.0]), 5.0);
    }

    #[test]
    fn a_perpendicular_vector_is_unit_and_square_on() {
        for v in [[1.0, 0.0, 0.0], [0.2, -0.9, 0.4], [0.0, 0.0, 1.0]] {
            let v = vec3_normalize(v);
            let p = perpendicular_vector(v);
            assert!((vec3_length_sq(p) - 1.0).abs() < 1e-5, "{p:?}");
            let dot = v[0] * p[0] + v[1] * p[1] + v[2] * p[2];
            assert!(dot.abs() < 1e-5, "{dot}");
        }
    }

    #[test]
    fn the_frame_around_a_direction_starts_with_that_direction() {
        let frame = contact_facing_basis([0.0, 0.0, 2.0]);
        assert_eq!(frame[0], [0.0, 0.0, 1.0]);
        assert_eq!(contact_facing_basis([0.0; 3]), IDENTITY);
    }

    #[test]
    fn the_arc_between_equal_frames_is_zero() {
        assert_eq!(effect_orient_arc(IDENTITY, IDENTITY), 0.0);
    }

    #[test]
    fn the_arc_between_a_frame_and_a_quarter_turn_is_a_quarter_turn() {
        let quarter = [[0.0, 1.0, 0.0], [-1.0, 0.0, 0.0], [0.0, 0.0, 1.0]];
        let arc = effect_orient_arc(IDENTITY, quarter);
        assert!((arc - core::f32::consts::FRAC_PI_2).abs() < 1e-3, "{arc}");
    }
}
