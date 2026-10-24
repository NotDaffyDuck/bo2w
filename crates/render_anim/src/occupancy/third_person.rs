use asset_world::ClipCollision;
use net::PresentedSnapshot;
use playerstate_t6::game::{
    CG_CAMERA_PULLBACK_BOX_HALF, CG_CAMERA_PULLBACK_CLIPMASK, CG_THIRD_PERSON_RANGE_DEFAULT,
    KillCamMode, OffsetThirdPersonViewInputs, ThirdPersonViewInputs, is_third_person_view,
    offset_third_person_view,
};
use sim::ClientId;

use render_scene::WorldCameraPose;

/// bo2mp: Black Ops II's multiplayer script sets the death-watch camera angle
/// to 354 (the old 356 was off by two degrees).
pub const CG_THIRD_PERSON_ANGLE_MP: f32 = 354.0;

pub fn presented_is_third_person(
    presented: &PresentedSnapshot,
    local: ClientId,
    in_killcam: bool,
) -> bool {
    let Some(ps) = presented.player(local) else {
        return false;
    };
    if in_killcam && ps.kill_cam_entity != playerstate_t6::game::ENTITYNUM_NONE {
        return true;
    }
    if remote_missile_camera(presented, local, 0).is_some()
        || vehicle_view(presented, local).is_some()
    {
        return true;
    }
    is_third_person_view(ThirdPersonViewInputs {
        pm_type: ps.pm_type,
        other_flags: ps.other_flags,
        link_flags: ps.link_flags,
        // bo2mp: a script's setclientthirdperson (his client dvar).
        cg_third_person: presented
            .snapshot()
            .and_then(|s| s.meta.for_client(local))
            .is_some_and(|m| m.client_dvars.iter().any(|(k, v)| k == "cg_thirdperson" && v == "1")),
        in_killcam,
        killcam_mode: KillCamMode::Mode0,
    })
}

pub fn remote_missile_camera(
    presented: &PresentedSnapshot,
    local: ClientId,
    at_time: i32,
) -> Option<WorldCameraPose> {
    let link = presented
        .snapshot()?
        .meta
        .for_client(local)?
        .remote_missile
        .filter(|link| link.unlink_at_ms.is_none())?;
    let missile = presented
        .presented_projectiles()
        .iter()
        .find(|p| p.authoritative_id() == Some(link.projectile))?;
    Some(WorldCameraPose {
        origin: missile.origin_at(at_time),
        angles: link.angles,
    })
}

/// bo2mp: riding a Black Ops II scorestreak vehicle (`bo2mp_vehicle_view`
/// from the server, the vehicle def's own values): "<script model> 0
/// <range> <height> 0 <up> <down> <fov>" sees it from behind (the RC-XD),
/// "<script model> 1 <x> <y> <z> <up> <down> <fov>" from its `tag_player`
/// (the Dragonfire, the VTOL Warship's gun); then its entity number, and a
/// driven one's camera spring (distance, seconds) and top speed.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct VehicleView {
    pub model: u32,
    pub first_person: bool,
    /// Third person: range, height. First person: the tag in model space.
    pub at: [f32; 3],
    /// How far he may look up and down.
    pub pitch: [f32; 2],
    /// Its field of view (0: his own).
    pub fov: f32,
    /// Its entity number (its gun's fire events).
    pub number: i32,
    /// `thirdPersonCameraSpringDistance`, `...SpringTime`, and its top speed.
    pub spring: [f32; 2],
    pub max_speed: f32,
}

