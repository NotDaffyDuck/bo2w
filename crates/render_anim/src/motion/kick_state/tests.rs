#![cfg(test)]
//! Behaviour pins for the view/gun kick state. The expected numbers were taken
//! from the implementation before it was rewritten, so a rewrite must land on
//! the same values (same random stream, same step sizes, same scaling).

use weapon_t6::kick::ShotKickPlayerFactors;

use super::*;

const SEED: u32 = 0xA341_316C;

fn params() -> GunKickTuning {
    GunKickTuning {
        scoped_view_kick_recenter_speed: 30.0,
        hipfire_view_kick_recenter_speed: 20.0,
        weapon_pitch_ceiling: 6.0,
        weapon_yaw_ceiling: 4.0,
        scoped_weapon_kick_damped_percent: 25.0,
        scoped_weapon_kick_pitch_bottom: 1.0,
        scoped_weapon_kick_pitch_top: 2.0,
        scoped_weapon_kick_yaw_bottom: -1.0,
        scoped_weapon_kick_yaw_top: 1.0,
        scoped_weapon_kick_accel: 400.0,
        scoped_weapon_kick_velocity_cap: 80.0,
        scoped_weapon_kick_velocity_falloff: 8.0,
        scoped_weapon_kick_resting_falloff: 6.0,
        scoped_camera_kick_pitch_bottom: 2.0,
        scoped_camera_kick_pitch_top: 3.0,
        scoped_camera_kick_yaw_bottom: -2.0,
        scoped_camera_kick_yaw_top: 2.0,
        hipfire_weapon_kick_damped_percent: 50.0,
        hipfire_weapon_kick_pitch_bottom: 3.0,
        hipfire_weapon_kick_pitch_top: 5.0,
        hipfire_weapon_kick_yaw_bottom: -3.0,
        hipfire_weapon_kick_yaw_top: 3.0,
        hipfire_weapon_kick_accel: 500.0,
        hipfire_weapon_kick_velocity_cap: 120.0,
        hipfire_weapon_kick_velocity_falloff: 10.0,
        hipfire_weapon_kick_resting_falloff: 7.0,
        hipfire_camera_kick_pitch_bottom: 4.0,
        hipfire_camera_kick_pitch_top: 6.0,
        hipfire_camera_kick_yaw_bottom: -4.0,
        hipfire_camera_kick_yaw_top: 4.0,
        hip_view_kick_min_magnitude: 0.0,
        ads_view_kick_min_magnitude: 0.0,
    }
}

fn close(got: &[f32], want: &[f32]) {
    assert_eq!(got.len(), want.len());
    for (g, w) in got.iter().zip(want) {
        let slack = 1e-4 * w.abs().max(1.0);
        assert!((g - w).abs() <= slack, "got {got:?}, wanted {want:?}");
    }
}

fn fresh() -> ViewRecoilState {
    ViewRecoilState::default()
}

#[test]
fn a_new_state_is_still_with_the_fixed_seed() {
    let s = fresh();
    assert_eq!(s.punch_spin_rate, [0.0; 3]);
    assert_eq!(s.recoil_tilt, [0.0; 3]);
    assert_eq!(s.gun_swing_rate, [0.0; 2]);
    assert_eq!(s.weapon_aim_angles, [0.0; 2]);
    assert_eq!(s.rng, SEED);
}

#[test]
fn params_default_to_all_zero() {
    let p = GunKickTuning::default();
    assert_eq!(p.weapon_pitch_ceiling, 0.0);
    assert_eq!(p.hip_view_kick_min_magnitude, 0.0);
    assert_eq!(p.scoped_weapon_kick_accel, 0.0);
}

