//! Where the gun points in the world, for the shot direction saved while aiming down
//! iron sights.

use game_math::{wrap_degrees, euler_basis, dir_to_euler};

/// Iron sights (no scope overlay) that are any way up save the gun's own aim rather than
/// the view's.
#[must_use]
pub fn iron_sight_keeps_axis(
    scoped_aiming: bool,
    gun_raise_share: f32,
    hud_crosshair_layer: i32,
) -> bool {
    scoped_aiming && hud_crosshair_layer == 0 && gun_raise_share != 0.0
}

/// A view-space offset (forward, right, up) turned into the world by `look_rotation`.
#[must_use]
pub fn gun_view_delta_to_world(origin: [f32; 3], look_rotation: [f32; 3]) -> [f32; 3] {
    let (ahead, side, above) = euler_basis(look_rotation);
    std::array::from_fn(|i| ahead[i] * origin[0] + side[i] * origin[1] + above[i] * origin[2])
}

/// The gun's forward in the world.
fn gun_forward_in_world(weapon_aim_angles: [f32; 3], look_rotation: [f32; 3]) -> [f32; 3] {
    // bo2mp: the gun's axis put on the view's (MatrixMultiply of the two
    // AnglesToAxis): the second axis is the view's left, not its side, so
    // a gun turned left aims left.
    let (gun, _, _) = euler_basis(weapon_aim_angles);
    let (ahead, side, above) = euler_basis(look_rotation);
    std::array::from_fn(|i| gun[0] * ahead[i] - gun[1] * side[i] + gun[2] * above[i])
}

/// The pitch and yaw the shot is saved with: the gun's own aim through iron sights, else
/// the view's.
#[must_use]
pub fn keep_gun_pitch_yaw(
    weapon_aim_angles: [f32; 3],
    look_rotation: [f32; 3],
    scoped_aiming: bool,
    gun_raise_share: f32,
    hud_crosshair_layer: i32,
) -> [f32; 2] {
    if !iron_sight_keeps_axis(scoped_aiming, gun_raise_share, hud_crosshair_layer) {
        return [look_rotation[0], look_rotation[1]];
    }
    let [pitch, yaw, _] = dir_to_euler(gun_forward_in_world(weapon_aim_angles, look_rotation));
    [wrap_degrees(pitch), wrap_degrees(yaw)]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_gun_turned_left_aims_left_of_the_view() {
        let [pitch, yaw] =
            keep_gun_pitch_yaw([2.0, 3.0, 0.0], [0.0, 90.0, 0.0], true, 1.0, 0);
        assert!((pitch - 2.0).abs() < 1e-3, "pitch {pitch}");
        assert!((yaw - 93.0).abs() < 1e-3, "yaw {yaw}");
    }

    #[test]
    fn hip_fire_and_scopes_save_the_view() {
        let view = [5.0, 30.0, 0.0];
        assert_eq!(
            keep_gun_pitch_yaw([2.0, 3.0, 0.0], view, false, 1.0, 0),
            [5.0, 30.0]
        );
        assert_eq!(
            keep_gun_pitch_yaw([2.0, 3.0, 0.0], view, true, 1.0, 1),
            [5.0, 30.0]
        );
        assert_eq!(
            keep_gun_pitch_yaw([2.0, 3.0, 0.0], view, true, 0.0, 0),
            [5.0, 30.0]
        );
    }

    #[test]
    fn a_view_offset_turns_with_the_view() {
        let delta = gun_view_delta_to_world([10.0, 0.0, 2.0], [0.0, 90.0, 0.0]);
        assert!(
            delta[0].abs() < 1e-4 && (delta[1] - 10.0).abs() < 1e-4,
            "{delta:?}"
        );
        assert!((delta[2] - 2.0).abs() < 1e-4);
    }
}
