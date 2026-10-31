//! Presented models: the posed DObjs, first-person viewmodel, remote bodies,
//! items, missiles and script models, and the draw plans they publish for the
//! renderer.

mod guest_gravgun;
mod plan_fill;
mod plugin;

pub mod motion;
pub mod strokes;
pub mod holes;
pub mod geometry;
pub mod guest_weapons;
pub mod illum;
pub mod occupancy;

pub use motion::*;
pub use strokes::*;
pub use plan_fill::*;
pub use lighting_t6::held_view_light_point;
pub use plugin::BodyAnimationPlugin;

pub use motion::puppet_pose::{
    HostSkeletonFrame, HostPoseDecline, AnimatedBody, AnimatedBodyFrame, AnimatedHeadMode,
};
pub use motion::gun_host::{
    AttachTagGoals, GunPoseInputs, HoldLifeStamp, HoldSettledLife, ViewmodelPoseVariety, GunPoseOutput,
    GunPoseDecline, GunPoseSnapshot, FpvShownCursor, QueuedGunNotes, WaitingFpvSpawn,
    QueuedGunSpawn, generate_fpv_posture,
};
pub use motion::fpv_prepared::{
    FpvHandsOverride, FpvPreparationCensus, FpvRigSet, FpvWeaponSlot, FpvWeaponTable,
    FpvWeaponView, PreparedFpv, prepare_fpv_compositions,
};
pub use motion::model_materials::{PreparedModelMaterials, prepare_model_materials};
pub use motion::gun_rig_ctl::{
    StepOutcome, EventReaction, FirstPersonGunAnimator, GunAnimEvent, GunPhase,
};
pub use illum::{
    attach_bone, base_trans, viewmodel_light_extent, gun_model_reach, radius_lighting_box_half,
    model_light_offset, script_prop_light_extent,
};
pub use occupancy::{
    DebrisRoomBits, PropPhysicsShape, DebrisPhysLevel, FpvGeometrySet, GunMountAnchor,
    GunMountStage, GunOffset, OwnSpawnPrimed, QueuedHurtFlinch, OtherBodyEffectTags, OtherPlayerBody,
    DrawFocus, ScriptPropDrawStage, ScriptPropSkinStage, ActiveGunMeshes, populate_gun_scene,
    spawn_viewmodel, stamp_matrix, sync_camera_from_shown, tick_viewmodel,
};
