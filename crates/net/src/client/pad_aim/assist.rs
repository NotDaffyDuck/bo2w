//! The three aids: slowing the turn over a target, holding the sights on one, and pulling them
//! toward one when the sights go up.

use super::view_math::{wrapped_turn, angles_to, blend};
use super::screen::{OnScreen, best};
use super::AimView;
use crate::client::local_input::PlayerControlSnapshot;

/// How much of the turn is kept while a target is under the sights, hip and sights up.
const SLOWDOWN_HIP: f32 = 0.4;
const SLOWDOWN_ADS: f32 = 0.5;
/// Screen regions, in 640x480 pixels around the crosshair, where each aid takes hold.
const SLOWDOWN_REGION: [f32; 2] = [90.0, 90.0];
const LOCK_ON_REGION: [f32; 2] = [90.0, 90.0];
const PULL_REGION: [f32; 2] = [160.0, 120.0];
/// The share of a locked target's apparent movement the sights follow.
const LOCK_ON_STRENGTH: f32 = 0.6;
/// The stick must be pushed past this for lock-on to follow the target.
const LOCK_ON_DEFLECTION: f32 = 0.05;
/// The pull's speed in degrees a second, and how long it lasts in seconds.
const PULL_SPEED: f32 = 40.0;
const PULL_TIME: f32 = 0.5;
/// The range used where a weapon gives none.
const DEFAULT_RANGE: f32 = 1500.0;
/// The pull ends within this many degrees of the target.
const PULL_DONE: f32 = 0.25;

/// How far the aids reach, between the weapon's hip and sights-up ranges.
pub(super) fn assist_range(view: &AimView, ads_lerp: f32) -> f32 {
    let hip = if view.ranges.hip > 0.0 {
        view.ranges.hip
    } else {
        DEFAULT_RANGE
    };
    let ads = if view.ranges.ads > 0.0 {
        view.ranges.ads
    } else {
        hip
    };
    blend(hip, ads, ads_lerp)
}

/// Turn slower while a target is under the sights.
pub(super) fn slow_over_target(
    seen: &[OnScreen],
    range: f32,
    ads_lerp: f32,
    delta: &mut [f32; 2],
) {
    if best(seen, range, SLOWDOWN_REGION, 1.0).is_some() {
        let keep = blend(SLOWDOWN_HIP, SLOWDOWN_ADS, ads_lerp);
        delta[0] *= keep;
        delta[1] *= keep;
    }
}

/// Keep the target locked on last frame while it stays in the region, else lock on the best one.
/// With the stick pushed (and no pull under way), follow the locked target's movement.
pub(super) fn lock_on(
    input: &mut PlayerControlSnapshot,
    view: &AimView,
    seen: &[OnScreen],
    range: f32,
    delta: &mut [f32; 2],
) {
    let kept = input
        .pad_lockon
        .and_then(|key| seen.iter().find(|s| s.target.key == key))
        .filter(|s| s.reaches(range, LOCK_ON_REGION, 1.0));
    let locked = kept.or_else(|| best(seen, range, LOCK_ON_REGION, 1.0));
    input.pad_lockon = locked.map(|s| s.target.key);

    let Some(locked) = locked else { return };
    if input.pad_autoaim.is_some() || input.pad_deflection <= LOCK_ON_DEFLECTION {
        return;
    }
    let target = locked.target;
    let after: [f32; 3] =
        std::array::from_fn(|i| target.aim[i] + (target.drift[i] - view.drift[i]) * view.dt);
    let now = angles_to(view.eye, target.aim);
    let next = angles_to(view.eye, after);
    delta[0] += LOCK_ON_STRENGTH * wrapped_turn(next[0], now[0]);
    delta[1] += LOCK_ON_STRENGTH * wrapped_turn(next[1], now[1]);
}

/// As the sights go up, choose the target to pull toward (if one is in range and near the
/// crosshair). `zoom` widens the region as the field of view narrows.
pub(super) fn pick_pull_target(
    input: &mut PlayerControlSnapshot,
    view: &AimView,
    seen: &[OnScreen],
    zoom: f32,
) {
    let range = if view.ranges.auto_aim > 0.0 {
        view.ranges.auto_aim
    } else {
        DEFAULT_RANGE
    };
    input.pad_autoaim = best(seen, range, PULL_REGION, (1.0 / zoom).max(1.0))
        .map(|s| (s.target.key, PULL_TIME));
}

