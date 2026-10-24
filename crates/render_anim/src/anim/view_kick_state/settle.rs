//! Letting a kick relax as time passes.

use weapon_t6::kick::{
    gun_recoil_single_angle, kick_angles, kick_angles_center_speed, lerp_gun_kick_spring,
};

use super::params::KickParams;
use super::state::ViewKickState;

/// The gun is integrated in slices no longer than this, so a long frame does
/// not make the spring overshoot.
const GUN_SLICE_MS: i32 = 5;

impl ViewKickState {
    /// Move the view and gun kick forward by `frametime_ms`.
    ///
    /// A frame of zero length (or a negative one, from a clock that stepped
    /// back) changes nothing. The view kick is stepped as a whole; the gun
    /// kick is walked in slices of up to five milliseconds, with the spring
    /// blended between its hip and aimed settings by `aim_frac`.
    pub fn advance(
        &mut self,
        kick: &KickParams,
        weapon_index: i32,
        aim_frac: f32,
        frametime_ms: i32,
    ) {
        if frametime_ms <= 0 {
            return;
        }

        let recentre = kick_angles_center_speed(
            weapon_index,
            aim_frac,
            kick.f_hip_view_kick_center_speed,
            kick.f_ads_view_kick_center_speed,
        );
        kick_angles(&mut self.kick_angles, &mut self.kick_avel, frametime_ms, recentre);

        let spring = lerp_gun_kick_spring(kick.hip_spring(), kick.ads_spring(), aim_frac);
        let [pitch_angle, yaw_angle] = &mut self.gun_angles;
        let [pitch_speed, yaw_speed] = &mut self.gun_speed;

        let mut remaining = frametime_ms;
        while remaining > 0 {
            let slice = remaining.min(GUN_SLICE_MS);
            let seconds = slice as f32 * 0.001;
            gun_recoil_single_angle(pitch_angle, pitch_speed, seconds, kick.gun_max_pitch, spring);
            gun_recoil_single_angle(yaw_angle, yaw_speed, seconds, kick.gun_max_yaw, spring);
            remaining -= slice;
        }
    }
}
