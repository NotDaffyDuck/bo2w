//! Retained model runtime shared by the authority and the client: pose a
//! model's bones from its animation tree and read back its collision shapes.
//! The pieces live in `dobj_t6` and `xanim_t6`; this crate is the one door
//! both sides come through.

mod destructible;

pub use destructible::{DebrisPreset, T5DestructibleDef, T5DestructiblePiece, T5DestructibleStage};

// Bone sets, tag attachment and the player controller.
pub use dobj_t6::{
    AIM_PITCH_CLAMP_RAD, Attach, Bone, DObj, DObjBoneOrientation, DObjBoneOrientationError,
    DObjError, HidePartBits, LocalPose, ModelPoseSrc, ModelSlot, PLAYER_CONTROLLER_TAGS, PartBits,
    PartPose, PlayerControllerInput, PlayerControllerResult, TP_HEAD_ATTACH_TAG,
    TP_WEAPON_ATTACH_TAGS, apply_aim_pitches, apply_legs_yaw, apply_player_controller,
    apply_standing_player_controller, bone_orientation, build_body_head_weapon_dobj,
    build_body_weapon_dobj, empty_tag_attach, pitch_spine_bone, player_controller_tag_origin,
    set_angles, set_control_tag_angles, set_local_tag, tp_head_attach_tag, tp_weapon_attach_tag,
    yaw_bone,
};

// Sampled clips and the node tree that blends them.
pub use xanim_t6::{
    AnimClip, ClipError, ClipNotify, FrameIndices, Keyed, RawDeltaTrans, RawXAnimParts, Rotation,
    SampledTrack, Track, Translation,
};
pub use xanim_t6::{
    XAnimNodeDefinition, XAnimNodeId, XAnimNodeKind, XAnimNodeState, XAnimTreeDefinition,
    XAnimTreeError, XAnimTreeRuntime,
};

// A posed model and the collision it carries.
pub use xanim_t6::{AnimInstance, DObjAnim};
pub use xanim_t6::{
    BoneCollision, CollSurfCollision, CollTri, CollisionBone, CollmapBox, DObjAnimRuntime,
    DObjCompositionDescriptor, DObjModelDescriptor, DObjPoseRequest, DObjReuseKey,
    DObjSemanticState, MaterializeError, ModelCollmap, RetainedModelCapability,
    SemanticResolveError, XAnimSemanticNode, XAnimSemanticNodeKind, XAnimTreeSnapshot,
    collision_bone_from_local_box, collision_dobj_with_controller, collision_models,
    collision_models_with_controller, model_token, pose_dobj, pose_dobj_with_controller,
    reuse_matches,
};
