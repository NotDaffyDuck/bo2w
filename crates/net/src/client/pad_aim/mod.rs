//! Turning the pad's right stick into a look: the stick's turn rate, and the three kinds of
//! aim assist (slowdown over a target, lock-on, and the pull toward a target on raising the
//! sights).

mod assist;
mod view_math;
mod screen;
mod turn;

use crate::client::local_input::PlayerControlSnapshot;

/// Something the sights can be drawn to: a box and the point to aim at, in world units.
#[derive(Clone, Copy, Debug)]
pub struct AimTarget {
    pub key: u64,
    pub box_lo: [f32; 3],
    pub box_hi: [f32; 3],
    pub aim: [f32; 3],
    pub drift: [f32; 3],
}

/// The player's view for one frame.
#[derive(Clone, Copy, Debug)]
pub struct AimView {
    pub eye: [f32; 3],
    /// Pitch and yaw, degrees.
    pub pitch_yaw: [f32; 2],
    pub drift: [f32; 3],
    pub ads_lerp: f32,
    /// Against a 65 degree field of view.
    pub view_angle_multiplier: f32,
    pub ranges: weapon_t6::combat_facts::AimAssistRanges,
    pub dt: f32,
}

/// Aim assist off, on for slowdown and lock-on, and on with the pull toward a target.
const ASSIST_OFF: u8 = 0;
const ASSIST_WITH_PULL: u8 = 2;

/// Work out this frame's look from the stick: write the angle change (pitch, yaw, degrees) to
/// `input.pad_look_delta` and keep the lock-on and pull state in `input` for the next frame.
pub fn pad_look_frame(
    input: &mut PlayerControlSnapshot,
    view: &AimView,
    targets: &[AimTarget],
    ads: bool,
) {
    let ads_lerp = view.ads_lerp.clamp(0.0, 1.0);
    let zoom = view.view_angle_multiplier.clamp(0.02, 2.0);
    let mut delta = turn::stick_turn(input, view.dt, ads_lerp, zoom);

    if input.pad_aim_assist == ASSIST_OFF {
        input.pad_lockon = None;
        input.pad_autoaim = None;
    } else {
        let seen = screen::project(view, targets);
        let range = assist::assist_range(view, ads_lerp);
        assist::slow_over_target(&seen, range, ads_lerp, &mut delta);
        assist::lock_on(input, view, &seen, range, &mut delta);
        if input.pad_aim_assist == ASSIST_WITH_PULL && ads && !input.pad_was_ads {
            assist::pick_pull_target(input, view, &seen, zoom);
        }
        if !ads {
            input.pad_autoaim = None;
        }
        assist::pull_toward_target(input, view, &seen, &mut delta);
    }
    input.pad_was_ads = ads;
    input.pad_look_delta = delta;
}

#[cfg(test)]
mod tests {
    use weapon_t6::combat_facts::AimAssistRanges;

    use super::*;

    const AHEAD: AimTarget = AimTarget {
        key: 42,
        box_lo: [500.0, -20.0, -20.0],
        box_hi: [520.0, 20.0, 20.0],
        aim: [510.0, 0.0, 0.0],
        drift: [0.0; 3],
    };

    fn view() -> AimView {
        AimView {
            eye: [0.0; 3],
            pitch_yaw: [0.0; 2],
            drift: [0.0; 3],
            ads_lerp: 0.0,
            view_angle_multiplier: 1.0,
            ranges: AimAssistRanges {
                auto_aim: 1000.0,
                hip: 1000.0,
                ads: 1000.0,
            },
            dt: 0.01,
        }
    }

    fn stick_right() -> PlayerControlSnapshot {
        PlayerControlSnapshot {
            pad_look: [1.0, 0.0],
            ..PlayerControlSnapshot::default()
        }
    }

    #[test]
    fn with_assist_off_the_look_is_the_stick_alone_and_old_locks_are_dropped() {
        let mut input = stick_right();
        input.pad_lockon = Some(1);
        input.pad_autoaim = Some((1, 0.4));
        pad_look_frame(&mut input, &view(), &[AHEAD], true);
        assert_eq!(input.pad_lockon, None);
        assert_eq!(input.pad_autoaim, None);
        assert!(input.pad_was_ads);
        // The first frame of a full push turns at one acceleration step: 12 degrees a second.
        assert!((input.pad_look_delta[1] - -0.12).abs() < 1e-5);
        assert_eq!(input.pad_look_delta[0], 0.0);
    }

    #[test]
    fn a_target_under_the_sights_slows_the_turn_and_is_locked_onto() {
        let mut input = stick_right();
        input.pad_aim_assist = 1;
        pad_look_frame(&mut input, &view(), &[AHEAD], false);
        assert_eq!(input.pad_lockon, Some(42));
        assert!((input.pad_look_delta[1] - -0.12 * 0.4).abs() < 1e-5);
    }

    #[test]
    fn a_target_out_of_range_or_behind_changes_nothing() {
        let mut input = stick_right();
        input.pad_aim_assist = 1;
        let mut behind = AHEAD;
        behind.box_lo = [-520.0, -20.0, -20.0];
        behind.box_hi = [-500.0, 20.0, 20.0];
        behind.aim = [-510.0, 0.0, 0.0];
        let mut short = view();
        short.ranges.hip = 100.0;
        pad_look_frame(&mut input, &view(), &[behind], false);
        assert_eq!(input.pad_lockon, None);
        pad_look_frame(&mut input, &short, &[AHEAD], false);
        assert_eq!(input.pad_lockon, None);
    }

    #[test]
    fn raising_the_sights_with_the_pull_setting_picks_a_target_once() {
        // Off to one side so the pull has some way to go.
        let mut aside = AHEAD;
        aside.box_lo = [500.0, 40.0, -20.0];
        aside.box_hi = [520.0, 80.0, 20.0];
        aside.aim = [510.0, 60.0, 0.0];
        let mut input = stick_right();
        input.pad_aim_assist = ASSIST_WITH_PULL;
        pad_look_frame(&mut input, &view(), &[aside], true);
        let (key, left) = input.pad_autoaim.expect("a pull is under way");
        assert_eq!(key, 42);
        assert!((left - 0.49).abs() < 1e-5);
        // Already aiming down the sights: no new pick, and lowering them drops the pull.
        input.pad_autoaim = None;
        pad_look_frame(&mut input, &view(), &[aside], true);
        assert_eq!(input.pad_autoaim, None);
        input.pad_autoaim = Some((42, 0.3));
        pad_look_frame(&mut input, &view(), &[aside], false);
        assert_eq!(input.pad_autoaim, None);
    }
}
