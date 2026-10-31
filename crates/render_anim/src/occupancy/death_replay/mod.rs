//! Kill-cam camera. While the viewer watches how they died, this follows the
//! thing that killed them (a missile, a strike, a turret) from just behind and
//! keeps the killer's victim in frame.

use asset_world::{CollisionClipData, SolidsTraceContact};
use assets::StagedWeapons;
use bevy::prelude::*;
use entity_t6::flight::path_kind;
use entity_t6::{EntitySnapshot, evaluate_trajectory};
use net::ShownWorldFrame;
use playerstate_t6::game::{NO_ENTITY_SLOT, DeathReplayStyle, killcam_blend_until_ms};
use render_scene::WorldEyePlacement;
use sim::{ClientId, FlyingProjectile};

mod follow;
mod cam_geometry;
mod javelin;
mod views;
use self::cam_geometry::*;

mod tests;

/// cos 10 degrees and cos 30 degrees: the aim may drift up to 10 degrees from
/// the victim for free, and is pulled fully back onto the victim by 30.
const COS_AIM_FREE: f32 = 0.9848077893257141;
const COS_AIM_LIMIT: f32 = 0.8660253882408142;
/// Looking steeply down is eased back toward the victim between these two
/// vertical components, and fully replaced below the second.
const AIM_Z_EASE_START: f32 = -0.95;
const AIM_Z_EASE_END: f32 = -0.98;

const DEFAULT_FOV: f32 = 50.0;
const JAVELIN_FOV: f32 = 60.0;
/// Solid-world mask the camera and the glide estimate are swept against.
const CAMERA_CLIP_MASK: u32 = 0x811;
const CAMERA_HALF_BOX: f32 = 5.0;

/// A missile that is about to hit stops being followed and glides to a halt
/// instead, so the camera does not fly into the wall with it.
#[derive(Clone, Copy)]
struct Glide {
    started_at: i32,
    duration_ms: f32,
    from: Vec3,
    velocity: Vec3,
}

impl Glide {
    fn position(&self, now: i32) -> Vec3 {
        let elapsed = (now - self.started_at).max(0) as f32;
        let t = elapsed.min(self.duration_ms);
        let travelled = if self.duration_ms > 0.0 {
            t * (1.0 - 0.5 * t / self.duration_ms)
        } else {
            0.0
        };
        self.from + self.velocity * travelled
    }
}

#[derive(Default)]
pub struct KillcamCamera {
    followed: Option<i32>,
    mode: Option<DeathReplayStyle>,
    subject: Vec3,
    subject_angles: [f32; 3],
    victim: Vec3,
    resting_on_ground: bool,
    previous_subject: Option<(i32, Vec3)>,
    glide: Option<Glide>,
    entered_at: i32,
    previous_stamp: i32,
    last_pose: Option<WorldEyePlacement>,
    blend_until: i32,
}
