//! Bevy plugin that registers the effect resources and systems.

use bevy::app::{App, Plugin};

use crate::{
    BattleEffectsSnapshot, EntityMarkBoard, EffectsEyePosition, FxCodeDrawPlan, EffectDumpRequest,
    EffectJournalCursor, EffectStainTunables, FxModelDrawList, FxCloudDrawPlan,
    EffectLevelColorImages, VisualStainMeshPlan, EffectsDynamicLightSet, PostEffectLightSet,
    EffectsSystemHandle, LaserSettings, ReadyFxBook, PreparedFxElemTable, ReadyFxProps,
    ReadyHitFx, StagedStreaks, TracerRenderGuard, StreakLevel, system,
};

/// Adds every resource the effects pipeline reads or writes, then its systems.
pub struct EffectsRenderPlugin;

impl Plugin for EffectsRenderPlugin {
    fn build(&self, app: &mut App) {
        // Output plans.
        app.init_resource::<FxCodeDrawPlan>();
        app.init_resource::<FxCloudDrawPlan>();
        app.init_resource::<VisualStainMeshPlan>();
        app.init_resource::<FxModelDrawList>();

        // Prepared inputs.
        app.init_resource::<ReadyFxBook>();
        app.init_resource::<ReadyFxProps>();
        app.init_resource::<PreparedFxElemTable>();
        app.init_resource::<ReadyHitFx>();
        app.init_resource::<StagedStreaks>();

        // Host-side state and settings.
        app.init_resource::<EffectsSystemHandle>();
        app.init_resource::<EffectsDynamicLightSet>();
        app.init_resource::<PostEffectLightSet>();
        app.init_resource::<EffectJournalCursor>();
        app.init_resource::<EffectsEyePosition>();
        app.init_resource::<EffectStainTunables>();
        app.init_resource::<LaserSettings>();
        app.init_resource::<EffectLevelColorImages>();

        // Diagnostics, tracers, vehicles and entity marks.
        app.init_resource::<BattleEffectsSnapshot>();
        app.init_resource::<EffectDumpRequest>();
        app.init_resource::<TracerRenderGuard>();
        app.init_resource::<StreakLevel>();
        app.init_resource::<EntityMarkBoard>();

        system::install_effect_coordination(app);
    }
}
