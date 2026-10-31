//! Render frontend. Each frame it turns game state into what the renderer
//! draws: draw-surface lists, worker commands and scene occupancy. Nothing in
//! this crate talks to the GPU.

extern crate self as render_frontend;

pub mod adapters;
pub mod assemble;
pub mod prepare;

pub use adapters::DrawAdaptersPlugin;
pub use assemble::DrawAssemblePlugin;
pub use prepare::DrawPreparePlugin;

// Draw surfaces and packing.
pub use self::assemble::draw_items::*;
pub use assemble::packed_word::{
    RIGID_MODEL_TESS_FLAGS_PACKED_BRANCH, PackableDrawCall, BundleKind,
    pack_cone_shadow_frontend, pack_daylight_shadow_frontend,
};

// Scene occupancy: what stands where, within which limits.
pub use prepare::visual_stage::{
    BRUSH_SLOTS_PER_FRAME, ANIM_OBJECT_SLOTS_PER_FRAME, RENDER_ENTITY_SLOTS_PER_FRAME,
    SLOT_INDEX_NONE, MODEL_SLOTS_PER_FRAME, FIRST_PERSON_WEAPON_SLOT,
    FIRST_PERSON_WEAPON_EFFECT_BITS, FIRST_PERSON_OFFHAND_SLOT, InsertBrushModelArgs,
    BrushModelPlacement, AnimatedObjectSpawn, DynObjectPlacementArgs, VisualStage,
    SceneBrushInstance, VisualStagePuppet, SceneModelInstance, SceneInsertFault,
};
pub use self::prepare::scene::world::*;

// Sun shadow casters.
pub use prepare::daylight::{add_level_tree_daylight_shadow_partition, pack_daylight_shadow_caster_lists};

// Worker command rings.
pub use prepare::task_orders::{
    AddTaskOrder, RoomViewVolumeTaskOrder, SightActorTaskOrder, SkinCachedPropOrder,
    TaskOrderBusyInput, TaskOrderError, TaskOrderQueues, enqueue_room_view_volume_orders,
};
pub use prepare::task_orders::{
    TASK_ORDER_BOUND_ACTOR, TASK_ORDER_ROOM_DEBRIS_SOLID, TASK_ORDER_ROOM_DEBRIS_MODEL,
    TASK_ORDER_ROOM_STAGE_ACTOR, TASK_ORDER_TALLY, TASK_ORDER_SIGHT_ACTOR, TASK_ORDER_SMODELCACHE,
    TASK_ORDER_CONE_SHADOW_ACTOR,
};