#[test]
fn hip_shot_draws_four_numbers_and_sets_speeds() {
    let p = params();
    let mut s = fresh();
    s.start_shot_kick(&p, 0.0, 0, 0, false);
    close(&s.punch_spin_rate, &[-4.3199015, -1.8164706, 0.9082353]);
    close(&s.gun_swing_rate, &[4.919675, -2.1268377]);
    assert_eq!(s.recoil_tilt, [0.0; 3]);
    assert_eq!(s.weapon_aim_angles, [0.0; 2]);
    assert_eq!(s.rng, 625033959);
}

#[test]
fn ads_shot_with_reduce_window_uses_the_ads_numbers() {
    let p = params();
    let mut s = fresh();
    s.start_shot_kick(&p, 1.0, 0, 0, true);
    close(&s.punch_spin_rate, &[-0.5399877, -0.22705883, 0.113529414]);
    close(&s.gun_swing_rate, &[0.48995936, -0.17723647]);
}

#[test]
fn mid_blend_shot_takes_the_hip_reduction_and_flags() {
    let p = params();
    let mut s = fresh();
    s.start_shot_kick(&p, 0.5, 3, 2, true);
    close(&s.punch_spin_rate, &[-2.1599507, -0.9082353, 0.45411766]);
    close(&s.gun_swing_rate, &[0.9799187, -0.35447294]);
}

#[test]
fn second_shot_replaces_view_velocity_but_adds_gun_speed() {
    let p = params();
    let mut s = fresh();
    s.start_shot_kick(&p, 0.5, 3, 2, true);
    s.start_shot_kick(&p, 0.5, 3, 2, true);
    close(&s.punch_spin_rate, &[-2.6822686, 1.9302897, -0.9651449]);
    close(&s.gun_swing_rate, &[1.869927, -0.3759785]);
    assert_eq!(s.rng, 2055117973);
}

#[test]
fn seed_fire_is_seed_fire_scaled_with_zero_reduction() {
    let p = params();
    for (frac, flags, scale, window) in [(0.0, 0, 0, false), (1.0, 5, 3, true), (0.3, 1, 2, true)] {
        let mut a = fresh();
        a.start_shot_kick(&p, frac, flags, scale, window);
        let mut b = fresh();
        b.start_shot_kick_scaled(
            &p,
            frac,
            ShotKickPlayerFactors {
                damping_spell_on: window,
                dampened_share: 0.0,
                weapon_state_bits: flags,
                kick_pct: scale,
            },
        );
        assert_eq!(a, b);
    }
}

#[test]
fn a_given_reduction_percent_is_replaced_inside_the_window() {
    let p = params();
    let scales = |active| ShotKickPlayerFactors {
        damping_spell_on: active,
        dampened_share: 77.0,
        weapon_state_bits: 0,
        kick_pct: 0,
    };
    // Window open: the weapon's own hip percent is used, not 77.
    let mut open = fresh();
    open.start_shot_kick_scaled(&p, 0.0, scales(true));
    let mut reference_in = fresh();
    reference_in.start_shot_kick(&p, 0.0, 0, 0, true);
    assert_eq!(open, reference_in);
    // Window closed: the caller's percent goes through untouched.
    let mut closed = fresh();
    closed.start_shot_kick_scaled(&p, 0.0, scales(false));
    let mut plain = fresh();
    plain.start_shot_kick_scaled(
        &p,
        0.0,
        ShotKickPlayerFactors {
            damping_spell_on: false,
            dampened_share: 0.0,
            weapon_state_bits: 0,
            kick_pct: 0,
        },
    );
    assert_eq!(closed.rng, plain.rng);
}

#[test]
fn small_hip_kicks_are_raised_to_the_minimum_magnitude() {
    let mut p = params();
    p.hip_view_kick_min_magnitude = 50.0;
    let mut s = fresh();
    s.start_shot_kick(&p, 0.0, 0, 0, false);
    close(&s.punch_spin_rate, &[-46.091053, -19.380777, 9.690389]);
    // The gun side is not stretched.
    close(&s.gun_swing_rate, &[4.919675, -2.1268377]);
    let mag = (s.punch_spin_rate[0].powi(2) + s.punch_spin_rate[1].powi(2)).sqrt();
    assert!((mag - 50.0).abs() < 1e-2);
}

