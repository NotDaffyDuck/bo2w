use super::*;

fn rider(first_person: bool, pitch: [f32; 2]) -> VehicleView {
    VehicleView {
        model: 7,
        first_person,
        at: [0.0; 3],
        pitch,
        fov: 0.0,
        number: 0,
        spring: [60.0, 1.0],
        max_speed: 100.0,
    }
}

#[test]
fn view_string_reads_every_field() {
    let view = parse_vehicle_view("12 1 1 2 3 40 50 90 33 60 0.5 250").unwrap();
    assert_eq!(view.model, 12);
    assert!(view.first_person);
    assert_eq!(view.at, [1.0, 2.0, 3.0]);
    assert_eq!(view.pitch, [40.0, 50.0]);
    assert_eq!(view.fov, 90.0);
    assert_eq!(view.number, 33);
    assert_eq!(view.spring, [60.0, 0.5]);
    assert_eq!(view.max_speed, 250.0);
}

#[test]
fn view_string_zero_fills_and_rejects_junk() {
    let view = parse_vehicle_view("5 0 120 30").unwrap();
    assert!(!view.first_person);
    assert_eq!(view.at, [120.0, 30.0, 0.0]);
    assert_eq!(view.pitch, [0.0; 2]);
    assert_eq!(view.max_speed, 0.0);
    assert!(parse_vehicle_view("").is_none());
    assert!(parse_vehicle_view("5").is_none());
    assert!(parse_vehicle_view("x 0").is_none());
    let garbled = parse_vehicle_view("5 2 a 9").unwrap();
    assert!(!garbled.first_person);
    assert_eq!(garbled.at, [0.0, 9.0, 0.0]);
}

#[test]
fn command_angles_fold_into_half_turns() {
    assert_eq!(command_angle_degrees(0, 0.0), 0.0);
    assert!((command_angle_degrees(0x4000, 0.0) - 90.0).abs() < 1e-3);
    assert!((command_angle_degrees(0xc000, 0.0) + 90.0).abs() < 1e-3);
    assert!((command_angle_degrees(0x4000, 100.0) + 170.0).abs() < 1e-3);
    // Only the low sixteen bits count.
    assert!((command_angle_degrees(0x1_4000, 0.0) - 90.0).abs() < 1e-3);
}

#[test]
fn tilt_follows_the_vehicle_pitch() {
    let level = pitch_tilt([0.0; 3], Quat::IDENTITY);
    assert!(level.abs() < 1e-3);
    // Nose up 30 degrees about Y: looking straight ahead is now 30 down relative to it.
    let climbing = Quat::from_rotation_y(-30f32.to_radians());
    let tilt = pitch_tilt([0.0; 3], climbing);
    assert!((tilt.abs() - 30.0).abs() < 0.1, "tilt {tilt}");
}

#[test]
fn spring_idles_without_a_spring_definition() {
    let mut spring = VehicleSpring::default();
    let mut view = rider(false, [0.0; 2]);
    view.spring = [0.0, 1.0];
    for i in 0..5 {
        let at = Vec3::new(i as f32 * 10.0, 0.0, 0.0);
        assert_eq!(spring.step(&view, at, Quat::IDENTITY, 0.05), 0.0);
    }
}

#[test]
fn spring_pulls_back_when_driven_and_resets_per_model() {
    let mut spring = VehicleSpring::default();
    let view = rider(false, [0.0; 2]);
    let mut offset = 0.0;
    for i in 0..200 {
        let at = Vec3::new(i as f32 * 5.0, 0.0, 0.0);
        // 100 units a second at dt 0.05: the whole top speed.
        offset = spring.step(&view, at, Quat::IDENTITY, 0.05);
    }
    assert!(offset > 55.0 && offset <= 60.0, "offset {offset}");
    let mut other = view;
    other.model = 8;
    let again = spring.step(&other, Vec3::ZERO, Quat::IDENTITY, 0.05);
    assert_eq!(again, 0.0);
    spring.reset();
    assert_eq!(spring.step(&view, Vec3::ZERO, Quat::IDENTITY, 0.05), 0.0);
}

#[test]
fn spring_holds_its_offset_on_a_zero_frame() {
    let mut spring = VehicleSpring::default();
    let view = rider(false, [0.0; 2]);
    for i in 0..40 {
        spring.step(
            &view,
            Vec3::new(i as f32 * 5.0, 0.0, 0.0),
            Quat::IDENTITY,
            0.05,
        );
    }
    let held = spring.offset;
    assert!(held > 0.0);
    assert_eq!(
        spring.step(&view, Vec3::new(500.0, 0.0, 0.0), Quat::IDENTITY, 0.0),
        held
    );
}