pub fn vehicle_view(presented: &PresentedSnapshot, local: ClientId) -> Option<VehicleView> {
    let snapshot = presented.snapshot()?;
    let text = snapshot
        .meta
        .script_dvars(local)
        .string("bo2mp_vehicle_view")?;
    let mut parts = text.split_whitespace();
    let model = parts.next()?.parse().ok()?;
    let mode: u8 = parts.next()?.parse().ok()?;
    let mut num = || {
        parts
            .next()
            .and_then(|v| v.parse::<f32>().ok())
            .unwrap_or(0.0)
    };
    let at = [num(), num(), num()];
    let pitch = [num(), num()];
    let fov = num();
    let number = num() as i32;
    let spring = [num(), num()];
    let max_speed = num();
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

/// His view riding the vehicle whose model sits at `origin`, turned by
/// `rotation`: from its tag (first person), or the vehicle's third-person
/// camera - `range` back from a point `height` over it, along his view,
/// stopped by walls. `spring` is how much further back its speed has
/// pushed the camera ([`VehicleSpring`]). `look` is his mouse's aim (the
/// command's angles): dead, his own view stops where he fell (turned to his
/// killer) while the car drives on his mouse, so its camera follows that.
pub fn vehicle_camera(
    presented: &PresentedSnapshot,
    local: ClientId,
    view: VehicleView,
    origin: [f32; 3],
    rotation: bevy::math::Quat,
    clip: Option<&ClipCollision>,
    spring: f32,
    look: Option<[i32; 3]>,
) -> WorldCameraPose {
    let mut angles = presented.player(local).map_or([0.0; 3], |ps| match look {
        Some(a)
            if matches!(
                ps.pm_type,
                playerstate_t6::game::PM_TYPE_DEAD | playerstate_t6::game::PM_TYPE_DEAD_LINKED
            ) =>
        {
            std::array::from_fn(|i| {
                let d = (a[i] & 0xffff) as f32 * (360.0 / 65536.0) + ps.delta_angles[i];
                (d + 180.0).rem_euclid(360.0) - 180.0
            })
        }
        _ => ps.viewangles,
    });
    angles[2] = 0.0;
    if view.pitch != [0.0; 2] {
        // Its limits follow the vehicle's own pitch along his view (a car
        // climbing a slope lets him look further up).
        let tilt = if view.first_person {
            0.0
        } else {
            let (fwd, _, _) = game_math::angle_vectors(angles);
            let flat = bevy::math::Vec3::new(fwd[0], fwd[1], 0.0).normalize_or_zero();
            90.0 - (rotation * bevy::math::Vec3::Z).dot(flat).clamp(-1.0, 1.0).acos().to_degrees()
        };
        angles[0] = angles[0].clamp(-view.pitch[0] + tilt, view.pitch[1] + tilt);
    }
    let o = bevy::math::Vec3::from_array(origin);
    if view.first_person {
        let eye = o + rotation * bevy::math::Vec3::from_array(view.at);
        return WorldCameraPose {
            origin: eye.to_array(),
            angles,
        };
    }
    let [range, height, _] = view.at;
    let range = range + spring;
    let look_at = o + rotation * bevy::math::Vec3::new(0.0, 0.0, height);
    let (fwd, _, _) = game_math::angle_vectors(angles);
    let want = look_at - bevy::math::Vec3::from_array(fwd) * range;
    let half = VEHICLE_CAMERA_BOX_HALF;
    let fraction = clip.map_or(1.0, |clip| {
        clip.sweep_box(
            look_at.to_array(),
            want.to_array(),
            [-half; 3],
            [half; 3],
            CG_CAMERA_PULLBACK_CLIPMASK,
        )
        .fraction
        .clamp(0.0, 1.0)
    });
    WorldCameraPose {
        origin: look_at.lerp(want, fraction).to_array(),
        angles,
    }
}

/// A driven vehicle's camera falling back with its speed, as Black Ops'
/// third-person vehicle camera does: toward `springDistance` times its
/// share of top speed, each frame by (springDistance / springTime ms) /
/// frame seconds times that share, at least a tenth.
#[derive(Default)]
pub struct VehicleSpring {
    model: Option<u32>,
    last: Option<bevy::math::Vec3>,
    /// Its forward speed, smoothed over the snapshots' steps.
    speed: f32,
    offset: f32,
}

impl VehicleSpring {
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    /// The camera's extra distance this frame, the vehicle at `origin`
    /// turned by `rotation`, `dt` seconds after the last.
    pub fn step(&mut self, view: &VehicleView, origin: bevy::math::Vec3, rotation: bevy::math::Quat, dt: f32) -> f32 {
        if self.model != Some(view.model) {
            *self = Self { model: Some(view.model), ..Self::default() };
        }
        let [dist, time] = view.spring;
        let last = self.last.replace(origin);
        if dist.abs() < 0.001 || time <= 0.0 || view.max_speed <= 0.0 || dt <= 0.0 {
            return self.offset;
        }
        if let Some(last) = last {
            let fwd = (origin - last).dot(rotation * bevy::math::Vec3::X) / dt;
            self.speed += (fwd - self.speed) * (dt / 0.1).min(1.0);
        }
        let ratio = (self.speed / view.max_speed).clamp(0.0, 1.0);
        let max = dist.abs();
        let timed = ((max / (time * 1000.0)) / dt * ratio).clamp(0.1, 1.0);
        let target = (dist * ratio).clamp(-max, max);
        self.offset = (1.0 - timed) * self.offset + target * timed;
        self.offset
    }
}

/// The vehicle camera's box against walls (the engine's own size).
const VEHICLE_CAMERA_BOX_HALF: f32 = 6.0;

pub fn death_watch_camera(
    presented: &PresentedSnapshot,
    local: ClientId,
    clip: Option<&ClipCollision>,
    corpse_j_mainroot: Option<[f32; 3]>,
) -> Option<WorldCameraPose> {
    let ps = presented.player(local)?;
    let clip = clip?;
    let offset = presented.view_offset();
    let snapshot = presented.snapshot()?;
    // The way to his killer comes with the snapshot that has him dead; on
    // the frame he dies his own state can be a snapshot ahead of it, and
    // the camera swung to his other side for that frame: it holds instead.
    if ps.pm_type >= playerstate_t6::game::PM_TYPE_DEAD
        && snapshot
            .players
            .iter()
            .find(|(c, _)| *c == local)
            .is_some_and(|(_, s)| s.pm_type < playerstate_t6::game::PM_TYPE_DEAD)
    {
        return None;
    }
    let yaw = snapshot
        .meta
        .for_client(local)
        .map(|meta| meta.look_at_killer_yaw as f32)
        .unwrap_or(ps.viewangles[1]);

    let half = CG_CAMERA_PULLBACK_BOX_HALF;
    let trace = |start: [f32; 3], end: [f32; 3]| {
        clip.sweep_box(
            start,
            end,
            [-half, -half, -half],
            [half, half, half],
            CG_CAMERA_PULLBACK_CLIPMASK,
        )
        .fraction
    };
    let view = offset_third_person_view(
        OffsetThirdPersonViewInputs {
            origin: [
                ps.origin[0] + offset[0],
                ps.origin[1] + offset[1],
                ps.origin[2] + offset[2],
            ],
            view_height_current: ps.view_height_current,
            viewangles: ps.viewangles,
            pm_type: ps.pm_type,
            look_at_killer_yaw: yaw,

            corpse_j_mainroot,
            other_flags: ps.other_flags,
            delta_time: ps.delta_time,
            cg_third_person_angle: CG_THIRD_PERSON_ANGLE_MP,
            cg_third_person_range: CG_THIRD_PERSON_RANGE_DEFAULT,
        },
        trace,
    );
    Some(WorldCameraPose {
        origin: view.origin,
        angles: view.angles,
    })
}
