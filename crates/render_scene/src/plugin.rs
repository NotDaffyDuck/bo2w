//! Registers every resource the scene crate defines, so a reader never finds one missing.

use bevy::prelude::*;

use crate::{
    DynAtPointLookup, HostGfxScene, LodRampSkinnedDvar, ModelLightingAtlasTileWrites,
    ModelLightingRequests, PreparedSceneView, PublishedCellVis, RLockPvs, RZnearDepthhackDvar,
    RZnearDvar, ResolvedModelLightingTable, SceneEntSkinInputs, SceneEntSurfaceCache, SimCamera,
    SmEnableDvar, SmSunEnableDvar, SpotShadowEntityOriginTrack, SpotShadowSceneOccupancy,
    TessMaterials, WorldDpvsCells, WorldPresentFacts,
};

pub struct RenderScenePlugin;

impl Plugin for RenderScenePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<HostGfxScene>()
            .init_resource::<SceneEntSkinInputs>()
            .init_resource::<SceneEntSurfaceCache>()
            .init_resource::<SpotShadowSceneOccupancy>()
            .init_resource::<SpotShadowEntityOriginTrack>()
            .init_resource::<ModelLightingRequests>()
            .init_resource::<ResolvedModelLightingTable>()
            .init_resource::<ModelLightingAtlasTileWrites>()
            .init_resource::<DynAtPointLookup>()
            .init_resource::<WorldDpvsCells>()
            .init_resource::<PublishedCellVis>()
            .init_resource::<TessMaterials>()
            .init_resource::<WorldPresentFacts>()
            .init_resource::<SimCamera>()
            .init_resource::<PreparedSceneView>()
            .init_resource::<RLockPvs>()
            .init_resource::<RZnearDvar>()
            .init_resource::<RZnearDepthhackDvar>()
            .init_resource::<LodRampSkinnedDvar>()
            .init_resource::<SmEnableDvar>()
            .init_resource::<SmSunEnableDvar>();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_plugin_registers_the_resources_readers_ask_for() {
        let mut app = App::new();
        app.add_plugins(RenderScenePlugin);
        let world = app.world();
        assert!(world.contains_resource::<HostGfxScene>());
        assert!(world.contains_resource::<PreparedSceneView>());
        assert!(world.contains_resource::<ResolvedModelLightingTable>());
        assert!(world.contains_resource::<SmSunEnableDvar>());
        assert!(world.contains_resource::<WorldDpvsCells>());
    }
}
