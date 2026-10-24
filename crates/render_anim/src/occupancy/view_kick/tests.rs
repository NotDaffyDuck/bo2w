//! Tests for the view-kick pieces that work on plain numbers.

use super::breath::HoldBreath;
use super::camera::linked_weapon_view_fov;
use super::feedback::{hit_aim_pitch, iron_gun_turn};
use super::params::dual_wield_kick;
use super::tick::shock_sway_scale;
use crate::anim::view_kick_state::KickParams;

fn near(a: f32, b: f32, within: f32) -> bool {
    (a - b).abs() < within
}

#[test]
fn shock_swing_starts_at_the_guns_scale_and_settles_to_one() {
    let at_start = shock_sway_scale(4000, 4000, 2.0);
    let midway = shock_sway_scale(2000, 4000, 2.0);
    let over = shock_sway_scale(0, 4000, 2.0);
    assert_eq!(at_start, 2.0);
    assert!(midway > 1.0 && midway < 2.0, "midway {midway}");
    assert_eq!(over, 1.0);
    // A gun with no shock scale never changes its sway.
    assert_eq!(shock_sway_scale(4000, 4000, 0.0), 1.0);
}

fn test_gun(base: f32) -> KickParams {
    KickParams {
        hip_view_kick_pitch_min: base,
        hip_view_kick_pitch_max: base * 2.0,
        hip_gun_kick_yaw_max: base * 3.0,
        ads_view_kick_pitch_max: 9.0,
        ..KickParams::default()
    }
}

#[test]
fn two_firing_guns_sum_their_hip_ranges_and_leave_the_aimed_ones() {
    let summed = dual_wield_kick(test_gun(1.0), &test_gun(10.0), true, true);
    assert_eq!(summed.hip_view_kick_pitch_min, 11.0);
    assert_eq!(summed.hip_view_kick_pitch_max, 22.0);
    assert_eq!(summed.hip_gun_kick_yaw_max, 33.0);
    assert_eq!(summed.ads_view_kick_pitch_max, 9.0);
}

#[test]
fn a_lone_left_gun_kicks_by_its_own_ranges_and_a_lone_right_gun_is_unchanged() {
    let left = dual_wield_kick(test_gun(1.0), &test_gun(10.0), false, true);
    assert_eq!(left.hip_view_kick_pitch_max, 20.0);
    let right = dual_wield_kick(test_gun(1.0), &test_gun(10.0), true, false);
    assert_eq!(right, test_gun(1.0));
}

#[test]
fn the_gun_turn_is_what_is_left_after_the_view_is_taken_out() {
    let level = iron_gun_turn([0.0, 1.5, 0.0], [0.0, 30.0, 0.0]);
    assert!(near(level[0], 0.0, 1e-3) && near(level[1], 1.5, 1e-3), "{level:?}");
    let wrapped = iron_gun_turn([-0.8, 0.0, 0.0], [0.0, 350.0, 0.0]);
    assert!(near(wrapped[0], -0.8, 1e-3) && near(wrapped[1], 0.0, 1e-3), "{wrapped:?}");
}

#[test]
fn a_gun_at_rest_turns_the_shot_by_nothing_in_any_view() {
    for view in [[0.0, 0.0, 0.0], [-40.0, 179.0, 0.0], [60.0, -90.0, 3.0]] {
        let turn = iron_gun_turn([0.0; 3], view);
        assert!(near(turn[0], 0.0, 1e-3) && near(turn[1], 0.0, 1e-3), "{view:?} {turn:?}");
    }
}

#[test]
fn in_a_rolled_view_a_pitched_gun_turns_the_shot_sideways() {
    let turn = iron_gun_turn([2.0, 0.0, 0.0], [0.0, 0.0, 90.0]);
    assert!(near(turn[0], 0.0, 0.05) && near(turn[1].abs(), 2.0, 0.05), "{turn:?}");
}

