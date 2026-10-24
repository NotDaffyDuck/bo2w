//! Turning a weapon's kick facts into the numbers the kick state steps with.

use asset_game::WeaponKickFacts;

use crate::anim::view_kick_state::KickParams;

/// The kick state's own copy of a weapon's kick facts: every range, speed
/// and decay carries over unchanged, hip group first, then the aimed group.
pub(super) fn kick_params(facts: &WeaponKickFacts) -> KickParams {
    KickParams {
        hip_gun_kick_reduced_kick_percent: facts.hip_gun_kick_reduced_kick_percent,
        hip_gun_kick_pitch_min: facts.hip_gun_kick_pitch_min,
        hip_gun_kick_pitch_max: facts.hip_gun_kick_pitch_max,
        hip_gun_kick_yaw_min: facts.hip_gun_kick_yaw_min,
        hip_gun_kick_yaw_max: facts.hip_gun_kick_yaw_max,
        hip_gun_kick_accel: facts.hip_gun_kick_accel,
        hip_gun_kick_speed_max: facts.hip_gun_kick_speed_max,
        hip_gun_kick_speed_decay: facts.hip_gun_kick_speed_decay,
        hip_gun_kick_static_decay: facts.hip_gun_kick_static_decay,
        hip_view_kick_pitch_min: facts.hip_view_kick_pitch_min,
        hip_view_kick_pitch_max: facts.hip_view_kick_pitch_max,
        hip_view_kick_yaw_min: facts.hip_view_kick_yaw_min,
        hip_view_kick_yaw_max: facts.hip_view_kick_yaw_max,
        hip_view_kick_min_magnitude: facts.hip_view_kick_min_magnitude,
        f_hip_view_kick_center_speed: facts.f_hip_view_kick_center_speed,
        ads_gun_kick_reduced_kick_percent: facts.ads_gun_kick_reduced_kick_percent,
        ads_gun_kick_pitch_min: facts.ads_gun_kick_pitch_min,
        ads_gun_kick_pitch_max: facts.ads_gun_kick_pitch_max,
        ads_gun_kick_yaw_min: facts.ads_gun_kick_yaw_min,
        ads_gun_kick_yaw_max: facts.ads_gun_kick_yaw_max,
        ads_gun_kick_accel: facts.ads_gun_kick_accel,
        ads_gun_kick_speed_max: facts.ads_gun_kick_speed_max,
        ads_gun_kick_speed_decay: facts.ads_gun_kick_speed_decay,
        ads_gun_kick_static_decay: facts.ads_gun_kick_static_decay,
        ads_view_kick_pitch_min: facts.ads_view_kick_pitch_min,
        ads_view_kick_pitch_max: facts.ads_view_kick_pitch_max,
        ads_view_kick_yaw_min: facts.ads_view_kick_yaw_min,
        ads_view_kick_yaw_max: facts.ads_view_kick_yaw_max,
        ads_view_kick_min_magnitude: facts.ads_view_kick_min_magnitude,
        f_ads_view_kick_center_speed: facts.f_ads_view_kick_center_speed,
        gun_max_pitch: facts.gun_max_pitch,
        gun_max_yaw: facts.gun_max_yaw,
    }
}

/// bo2zm: Black Ops II dual wield, hip fire only. With both guns firing
/// a shot kicks with the two guns' hip ranges added together; with only the
/// left gun firing it kicks with the left gun's own ranges.
pub(super) fn dual_wield_kick(
    right: KickParams,
    left: &KickParams,
    right_firing: bool,
    left_firing: bool,
) -> KickParams {
    let mut k = right;
    if right_firing && left_firing {
        k.hip_view_kick_pitch_min += left.hip_view_kick_pitch_min;
        k.hip_view_kick_pitch_max += left.hip_view_kick_pitch_max;
        k.hip_view_kick_yaw_min += left.hip_view_kick_yaw_min;
        k.hip_view_kick_yaw_max += left.hip_view_kick_yaw_max;
        k.hip_gun_kick_pitch_min += left.hip_gun_kick_pitch_min;
        k.hip_gun_kick_pitch_max += left.hip_gun_kick_pitch_max;
        k.hip_gun_kick_yaw_min += left.hip_gun_kick_yaw_min;
        k.hip_gun_kick_yaw_max += left.hip_gun_kick_yaw_max;
    } else if left_firing {
        k.hip_view_kick_pitch_min = left.hip_view_kick_pitch_min;
        k.hip_view_kick_pitch_max = left.hip_view_kick_pitch_max;
        k.hip_view_kick_yaw_min = left.hip_view_kick_yaw_min;
        k.hip_view_kick_yaw_max = left.hip_view_kick_yaw_max;
        k.hip_gun_kick_pitch_min = left.hip_gun_kick_pitch_min;
        k.hip_gun_kick_pitch_max = left.hip_gun_kick_pitch_max;
        k.hip_gun_kick_yaw_min = left.hip_gun_kick_yaw_min;
        k.hip_gun_kick_yaw_max = left.hip_gun_kick_yaw_max;
    }
    k
}
