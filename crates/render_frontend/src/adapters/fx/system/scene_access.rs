//! The world the effect systems read: the spawned map and the marks left on its entities.

use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use render_fx::EntityMarks;

use crate::adapters::fx::world_mark::FrontendFxScene;
use crate::prepare::scene::world::WorldScene;

/// Both halves of the world an effect is placed against. Either may be missing early in a load.
#[derive(SystemParam)]
pub(super) struct FxSceneAccess<'w> {
    scene: Option<Res<'w, WorldScene>>,
    marks: Res<'w, EntityMarks>,
}

impl FxSceneAccess<'_> {
    pub(super) fn scene(&self) -> Option<&WorldScene> {
        self.scene.as_deref()
    }

    pub(super) fn marks(&self) -> &EntityMarks {
        &self.marks
    }

    /// The world as the effect host sees it, or `None` until a map is spawned.
    pub(super) fn view(&self) -> Option<FrontendFxScene<'_>> {
        FrontendFxScene::wrap(self.scene(), self.marks())
    }
}