/// Pull the view toward the chosen target for as long as it lasts, is still on screen and the view
/// is not yet on it.
pub(super) fn pull_toward_target(
    input: &mut PlayerControlSnapshot,
    view: &AimView,
    seen: &[OnScreen],
    delta: &mut [f32; 2],
) {
    let Some((lock_key, remaining)) = input.pad_autoaim else {
        return;
    };
    let target = seen.iter().find(|s| s.target.key == lock_key).filter(|_| remaining > 0.0);
    let Some(target) = target else {
        input.pad_autoaim = None;
        return;
    };
    let goal = angles_to(view.eye, target.target.aim);
    let pitch = wrapped_turn(goal[0], view.pitch_yaw[0]);
    let yaw = wrapped_turn(goal[1], view.pitch_yaw[1]);
    let length = pitch.hypot(yaw);
    let step = (PULL_SPEED * view.dt).min(length);
    if length > 1e-4 {
        delta[0] += pitch / length * step;
        delta[1] += yaw / length * step;
    }
    input.pad_autoaim = (length > PULL_DONE).then_some((lock_key, remaining - view.dt));
}

#[cfg(test)]
mod tests {
    use weapon_t6::combat_facts::AimAssistRanges;

    use super::super::{AimTarget, screen::project};
    use super::*;

    fn view() -> AimView {
        AimView {
            eye: [0.0; 3],
            pitch_yaw: [0.0; 2],
            drift: [0.0; 3],
            ads_lerp: 0.0,
            view_angle_multiplier: 1.0,
            ranges: AimAssistRanges {
                auto_aim: 0.0,
                hip: 0.0,
                ads: 0.0,
            },
            dt: 0.01,
        }
    }

    fn target_at(key: u64, y: f32) -> AimTarget {
        AimTarget {
            key,
            box_lo: [500.0, y - 20.0, -20.0],
            box_hi: [520.0, y + 20.0, 20.0],
            aim: [510.0, y, 0.0],
            drift: [0.0; 3],
        }
    }

    #[test]
    fn a_range_of_zero_means_the_default_and_the_sights_range_falls_back_to_the_hip() {
        let mut v = view();
        assert_eq!(assist_range(&v, 0.0), DEFAULT_RANGE);
        assert_eq!(assist_range(&v, 1.0), DEFAULT_RANGE);
        v.ranges.hip = 800.0;
        assert_eq!(assist_range(&v, 1.0), 800.0);
        v.ranges.ads = 1200.0;
        assert_eq!(assist_range(&v, 0.0), 800.0);
        assert_eq!(assist_range(&v, 1.0), 1200.0);
        assert_eq!(assist_range(&v, 0.5), 1000.0);
    }

    #[test]
    fn slowdown_keeps_two_fifths_at_the_hip_and_half_with_the_sights_up() {
        let seen = project(&view(), &[target_at(1, 0.0)]);
        let mut hip = [10.0, 10.0];
        slow_over_target(&seen, 1000.0, 0.0, &mut hip);
        assert!((hip[0] - 4.0).abs() < 1e-4 && (hip[1] - 4.0).abs() < 1e-4);
        let mut sights = [10.0, 10.0];
        slow_over_target(&seen, 1000.0, 1.0, &mut sights);
        assert!((sights[0] - 5.0).abs() < 1e-4 && (sights[1] - 5.0).abs() < 1e-4);
        let mut nothing = [10.0, 10.0];
        slow_over_target(&[], 1000.0, 0.0, &mut nothing);
        assert_eq!(nothing, [10.0, 10.0]);
    }

