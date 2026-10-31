//! The free-flying world camera (a development aid) and the pose override
//! used by look-at tests.

pub use render_scene::{
    FreeRoamCamera, FirstPersonLens, GameplayCamera, WorldEyePlacement, transform_from_game_view,
};

use bevy::prelude::*;

/// Radians per second the arrow keys turn the camera.
const TURN_RATE: f32 = 1.6;
/// The camera never pitches further than this from level, either way.
const PITCH_LIMIT: f32 = 1.54;
/// Speed factor while the left shift key is held.
const SPRINT_FACTOR: f32 = 6.0;

/// +1 while only `plus` is held, -1 while only `minus` is, else 0.
fn key_axis(keys: &ButtonInput<KeyCode>, plus: KeyCode, minus: KeyCode) -> f32 {
    f32::from(keys.pressed(plus)) - f32::from(keys.pressed(minus))
}

/// Unit vector along the camera's facing, flattened onto the ground plane
/// (north when the camera looks straight up or down).
fn ground_heading(transform: &Transform) -> Vec3 {
    let facing = *transform.forward();
    let flat_list = Vec3::new(facing.x, facing.y, 0.0);
    if flat_list.length_squared() > 1e-6 {
        flat_list.normalize()
    } else {
        Vec3::Y
    }
}

/// Arrow keys turn, WASD walks on the ground plane, E and Q rise and sink.
/// Does nothing while the simulation owns the camera or the fly camera is
/// frozen.
pub(crate) fn free_camera(
    time: Res<Time>,
    held: Res<ButtonInput<KeyCode>>,
    sim_cam: Option<Res<GameplayCamera>>,
    mut cameras: Query<(&mut Transform, &mut FreeRoamCamera)>,
) {
    if sim_cam.is_some_and(|c| c.enabled || c.hold_free_camera) {
        return;
    }
    let delta_t = time.delta_secs();
    let turn = TURN_RATE * delta_t;
    for (mut xf, mut cam) in &mut cameras {
        let yaw_step = turn * key_axis(&held, KeyCode::ArrowLeft, KeyCode::ArrowRight);
        let wanted_pitch = cam.tilt + turn * key_axis(&held, KeyCode::ArrowUp, KeyCode::ArrowDown);
        let pitch = wanted_pitch.clamp(-PITCH_LIMIT, PITCH_LIMIT);
        let pitch_step = pitch - cam.tilt;
        cam.turn_deg += yaw_step;
        cam.tilt = pitch;
        if yaw_step != 0.0 {
            xf.rotate_z(yaw_step);
        }
        if pitch_step != 0.0 {
            xf.rotate_local_x(pitch_step);
        }

        let heading = ground_heading(&xf);
        let strafe = Vec3::new(heading.y, -heading.x, 0.0);
        let wish = heading * key_axis(&held, KeyCode::KeyW, KeyCode::KeyS)
            + strafe * key_axis(&held, KeyCode::KeyD, KeyCode::KeyA)
            + Vec3::Z * key_axis(&held, KeyCode::KeyE, KeyCode::KeyQ);
        if wish != Vec3::ZERO {
            let boost = if held.pressed(KeyCode::ShiftLeft) {
                SPRINT_FACTOR
            } else {
                1.0
            };
            xf.translation += wish.normalize() * (cam.pace * boost * delta_t);
        }
    }
}

