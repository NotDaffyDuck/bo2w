//! Black Ops II's XAnim playback, from `facts/xanim-dobj-t6.md` sections 2,
//! 3, 4 and 8:
//!
//! - [`quat2`] / [`quat4`] / [`unpack_trans`]: the stored samples as floats;
//! - [`time_index`] / [`time_index_dense`]: `XAnim_GetTimeIndex`, the key
//!   pair a frame position falls in, and [`snap_jumpcut`] for "jumpcut"
//!   notetracks;
//! - [`advance_leaf_time`]: one leaf's time step, with the loop wrap, the
//!   non-loop end park and the backwards guard;
//! - [`fired_notes`]: which notetracks one update fires, with the synthetic
//!   "end";
//! - [`rel_delta_trans`]: the root-motion move over one update;
//! - [`SkeletalClip`]: one XAnimParts decoded for playback ([`SkeletalClip::from_parts`]),
//!   built on the pieces above;
//! - [`AnimGraphInstance`]: a tree of clips with goal weights and rates, stepped
//!   per frame;
//! - [`DObjAnim`]: a clip's tracks mapped onto a DObj's parts, and the anims
//!   blended into its local pose;
//! - [`pose_dobj`] / [`SkeletonAnimState`]: a DObj posed from its tree's active
//!   leaves, and the model collision built from that pose;
//! - [`DObjSemanticState`]: a DObj's models and anim tree described by name.

mod cut;
mod delta;
mod dobj_anim;
mod skel_reuse;
mod notes;
mod kept_pose;
mod sample;
mod pose_desc;
mod time;
mod tree;

pub use cut::{
    SkeletalClip, ClipLoadFault, ClipEventMark, KeyframeIds, TimedValue, StoredRootMotion, StoredClipParts, Rotation,
    EvaluatedChannel, Track, RootShift,
};
pub use delta::*;
pub use dobj_anim::{ClipPlayback, DObjAnim};
pub use skel_reuse::{SkeletonAnimState, SkeletonCacheKey, model_token, reuse_matches};
pub use notes::*;
pub use kept_pose::{
    BoneHitShape, SurfaceHitMesh, HitTriangle, HitShapeBone, CollmapBox, ModelPoseQuery,
    RebuildFault, ModelCollmap, KeptModelAbility, collision_bone_from_local_box,
    hit_skeleton_with_aim, hit_shape_models, hit_shape_models_with_aim, pose_dobj,
    pose_skeleton_with_aim,
};
pub use sample::*;
pub use pose_desc::{
    RigLayout, RigMeshEntry, DObjRagdoll, RigPoseRecord,
    RoleLookupFault, TagOffset, GraphNodeRecord, AnimGraphRole,
    GraphCapture,
};
pub use time::*;
pub use tree::{
    AnimGraphNodeSpec, GraphSlot, AnimGraphNodeType, NodeClock, AnimGraphBlueprint,
    AnimGraphFault, AnimGraphInstance,
};