/// The hit pitch, `ms` after a hit at time 1000 that tilts 5 down and rolls 2.
fn hit_pitch_after(ms: i32, frac: f32, overlay: i32, iron: bool) -> f32 {
    hit_aim_pitch(1000 + ms, 1000, 5.0, 2.0, frac, overlay, iron)
}

#[test]
fn from_the_hip_only_the_camera_flinch_reaches_the_shot() {
    assert!(near(hit_pitch_after(100, 0.0, 0, true), 5.0, 1e-4));
    // No hit recorded (time 0): nothing.
    assert_eq!(hit_aim_pitch(1100, 0, 5.0, 2.0, 0.0, 0, true), 0.0);
}

#[test]
fn aimed_through_iron_sights_the_gun_tilt_is_added_but_a_scope_adds_none() {
    // Camera part at half strength (2.5) plus the gun's tilt at half (2.5).
    assert!(near(hit_pitch_after(100, 1.0, 0, true), 5.0, 1e-4));
    assert!(near(hit_pitch_after(100, 1.0, 0, false), 2.5, 1e-4));
    // A scope: the camera part only, at its scope factor.
    assert!(near(hit_pitch_after(100, 1.0, 1, true), 3.75, 1e-4));
}

#[test]
fn a_flinch_is_out_of_the_shot_by_half_a_second() {
    for frac in [0.0, 0.5, 1.0] {
        assert!(hit_pitch_after(50, frac, 0, true) > 0.0, "frac {frac}");
        assert_eq!(hit_pitch_after(500, frac, 0, true), 0.0, "frac {frac}");
        assert_eq!(hit_pitch_after(700, frac, 0, true), 0.0, "frac {frac}");
    }
}

#[test]
fn a_view_linked_to_a_gun_snaps_to_the_guns_zoom_once_he_aims() {
    let not_linked = [(0, 233, 1.0, 10.0), (4, 0, 1.0, 10.0)];
    for (flags, gun, frac, zoom) in not_linked {
        assert_eq!(linked_weapon_view_fov(flags, gun, frac, zoom), None);
    }
    assert_eq!(linked_weapon_view_fov(4, 233, 0.0, 10.0), Some(45.0));
    assert_eq!(linked_weapon_view_fov(4 | 1, 233, 0.01, 10.0), Some(10.0));
    assert_eq!(linked_weapon_view_fov(4, 233, 1.0, 10.0), Some(10.0));
    // A gun with no zoom of its own stays at 45.
    assert_eq!(linked_weapon_view_fov(4, 233, 1.0, 0.0), Some(45.0));
}

/// Runs the breath for `ms` in 8 ms frames and returns the last scale.
fn breathe(breath: &mut HoldBreath, button_down: bool, ms: i32) -> f32 {
    let mut scale = 1.0;
    for _ in 0..ms / 8 {
        scale = breath.advance(true, 1.0, button_down, 8.0);
    }
    scale
}

#[test]
fn holding_the_breath_steadies_the_drift_then_gasps_and_blocks_a_second_hold() {
    let mut breath = HoldBreath::default();
    assert!(breathe(&mut breath, true, 3000) < 0.1);
    // Past the hold time the hold ends and the drift gasps above normal.
    breathe(&mut breath, true, 2000);
    assert!(!breath.holding);
    assert!(breath.scale > 1.5);
    // Still pressed with the timer not back at zero: no new hold.
    breathe(&mut breath, true, 200);
    assert!(!breath.holding);
    // Let go for a whole gasp plus hold time: normal again, and a hold can start.
    breathe(&mut breath, false, 6000);
    assert!(near(breath.scale, 1.0, 0.01));
    breathe(&mut breath, true, 100);
    assert!(breath.holding);
}

#[test]
fn firing_ends_a_hold() {
    let mut breath = HoldBreath::default();
    breathe(&mut breath, true, 1000);
    assert!(breath.holding);
    breath.fired(1.0, true);
    assert!(!breath.holding);
}
