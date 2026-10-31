//! The chase camera: when the view leaves the player's eyes, and where the camera sits once it has.

use super::flag_bits::secondary_state_bits;
use super::kill_view::{DeathReplayStyle, filmed_from_outside};
use super::record::*;

/// How far behind the player the camera sits unless the player changed it.
pub const CHASE_CAM_DISTANCE_FALLBACK: f32 = 120.0;
/// Half the edge of the box the camera traces with while it backs away.
pub const CHASE_CAM_PROBE_HALF_EXTENT: f32 = 4.0;
/// The contents that stop the camera backing away.
pub const CHASE_CAM_RETREAT_MASK: u32 = 0x0281_0011;
/// Set on whatever the player is linked to when riding it puts the camera outside.
pub const LINK_FLAGS_FORCE_THIRD_PERSON: u32 = 1 << 2;

/// The camera never tips further down than this, in degrees.
const STEEPEST_PITCH: f32 = 45.0;
/// The camera's pivot sits this far above the eye.
const PIVOT_LIFT: f32 = 8.0;
/// The camera tilts by this share of the player's pitch.
const TILT_SHARE: f32 = 0.5;
/// The point the camera keeps in frame is this far ahead of the eye.
const AIM_POINT_REACH: f32 = 512.0;
/// When a wall stops the camera, it rises by up to this much for the distance it lost.
const RISE_WHEN_BLOCKED: f32 = 32.0;
/// Radians to degrees, negated because a positive pitch looks down.
const RADIANS_TO_PITCH: f32 = -57.2957795;
/// An `secondary_state_bits` bit that on its own puts the camera outside.
const FORCED_OUTSIDE: u32 = 1 << 1;

/// What decides whether the camera is outside the player's head this frame.
#[derive(Clone, Copy, Debug)]
pub struct ThirdPersonViewInputs {
    pub movement_mode: i32,
    pub secondary_state_bits: u32,
    pub attach_bits: u32,
    /// The player's own third-person setting.
    pub chase_cam_enabled: bool,
    pub replaying_final_death: bool,
    pub killcam_mode: DeathReplayStyle,
}

/// True when the camera is outside the head: the player is dead, a flag or a vehicle forces it, the kill cam films
/// from outside, or the player asked for it (which a kill-cam seat only honours while the kill cam runs).
pub fn is_third_person_view(view: ThirdPersonViewInputs) -> bool {
    let asked_for = view.chase_cam_enabled && (view.replaying_final_death || view.secondary_state_bits & secondary_state_bits::DEAD_KILLCAM_TPV == 0);
    view.movement_mode >= MOVE_KIND_DEAD
        || view.secondary_state_bits & FORCED_OUTSIDE != 0
        || asked_for
        || filmed_from_outside(view.replaying_final_death, view.killcam_mode)
        || view.attach_bits & LINK_FLAGS_FORCE_THIRD_PERSON != 0
}

/// The yaw a dead player's camera turns to: towards the killer if there is one, otherwise where the player looked.
/// Whole degrees, cut towards zero.
pub fn look_at_killer_yaw(killer_pos: Option<[f32; 3]>, own_pos: [f32; 3], own_yaw: f32) -> i32 {
    let yaw = killer_pos.map_or(own_yaw, |at| game_math::heading_of(at[0] - own_pos[0], at[1] - own_pos[1]));
    yaw as i32
}

/// What places the chase camera this frame.
#[derive(Clone, Copy, Debug)]
pub struct ChaseCamPlacementInputs {
    pub origin: [f32; 3],
    pub eye_height_now: f32,
    pub look_angles: [f32; 3],
    pub move_mode: i32,
    /// Where a dead player's camera turns ([`look_at_killer_yaw`]).
    pub killer_facing_yaw: f32,
    /// The corpse's root bone, when the body is in the world; a dead player's camera follows it.
    pub body_root_joint_pos: Option<[f32; 3]>,
    pub secondary_state_bits: u32,
    pub time_slice: i32,
    /// The player's camera swing around them, in degrees.
    pub chase_cam_yaw_offset: f32,
    pub chase_cam_back_distance: f32,
}

