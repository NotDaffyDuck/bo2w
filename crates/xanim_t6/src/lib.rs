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
//! - [`AnimClip`]: one XAnimParts decoded for playback ([`AnimClip::from_parts`]),
//!   built on the pieces above;
//! - [`XAnimTreeRuntime`]: a tree of clips with goal weights and rates, stepped
//!   per frame;
//! - [`DObjAnim`]: a clip's tracks mapped onto a DObj's parts, and the anims
//!   blended into its local pose;
//! - [`pose_dobj`] / [`DObjAnimRuntime`]: a DObj posed from its tree's active
//!   leaves, and the model collision built from that pose;
//! - [`DObjSemanticState`]: a DObj's models and anim tree described by name.

mod clip;
mod delta;
mod dobj_anim;
mod dobj_runtime;
mod notes;
mod retained;
mod sample;
mod semantic;
mod time;
mod tree;

pub use clip::{
    AnimClip, ClipError, ClipNotify, FrameIndices, Keyed, RawDeltaTrans, RawXAnimParts, Rotation,
    SampledTrack, Track, Translation,
};
pub use delta::*;
pub use dobj_anim::{AnimInstance, DObjAnim};
pub use dobj_runtime::{DObjAnimRuntime, DObjReuseKey, model_token, reuse_matches};
pub use notes::*;
pub use retained::{
    BoneCollision, CollSurfCollision, CollTri, CollisionBone, CollmapBox, DObjPoseRequest,
    MaterializeError, ModelCollmap, RetainedModelCapability, collision_bone_from_local_box,
    collision_dobj_with_controller, collision_models, collision_models_with_controller, pose_dobj,
    pose_dobj_with_controller,
};
pub use sample::*;
pub use semantic::{
    DObjCompositionDescriptor, DObjModelDescriptor, DObjSemanticState, SemanticResolveError,
    XAnimSemanticNode, XAnimSemanticNodeKind, XAnimTreeSnapshot,
};
pub use time::*;
pub use tree::{
    XAnimNodeDefinition, XAnimNodeId, XAnimNodeKind, XAnimNodeState, XAnimTreeDefinition,
    XAnimTreeError, XAnimTreeRuntime,
};
