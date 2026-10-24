use super::*;

/// The vehicle the local player is riding.
pub fn vehicle_view(presented: &PresentedSnapshot, local: ClientId) -> Option<VehicleView> {
    let text = presented
        .snapshot()?
        .meta
        .script_dvars(local)
        .string(VEHICLE_VIEW_DVAR)?;
    parse_vehicle_view(text)
}

/// Reads the view string; a missing or unreadable number counts as zero.
pub(super) fn parse_vehicle_view(text: &str) -> Option<VehicleView> {
    let mut words = text.split_whitespace();
    let model = words.next()?.parse().ok()?;
    let mode: u8 = words.next()?.parse().ok()?;
    let mut next_number = || {
        words
            .next()
            .and_then(|w| w.parse::<f32>().ok())
            .unwrap_or(0.0)
    };
    let at = [next_number(), next_number(), next_number()];
    let pitch = [next_number(), next_number()];
    let fov = next_number();
    let number = next_number() as i32;
    let spring = [next_number(), next_number()];
    let max_speed = next_number();
    Some(VehicleView {
        model,
        first_person: mode == 1,
        at,
        pitch,
        fov,
        number,
        spring,
        max_speed,
    })
}

/// A command angle (16-bit fixed point) plus the server's delta, in degrees
/// folded to -180..180.
pub(super) fn command_angle_degrees(command: i32, delta: f32) -> f32 {
    let raw = (command & 0xffff) as f32 * (360.0 / 65536.0) + delta;
    (raw + 180.0).rem_euclid(360.0) - 180.0
}

/// How far the vehicle's own pitch along the view direction shifts the
/// allowed look range, in degrees.
pub(super) fn pitch_tilt(angles: [f32; 3], rotation: Quat) -> f32 {
    let (forward, _, _) = angle_vectors(angles);
    let level = Vec3::new(forward[0], forward[1], 0.0).normalize_or_zero();
    let along = (rotation * Vec3::Z).dot(level).clamp(-1.0, 1.0);
    90.0 - along.acos().to_degrees()
}

/// The pose of a rider whose vehicle model sits at `origin` turned by
/// `rotation`. From a seat it is the seat's place; from behind it is
/// `range` back along the view from a point `height` above the vehicle,
/// pulled in by walls. `spring` is extra range from the vehicle's speed (see
/// [`VehicleSpring`]). `look` carries the mouse aim from the command: a dead
/// rider's own view stays frozen toward the killer, but the car still drives
/// where the mouse points, so its camera follows the mouse.
pub fn vehicle_camera(
    presented: &PresentedSnapshot,
    local: ClientId,
    view: VehicleView,
    origin: [f32; 3],
    rotation: Quat,
    clip: Option<&ClipCollision>,
    spring: f32,
    look: Option<[i32; 3]>,
) -> WorldCameraPose {
    let mut angles = match presented.player(local) {
        None => [0.0; 3],
        Some(ps) => match look {
            Some(cmd) if matches!(ps.pm_type, PM_TYPE_DEAD | PM_TYPE_DEAD_LINKED) => {
                std::array::from_fn(|axis| command_angle_degrees(cmd[axis], ps.delta_angles[axis]))
            }
            _ => ps.viewangles,
        },
    };
    angles[2] = 0.0;
    if view.pitch != [0.0; 2] {
        let tilt = if view.first_person {
            0.0
        } else {
            pitch_tilt(angles, rotation)
        };
        angles[0] = angles[0].clamp(tilt - view.pitch[0], tilt + view.pitch[1]);
    }
    let base = Vec3::from_array(origin);
    if view.first_person {
        let seat = base + rotation * Vec3::from_array(view.at);
        return WorldCameraPose {
            origin: seat.to_array(),
            angles,
        };
    }
    let range = view.at[0] + spring;
    let pivot = base + rotation * Vec3::new(0.0, 0.0, view.at[1]);
    let (forward, _, _) = angle_vectors(angles);
    let wanted = pivot - Vec3::from_array(forward) * range;
    let reach = match clip {
        None => 1.0,
        Some(clip) => clip
            .sweep_box(
                pivot.to_array(),
                wanted.to_array(),
                [-VEHICLE_CAM_HALF; 3],
                [VEHICLE_CAM_HALF; 3],
                CG_CAMERA_PULLBACK_CLIPMASK,
            )
            .fraction
            .clamp(0.0, 1.0),
    };
    WorldCameraPose {
        origin: pivot.lerp(wanted, reach).to_array(),
        angles,
    }
}
