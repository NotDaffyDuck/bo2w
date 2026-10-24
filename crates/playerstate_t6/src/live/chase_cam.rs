//! The chase camera: when the view leaves the player's eyes, and where the camera sits once it has.

use game_math::{angle_vectors, vec_to_yaw};

use super::flag_bits::other_flags;
use super::killcam::{KillCamMode, filmed_from_outside};
use super::record::{PM_TYPE_DEAD, PM_TYPE_INTERMISSION, PM_TYPE_SPECTATOR};

/// How far behind the player the camera sits unless the player changed it.
pub const CG_THIRD_PERSON_RANGE_DEFAULT: f32 = 120.0;
/// Half the edge of the box the camera traces with while it backs away.
pub const CG_CAMERA_PULLBACK_BOX_HALF: f32 = 4.0;
/// The contents that stop the camera backing away.
pub const CG_CAMERA_PULLBACK_CLIPMASK: u32 = 0x0281_0011;
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
/// An `other_flags` bit that on its own puts the camera outside.
const FORCED_OUTSIDE: u32 = 1 << 1;

/// What decides whether the camera is outside the player's head this frame.
#[derive(Clone, Copy, Debug)]
pub struct ThirdPersonViewInputs {
    pub pm_type: i32,
    pub other_flags: u32,
    pub link_flags: u32,
    /// The player's own third-person setting.
    pub cg_third_person: bool,
    pub in_killcam: bool,
    pub killcam_mode: KillCamMode,
}

/// True when the camera is outside the head: the player is dead, a flag or a vehicle forces it, the kill cam films
/// from outside, or the player asked for it (which a kill-cam seat only honours while the kill cam runs).
pub fn is_third_person_view(view: ThirdPersonViewInputs) -> bool {
    let asked_for = view.cg_third_person && (view.in_killcam || view.other_flags & other_flags::DEAD_KILLCAM_TPV == 0);
    view.pm_type >= PM_TYPE_DEAD
        || view.other_flags & FORCED_OUTSIDE != 0
        || asked_for
        || filmed_from_outside(view.in_killcam, view.killcam_mode)
        || view.link_flags & LINK_FLAGS_FORCE_THIRD_PERSON != 0
}

/// The yaw a dead player's camera turns to: towards the killer if there is one, otherwise where the player looked.
/// Whole degrees, cut towards zero.
pub fn look_at_killer_yaw(attacker_origin: Option<[f32; 3]>, self_origin: [f32; 3], self_view_yaw: f32) -> i32 {
    let yaw = attacker_origin.map_or(self_view_yaw, |at| vec_to_yaw(at[0] - self_origin[0], at[1] - self_origin[1]));
    yaw as i32
}

/// What places the chase camera this frame.
#[derive(Clone, Copy, Debug)]
pub struct OffsetThirdPersonViewInputs {
    pub origin: [f32; 3],
    pub view_height_current: f32,
    pub viewangles: [f32; 3],
    pub pm_type: i32,
    /// Where a dead player's camera turns ([`look_at_killer_yaw`]).
    pub look_at_killer_yaw: f32,
    /// The corpse's root bone, when the body is in the world; a dead player's camera follows it.
    pub corpse_j_mainroot: Option<[f32; 3]>,
    pub other_flags: u32,
    pub delta_time: i32,
    /// The player's camera swing around them, in degrees.
    pub cg_third_person_angle: f32,
    pub cg_third_person_range: f32,
}

/// Where the chase camera ended up and which way it points.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ChaseView {
    pub origin: [f32; 3],
    pub angles: [f32; 3],
}

/// `from + (to - from) * share` on each axis.
fn toward(from: [f32; 3], to: [f32; 3], share: f32) -> [f32; 3] {
    core::array::from_fn(|k| from[k] + (to[k] - from[k]) * share)
}

/// `from + dir * reach` on each axis.
fn along(from: [f32; 3], dir: [f32; 3], reach: f32) -> [f32; 3] {
    core::array::from_fn(|k| from[k] + dir[k] * reach)
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
pub fn offset_third_person_view(
    i: OffsetThirdPersonViewInputs,
    trace_fraction: impl FnMut([f32; 3], [f32; 3]) -> f32,
) -> ChaseView {
    assert!(
        i.other_flags & other_flags::DEAD_KILLCAM_TPV == 0 || i.delta_time != 0,
        "kill-cam seat on a live tick: only archived records become seats"
    );
    let dead = i.pm_type >= PM_TYPE_DEAD;
    let eye = match i.corpse_j_mainroot {
        Some(body) if dead => body,
        _ => [i.origin[0], i.origin[1], i.origin[2] + i.view_height_current],
    };
    let yaw = if dead { i.look_at_killer_yaw } else { i.viewangles[1] };
    let pitch = if i.viewangles[0] > STEEPEST_PITCH { STEEPEST_PITCH } else { i.viewangles[0] };
    let aim_point = along(eye, angle_vectors([pitch, yaw, 0.0]).0, AIM_POINT_REACH);

    let cam_yaw = yaw - i.cg_third_person_angle;
    let behind = angle_vectors([pitch * TILT_SHARE, cam_yaw, 0.0]).0;
    let pivot = [eye[0], eye[1], eye[2] + PIVOT_LIFT];
    let origin = back_away(eye, along(pivot, behind, -i.cg_third_person_range), trace_fraction);

    let [dx, dy, dz]: [f32; 3] = core::array::from_fn(|k| aim_point[k] - origin[k]);
    let flat = libm::sqrtf(dx * dx + dy * dy).max(1.0);
    ChaseView { origin, angles: [libm::atan2f(dz, flat) * RADIANS_TO_PITCH, cam_yaw, 0.0] }
}

/// True when the first-person gun runs this frame: not for spectators, the end-of-match screen or the dead.
pub fn viewweapon_frame_runs(pm_type: i32) -> bool {
    pm_type < PM_TYPE_DEAD && !matches!(pm_type, PM_TYPE_SPECTATOR | PM_TYPE_INTERMISSION)
}
