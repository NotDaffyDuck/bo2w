//! The host's scene lists, re-exported from `render_scene`.

pub use render_scene::{
    InsertBrushModelArgs, BrushModelPlacement, AnimatedObjectSpawn, DynObjectPlacementArgs, VisualStage, SceneBrushInstance, VisualStagePuppet,
    SceneModelInstance, SceneInsertFault, SceneInsertCategory, StageActorSkinnedFacets,
};
pub use render_scene::{
    BRUSH_SLOTS_PER_FRAME, ANIM_OBJECT_SLOTS_PER_FRAME, RENDER_ENTITY_SLOTS_PER_FRAME,
    SLOT_INDEX_NONE, MODEL_SLOTS_PER_FRAME, FIRST_PERSON_WEAPON_SLOT,
    FIRST_PERSON_WEAPON_EFFECT_BITS, FIRST_PERSON_OFFHAND_SLOT, pack_stage_word,
    stage_info_actor_num,
};
