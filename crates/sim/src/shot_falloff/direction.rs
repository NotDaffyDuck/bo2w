/// The unit vector `look_rotation` (pitch, yaw, roll in degrees) points along. Roll does not change where it points.
pub fn angles_to_forward(look_rotation: [f32; 3]) -> [f32; 3] {
    let (pitch, yaw) = (look_rotation[0].to_radians(), look_rotation[1].to_radians());
    let level = pitch.cos();
    [level * yaw.cos(), level * yaw.sin(), -pitch.sin()]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn near(a: [f32; 3], b: [f32; 3]) -> bool {
        a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-5)
    }

    #[test]
    fn zero_angles_face_along_x() {
        assert!(near(angles_to_forward([0.0; 3]), [1.0, 0.0, 0.0]));
    }

    #[test]
    fn yaw_turns_left_and_pitch_looks_down() {
        assert!(near(angles_to_forward([0.0, 90.0, 0.0]), [0.0, 1.0, 0.0]));
        assert!(near(angles_to_forward([90.0, 0.0, 0.0]), [0.0, 0.0, -1.0]));
    }

    #[test]
    fn roll_is_ignored() {
        assert_eq!(angles_to_forward([10.0, 20.0, 0.0]), angles_to_forward([10.0, 20.0, 77.0]));
    }
}
