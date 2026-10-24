mod def;
mod draw;
mod elem;
mod gaps;
mod glass;
mod lifetime;
mod marks;
mod motion;
mod orient;
mod play;
mod sort;
mod spawn;
mod system;
mod trail;
mod update;

pub use def::{FxEffectDefInfo, FxElemDefInfo};
pub use draw::{
    FxCloudInstance, FxDrawElemContext, FxDrawTrailContext, FxDrawTrailSampleContext,
    FxElemLightInstance, FxGenerateVertsOut, FxModelInstance, FxSpriteInstance, FxTrailDrawDef,
    FxTrailMeshInstance, FxTrailSampleVisual, generate_verts, generate_verts_with_trails,
};
pub use elem::FxElemSlot;
pub use gaps::{ChildSpawn, CodeMeshStep, FxGap, FxGapCause, FxGaps};
pub use glass::{FxGlassSystemHost, GlassWorldContact, GlassWorldTrace};
pub use glass_t6::{
    T6_GLASS_LOW_LOD_DIST, T6Glass, T6PaneShape, T6Shard, T6ShardCounts, T6ShardMotion,
    T6ShardVertex,
};
pub use lifetime::{
    FxMsec, LE_MOVING_TRACER, LE_TR_LINEAR, LOCAL_ENTITY_POOL_CAPACITY, LocalEntityPool,
    LocalEntitySlot, local_entity_is_live, set_presentation_clock, tracer_travel_msec,
};
pub use marks::{
    FxMarksSystemHost, GfxMarkMeshCensus, GfxMarkMeshSurf, MarkImpactRequest, MarkImpactResult,
    MarkReceiverEnable, MarkTraceRecord,
};
pub use motion::{evaluate_elem_collide_motion, evaluate_elem_motion};
pub use play::{
    FxPlayPose, FxPlayRequest, PlayResult, axis_from_hit_normal, play_bolted, play_oriented,
    spawn_impact_or_death_effect, spawn_oriented,
};
pub use spawn::{sort_effect_elems, spawn_looping_partial};
pub use system::{
    FX_CATALOG_INDEX_NONE, FxBoltOrientation, FxBoltTarget, FxEffectSlot, FxPackedLightingSrc,
    FxResolvedBoltPose, FxSystemHost, PendingDecalSpawn, PendingRunnerSpawn, PendingSoundSpawn,
    PendingTrailImpact, SpawnFail,
};
pub use trail::FxTrailCollideHit;
pub use update::{
    FxChildKind, FxChildSpawnRequest, FxElemMotionQuery, FxElemMotionResult, FxElemTraceHit,
    FxEmitQuery, FxImpactSpawn, PendingCollide, update,
};
