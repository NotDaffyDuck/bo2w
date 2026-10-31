//! Kick numbers read from a weapon's file.
//!
//! A weapon has two full sets of them: one used when shooting from the hip and
//! one used when looking down the sights. Whatever fires or steps a kick mixes
//! the two according to how deep in the sights the weapon is (0 is hip, 1 is
//! fully aimed).

use weapon_t6::kick::{ViewmodelKickBounds, ViewmodelKickReturn, CameraKickBounds};

/// All the kick values from one weapon file.
///
/// Angles are degrees. Where a "range" is mentioned, each shot draws a random
/// value from between its two limits.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct GunKickTuning {
    /// Pull-back speed of the view kick towards centre while aimed.
    pub scoped_view_kick_recenter_speed: f32,
    /// Pull-back speed of the view kick towards centre when shooting from the hip.
    pub hipfire_view_kick_recenter_speed: f32,
    /// Largest up/down swing of the gun; past it the gun is held.
    pub weapon_pitch_ceiling: f32,
    /// Largest sideways swing of the gun; past it the gun is held.
    pub weapon_yaw_ceiling: f32,

    /// Share of the gun kick that remains, in percent, during the reduced-kick window (aimed).
    pub scoped_weapon_kick_damped_percent: f32,
    /// Gun kick when aimed, pitch limits for each shot.
    pub scoped_weapon_kick_pitch_bottom: f32,
    pub scoped_weapon_kick_pitch_top: f32,
    /// Gun kick when aimed, yaw limits for each shot.
    pub scoped_weapon_kick_yaw_bottom: f32,
    pub scoped_weapon_kick_yaw_top: f32,
    /// Gun spring when aimed: acceleration back to centre.
    pub scoped_weapon_kick_accel: f32,
    /// Gun spring when aimed: top turning speed.
    pub scoped_weapon_kick_velocity_cap: f32,
    /// Gun spring when aimed: fraction of the speed shed per second.
    pub scoped_weapon_kick_velocity_falloff: f32,
    /// Gun spring when aimed: speed shed per second at any speed.
    pub scoped_weapon_kick_resting_falloff: f32,
    /// View kick when aimed, pitch limits for each shot.
    pub scoped_camera_kick_pitch_bottom: f32,
    pub scoped_camera_kick_pitch_top: f32,
    /// View kick when aimed, yaw limits for each shot.
    pub scoped_camera_kick_yaw_bottom: f32,
    pub scoped_camera_kick_yaw_top: f32,

    /// Share of the gun kick that remains, in percent, during the reduced-kick window (hip).
    pub hipfire_weapon_kick_damped_percent: f32,
    /// Gun kick from the hip, pitch limits for each shot.
    pub hipfire_weapon_kick_pitch_bottom: f32,
    pub hipfire_weapon_kick_pitch_top: f32,
    /// Gun kick from the hip, yaw limits for each shot.
    pub hipfire_weapon_kick_yaw_bottom: f32,
    pub hipfire_weapon_kick_yaw_top: f32,
    /// Gun spring from the hip: acceleration back to centre.
    pub hipfire_weapon_kick_accel: f32,
    /// Gun spring from the hip: top turning speed.
    pub hipfire_weapon_kick_velocity_cap: f32,
    /// Gun spring from the hip: fraction of the speed shed per second.
    pub hipfire_weapon_kick_velocity_falloff: f32,
    /// Gun spring from the hip: speed shed per second at any speed.
    pub hipfire_weapon_kick_resting_falloff: f32,
    /// View kick from the hip, pitch limits for each shot.
    pub hipfire_camera_kick_pitch_bottom: f32,
    pub hipfire_camera_kick_pitch_top: f32,
    /// View kick from the hip, yaw limits for each shot.
    pub hipfire_camera_kick_yaw_bottom: f32,
    pub hipfire_camera_kick_yaw_top: f32,

    /// bo2zm: lower bound on the view kick of a Black Ops II hip shot. A weaker
    /// roll is scaled up to it, and 0 switches the bound off.
    pub hip_view_kick_min_magnitude: f32,
    /// The same lower bound when fully aimed.
    pub ads_view_kick_min_magnitude: f32,
}

impl GunKickTuning {
    /// View kick limits for one stance, shaped as the recoil maths wants them.
    fn view_range(&self, aimed: bool) -> CameraKickBounds {
        let (pitch, yaw) = match aimed {
            true => (
                (self.scoped_camera_kick_pitch_bottom, self.scoped_camera_kick_pitch_top),
                (self.scoped_camera_kick_yaw_bottom, self.scoped_camera_kick_yaw_top),
            ),
            false => (
                (self.hipfire_camera_kick_pitch_bottom, self.hipfire_camera_kick_pitch_top),
                (self.hipfire_camera_kick_yaw_bottom, self.hipfire_camera_kick_yaw_top),
            ),
        };
        CameraKickBounds {
            pitch_lower_bound: pitch.0,
            pitch_upper_bound: pitch.1,
            turn_floor: yaw.0,
            turn_ceiling: yaw.1,
        }
    }

    /// Gun kick limits for one stance.
    fn gun_range(&self, aimed: bool) -> ViewmodelKickBounds {
        let (pitch, yaw) = match aimed {
            true => (
                (self.scoped_weapon_kick_pitch_bottom, self.scoped_weapon_kick_pitch_top),
                (self.scoped_weapon_kick_yaw_bottom, self.scoped_weapon_kick_yaw_top),
            ),
            false => (
                (self.hipfire_weapon_kick_pitch_bottom, self.hipfire_weapon_kick_pitch_top),
                (self.hipfire_weapon_kick_yaw_bottom, self.hipfire_weapon_kick_yaw_top),
            ),
        };
        ViewmodelKickBounds {
            pitch_lower_bound: pitch.0,
            pitch_upper_bound: pitch.1,
            turn_floor: yaw.0,
            turn_ceiling: yaw.1,
        }
    }

