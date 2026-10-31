//! The gun's turn sway on the player's view: the spring state that
//! `weapon_t6::sway` moves each frame, plus the view angles of the frame before.

use weapon_t6::sway::{
    DriftReturnState, GunDriftTuning, calculate_weapon_movement_sway, blend_drift_tuning,
};

#[derive(Clone, Debug, Default, PartialEq)]
pub struct SwaySpring {
    spring_set: DriftReturnState,
    /// The view angles the last frame ended on; `None` before the first frame.
    earlier_look_angles: Option<[f32; 3]>,
}

impl SwaySpring {
    pub fn settle_zero(&mut self) {
        *self = Self::default();
    }

    pub fn springs(&self) -> DriftReturnState {
        self.spring_set
    }

    /// Black Ops II keeps following the view while the scope hides the gun, so the
    /// frame the scope comes off does not see a turn.
    pub fn follow_view(&mut self, look_rotation: [f32; 3]) {
        self.earlier_look_angles = Some(look_rotation);
    }

    /// One frame of sway. Nothing moves for a non-positive or non-finite `dt_secs`,
    /// or while an overlay (the scope) covers the gun. The first frame only records
    /// the view angles. Aiming blends the hip and aimed parameters by `gun_raise_share`.
    #[allow(clippy::too_many_arguments, reason = "kept as written")]
    pub fn progress(
        &mut self,
        hip: GunDriftTuning,
        ads: GunDriftTuning,
        look_rotation: [f32; 3],
        gun_raise_share: f32,
        scoped_aiming: bool,
        overlay_active_in: bool,
        landing_factor: f32,
        delta_seconds: f32,
    ) {
        // A scope drawn over the gun hides it, so it does not sway either.
        if gun_raise_share > 0.0 && overlay_active_in {
            return;
        }
        if !(delta_seconds > 0.0 && delta_seconds.is_finite()) {
            return;
        }
        let last_frame_view = self.earlier_look_angles.replace(look_rotation);
        let Some(turned_from) = last_frame_view else {
            return;
        };
        let params = match scoped_aiming {
            true => blend_drift_tuning(hip, ads, gun_raise_share),
            false => hip,
        };
        let springs = &mut self.spring_set;
        calculate_weapon_movement_sway(
            springs,
            look_rotation,
            turned_from,
            params,
            landing_factor,
            delta_seconds,
        );
    }
}

mod tests {
    #![cfg(test)]
    use super::*;

    const PARAMS: GunDriftTuning = GunDriftTuning {
        widest_cone: 4.0,
        ease_rate: 6.0,
        tilt_multiplier: 0.1,
        turn_factor: 0.1,
        sideways_factor: 0.2,
        upward_factor: 0.2,
    };
    const FRAME: f32 = 1.0 / 60.0;

    fn step(sway: &mut SwaySpring, yaw: f32, frac: f32, overlay: bool) {
        sway.progress(
            PARAMS,
            PARAMS,
            [0.0, yaw, 0.0],
            frac,
            true,
            overlay,
            1.0,
            FRAME,
        );
    }

    /// Turns 90 degrees behind the scope, then lets go of the aim.
    fn yaw_spring_after_scope(follow_the_view: bool) -> f32 {
        let mut sway = SwaySpring::default();
        step(&mut sway, 0.0, 0.0, true);
        for frame in 1..=30 {
            let yaw = frame as f32 * 3.0;
            if follow_the_view {
                sway.follow_view([0.0, yaw, 0.0]);
            } else {
                step(&mut sway, yaw, 1.0, true);
            }
        }
        step(&mut sway, 90.0, 0.0, true);
        sway.springs().yaw.abs()
    }

    #[test]
    fn coming_off_the_scope_does_not_swing_the_gun_when_the_view_was_followed() {
        assert_eq!(yaw_spring_after_scope(true), 0.0);
        assert!(yaw_spring_after_scope(false) > 0.03);
    }

    #[test]
    fn the_first_frame_only_records_the_view() {
        let mut sway = SwaySpring::default();
        step(&mut sway, 40.0, 0.0, false);
        assert_eq!(sway.springs(), DriftReturnState::default());
        step(&mut sway, 50.0, 0.0, false);
        assert!(
            sway.springs().yaw != 0.0,
            "a turn on the second frame moves the gun"
        );
    }

    #[test]
    fn a_bad_frame_time_changes_nothing() {
        let mut sway = SwaySpring::default();
        step(&mut sway, 0.0, 0.0, false);
        let before = sway.clone();
        for dt in [0.0, -1.0, f32::NAN, f32::INFINITY] {
            sway.progress(PARAMS, PARAMS, [0.0, 30.0, 0.0], 0.0, false, false, 1.0, dt);
            assert_eq!(sway, before);
        }
    }

    #[test]
    fn reset_forgets_the_springs_and_the_previous_view() {
        let mut sway = SwaySpring::default();
        step(&mut sway, 0.0, 0.0, false);
        step(&mut sway, 20.0, 0.0, false);
        sway.settle_zero();
        assert_eq!(sway, SwaySpring::default());
    }
}
