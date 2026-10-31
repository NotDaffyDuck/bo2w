#![cfg(test)]
use super::*;

#[test]
fn light_sample_sits_above_the_item() {
    assert_eq!(
        item_light_probe_point([1.0, 2.0, 3.0]),
        [1.0, 2.0, 3.0 + ITEM_LIGHT_LIFT]
    );
}

#[test]
fn weapon_zero_is_no_weapon() {
    assert_eq!(item_weapon_index(0), None);
    assert_eq!(item_weapon_index(7), Some(7));
}

#[test]
fn unrotated_item_only_translates() {
    let m = item_world_from_local([10.0, 20.0, 30.0], [0.0; 3]);
    assert!(
        m.transform_point3(Vec3::ZERO)
            .abs_diff_eq(Vec3::new(10.0, 20.0, 30.0), 1e-5)
    );
    assert!(m.transform_vector3(Vec3::X).abs_diff_eq(Vec3::X, 1e-5));
}

#[test]
fn yaw_turns_the_item_about_up() {
    let m = item_world_from_local([0.0; 3], [0.0, 90.0, 0.0]);
    assert!(m.transform_vector3(Vec3::X).abs_diff_eq(Vec3::Y, 1e-5));
    assert!(m.transform_vector3(Vec3::Z).abs_diff_eq(Vec3::Z, 1e-5));
}

#[test]
fn pitch_tips_the_nose_down() {
    let m = item_world_from_local([0.0; 3], [90.0, 0.0, 0.0]);
    assert!(m.transform_vector3(Vec3::X).abs_diff_eq(-Vec3::Z, 1e-5));
}

#[test]
fn log_throttle_fires_once_per_second() {
    assert!(first_frame_of_second(2000));
    assert!(first_frame_of_second(2015));
    assert!(!first_frame_of_second(2016));
    assert!(!first_frame_of_second(2500));
}

#[test]
fn restart_keeps_counters_and_bumps_on_request() {
    let mut plan = PickupPaintPlan::default();
    plan.plan_rev = 5;
    plan.generation = 9;
    restart_plan(&mut plan, false);
    assert_eq!((plan.plan_rev, plan.generation), (5, 9));
    restart_plan(&mut plan, true);
    assert_eq!((plan.plan_rev, plan.generation), (6, 10));
}
