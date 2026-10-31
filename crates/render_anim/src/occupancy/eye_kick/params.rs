//! Turning a weapon's kick facts into the numbers the kick state steps with.

use asset_game::WeaponRecoilSummary;

use crate::motion::kick_state::GunKickTuning;

/// The kick state's own copy of a weapon's kick facts: every range, speed
/// and decay carries over unchanged, hip group first, then the aimed group.
pub(super) fn kick_params(facts: &WeaponRecoilSummary) -> GunKickTuning {
    GunKickTuning {
        hipfire_weapon_kick_damped_percent: facts.hipfire_weapon_kick_damped_percent,
        hipfire_weapon_kick_pitch_bottom: facts.hipfire_weapon_kick_pitch_bottom,
        hipfire_weapon_kick_pitch_top: facts.hipfire_weapon_kick_pitch_top,
        hipfire_weapon_kick_yaw_bottom: facts.hipfire_weapon_kick_yaw_bottom,
        hipfire_weapon_kick_yaw_top: facts.hipfire_weapon_kick_yaw_top,
        hipfire_weapon_kick_accel: facts.hipfire_weapon_kick_accel,
        hipfire_weapon_kick_velocity_cap: facts.hipfire_weapon_kick_velocity_cap,
        hipfire_weapon_kick_velocity_falloff: facts.hipfire_weapon_kick_velocity_falloff,
        hipfire_weapon_kick_resting_falloff: facts.hipfire_weapon_kick_resting_falloff,
        hipfire_camera_kick_pitch_bottom: facts.hipfire_camera_kick_pitch_bottom,
        hipfire_camera_kick_pitch_top: facts.hipfire_camera_kick_pitch_top,
        hipfire_camera_kick_yaw_bottom: facts.hipfire_camera_kick_yaw_bottom,
        hipfire_camera_kick_yaw_top: facts.hipfire_camera_kick_yaw_top,
        hip_view_kick_min_magnitude: facts.hip_view_kick_min_magnitude,
        hipfire_view_kick_recenter_speed: facts.hipfire_view_kick_recenter_speed,
        scoped_weapon_kick_damped_percent: facts.scoped_weapon_kick_damped_percent,
        scoped_weapon_kick_pitch_bottom: facts.scoped_weapon_kick_pitch_bottom,
        scoped_weapon_kick_pitch_top: facts.scoped_weapon_kick_pitch_top,
        scoped_weapon_kick_yaw_bottom: facts.scoped_weapon_kick_yaw_bottom,
        scoped_weapon_kick_yaw_top: facts.scoped_weapon_kick_yaw_top,
        scoped_weapon_kick_accel: facts.scoped_weapon_kick_accel,
        scoped_weapon_kick_velocity_cap: facts.scoped_weapon_kick_velocity_cap,
        scoped_weapon_kick_velocity_falloff: facts.scoped_weapon_kick_velocity_falloff,
        scoped_weapon_kick_resting_falloff: facts.scoped_weapon_kick_resting_falloff,
        scoped_camera_kick_pitch_bottom: facts.scoped_camera_kick_pitch_bottom,
        scoped_camera_kick_pitch_top: facts.scoped_camera_kick_pitch_top,
        scoped_camera_kick_yaw_bottom: facts.scoped_camera_kick_yaw_bottom,
        scoped_camera_kick_yaw_top: facts.scoped_camera_kick_yaw_top,
        ads_view_kick_min_magnitude: facts.ads_view_kick_min_magnitude,
        scoped_view_kick_recenter_speed: facts.scoped_view_kick_recenter_speed,
        weapon_pitch_ceiling: facts.weapon_pitch_ceiling,
        weapon_yaw_ceiling: facts.weapon_yaw_ceiling,
    }
}

/// bo2zm: Black Ops II dual wield, hip fire only. With both guns firing
/// a shot kicks with the two guns' hip ranges added together; with only the
/// left gun firing it kicks with the left gun's own ranges.
pub(super) fn dual_wield_kick(
    right: GunKickTuning,
    left: &GunKickTuning,
    right_firing: bool,
    left_firing: bool,
) -> GunKickTuning {
    let mut k = right;
    if right_firing && left_firing {
        k.hipfire_camera_kick_pitch_bottom += left.hipfire_camera_kick_pitch_bottom;
        k.hipfire_camera_kick_pitch_top += left.hipfire_camera_kick_pitch_top;
        k.hipfire_camera_kick_yaw_bottom += left.hipfire_camera_kick_yaw_bottom;
        k.hipfire_camera_kick_yaw_top += left.hipfire_camera_kick_yaw_top;
        k.hipfire_weapon_kick_pitch_bottom += left.hipfire_weapon_kick_pitch_bottom;
        k.hipfire_weapon_kick_pitch_top += left.hipfire_weapon_kick_pitch_top;
        k.hipfire_weapon_kick_yaw_bottom += left.hipfire_weapon_kick_yaw_bottom;
        k.hipfire_weapon_kick_yaw_top += left.hipfire_weapon_kick_yaw_top;
    } else if left_firing {
        k.hipfire_camera_kick_pitch_bottom = left.hipfire_camera_kick_pitch_bottom;
        k.hipfire_camera_kick_pitch_top = left.hipfire_camera_kick_pitch_top;
        k.hipfire_camera_kick_yaw_bottom = left.hipfire_camera_kick_yaw_bottom;
        k.hipfire_camera_kick_yaw_top = left.hipfire_camera_kick_yaw_top;
        k.hipfire_weapon_kick_pitch_bottom = left.hipfire_weapon_kick_pitch_bottom;
        k.hipfire_weapon_kick_pitch_top = left.hipfire_weapon_kick_pitch_top;
        k.hipfire_weapon_kick_yaw_bottom = left.hipfire_weapon_kick_yaw_bottom;
        k.hipfire_weapon_kick_yaw_top = left.hipfire_weapon_kick_yaw_top;
    }
    k
}
