//! Registers every resource the scene crate defines, so a reader never finds one missing.

use bevy::prelude::*;

use crate::ConeShadowStageOccupancy;
use crate::DetailRampSkinnedTunable;
use crate::EntitySkinningInputSet;
use crate::EntitySurfaceMemo;
use crate::ExportedCellVisibility;
use crate::FrozenVisibilitySet;
use crate::GameplayCamera;
use crate::HostVisualStage;
use crate::LevelPresentFacts;
use crate::LevelSightRooms;
use crate::LooseAtSpotLookup;
use crate::ModelIllumRequests;
use crate::ModelLightingAtlasTileWrites;
use crate::GunNearClipSetting;
use crate::NearClipSetting;
use crate::ResolvedModelIllumTable;
use crate::SmEnableDvar;
use crate::SmSunEnableDvar;
use crate::SpotShadowEntityOriginLog;
use crate::StagedStageEye;
use crate::StitchMaterials;

pub struct SceneRenderingModule;

impl Plugin for SceneRenderingModule {
    fn build(&self, app: &mut App) {
        occupancy_tables(app);
        lighting_tables(app);
        view_state(app);
    }
}

/// Who is in the scene this frame and what each entity is drawn with.
fn occupancy_tables(app: &mut App) {
    app.init_resource::<HostVisualStage>();
    app.init_resource::<EntitySkinningInputSet>();
    app.init_resource::<EntitySurfaceMemo>();
    app.init_resource::<ConeShadowStageOccupancy>();
    app.init_resource::<SpotShadowEntityOriginLog>();
}

/// Lighting asked for, lighting found, and where the world's cells and
/// materials stand.
fn lighting_tables(app: &mut App) {
    app.init_resource::<ModelIllumRequests>();
    app.init_resource::<ResolvedModelIllumTable>();
    app.init_resource::<ModelLightingAtlasTileWrites>();
    app.init_resource::<LooseAtSpotLookup>();
    app.init_resource::<LevelSightRooms>();
    app.init_resource::<ExportedCellVisibility>();
    app.init_resource::<StitchMaterials>();
    app.init_resource::<LevelPresentFacts>();
}

/// The camera, the eye built from it, and the settings that shape the view.
fn view_state(app: &mut App) {
    app.init_resource::<GameplayCamera>();
    app.init_resource::<StagedStageEye>();
    app.init_resource::<FrozenVisibilitySet>();
    app.init_resource::<NearClipSetting>();
    app.init_resource::<GunNearClipSetting>();
    app.init_resource::<DetailRampSkinnedTunable>();
    app.init_resource::<SmEnableDvar>();
    app.init_resource::<SmSunEnableDvar>();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_plugin_registers_the_resources_readers_ask_for() {
        let mut app = App::new();
        app.add_plugins(SceneRenderingModule);
        let world = app.world();
        assert!(world.contains_resource::<HostVisualStage>());
        assert!(world.contains_resource::<StagedStageEye>());
        assert!(world.contains_resource::<ResolvedModelIllumTable>());
        assert!(world.contains_resource::<SmSunEnableDvar>());
        assert!(world.contains_resource::<LevelSightRooms>());
    }
}