/// Where the chase camera ended up and which way it points.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ChaseView {
    pub origin: [f32; 3],
    pub angles: [f32; 3],
}

/// `start + (goal - start) * share` on each axis.
fn toward(start: [f32; 3], goal: [f32; 3], share: f32) -> [f32; 3] {
    core::array::from_fn(|k| start[k] + (goal[k] - start[k]) * share)
}

/// `start + heading * reach` on each axis.
fn along(start: [f32; 3], heading: [f32; 3], reach: f32) -> [f32; 3] {
    core::array::from_fn(|k| start[k] + heading[k] * reach)
}

/// Backs the camera from the eye to where it wants to be. If something is in the way it stops there, rises a little
/// for the distance it lost, and traces once more towards that raised spot. `reach` answers how much of a straight
/// move gets through (1 means all of it).
fn back_away(eye: [f32; 3], wanted: [f32; 3], mut reach: impl FnMut([f32; 3], [f32; 3]) -> f32) -> [f32; 3] {
    let got = reach(eye, wanted);
    if got == 1.0 {
        return wanted;
    }
    let mut raised = toward(eye, wanted, got);
    raised[2] += (1.0 - got) * RISE_WHEN_BLOCKED;
    toward(eye, raised, reach(eye, raised))
}

/// Places the chase camera: behind the eye (or a dead player's body), swung and tilted by the player's settings,
/// kept out of walls, and pitched to keep a point ahead of the player in frame.
///
/// # Panics
/// On a kill-cam seat that claims to be a live tick: seats are always built from archived records.
pub fn place_chase_camera(
    cam: ChaseCamPlacementInputs,
    trace_clear_share: impl FnMut([f32; 3], [f32; 3]) -> f32,
) -> ChaseView {
    assert!(
        cam.secondary_state_bits & secondary_state_bits::DEAD_KILLCAM_TPV == 0 || cam.time_slice != 0,
        "kill-cam seat on a live tick: only archived records become seats"
    );
    let dead = cam.move_mode >= MOVE_KIND_DEAD;
    let eye = match cam.body_root_joint_pos {
        Some(body) if dead => body,
        _ => [cam.origin[0], cam.origin[1], cam.origin[2] + cam.eye_height_now],
    };
    let yaw = if dead { cam.killer_facing_yaw } else { cam.look_angles[1] };
    let pitch = if cam.look_angles[0] > STEEPEST_PITCH { STEEPEST_PITCH } else { cam.look_angles[0] };
    let aim_point = along(eye, game_math::euler_basis([pitch, yaw, 0.0]).0, AIM_POINT_REACH);

    let cam_yaw = yaw - cam.chase_cam_yaw_offset;
    let behind = game_math::euler_basis([pitch * TILT_SHARE, cam_yaw, 0.0]).0;
    let pivot = [eye[0], eye[1], eye[2] + PIVOT_LIFT];
    let origin = back_away(eye, along(pivot, behind, -cam.chase_cam_back_distance), trace_clear_share);

    let [step_along_x, dy, dz]: [f32; 3] = core::array::from_fn(|k| aim_point[k] - origin[k]);
    let flat = libm::sqrtf(step_along_x * step_along_x + dy * dy).max(1.0);
    ChaseView { origin, angles: [libm::atan2f(dz, flat) * RADIANS_TO_PITCH, cam_yaw, 0.0] }
}

/// True when the first-person gun runs this frame: not for spectators, the end-of-match screen or the dead.
pub fn viewweapon_frame_runs(
    movement_mode: i32,
) -> bool {
    movement_mode < MOVE_KIND_DEAD && !matches!(movement_mode, MOVE_MODE_OBSERVER | MOVE_MODE_INTERLUDE)
}
