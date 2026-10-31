//! Effects rendering: host-side effect state, per-frame plans and the plugin that wires
//! them into the app.

pub mod battle_fx;
pub mod surface_build;
mod entity_marks;
pub mod shot_fx;
pub mod host;
pub mod fx_models;
mod model_draw;
pub(crate) mod load_plugin;
pub mod effect_play;
mod product;
pub mod system;
pub mod tracer;

pub use self::surface_build::*;
pub use self::entity_marks::{
    BoneSlot, BoundEntityMark, EntityMarkBoard, EntityMarkState, MarkPaint, MarkPlacement,
    PendingEntityMark, Refusals,
};
pub use self::shot_fx::impact_needs_client_trace;
pub use host::{
    BeamFate, BeamReport, ColorMaps, BattleEffectsSnapshot, CreateFxLedger, EffectsEyePosition, EffectDumpRequest,
    EffectJournalCursor, EffectStainTunables, EffectLevelColorImages, EffectsDynamicLightSet, PostEffectLightSet,
    EffectsSystemHandle, JournalCount, LaserSettings, MarkSwitch, PostLightFate, ReadyFxBook,
    PreparedFxElemTable, ReadyFxProps, ReadyHitFx, TracerSample,
};
pub use fx_models::push_effect_model_asset;
pub use self::model_draw::{
    DropTally, FxModelSurfaces, FxModelDrawList, FxModelStagingPlan, ModelDrop, PreparedFxModelPlan,
    FIGURE_OBJECT_ID_EFFECT_BASE,
};
pub use self::load_plugin::EffectsRenderPlugin;
pub use self::product::{FxFrameResult, FxFrameView, FxSkipCause, GeneratedFxFrame};
pub use tracer::{
    StagedStreaks, PendingTracerBeam, TracerRenderGuard, TracerSpawnRefusal, StreakLevel,
    beam_undivided_colors, step_tracer_beams, attempt_bullet_streak,
};
