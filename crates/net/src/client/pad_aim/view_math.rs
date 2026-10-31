//! Angles in degrees as (pitch, yaw); positions as [x, y, z].

pub(super) fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

pub(super) fn minus(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

pub(super) fn blend(lo: f32, hi: f32, share: f32) -> f32 {
    lo + (hi - lo) * share
}

/// The forward, right and up directions of a view.
pub(super) fn axes(pitch_yaw: [f32; 2]) -> ([f32; 3], [f32; 3], [f32; 3]) {
    let (sin_pitch, cos_pitch) = pitch_yaw[0].to_radians().sin_cos();
    let (yaw_sin, yaw_cos) = pitch_yaw[1].to_radians().sin_cos();
    (
        [cos_pitch * yaw_cos, cos_pitch * yaw_sin, -sin_pitch],
        [yaw_sin, -yaw_cos, 0.0],
        [sin_pitch * yaw_cos, sin_pitch * yaw_sin, cos_pitch],
    )
}

/// The view angles that look from `eye` at `point`.
pub(super) fn angles_to(eye: [f32; 3], spot: [f32; 3]) -> [f32; 2] {
    let d = minus(spot, eye);
    let ground = d[0].hypot(d[1]);
    [-d[2].atan2(ground).to_degrees(), d[1].atan2(d[0]).to_degrees()]
}

/// The turn from `from` to `to` by the short way round, in -180..180.
pub(super) fn wrapped_turn(to: f32, from: f32) -> f32 {
    (to - from + 540.0).rem_euclid(360.0) - 180.0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-4
    }

    #[test]
    fn a_level_view_at_zero_yaw_looks_down_the_x_axis() {
        let (ahead, side, above) = axes([0.0, 0.0]);
        assert!(close(ahead[0], 1.0) && close(ahead[1], 0.0) && close(ahead[2], 0.0));
        assert!(close(side[1], -1.0));
        assert!(close(above[2], 1.0));
    }

    #[test]
    fn looking_up_tilts_forward_toward_positive_z() {
        let (ahead, _, _) = axes([-90.0, 0.0]);
        assert!(close(ahead[2], 1.0));
    }

    #[test]
    fn the_angles_to_a_point_undo_the_axes() {
        let pitch_yaw = angles_to([0.0; 3], [100.0, 100.0, 0.0]);
        assert!(close(pitch_yaw[0], 0.0) && close(pitch_yaw[1], 45.0));
        let above = angles_to([0.0; 3], [100.0, 0.0, 100.0]);
        assert!(close(above[0], -45.0));
    }

    #[test]
    fn turns_go_the_short_way_round() {
        assert!(close(wrapped_turn(10.0, 350.0), 20.0));
        assert!(close(wrapped_turn(350.0, 10.0), -20.0));
        assert!(close(wrapped_turn(90.0, 90.0), 0.0));
    }

    #[test]
    fn lerp_runs_from_one_end_to_the_other() {
        assert_eq!(blend(2.0, 6.0, 0.0), 2.0);
        assert_eq!(blend(2.0, 6.0, 1.0), 6.0);
        assert_eq!(blend(2.0, 6.0, 0.25), 3.0);
    }
}
