//! Behaviour pins for the view/gun kick state. The expected numbers were taken
//! from the implementation before it was rewritten, so a rewrite must land on
//! the same values (same random stream, same step sizes, same scaling).

use weapon_t6::kick::FireRecoilPsScales;

use super::*;

const SEED: u32 = 0xA341_316C;

fn params() -> KickParams {
    KickParams {
        f_ads_view_kick_center_speed: 30.0,
        f_hip_view_kick_center_speed: 20.0,
        gun_max_pitch: 6.0,
        gun_max_yaw: 4.0,
        ads_gun_kick_reduced_kick_percent: 25.0,
        ads_gun_kick_pitch_min: 1.0,
        ads_gun_kick_pitch_max: 2.0,
        ads_gun_kick_yaw_min: -1.0,
        ads_gun_kick_yaw_max: 1.0,
        ads_gun_kick_accel: 400.0,
        ads_gun_kick_speed_max: 80.0,
        ads_gun_kick_speed_decay: 8.0,
        ads_gun_kick_static_decay: 6.0,
        ads_view_kick_pitch_min: 2.0,
        ads_view_kick_pitch_max: 3.0,
        ads_view_kick_yaw_min: -2.0,
        ads_view_kick_yaw_max: 2.0,
        hip_gun_kick_reduced_kick_percent: 50.0,
        hip_gun_kick_pitch_min: 3.0,
        hip_gun_kick_pitch_max: 5.0,
        hip_gun_kick_yaw_min: -3.0,
        hip_gun_kick_yaw_max: 3.0,
        hip_gun_kick_accel: 500.0,
        hip_gun_kick_speed_max: 120.0,
        hip_gun_kick_speed_decay: 10.0,
        hip_gun_kick_static_decay: 7.0,
        hip_view_kick_pitch_min: 4.0,
        hip_view_kick_pitch_max: 6.0,
        hip_view_kick_yaw_min: -4.0,
        hip_view_kick_yaw_max: 4.0,
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

fn fresh() -> ViewKickState {
    ViewKickState::default()
}

#[test]
fn a_new_state_is_still_with_the_fixed_seed() {
    let s = fresh();
    assert_eq!(s.kick_avel, [0.0; 3]);
    assert_eq!(s.kick_angles, [0.0; 3]);
    assert_eq!(s.gun_speed, [0.0; 2]);
    assert_eq!(s.gun_angles, [0.0; 2]);
    assert_eq!(s.rng, SEED);
}

#[test]
fn params_default_to_all_zero() {
    let p = KickParams::default();
    assert_eq!(p.gun_max_pitch, 0.0);
    assert_eq!(p.hip_view_kick_min_magnitude, 0.0);
    assert_eq!(p.ads_gun_kick_accel, 0.0);
}

#[test]
fn hip_shot_draws_four_numbers_and_sets_speeds() {
    let p = params();
    let mut s = fresh();
    s.seed_fire(&p, 0.0, 0, 0, false);
    close(&s.kick_avel, &[-4.3199015, -1.8164706, 0.9082353]);
    close(&s.gun_speed, &[4.919675, -2.1268377]);
    assert_eq!(s.kick_angles, [0.0; 3]);
    assert_eq!(s.gun_angles, [0.0; 2]);
    assert_eq!(s.rng, 625033959);
}

#[test]
fn ads_shot_with_reduce_window_uses_the_ads_numbers() {
    let p = params();
    let mut s = fresh();
    s.seed_fire(&p, 1.0, 0, 0, true);
    close(&s.kick_avel, &[-0.5399877, -0.22705883, 0.113529414]);
    close(&s.gun_speed, &[0.48995936, -0.17723647]);
}

#[test]
fn mid_blend_shot_takes_the_hip_reduction_and_flags() {
    let p = params();
    let mut s = fresh();
    s.seed_fire(&p, 0.5, 3, 2, true);
    close(&s.kick_avel, &[-2.1599507, -0.9082353, 0.45411766]);
    close(&s.gun_speed, &[0.9799187, -0.35447294]);
}

#[test]
fn second_shot_replaces_view_velocity_but_adds_gun_speed() {
    let p = params();
    let mut s = fresh();
    s.seed_fire(&p, 0.5, 3, 2, true);
    s.seed_fire(&p, 0.5, 3, 2, true);
    close(&s.kick_avel, &[-2.6822686, 1.9302897, -0.9651449]);
    close(&s.gun_speed, &[1.869927, -0.3759785]);
    assert_eq!(s.rng, 2055117973);
}

#[test]
fn seed_fire_is_seed_fire_scaled_with_zero_reduction() {
    let p = params();
    for (frac, flags, scale, window) in [(0.0, 0, 0, false), (1.0, 5, 3, true), (0.3, 1, 2, true)] {
        let mut a = fresh();
        a.seed_fire(&p, frac, flags, scale, window);
        let mut b = fresh();
        b.seed_fire_scaled(
            &p,
            frac,
            FireRecoilPsScales {
                reduce_window_active: window,
                reduced_percent: 0.0,
                weap_flags: flags,
                recoil_scale: scale,
            },
        );
        assert_eq!(a, b);
    }
}

#[test]
fn a_given_reduction_percent_is_replaced_inside_the_window() {
    let p = params();
    let scales = |active| FireRecoilPsScales {
        reduce_window_active: active,
        reduced_percent: 77.0,
        weap_flags: 0,
        recoil_scale: 0,
    };
    // Window open: the weapon's own hip percent is used, not 77.
    let mut open = fresh();
    open.seed_fire_scaled(&p, 0.0, scales(true));
    let mut reference = fresh();
    reference.seed_fire(&p, 0.0, 0, 0, true);
    assert_eq!(open, reference);
    // Window closed: the caller's percent goes through untouched.
    let mut closed = fresh();
    closed.seed_fire_scaled(&p, 0.0, scales(false));
    let mut plain = fresh();
    plain.seed_fire_scaled(
        &p,
        0.0,
        FireRecoilPsScales {
            reduce_window_active: false,
            reduced_percent: 0.0,
            weap_flags: 0,
            recoil_scale: 0,
        },
    );
    assert_eq!(closed.rng, plain.rng);
}

#[test]
fn small_hip_kicks_are_raised_to_the_minimum_magnitude() {
    let mut p = params();
    p.hip_view_kick_min_magnitude = 50.0;
    let mut s = fresh();
    s.seed_fire(&p, 0.0, 0, 0, false);
    close(&s.kick_avel, &[-46.091053, -19.380777, 9.690389]);
    // The gun side is not stretched.
    close(&s.gun_speed, &[4.919675, -2.1268377]);
    let mag = (s.kick_avel[0].powi(2) + s.kick_avel[1].powi(2)).sqrt();
    assert!((mag - 50.0).abs() < 1e-2);
}

#[test]
fn minimum_magnitude_never_shrinks_a_bigger_kick() {
    let mut p = params();
    p.hip_view_kick_min_magnitude = 0.5;
    let mut s = fresh();
    s.seed_fire(&p, 0.0, 0, 0, false);
    close(&s.kick_avel, &[-4.3199015, -1.8164706, 0.9082353]);
}

#[test]
fn ads_minimum_applies_only_at_full_ads() {
    let mut p = params();
    p.ads_view_kick_min_magnitude = 80.0;
    let mut hip = fresh();
    hip.seed_fire(&p, 0.0, 0, 0, false);
    close(&hip.kick_avel, &[-4.3199015, -1.8164706, 0.9082353]);
    let mut ads = fresh();
    ads.seed_fire(&p, 1.0, 0, 0, false);
    let mag = (ads.kick_avel[0].powi(2) + ads.kick_avel[1].powi(2)).sqrt();
    assert!(mag > 10.0, "ads minimum should have lifted the kick, got {mag}");
}

#[test]
fn advance_with_no_time_changes_nothing() {
    let p = params();
    let mut s = fresh();
    s.seed_fire(&p, 0.0, 0, 0, false);
    let before = s.clone();
    s.advance(&p, 1, 0.0, 0);
    assert_eq!(s, before);
    s.advance(&p, 1, 0.0, -16);
    assert_eq!(s, before);
}

#[test]
fn advance_twelve_ms_after_a_hip_shot() {
    let p = params();
    let mut s = fresh();
    s.seed_fire(&p, 0.0, 0, 0, false);
    s.advance(&p, 1, 0.0, 12);
    close(&s.kick_avel, &[-4.1799016, -1.6764706, 0.76823527]);
    close(&s.kick_angles, &[-0.05105882, -0.02101765, 0.010118824]);
    close(&s.gun_speed, &[0.0, 0.0]);
    close(&s.gun_angles, &[0.0, 0.0]);
}

#[test]
fn advance_thirty_three_ms_after_an_ads_shot() {
    let p = params();
    let mut s = fresh();
    s.seed_fire(&p, 1.0, 0, 0, true);
    s.advance(&p, 1, 1.0, 33);
    close(&s.kick_avel, &[0.30001235, 0.61294127, 0.0]);
    close(&s.kick_angles, &[-0.006164744, -0.0010527294, 0.0]);
    close(&s.gun_speed, &[0.0, 0.0]);
}

#[test]
fn advancing_a_quiet_state_keeps_it_quiet() {
    let p = params();
    let mut s = fresh();
    s.advance(&p, 1, 0.0, 50);
    assert_eq!(s, fresh());
}

#[test]
fn reset_zeroes_motion_but_keeps_the_random_stream() {
    let p = params();
    let mut s = fresh();
    s.seed_fire(&p, 0.5, 3, 2, true);
    s.advance(&p, 1, 0.5, 20);
    let rng = s.rng;
    assert_ne!(rng, SEED);
    s.reset();
    assert_eq!(s.kick_avel, [0.0; 3]);
    assert_eq!(s.kick_angles, [0.0; 3]);
    assert_eq!(s.gun_speed, [0.0; 2]);
    assert_eq!(s.gun_angles, [0.0; 2]);
    assert_eq!(s.rng, rng);
}

#[test]
fn view_angles_get_the_kick_added_per_axis() {
    assert_eq!(
        add_kick_to_viewangles([10.0, -20.0, 5.0], [1.5, 2.0, -0.5]),
        [11.5, -18.0, 4.5]
    );
    assert_eq!(add_kick_to_viewangles([1.0, 2.0, 3.0], [0.0; 3]), [1.0, 2.0, 3.0]);
}
