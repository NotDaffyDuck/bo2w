//! The first-person arms and gun: spawning the viewmodel for the held weapon,
//! ticking its animation, skinning it and placing it in front of the camera.
//!
//! Each frame runs in this order (see [`register`]): the view-kick spring and
//! camera first, then a queued spawn, then the pose tick, then geometry,
//! placement and the bone poses that effects attach to.

mod gun_mesh;
mod occupy;
mod gun_place;
mod register;
mod gun_status;
mod spawn;
mod tick;
mod weapon;

use bevy::prelude::*;

pub use self::gun_mesh::{publish_dobj_pose, stamp_matrix};
pub use self::occupy::populate_gun_scene;
pub use self::gun_place::place_viewmodel;
pub use self::register::wire_gun_display_systems;
pub use self::gun_status::{
    ViewmodelStatus, GunStatusShortfall, OwnSpawnPrimed, GunMeshHandleSet, ActiveGunMeshes,
};
pub use self::spawn::{GunSpawnSystemParams, spawn_viewmodel};
pub use self::tick::tick_viewmodel;
pub(crate) use self::weapon::fpv_viewmodel_weapon;

pub use crate::motion::gun_host::{
    AttachTagGoals, HoldLifeStamp, HoldSettledLife, GunPoseOutput, FpvShownCursor,
    QueuedGunNotes, WaitingFpvSpawn, QueuedGunSpawn,
};

/// Placement of the viewmodel root is written this frame (matrix stamped).
#[derive(SystemSet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct GunMountStage;

/// The first-person vertices for this frame are written. The merge downstream
/// orders itself after this set.
#[derive(SystemSet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FpvGeometrySet;

/// Marks the viewmodel entity.
#[derive(Component)]
pub struct GunViewLayer;

/// The entity under the camera whose transform places the viewmodel.
#[derive(Component)]
pub struct GunMountAnchor;
