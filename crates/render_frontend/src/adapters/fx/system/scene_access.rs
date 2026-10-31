//! The world the effect systems read: the spawned map and the marks left on its entities.

use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use render_fx::EntityMarkBoard;

use crate::adapters::fx::world_decal::FrontendEffectStage;
use crate::prepare::scene::world::LevelStage;

/// Both halves of the world an effect is placed against. Either may be missing early in a load.
#[derive(SystemParam)]
pub(super) struct EffectStageAccess<'w> {
    level: Option<Res<'w, LevelStage>>,
    mark_board: Res<'w, EntityMarkBoard>,
}

impl EffectStageAccess<'_> {
    pub(super) fn scene(&self) -> Option<&LevelStage> {
        self.level.as_deref()
    }

    pub(super) fn marks(&self) -> &EntityMarkBoard {
        &self.mark_board
    }

    /// The world as the effect host sees it, or `None` until a map is spawned.
    pub(super) fn view(&self) -> Option<FrontendEffectStage<'_>> {
        let marks = self.marks();
        FrontendEffectStage::wrap(self.scene(), marks)
    }
}
