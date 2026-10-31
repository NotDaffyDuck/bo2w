//! Letting a kick relax as time passes.

use weapon_t6::kick::{
    viewmodel_kick_one_axis, recoil_tilt, recoil_tilt_return_rate, blend_gun_kick_spring,
};

use super::params::GunKickTuning;
use super::state::ViewRecoilState;

/// The gun is integrated in slices no longer than this, so a long frame does
/// not make the spring overshoot.
const GUN_SLICE_MS: i32 = 5;

impl ViewRecoilState {
    /// Move the view and gun kick forward by `step_duration_ms`.
    ///
    /// A frame of zero length (or a negative one, from a clock that stepped
    /// back) changes nothing. The view kick is stepped as a whole; the gun
    /// kick is walked in slices of up to five milliseconds, with the spring
    /// blended between its hip and aimed settings by `aim_frac`.
    pub fn step_on(
        &mut self,
        kick: &GunKickTuning,
        held_weapon_no: i32,
        aim_frac: f32,
        step_duration_ms: i32,
    ) {
        if step_duration_ms < 1 {
            return;
        }

        let (hip_speed, ads_speed) = (
            kick.hipfire_view_kick_recenter_speed,
            kick.scoped_view_kick_recenter_speed,
        );
        let center = recoil_tilt_return_rate(held_weapon_no, aim_frac, hip_speed, ads_speed);
        recoil_tilt(
            &mut self.recoil_tilt,
            &mut self.punch_spin_rate,
            step_duration_ms,
            center,
        );

        let spring = blend_gun_kick_spring(kick.hip_spring(), kick.ads_spring(), aim_frac);
        let [pitch, yaw] = [(kick.weapon_pitch_ceiling, 0), (kick.weapon_yaw_ceiling, 1)];
        let slices = std::iter::successors(Some(step_duration_ms), |left| Some(left - GUN_SLICE_MS))
            .take_while(|left| *left > 0)
            .map(|left| left.min(GUN_SLICE_MS));
        for slice in slices {
            let seconds = slice as f32 * 0.001;
            for (limit, axis) in [pitch, yaw] {
                let (angle, speed) = (&mut self.weapon_aim_angles[axis], &mut self.gun_swing_rate[axis]);
                viewmodel_kick_one_axis(angle, speed, seconds, limit, spring);
            }
        }
    }
}