/// `BO2MP_LOOK_CAMERA="x y z yaw pitch"` pins the world camera to that pose
/// whatever the player does. Test aid for views nobody can stand in.
pub(crate) fn look_camera_override(mut cameras: Query<&mut Transform, With<FreeRoamCamera>>) {
    static OVERRIDE: std::sync::OnceLock<Option<[f32; 5]>> = std::sync::OnceLock::new();
    let parsed = OVERRIDE.get_or_init(|| {
        let text = std::env::var("BO2MP_LOOK_CAMERA").ok()?;
        let numbers: Vec<f32> = text
            .split(char::is_whitespace)
            .filter(|w| !w.is_empty())
            .filter_map(|w| w.parse().ok())
            .collect();
        <[f32; 5]>::try_from(numbers).ok()
    });
    let Some([x, y, z, yaw, pitch]) = *parsed else {
        return;
    };
    let pose = transform_from_game_view(WorldEyePlacement {
        origin: [x, y, z],
        turn: [pitch, yaw, 0.0],
    });
    for mut transform in &mut cameras {
        *transform = pose;
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use bevy::ecs::system::RunSystemOnce;

    use super::*;

    fn near(a: Vec3, b: Vec3) -> bool {
        (a - b).length() < 1e-3
    }

    /// A world with one fly camera at the origin looking along +Y, the clock
    /// advanced `dt` seconds, and `held` keys down.
    fn flying(dt: f32, held: &[KeyCode], speed: f32) -> World {
        let mut world = World::new();
        let mut time = Time::<()>::default();
        time.advance_by(Duration::from_secs_f32(dt));
        world.insert_resource(time);
        let mut keys = ButtonInput::<KeyCode>::default();
        for &key in held {
            keys.press(key);
        }
        world.insert_resource(keys);
        world.spawn((
            Transform::IDENTITY.looking_to(Vec3::Y, Vec3::Z),
            FreeRoamCamera {
                turn_deg: 0.0,
                tilt: 0.0,
                pace: speed,
            },
        ));
        world
    }

    fn fly(world: &mut World) -> (Transform, f32, f32) {
        world.run_system_once(free_camera).unwrap();
        let (transform, cam) = world
            .query::<(&Transform, &FreeRoamCamera)>()
            .single(world)
            .unwrap();
        (*transform, cam.turn_deg, cam.tilt)
    }

    #[test]
    fn a_key_axis_is_plus_minus_or_nothing() {
        let mut keys = ButtonInput::<KeyCode>::default();
        assert_eq!(key_axis(&keys, KeyCode::KeyW, KeyCode::KeyS), 0.0);
        keys.press(KeyCode::KeyW);
        assert_eq!(key_axis(&keys, KeyCode::KeyW, KeyCode::KeyS), 1.0);
        keys.press(KeyCode::KeyS);
        assert_eq!(key_axis(&keys, KeyCode::KeyW, KeyCode::KeyS), 0.0);
        keys.release(KeyCode::KeyW);
        assert_eq!(key_axis(&keys, KeyCode::KeyW, KeyCode::KeyS), -1.0);
    }

    #[test]
    fn the_ground_heading_is_flat_and_unit_length() {
        let level = Transform::IDENTITY.looking_to(Vec3::X, Vec3::Z);
        assert!(near(ground_heading(&level), Vec3::X));
        let pitched = Transform::IDENTITY.looking_to(Vec3::new(0.0, 3.0, 4.0), Vec3::Z);
        assert!(near(ground_heading(&pitched), Vec3::Y));
    }

    #[test]
    fn looking_straight_up_or_down_heads_north() {
        let up = Transform::IDENTITY.looking_to(Vec3::Z, Vec3::Y);
        assert_eq!(ground_heading(&up), Vec3::Y);
        let down = Transform::IDENTITY.looking_to(Vec3::NEG_Z, Vec3::Y);
        assert_eq!(ground_heading(&down), Vec3::Y);
    }

    #[test]
    fn w_and_s_walk_along_the_heading_at_the_cameras_speed() {
        let (moved, ..) = fly(&mut flying(0.5, &[KeyCode::KeyW], 100.0));
        assert!(near(moved.translation, Vec3::Y * 50.0));
        let (moved, ..) = fly(&mut flying(0.5, &[KeyCode::KeyS], 100.0));
        assert!(near(moved.translation, Vec3::NEG_Y * 50.0));
    }

    #[test]
    fn d_and_a_strafe_right_and_left_and_e_and_q_rise_and_sink() {
        let (moved, ..) = fly(&mut flying(1.0, &[KeyCode::KeyD], 100.0));
        assert!(near(moved.translation, Vec3::new(100.0, 0.0, 0.0)));
        let (moved, ..) = fly(&mut flying(1.0, &[KeyCode::KeyA], 100.0));
        assert!(near(moved.translation, Vec3::new(-100.0, 0.0, 0.0)));
        let (moved, ..) = fly(&mut flying(1.0, &[KeyCode::KeyE], 100.0));
        assert!(near(moved.translation, Vec3::Z * 100.0));
        let (moved, ..) = fly(&mut flying(1.0, &[KeyCode::KeyQ], 100.0));
        assert!(near(moved.translation, Vec3::NEG_Z * 100.0));
    }

    #[test]
    fn walking_diagonally_is_no_faster_than_walking_straight() {
        let (moved, ..) = fly(&mut flying(1.0, &[KeyCode::KeyW, KeyCode::KeyD], 100.0));
        assert!((moved.translation.length() - 100.0).abs() < 1e-3);
        assert!(moved.translation.x > 0.0 && moved.translation.y > 0.0);
    }

    #[test]
    fn opposite_keys_cancel() {
        let (moved, ..) = fly(&mut flying(1.0, &[KeyCode::KeyW, KeyCode::KeyS], 100.0));
        assert_eq!(moved.translation, Vec3::ZERO);
    }

    #[test]
    fn left_shift_makes_the_camera_six_times_faster() {
        let (moved, ..) = fly(&mut flying(1.0, &[KeyCode::KeyW, KeyCode::ShiftLeft], 100.0));
        assert!(near(moved.translation, Vec3::Y * 600.0));
        // Shift alone goes nowhere.
        let (moved, ..) = fly(&mut flying(1.0, &[KeyCode::ShiftLeft], 100.0));
        assert_eq!(moved.translation, Vec3::ZERO);
    }

    #[test]
    fn nothing_moves_when_no_time_has_passed() {
        let (moved, yaw, pitch) = fly(&mut flying(0.0, &[KeyCode::KeyW, KeyCode::ArrowLeft], 100.0));
        assert_eq!(moved.translation, Vec3::ZERO);
        assert_eq!((yaw, pitch), (0.0, 0.0));
    }

    #[test]
    fn the_left_arrow_turns_the_camera_counter_clockwise_at_the_turn_rate() {
        let (turned, yaw, pitch) = fly(&mut flying(0.5, &[KeyCode::ArrowLeft], 100.0));
        assert!((yaw - 0.8).abs() < 1e-5);
        assert_eq!(pitch, 0.0);
        // Facing +Y, a left turn brings the nose toward -X.
        let heading = ground_heading(&turned);
        assert!(heading.x < 0.0 && heading.y > 0.0);
        let (turned, yaw, _) = fly(&mut flying(0.5, &[KeyCode::ArrowRight], 100.0));
        assert!((yaw + 0.8).abs() < 1e-5);
        assert!(ground_heading(&turned).x > 0.0);
    }

    #[test]
    fn after_a_turn_walking_follows_the_new_heading() {
        // A quarter turn left puts -X ahead.
        let quarter = std::f32::consts::FRAC_PI_2;
        let mut world = flying(quarter / 1.6, &[KeyCode::ArrowLeft], 100.0);
        let (turned, ..) = fly(&mut world);
        assert!(near(ground_heading(&turned), Vec3::NEG_X));

        world.resource_mut::<ButtonInput<KeyCode>>().release(KeyCode::ArrowLeft);
        world.resource_mut::<ButtonInput<KeyCode>>().press(KeyCode::KeyW);
        let mut time = Time::<()>::default();
        time.advance_by(Duration::from_secs(1));
        world.insert_resource(time);
        let (walked, ..) = fly(&mut world);
        assert!(near(walked.translation, turned.translation + Vec3::new(-100.0, 0.0, 0.0)));
    }

    #[test]
    fn the_up_arrow_tilts_the_view_up_and_stops_just_short_of_straight_up() {
        let (tilted, _, pitch) = fly(&mut flying(0.5, &[KeyCode::ArrowUp], 100.0));
        assert!((pitch - 0.8).abs() < 1e-5);
        assert!(tilted.forward().z > 0.0);

        let (tilted, _, pitch) = fly(&mut flying(10.0, &[KeyCode::ArrowUp], 100.0));
        assert_eq!(pitch, PITCH_LIMIT);
        assert!((tilted.forward().z - PITCH_LIMIT.sin()).abs() < 1e-3);
        let (tilted, _, pitch) = fly(&mut flying(10.0, &[KeyCode::ArrowDown], 100.0));
        assert_eq!(pitch, -PITCH_LIMIT);
        assert!((tilted.forward().z + PITCH_LIMIT.sin()).abs() < 1e-3);
    }

    #[test]
    fn the_simulation_owning_the_camera_freezes_the_fly_camera() {
        let enabled = true;
        for sim in [
            GameplayCamera {
                enabled,
                hold_free_camera: false,
            },
            GameplayCamera {
                enabled: false,
                hold_free_camera: true,
            },
        ] {
            let mut world = flying(1.0, &[KeyCode::KeyW, KeyCode::ArrowLeft], 100.0);
            world.insert_resource(sim);
            let (moved, yaw, _) = fly(&mut world);
            assert_eq!(moved.translation, Vec3::ZERO);
            assert_eq!(yaw, 0.0);
        }
        let mut world = flying(1.0, &[KeyCode::KeyW], 100.0);
        world.insert_resource(GameplayCamera::default());
        let (moved, ..) = fly(&mut world);
        assert!(moved.translation.y > 0.0);
    }

    #[test]
    fn every_fly_camera_moves() {
        let mut world = flying(1.0, &[KeyCode::KeyE], 10.0);
        world.spawn((Transform::from_xyz(5.0, 0.0, 0.0), FreeRoamCamera { turn_deg: 0.0, tilt: 0.0, pace: 20.0 }));
        world.run_system_once(free_camera).unwrap();
        let mut heights: Vec<f32> = world
            .query_filtered::<&Transform, With<FreeRoamCamera>>()
            .iter(&world)
            .map(|t| t.translation.z)
            .collect();
        heights.sort_by(f32::total_cmp);
        assert_eq!(heights, [10.0, 20.0]);
    }

    #[test]
    fn an_iw_view_puts_the_camera_at_the_origin_looking_along_its_angles() {
        let pose = |angles| WorldEyePlacement {
            origin: [1.0, 2.0, 3.0],
            turn: angles,
        };
        let level = transform_from_game_view(pose([0.0, 0.0, 0.0]));
        assert_eq!(level.translation, Vec3::new(1.0, 2.0, 3.0));
        assert!(near(*level.forward(), Vec3::X));
        assert!(near(*level.up(), Vec3::Z));

        let north = transform_from_game_view(pose([0.0, 90.0, 0.0]));
        assert!(near(*north.forward(), Vec3::Y));
        let west = transform_from_game_view(pose([0.0, 180.0, 0.0]));
        assert!(near(*west.forward(), Vec3::NEG_X));
    }

    const AT_ORIGIN: [f32; 3] = [0.0; 3];

    #[test]
    fn a_positive_iw_pitch_looks_down() {
        let down = transform_from_game_view(WorldEyePlacement {
            origin: AT_ORIGIN,
            turn: [90.0, 0.0, 0.0],
        });
        assert!(near(*down.forward(), Vec3::NEG_Z));
        let slightly = transform_from_game_view(WorldEyePlacement {
            origin: AT_ORIGIN,
            turn: [30.0, 0.0, 0.0],
        });
        assert!(slightly.forward().z < 0.0 && slightly.forward().x > 0.0);
    }

    #[test]
    fn an_iw_roll_tilts_the_up_vector_sideways() {
        let roll = [0.0, 0.0, 90.0];
        let rolled = transform_from_game_view(WorldEyePlacement {
            origin: AT_ORIGIN,
            turn: roll,
        });
        assert!(near(*rolled.forward(), Vec3::X));
        assert!(rolled.up().z.abs() < 1e-3);
        assert!(rolled.up().y.abs() > 0.99);
    }
}
