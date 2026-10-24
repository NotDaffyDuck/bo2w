//! The camera's components and how a game-space view becomes a transform.

use bevy::prelude::*;

/// Where the game says the view is: an origin and pitch/yaw/roll in degrees.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WorldCameraPose {
    pub origin: [f32; 3],
    pub angles: [f32; 3],
}

/// Marks the camera that renders the player's view.
#[derive(Component)]
pub struct FpvLens;

/// A free camera the player flies by hand.
#[derive(Component)]
pub struct FlyCamera {
    pub yaw: f32,
    pub pitch: f32,
    pub speed: f32,
}

/// Whether the simulation drives the camera, and whether the fly camera holds still meanwhile.
#[derive(Resource, Clone, Copy, Debug, Default)]
pub struct SimCamera {
    pub enabled: bool,
    pub freeze_fly: bool,
}

/// The camera transform for a game view. The game looks along +X with +Z up and
/// roll leaning the view; the transform looks along its own -Z with +Y up, so the
/// rotation takes the game's right, up and negated forward as its columns.
pub fn transform_from_iw_view(view: WorldCameraPose) -> Transform {
    let [pitch, yaw, roll] = view.angles.map(f32::to_radians);
    let (sin_p, cos_p) = pitch.sin_cos();
    let (sin_y, cos_y) = yaw.sin_cos();
    let (sin_r, cos_r) = roll.sin_cos();

    let forward = Vec3::new(cos_p * cos_y, cos_p * sin_y, -sin_p);
    let right = Vec3::new(
        cos_r * sin_y - sin_r * sin_p * cos_y,
        -cos_r * cos_y - sin_r * sin_p * sin_y,
        -sin_r * cos_p,
    );
    let up = Vec3::new(
        cos_r * sin_p * cos_y + sin_r * sin_y,
        cos_r * sin_p * sin_y - sin_r * cos_y,
        cos_r * cos_p,
    );
    Transform {
        translation: Vec3::from_array(view.origin),
        rotation: Quat::from_mat3(&Mat3::from_cols(right, up, -forward)),
        ..default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn facing(angles: [f32; 3]) -> Transform {
        transform_from_iw_view(WorldCameraPose { origin: [1.0, 2.0, 3.0], angles })
    }

    fn close(a: Vec3, b: Vec3) -> bool {
        a.distance(b) < 1e-5
    }

    #[test]
    fn zero_angles_look_along_x_with_z_up() {
        let t = facing([0.0; 3]);
        assert_eq!(t.translation, Vec3::new(1.0, 2.0, 3.0));
        assert!(close(t.rotation * Vec3::NEG_Z, Vec3::X));
        assert!(close(t.rotation * Vec3::Y, Vec3::Z));
        assert!(close(t.rotation * Vec3::X, Vec3::NEG_Y));
    }

    #[test]
    fn yaw_turns_toward_y() {
        let t = facing([0.0, 90.0, 0.0]);
        assert!(close(t.rotation * Vec3::NEG_Z, Vec3::Y));
        assert!(close(t.rotation * Vec3::Y, Vec3::Z));
    }

    #[test]
    fn positive_pitch_looks_down() {
        let t = facing([90.0, 0.0, 0.0]);
        assert!(close(t.rotation * Vec3::NEG_Z, Vec3::NEG_Z));
    }

    #[test]
    fn a_quarter_roll_swings_up_to_the_side() {
        let t = facing([0.0, 0.0, 90.0]);
        assert!(close(t.rotation * Vec3::NEG_Z, Vec3::X));
        assert!(close(t.rotation * Vec3::Y, Vec3::NEG_Y));
        assert!(close(t.rotation * Vec3::X, Vec3::NEG_Z));
    }
}
