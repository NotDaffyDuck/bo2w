#![cfg(test)]
use super::*;

fn body_at(origin: Vec3, motion_vec: Vec3) -> PhysBody {
    PhysBody {
        pos: origin,
        motion_vec,
        mins: [-2.0; 3],
        maxs: [2.0; 3],
        rebound: 0.5,
        drag: 0.5,
        grounded: true,
        still_ms: 0,
        noted_fast: false,
        noted_stuck: false,
    }
}

#[test]
fn free_fall_accelerates_down() {
    let mut body = body_at(Vec3::ZERO, Vec3::ZERO);
    advance_in_air(&mut body, 0.1);
    assert!((body.motion_vec.z + 80.0).abs() < 1e-4);
    assert!((body.pos.z + 8.0).abs() < 1e-4);
    assert!(!body.grounded);
}

#[test]
fn a_hard_hit_bounces_and_loses_slide() {
    let out = contact_response(Vec3::new(100.0, 0.0, -400.0), Vec3::Z, 0.5, 0.25, 0.017);
    assert!((out.z - 200.0).abs() < 1e-3, "{out:?}");
    // push = 600, friction takes 150 of the 100 units of slide: all of it.
    assert!(out.x.abs() < 1e-3, "{out:?}");
    let gentle = contact_response(Vec3::new(1000.0, 0.0, -400.0), Vec3::Z, 0.5, 0.25, 0.017);
    assert!((gentle.x - 850.0).abs() < 1e-2, "{gentle:?}");
}

#[test]
fn a_resting_contact_does_not_bounce() {
    let dt = 0.017;
    let resting = -GRAVITY * dt;
    let out = contact_response(Vec3::new(0.0, 0.0, -resting), Vec3::Z, 0.9, 0.0, dt);
    assert!(out.z.abs() < 1e-3, "{out:?}");
}

#[test]
fn leaving_a_surface_keeps_its_velocity() {
    let v = Vec3::new(3.0, 4.0, 5.0);
    assert_eq!(contact_response(v, Vec3::Z, 0.5, 0.5, 0.017), v);
}

#[test]
fn negative_bounce_is_treated_as_none() {
    let out = contact_response(Vec3::new(0.0, 0.0, -500.0), Vec3::Z, -3.0, 0.0, 0.017);
    assert!(out.z.abs() < 1e-3);
}

#[test]
fn a_still_grounded_piece_sleeps_after_the_delay() {
    let mut world = DebrisPhysLevel::default();
    let sleeper = Entity::from_raw_u32(1).unwrap();
    let mover = Entity::from_raw_u32(2).unwrap();
    world
        .awake
        .insert(sleeper, body_at(Vec3::ZERO, Vec3::new(1.0, 0.0, 0.0)));
    world
        .awake
        .insert(mover, body_at(Vec3::ZERO, Vec3::new(50.0, 0.0, 0.0)));
    assert!(settle(&mut world, 100).is_empty());
    assert_eq!(settle(&mut world, 100), vec![sleeper]);
    assert!(world.still_moving(mover));
    world.remove_physics_body(mover);
    assert!(!world.still_moving(mover));
}

#[test]
fn the_clock_starts_on_first_call_and_caps_long_frames() {
    let mut world = DebrisPhysLevel::default();
    let faller = Entity::from_raw_u32(3).unwrap();
    world
        .awake
        .insert(faller, body_at(Vec3::new(0.0, 0.0, 100.0), Vec3::ZERO));
    assert_eq!(run_steps(&mut world, None, 1000), 0);
    assert_eq!(run_steps(&mut world, None, 1000), 0);
    assert_eq!(run_steps(&mut world, None, 900), 0);
    // One 40 ms frame is two 20 ms steps.
    assert_eq!(run_steps(&mut world, None, 1040), 40);
    let body = world.awake[&faller];
    assert!(body.pos.z < 100.0);
    // A frame that would need many steps still runs just two.
    assert_eq!(run_steps(&mut world, None, 2000), 960);
    assert_eq!(world.clock_ms, Some(2000));
}