    #[test]
    fn lock_on_keeps_its_target_until_it_leaves_the_region() {
        let v = view();
        let mut input = PlayerControlSnapshot::default();
        input.pad_lockon = Some(2);
        // Both targets are in the region; the one already locked on stays locked on.
        let seen = project(&v, &[target_at(1, 0.0), target_at(2, 40.0)]);
        let mut delta = [0.0; 2];
        lock_on(&mut input, &v, &seen, 1000.0, &mut delta);
        assert_eq!(input.pad_lockon, Some(2));
        // Gone from the screen: the best remaining one is locked on.
        let seen = project(&v, &[target_at(1, 0.0)]);
        lock_on(&mut input, &v, &seen, 1000.0, &mut delta);
        assert_eq!(input.pad_lockon, Some(1));
        lock_on(&mut input, &v, &[], 1000.0, &mut delta);
        assert_eq!(input.pad_lockon, None);
    }

    #[test]
    fn a_locked_target_that_walks_away_drags_the_sights_after_it_with_the_stick_pushed() {
        let v = view();
        let mut moving = target_at(1, 0.0);
        moving.drift = [0.0, 100.0, 0.0];
        let seen = project(&v, &[moving]);

        let mut input = PlayerControlSnapshot::default();
        let mut still_stick = [0.0; 2];
        lock_on(&mut input, &v, &seen, 1000.0, &mut still_stick);
        assert_eq!(still_stick, [0.0; 2]);

        input.pad_deflection = 1.0;
        let mut delta = [0.0; 2];
        lock_on(&mut input, &v, &seen, 1000.0, &mut delta);
        // One tick of 100 units a second at 510 units: atan(1 / 510) degrees, times the strength.
        let expected = LOCK_ON_STRENGTH * (1.0f32 / 510.0).atan().to_degrees();
        assert!((delta[1] - expected).abs() < 1e-4, "{} vs {expected}", delta[1]);
        assert!(delta[0].abs() < 1e-4);

        // No follow while a pull is under way.
        input.pad_autoaim = Some((1, 0.4));
        let mut during_pull = [0.0; 2];
        lock_on(&mut input, &v, &seen, 1000.0, &mut during_pull);
        assert_eq!(during_pull, [0.0; 2]);
    }

    #[test]
    fn the_pull_picks_the_nearest_the_crosshair_within_its_range() {
        let v = view();
        let mut input = PlayerControlSnapshot::default();
        let seen = project(&v, &[target_at(1, 60.0), target_at(2, 20.0)]);
        pick_pull_target(&mut input, &v, &seen, 1.0);
        assert_eq!(input.pad_autoaim, Some((2, PULL_TIME)));
        let mut near = view();
        near.ranges.auto_aim = 100.0;
        pick_pull_target(&mut input, &near, &seen, 1.0);
        assert_eq!(input.pad_autoaim, None);
    }

    #[test]
    fn the_pull_steps_toward_the_target_and_runs_down_its_time() {
        let v = view();
        let seen = project(&v, &[target_at(1, 60.0)]);
        let mut input = PlayerControlSnapshot::default();
        input.pad_autoaim = Some((1, PULL_TIME));
        let mut delta = [0.0; 2];
        pull_toward_target(&mut input, &v, &seen, &mut delta);
        // Forty degrees a second over a hundredth of a second, all of it in yaw.
        assert!(delta[0].abs() < 1e-5);
        assert!((delta[1] - 0.4).abs() < 1e-5);
        let (key, left) = input.pad_autoaim.unwrap();
        assert_eq!(key, 1);
        assert!((left - 0.49).abs() < 1e-6);
    }

    #[test]
    fn the_pull_ends_on_arrival_when_time_runs_out_or_the_target_is_gone() {
        let v = view();
        let on_target = project(&v, &[target_at(1, 0.0)]);
        let mut input = PlayerControlSnapshot::default();
        input.pad_autoaim = Some((1, 0.3));
        let mut delta = [0.0; 2];
        pull_toward_target(&mut input, &v, &on_target, &mut delta);
        assert_eq!(input.pad_autoaim, None);
        assert_eq!(delta, [0.0; 2]);

        let off_axis = project(&v, &[target_at(1, 60.0)]);
        input.pad_autoaim = Some((1, 0.0));
        pull_toward_target(&mut input, &v, &off_axis, &mut delta);
        assert_eq!(input.pad_autoaim, None);

        input.pad_autoaim = Some((9, 0.4));
        pull_toward_target(&mut input, &v, &off_axis, &mut delta);
        assert_eq!(input.pad_autoaim, None);
        assert_eq!(delta, [0.0; 2]);
    }
}