    /// The gun's return spring for one stance.
    fn spring(&self, aimed: bool) -> ViewmodelKickReturn {
        let [speed_up_rate, velocity_cap, velocity_falloff, resting_falloff] = match aimed {
            true => [
                self.scoped_weapon_kick_accel,
                self.scoped_weapon_kick_velocity_cap,
                self.scoped_weapon_kick_velocity_falloff,
                self.scoped_weapon_kick_resting_falloff,
            ],
            false => [
                self.hipfire_weapon_kick_accel,
                self.hipfire_weapon_kick_velocity_cap,
                self.hipfire_weapon_kick_velocity_falloff,
                self.hipfire_weapon_kick_resting_falloff,
            ],
        };
        ViewmodelKickReturn {
            speed_up_rate,
            velocity_cap,
            velocity_falloff,
            resting_falloff,
        }
    }

    /// Hip view kick limits.
    pub(super) fn hip_view_range(&self) -> CameraKickBounds {
        self.view_range(false)
    }

    /// Aimed view kick limits.
    pub(super) fn ads_view_range(&self) -> CameraKickBounds {
        self.view_range(true)
    }

    /// Hip gun kick limits.
    pub(super) fn hip_gun_range(&self) -> ViewmodelKickBounds {
        self.gun_range(false)
    }

    /// Aimed gun kick limits.
    pub(super) fn ads_gun_range(&self) -> ViewmodelKickBounds {
        self.gun_range(true)
    }

    /// The return spring while shooting from the hip.
    pub(super) fn hip_spring(&self) -> ViewmodelKickReturn {
        self.spring(false)
    }

    /// The return spring while aimed.
    pub(super) fn ads_spring(&self) -> ViewmodelKickReturn {
        self.spring(true)
    }
}

mod tests {
    #![cfg(test)]
    use super::*;

    fn numbered() -> GunKickTuning {
        GunKickTuning {
            scoped_weapon_kick_pitch_bottom: 1.0,
            scoped_weapon_kick_pitch_top: 2.0,
            scoped_weapon_kick_yaw_bottom: 3.0,
            scoped_weapon_kick_yaw_top: 4.0,
            scoped_weapon_kick_accel: 5.0,
            scoped_weapon_kick_velocity_cap: 6.0,
            scoped_weapon_kick_velocity_falloff: 7.0,
            scoped_weapon_kick_resting_falloff: 8.0,
            scoped_camera_kick_pitch_bottom: 9.0,
            scoped_camera_kick_pitch_top: 10.0,
            scoped_camera_kick_yaw_bottom: 11.0,
            scoped_camera_kick_yaw_top: 12.0,
            hipfire_weapon_kick_pitch_bottom: -1.0,
            hipfire_weapon_kick_pitch_top: -2.0,
            hipfire_weapon_kick_yaw_bottom: -3.0,
            hipfire_weapon_kick_yaw_top: -4.0,
            hipfire_weapon_kick_accel: -5.0,
            hipfire_weapon_kick_velocity_cap: -6.0,
            hipfire_weapon_kick_velocity_falloff: -7.0,
            hipfire_weapon_kick_resting_falloff: -8.0,
            hipfire_camera_kick_pitch_bottom: -9.0,
            hipfire_camera_kick_pitch_top: -10.0,
            hipfire_camera_kick_yaw_bottom: -11.0,
            hipfire_camera_kick_yaw_top: -12.0,
            ..GunKickTuning::default()
        }
    }

    #[test]
    fn view_ranges_take_their_own_stance_fields() {
        let params = numbered();
        let aimed = params.ads_view_range();
        assert_eq!(
            [aimed.pitch_lower_bound, aimed.pitch_upper_bound, aimed.turn_floor, aimed.turn_ceiling],
            [9.0, 10.0, 11.0, 12.0]
        );
        let hip = params.hip_view_range();
        assert_eq!(
            [hip.pitch_lower_bound, hip.pitch_upper_bound, hip.turn_floor, hip.turn_ceiling],
            [-9.0, -10.0, -11.0, -12.0]
        );
    }

    #[test]
    fn gun_ranges_take_their_own_stance_fields() {
        let params = numbered();
        let aimed = params.ads_gun_range();
        assert_eq!(
            [aimed.pitch_lower_bound, aimed.pitch_upper_bound, aimed.turn_floor, aimed.turn_ceiling],
            [1.0, 2.0, 3.0, 4.0]
        );
        let hip = params.hip_gun_range();
        assert_eq!(
            [hip.pitch_lower_bound, hip.pitch_upper_bound, hip.turn_floor, hip.turn_ceiling],
            [-1.0, -2.0, -3.0, -4.0]
        );
    }

    #[test]
    fn springs_take_their_own_stance_fields() {
        let params = numbered();
        let aimed = params.ads_spring();
        assert_eq!(
            [aimed.speed_up_rate, aimed.velocity_cap, aimed.velocity_falloff, aimed.resting_falloff],
            [5.0, 6.0, 7.0, 8.0]
        );
        let hip = params.hip_spring();
        assert_eq!(
            [hip.speed_up_rate, hip.velocity_cap, hip.velocity_falloff, hip.resting_falloff],
            [-5.0, -6.0, -7.0, -8.0]
        );
    }
}
