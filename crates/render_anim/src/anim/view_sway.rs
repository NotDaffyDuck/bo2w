//! The gun's turn sway on the player's view: the spring state that
//! `weapon_t6::sway` moves each frame, plus the view angles of the frame before.

use weapon_t6::sway::{
    SwaySpringState, WeaponSwayParams, calculate_weapon_movement_sway, lerp_sway_params,
};

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ViewSwayState {
    springs: SwaySpringState,
    /// The view angles the last frame ended on; `None` before the first frame.
    prev_view_angles: Option<[f32; 3]>,
}

impl ViewSwayState {
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    pub fn springs(&self) -> SwaySpringState {
        self.springs
    }

    /// Black Ops II keeps following the view while the scope hides the gun, so the
    /// frame the scope comes off does not see a turn.
    pub fn follow_view(&mut self, view_angles: [f32; 3]) {
        self.prev_view_angles = Some(view_angles);
    }

    /// One frame of sway. Nothing moves for a non-positive or non-finite `dt_secs`,
    /// or while an overlay (the scope) covers the gun. The first frame only records
    /// the view angles. Aiming blends the hip and aimed parameters by `weapon_pos_frac`.
    #[allow(clippy::too_many_arguments)]
    pub fn advance(
        &mut self,
        hip: WeaponSwayParams,
        ads: WeaponSwayParams,
        view_angles: [f32; 3],
        weapon_pos_frac: f32,
        aim_down_sight: bool,
        overlay_active: bool,
        landing_scale: f32,
        dt_secs: f32,
    ) {
        let usable_dt = dt_secs.is_finite() && dt_secs > 0.0;
        let covered = overlay_active && weapon_pos_frac > 0.0;
        if !usable_dt || covered {
            return;
        }
        let Some(prev) = self.prev_view_angles.replace(view_angles) else {
            return;
        };
        let params = if aim_down_sight {
            lerp_sway_params(hip, ads, weapon_pos_frac)
        } else {
            hip
        };
        calculate_weapon_movement_sway(
            &mut self.springs,
            view_angles,
            prev,
            params,
            landing_scale,
            dt_secs,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PARAMS: WeaponSwayParams = WeaponSwayParams {
        max_angle: 4.0,
        lerp_speed: 6.0,
        pitch_scale: 0.1,
        yaw_scale: 0.1,
        horiz_scale: 0.2,
        vert_scale: 0.2,
    };
    const FRAME: f32 = 1.0 / 60.0;

    fn step(sway: &mut ViewSwayState, yaw: f32, frac: f32, overlay: bool) {
        sway.advance(PARAMS, PARAMS, [0.0, yaw, 0.0], frac, true, overlay, 1.0, FRAME);
    }

    /// Turns 90 degrees behind the scope, then lets go of the aim.
    fn yaw_spring_after_scope(follow_the_view: bool) -> f32 {
        let mut sway = ViewSwayState::default();
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
        let mut sway = ViewSwayState::default();
        step(&mut sway, 40.0, 0.0, false);
        assert_eq!(sway.springs(), SwaySpringState::default());
        step(&mut sway, 50.0, 0.0, false);
        assert!(sway.springs().yaw != 0.0, "a turn on the second frame moves the gun");
    }

    #[test]
    fn a_bad_frame_time_changes_nothing() {
        let mut sway = ViewSwayState::default();
        step(&mut sway, 0.0, 0.0, false);
        let before = sway.clone();
        for dt in [0.0, -1.0, f32::NAN, f32::INFINITY] {
            sway.advance(PARAMS, PARAMS, [0.0, 30.0, 0.0], 0.0, false, false, 1.0, dt);
            assert_eq!(sway, before);
        }
    }

    #[test]
    fn reset_forgets_the_springs_and_the_previous_view() {
        let mut sway = ViewSwayState::default();
        step(&mut sway, 0.0, 0.0, false);
        step(&mut sway, 20.0, 0.0, false);
        sway.reset();
        assert_eq!(sway, ViewSwayState::default());
    }
}
