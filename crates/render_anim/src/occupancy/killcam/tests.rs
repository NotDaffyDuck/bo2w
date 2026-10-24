use super::*;

fn close(a: Vec3, b: Vec3) -> bool {
    (a - b).length() < 1e-4
}

#[test]
fn weapon_facts_pick_the_mode() {
    assert!(matches!(
        mode_for_weapon(3, 7, 2),
        KillCamMode::Mode7Javelin
    ));
    assert!(matches!(mode_for_weapon(2, 7, 2), KillCamMode::Mode8Remote));
    assert!(matches!(mode_for_weapon(0, 7, 2), KillCamMode::Mode5Rocket));
    assert!(matches!(
        mode_for_weapon(0, 1, 2),
        KillCamMode::Mode4MissileAlt
    ));
    assert!(matches!(
        mode_for_weapon(0, 0, 0),
        KillCamMode::Mode3Missile
    ));
}

#[test]
fn entity_kind_picks_the_mode() {
    assert!(mode_for_entity_type(12).is_none());
    assert!(matches!(
        mode_for_entity_type(11),
        Some(KillCamMode::Mode6Turret)
    ));
    assert!(matches!(
        mode_for_entity_type(4),
        Some(KillCamMode::Mode2Airstrike)
    ));
}

#[test]
fn chase_distances_by_mode_and_ground() {
    assert_eq!(chase_offsets(KillCamMode::Mode3Missile, false), (5.0, 35.0));
    assert_eq!(chase_offsets(KillCamMode::Mode5Rocket, false), (10.0, 70.0));
    assert_eq!(
        chase_offsets(KillCamMode::Mode8Remote, false),
        (60.0, 300.0)
    );
    assert_eq!(chase_offsets(KillCamMode::Mode3Missile, true), (15.0, 30.0));
    assert_eq!(
        chase_offsets(KillCamMode::Mode4MissileAlt, true),
        (15.0, 30.0)
    );
    assert_eq!(chase_offsets(KillCamMode::Mode5Rocket, true), (0.0, 0.0));
}

#[test]
fn focus_distance_modes() {
    assert!(has_focus_distance(KillCamMode::Mode2Airstrike));
    assert!(has_focus_distance(KillCamMode::Mode8Remote));
    assert!(!has_focus_distance(KillCamMode::Mode6Turret));
    assert!(!has_focus_distance(KillCamMode::Mode7Javelin));
}

#[test]
fn glide_slows_to_a_stop() {
    let glide = Glide {
        started_at: 100,
        duration_ms: 200.0,
        from: Vec3::new(1.0, 2.0, 3.0),
        velocity: Vec3::new(2.0, 0.0, 0.0),
    };
    assert!(close(glide.position(50), glide.from));
    assert!(close(glide.position(100), glide.from));
    // Halfway: 100 * (1 - 0.25) = 75 units of velocity-time.
    assert!(close(glide.position(200), Vec3::new(151.0, 2.0, 3.0)));
    // At and past the end the camera rests at 100 * (1 - 0.5) * 2... = 200.
    let rest = glide.position(300);
    assert!(close(rest, Vec3::new(201.0, 2.0, 3.0)));
    assert!(close(glide.position(900), rest));
    let instant = Glide {
        duration_ms: 0.0,
        ..glide
    };
    assert!(close(instant.position(500), instant.from));
}

#[test]
fn flat_left_of_vertical_falls_back_to_x() {
    assert!(close(flat_left(Vec3::Z), Vec3::X));
    assert!(close(flat_left(Vec3::X), Vec3::Y));
    assert!(close(flat_left(Vec3::Y), -Vec3::X));
}

#[test]
fn facing_angles_turn_with_the_direction() {
    let ahead = facing_angles(Vec3::X);
    assert!(ahead[0].abs() < 1e-3 && ahead[1].abs() < 1e-3);
    let left = facing_angles(Vec3::Y);
    assert!(game_math::angle_subtract(left[1], 90.0).abs() < 1e-3);
}

#[test]
fn aim_stays_on_a_victim_that_lines_up() {
    let camera = Vec3::ZERO;
    let aim = look_at_both(
        Vec3::new(100.0, 0.0, 0.0),
        Vec3::new(200.0, 0.0, 0.0),
        camera,
    );
    assert!(close(aim, Vec3::X));
}

#[test]
fn aim_gives_up_on_a_subject_far_from_the_victim() {
    let camera = Vec3::ZERO;
    let aim = look_at_both(
        Vec3::new(100.0, 0.0, 0.0),
        Vec3::new(0.0, 100.0, 0.0),
        camera,
    );
    assert!(close(aim, Vec3::X));
}

#[test]
fn aim_never_stares_straight_down_past_the_victim() {
    let camera = Vec3::ZERO;
    let victim = Vec3::new(100.0, 0.0, 0.0);
    let subject = Vec3::new(0.0, 0.0, -100.0);
    let aim = look_at_both(victim, subject, camera);
    assert!(aim.z > -0.95);
    assert!((aim.length() - 1.0).abs() < 1e-4);
}

#[test]
fn pull_back_without_a_world_goes_straight_to_the_end() {
    let end = Vec3::new(5.0, 6.0, 7.0);
    assert!(close(pull_back(None, Vec3::ZERO, end), end));
}
