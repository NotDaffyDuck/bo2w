//! Presented models: the posed DObjs, first-person viewmodel, remote bodies,
//! items, missiles and script models, and the draw plans they publish for the
//! renderer.

mod draw_build;
mod guest_gravgun;
mod plugin;

pub mod anim;
pub mod draw;
pub mod gaps;
pub mod geometry;
pub mod guest_weapons;
pub mod lighting;
pub mod occupancy;

pub use anim::*;
pub use draw::*;
pub use draw_build::*;
pub use lighting_t6::viewmodel_lighting_origin;
pub use plugin::RenderAnimPlugin;

pub use anim::dobj_pose::{
    HostDObjPoseFrame, HostDObjPoseRefuse, PosedPlayer, PosedPlayerFrame, PosedPlayerHead,
};
pub use anim::fpv_host::{
    FpvBoltTargets, FpvGenerateArgs, FpvHeldLife, FpvHeldSettled, FpvPoseKind, FpvPoseProduct,
    FpvPoseRefuse, FpvPosedFrame, FpvPresentCursor, PendingFpvNotetracks, PendingFpvSpawn,
    PendingFpvSpawnRequest, generate_fpv_pose,
};
pub use anim::fpv_prepared::{
    FpvPreparationCensus, FpvRigSet, FpvWeaponSlot, FpvWeaponTable, FpvWeaponView, PreparedFpv,
    prepare_fpv_compositions,
};
pub use anim::model_materials::{PreparedModelMaterials, prepare_model_materials};
pub use anim::viewmodel_controller::{
    AdvanceResult, EventResult, ViewmodelController, ViewmodelEvent, WeaponState,
};
pub use lighting::{
    attach_bone, base_trans, fpv_dobj_lighting_box_half, fpv_dobj_radius, radius_lighting_box_half,
    script_model_lighting_box_half,
};
pub use occupancy::{
    DynEntCellBits, DynEntPhysClip, DynEntPhysWorld, FpvGeometrySet, FpvPlacementRoot,
    FpvPlacementSet, GunOffset, LocalSpawnArmed, PendingViewHurt, RemoteFxBolts, RemotePlayer,
    RenderFocus, ScriptModelDrawSet, ScriptModelSkinSet, SessionViewmodel, occupy_fpv_scene,
    spawn_pending_fpv, stamp_fpv_placement_matrix, sync_camera_from_presented, tick_fpv_viewmodel,
};
