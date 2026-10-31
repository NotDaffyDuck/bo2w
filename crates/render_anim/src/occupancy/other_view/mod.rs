//! Cameras that sit outside the player's own eyes: the death watch, a
//! guided missile's nose, and a ridden vehicle.

use asset_world::CollisionClipData;
use bevy::math::{Quat, Vec3};
use game_math::euler_basis;
use net::ShownWorldFrame;
use playerstate_t6::game::{
    CHASE_CAM_PROBE_HALF_EXTENT, CHASE_CAM_RETREAT_MASK, CHASE_CAM_DISTANCE_FALLBACK,
    NO_ENTITY_SLOT, DeathReplayStyle, ChaseCamPlacementInputs, MOVE_KIND_DEAD, MOVE_KIND_DEAD_TETHERED,
    ThirdPersonViewInputs, is_third_person_view, place_chase_camera,
};
use render_scene::WorldEyePlacement;
use sim::ClientId;

mod death_watch;
mod remote_missile;
mod spring;
mod vehicle_cam;
pub use self::death_watch::*;
pub use self::remote_missile::*;
pub use self::vehicle_cam::*;

mod tests;

/// Yaw offset the multiplayer scripts give the death-watch camera.
pub const TP_CAMERA_ANGLE_MP: f32 = 354.0;

/// Half-extent of the box a vehicle camera sweeps against the world.
const VEHICLE_CAM_HALF: f32 = 6.0;

/// Dvar the server sets for a rider, as `"<model> <mode> ..."`.
const VEHICLE_VIEW_DVAR: &str = "bo2mp_vehicle_view";

/// Whether the local player's view is drawn from outside the body.
pub fn shown_in_third_person(
    shown_frame: &ShownWorldFrame,
    watcher: ClientId,
    replaying_final_death: bool,
) -> bool {
    let Some(pstate) = shown_frame.player(watcher) else {
        return false;
    };
    if replaying_final_death && pstate.death_replay_target != NO_ENTITY_SLOT {
        return true;
    }
    if remote_missile_camera(shown_frame, watcher, 0).is_some()
        || vehicle_view(shown_frame, watcher).is_some()
    {
        return true;
    }
    let script_forced = shown_frame
        .snapshot()
        .and_then(|snap| snap.meta.for_client(watcher))
        .is_some_and(|meta| {
            meta.client_dvars
                .iter()
                .any(|(name, value)| name == "cg_thirdperson" && value == "1")
        });
    is_third_person_view(ThirdPersonViewInputs {
        movement_mode: pstate.movement_mode,
        secondary_state_bits: pstate.secondary_state_bits,
        attach_bits: pstate.attach_bits,
        chase_cam_enabled: script_forced,
        replaying_final_death,
        killcam_mode: DeathReplayStyle::Mode0,
    })
}

/// How a scorestreak vehicle is ridden, from the server's view string:
/// `"<model> 0 <range> <height> 0 <up> <down> <fov> ..."` looks at the
/// vehicle from behind, `"<model> 1 <x> <y> <z> <up> <down> <fov> ..."`
/// looks out of a seat in it. Both continue with the vehicle's entity number,
/// the camera spring (distance, seconds) and the top speed of a driven one.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct VehicleView {
    pub xmodel: u32,
    pub first_person: bool,
    /// Seen from behind: range and height. From inside: the seat in model space.
    pub at: [f32; 3],
    /// Limits of looking up and down.
    pub pitch: [f32; 2],
    /// Field of view; 0 keeps the player's own.
    pub seat_fov: f32,
    /// Entity number, which the vehicle's gun events carry.
    pub number: i32,
    /// Spring distance and spring time of the follow camera.
    pub spring: [f32; 2],
    pub top_velocity: f32,
}

/// Lets a driven vehicle's follow camera drop back as it speeds up: the
/// camera heads for the spring distance scaled by the share of top speed
/// reached, closing the gap per frame by (distance / time) over the frame
/// length, times that share, never less than a tenth.
#[derive(Default)]
pub struct VehicleSpring {
    mdl: Option<u32>,
    prev_origin: Option<Vec3>,
    /// Forward speed, smoothed across snapshot steps.
    speed: f32,
    offset: f32,
}