#[test]
fn minimum_magnitude_never_shrinks_a_bigger_kick() {
    let mut p = params();
    p.hip_view_kick_min_magnitude = 0.5;
    let mut s = fresh();
    s.start_shot_kick(&p, 0.0, 0, 0, false);
    close(&s.punch_spin_rate, &[-4.3199015, -1.8164706, 0.9082353]);
}

#[test]
fn ads_minimum_applies_only_at_full_ads() {
    let mut p = params();
    p.ads_view_kick_min_magnitude = 80.0;
    let mut hip = fresh();
    hip.start_shot_kick(&p, 0.0, 0, 0, false);
    close(&hip.punch_spin_rate, &[-4.3199015, -1.8164706, 0.9082353]);
    let mut ads = fresh();
    ads.start_shot_kick(&p, 1.0, 0, 0, false);
    let mag = (ads.punch_spin_rate[0].powi(2) + ads.punch_spin_rate[1].powi(2)).sqrt();
    assert!(mag > 10.0, "ads minimum should have lifted the kick, got {mag}");
}

#[test]
fn advance_with_no_time_changes_nothing() {
    let p = params();
    let mut s = fresh();
    s.start_shot_kick(&p, 0.0, 0, 0, false);
    let before = s.clone();
    s.step_on(&p, 1, 0.0, 0);
    assert_eq!(s, before);
    s.step_on(&p, 1, 0.0, -16);
    assert_eq!(s, before);
}

#[test]
fn advance_twelve_ms_after_a_hip_shot() {
    let p = params();
    let mut s = fresh();
    s.start_shot_kick(&p, 0.0, 0, 0, false);
    s.step_on(&p, 1, 0.0, 12);
    close(&s.punch_spin_rate, &[-4.1799016, -1.6764706, 0.76823527]);
    close(&s.recoil_tilt, &[-0.05105882, -0.02101765, 0.010118824]);
    close(&s.gun_swing_rate, &[0.0, 0.0]);
    close(&s.weapon_aim_angles, &[0.0, 0.0]);
}

#[test]
fn advance_thirty_three_ms_after_an_ads_shot() {
    let p = params();
    let mut s = fresh();
    s.start_shot_kick(&p, 1.0, 0, 0, true);
    s.step_on(&p, 1, 1.0, 33);
    close(&s.punch_spin_rate, &[0.30001235, 0.61294127, 0.0]);
    close(&s.recoil_tilt, &[-0.006164744, -0.0010527294, 0.0]);
    close(&s.gun_swing_rate, &[0.0, 0.0]);
}

#[test]
fn advancing_a_quiet_state_keeps_it_quiet() {
    let p = params();
    let mut s = fresh();
    s.step_on(&p, 1, 0.0, 50);
    assert_eq!(s, fresh());
}

#[test]
fn reset_zeroes_motion_but_keeps_the_random_stream() {
    let p = params();
    let mut s = fresh();
    s.start_shot_kick(&p, 0.5, 3, 2, true);
    s.step_on(&p, 1, 0.5, 20);
    let rng = s.rng;
    assert_ne!(rng, SEED);
    s.restart();
    assert_eq!(s.punch_spin_rate, [0.0; 3]);
    assert_eq!(s.recoil_tilt, [0.0; 3]);
    assert_eq!(s.gun_swing_rate, [0.0; 2]);
    assert_eq!(s.weapon_aim_angles, [0.0; 2]);
    assert_eq!(s.rng, rng);
}

#[test]
fn view_angles_get_the_kick_added_per_axis() {
    assert_eq!(
        kicked_view_angles([10.0, -20.0, 5.0], [1.5, 2.0, -0.5]),
        [11.5, -18.0, 4.5]
    );
    assert_eq!(kicked_view_angles([1.0, 2.0, 3.0], [0.0; 3]), [1.0, 2.0, 3.0]);
}
