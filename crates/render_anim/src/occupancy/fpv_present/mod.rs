//! The first-person arms and gun: spawning the viewmodel for the held weapon,
//! ticking its animation, skinning it and placing it in front of the camera.
//!
//! Each frame runs in this order (see [`register`]): the view-kick spring and
//! camera first, then a queued spawn, then the pose tick, then geometry,
//! placement and the bone poses that effects attach to.

mod geometry;
mod occupy;
mod placement;
mod register;
mod session;
mod spawn;
mod tick;
mod weapon;

use bevy::prelude::*;

pub use self::geometry::{publish_fpv_dobj_pose, stamp_fpv_placement_matrix};
pub use self::occupy::occupy_fpv_scene;
pub use self::placement::apply_fpv_placement;
pub use self::register::register_fpv_present_systems;
pub use self::session::{
    FpvState, FpvStatusGap, LocalSpawnArmed, SessionFpvMeshesHandles, SessionViewmodel,
};
pub use self::spawn::{SpawnPendingFpvInputs, spawn_pending_fpv};
pub use self::tick::tick_fpv_viewmodel;
pub(crate) use self::weapon::fpv_viewmodel_weapon;

pub use crate::anim::fpv_host::{
    FpvBoltTargets, FpvHeldLife, FpvHeldSettled, FpvPoseProduct, FpvPresentCursor,
    PendingFpvNotetracks, PendingFpvSpawn, PendingFpvSpawnRequest,
};

/// Placement of the viewmodel root is written this frame (matrix stamped).
#[derive(SystemSet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FpvPlacementSet;

/// The first-person vertices for this frame are written. The merge downstream
/// orders itself after this set.
#[derive(SystemSet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FpvGeometrySet;

/// Marks the viewmodel entity.
#[derive(Component)]
pub struct FpvViewmodel;

/// The entity under the camera whose transform places the viewmodel.
#[derive(Component)]
pub struct FpvPlacementRoot;
