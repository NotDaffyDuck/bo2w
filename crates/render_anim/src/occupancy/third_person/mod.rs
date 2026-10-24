//! Cameras that sit outside the player's own eyes: the death watch, a
//! guided missile's nose, and a ridden vehicle.

use asset_world::ClipCollision;
use bevy::math::{Quat, Vec3};
use game_math::angle_vectors;
use net::PresentedSnapshot;
use playerstate_t6::game::{
    CG_CAMERA_PULLBACK_BOX_HALF, CG_CAMERA_PULLBACK_CLIPMASK, CG_THIRD_PERSON_RANGE_DEFAULT,
    ENTITYNUM_NONE, KillCamMode, OffsetThirdPersonViewInputs, PM_TYPE_DEAD, PM_TYPE_DEAD_LINKED,
    ThirdPersonViewInputs, is_third_person_view, offset_third_person_view,
};
use render_scene::WorldCameraPose;
use sim::ClientId;

mod death_watch;
mod remote_missile;
mod spring;
mod vehicle;
pub use self::death_watch::*;
pub use self::remote_missile::*;
pub use self::vehicle::*;

#[cfg(test)]
mod tests;

/// Yaw offset the multiplayer scripts give the death-watch camera.
pub const CG_THIRD_PERSON_ANGLE_MP: f32 = 354.0;

/// Half-extent of the box a vehicle camera sweeps against the world.
const VEHICLE_CAM_HALF: f32 = 6.0;

/// Dvar the server sets for a rider, as `"<model> <mode> ..."`.
const VEHICLE_VIEW_DVAR: &str = "bo2mp_vehicle_view";

/// Whether the local player's view is drawn from outside the body.
pub fn presented_is_third_person(
    presented: &PresentedSnapshot,
    local: ClientId,
    in_killcam: bool,
) -> bool {
    let Some(ps) = presented.player(local) else {
        return false;
    };
    if in_killcam && ps.kill_cam_entity != ENTITYNUM_NONE {
        return true;
    }
    if remote_missile_camera(presented, local, 0).is_some()
        || vehicle_view(presented, local).is_some()
    {
        return true;
    }
    let script_forced = presented
        .snapshot()
        .and_then(|snap| snap.meta.for_client(local))
        .is_some_and(|meta| {
            meta.client_dvars
                .iter()
                .any(|(name, value)| name == "cg_thirdperson" && value == "1")
        });
    is_third_person_view(ThirdPersonViewInputs {
        pm_type: ps.pm_type,
        other_flags: ps.other_flags,
        link_flags: ps.link_flags,
        cg_third_person: script_forced,
        in_killcam,
        killcam_mode: KillCamMode::Mode0,
    })
}

/// How a scorestreak vehicle is ridden, from the server's view string:
/// `"<model> 0 <range> <height> 0 <up> <down> <fov> ..."` looks at the
/// vehicle from behind, `"<model> 1 <x> <y> <z> <up> <down> <fov> ..."`
/// looks out of a seat in it. Both continue with the vehicle's entity number,
/// the camera spring (distance, seconds) and the top speed of a driven one.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct VehicleView {
    pub model: u32,
    pub first_person: bool,
    /// Seen from behind: range and height. From inside: the seat in model space.
    pub at: [f32; 3],
    /// Limits of looking up and down.
    pub pitch: [f32; 2],
    /// Field of view; 0 keeps the player's own.
    pub fov: f32,
    /// Entity number, which the vehicle's gun events carry.
    pub number: i32,
    /// Spring distance and spring time of the follow camera.
    pub spring: [f32; 2],
    pub max_speed: f32,
}

/// Lets a driven vehicle's follow camera drop back as it speeds up: the
/// camera heads for the spring distance scaled by the share of top speed
/// reached, closing the gap per frame by (distance / time) over the frame
/// length, times that share, never less than a tenth.
#[derive(Default)]
pub struct VehicleSpring {
    model: Option<u32>,
    last_origin: Option<Vec3>,
    /// Forward speed, smoothed across snapshot steps.
    speed: f32,
    offset: f32,
}
