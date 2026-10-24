//! Where the gun points in the world, for the shot direction saved while aiming down
//! iron sights.

use game_math::{angle_normalize_360, angle_vectors, vect_to_angles};

/// Iron sights (no scope overlay) that are any way up save the gun's own aim rather than
/// the view's.
#[must_use]
pub fn viewweapon_iron_ads_saves_composed_axis(
    aim_down_sight: bool,
    weapon_pos_frac: f32,
    overlay_reticle: i32,
) -> bool {
    aim_down_sight && overlay_reticle == 0 && weapon_pos_frac != 0.0
}

/// A view-space offset (forward, right, up) turned into the world by `view_angles`.
#[must_use]
pub fn viewweapon_view_to_world_delta(origin: [f32; 3], view_angles: [f32; 3]) -> [f32; 3] {
    let (forward, right, up) = angle_vectors(view_angles);
    std::array::from_fn(|i| forward[i] * origin[0] + right[i] * origin[1] + up[i] * origin[2])
}

/// The gun's forward in the world.
fn gun_forward_in_world(gun_angles: [f32; 3], view_angles: [f32; 3]) -> [f32; 3] {
    // bo2mp: the gun's axis put on the view's (MatrixMultiply of the two
    // AnglesToAxis): the second axis is the view's left, not its right, so
    // a gun turned left aims left.
    let (gun, _, _) = angle_vectors(gun_angles);
    let (forward, right, up) = angle_vectors(view_angles);
    std::array::from_fn(|i| gun[0] * forward[i] - gun[1] * right[i] + gun[2] * up[i])
}

/// The pitch and yaw the shot is saved with: the gun's own aim through iron sights, else
/// the view's.
#[must_use]
pub fn viewweapon_save_gun_pitch_yaw(
    gun_angles: [f32; 3],
    view_angles: [f32; 3],
    aim_down_sight: bool,
    weapon_pos_frac: f32,
    overlay_reticle: i32,
) -> [f32; 2] {
    if !viewweapon_iron_ads_saves_composed_axis(aim_down_sight, weapon_pos_frac, overlay_reticle) {
        return [view_angles[0], view_angles[1]];
    }
    let [pitch, yaw, _] = vect_to_angles(gun_forward_in_world(gun_angles, view_angles));
    [angle_normalize_360(pitch), angle_normalize_360(yaw)]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_gun_turned_left_aims_left_of_the_view() {
        let [pitch, yaw] =
            viewweapon_save_gun_pitch_yaw([2.0, 3.0, 0.0], [0.0, 90.0, 0.0], true, 1.0, 0);
        assert!((pitch - 2.0).abs() < 1e-3, "pitch {pitch}");
        assert!((yaw - 93.0).abs() < 1e-3, "yaw {yaw}");
    }

    #[test]
    fn hip_fire_and_scopes_save_the_view() {
        let view = [5.0, 30.0, 0.0];
        assert_eq!(
            viewweapon_save_gun_pitch_yaw([2.0, 3.0, 0.0], view, false, 1.0, 0),
            [5.0, 30.0]
        );
        assert_eq!(
            viewweapon_save_gun_pitch_yaw([2.0, 3.0, 0.0], view, true, 1.0, 1),
            [5.0, 30.0]
        );
        assert_eq!(
            viewweapon_save_gun_pitch_yaw([2.0, 3.0, 0.0], view, true, 0.0, 0),
            [5.0, 30.0]
        );
    }

    #[test]
    fn a_view_offset_turns_with_the_view() {
        let delta = viewweapon_view_to_world_delta([10.0, 0.0, 2.0], [0.0, 90.0, 0.0]);
        assert!(
            delta[0].abs() < 1e-4 && (delta[1] - 10.0).abs() < 1e-4,
            "{delta:?}"
        );
        assert!((delta[2] - 2.0).abs() < 1e-4);
    }
}
