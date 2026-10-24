//! Clearing the world's draw plans when a match ends, so nothing from the old
//! world is drawn in the next one.

use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use frame::{MatchTornDown, SessionSwapApplied};
use net::ClientSet;

use super::drawsurf::fog::ScriptFogPresentation;
use super::drawsurf::{
    DrawMethodDfog, DrawSurfList, FxCodeMeshPlan, FxModelDrawPlan, FxParticleCloudPlan,
    GfxMarkMeshPlan, GlassTable, MapFrameFog,
};

/// Every plan built from the world that is being torn down.
#[derive(SystemParam)]
pub(crate) struct WorldDrawPlans<'w> {
    dfog: ResMut<'w, DrawMethodDfog>,
    draw_list: ResMut<'w, DrawSurfList>,
    glass: ResMut<'w, GlassTable>,
    fx_code: ResMut<'w, FxCodeMeshPlan>,
    particles: ResMut<'w, FxParticleCloudPlan>,
    marks: ResMut<'w, GfxMarkMeshPlan>,
    fx_models: ResMut<'w, FxModelDrawPlan>,
}

impl WorldDrawPlans<'_> {
    fn clear(&mut self) {
        self.dfog.0 = false;
        *self.draw_list = DrawSurfList::default();
        self.glass.reset();
        *self.fx_code = FxCodeMeshPlan::default();
        *self.particles = FxParticleCloudPlan::default();
        *self.marks = GfxMarkMeshPlan::default();
        *self.fx_models = FxModelDrawPlan::default();
    }
}

/// On any teardown message: drop the map fog and everything planned from the
/// old world.
pub(crate) fn reset_world_draw_plans_on_teardown(
    mut torn: MessageReader<MatchTornDown>,
    mut commands: Commands,
    mut plans: WorldDrawPlans,
) {
    if torn.read().next().is_none() {
        return;
    }
    commands.remove_resource::<MapFrameFog>();
    commands.insert_resource(ScriptFogPresentation::default());
    plans.clear();
}

pub(crate) fn register_match_reset_systems(app: &mut App) {
    app.add_systems(
        Update,
        reset_world_draw_plans_on_teardown
            .after(SessionSwapApplied)
            .after(crate::prepare::scene::spawn::reset_world_spawn_on_teardown)
            .before(crate::prepare::scene::spawn::shutdown_world_on_teardown)
            .in_set(ClientSet::Load),
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use asset_world::ExpFog;
    use frame::{MatchKey, TeardownReason, WorldGeneration};

    fn app() -> App {
        let mut app = App::new();
        app.add_message::<MatchTornDown>()
            .init_resource::<DrawMethodDfog>()
            .init_resource::<DrawSurfList>()
            .init_resource::<GlassTable>()
            .init_resource::<FxCodeMeshPlan>()
            .init_resource::<FxParticleCloudPlan>()
            .init_resource::<GfxMarkMeshPlan>()
            .init_resource::<FxModelDrawPlan>()
            .add_systems(Update, reset_world_draw_plans_on_teardown);
        app.insert_resource(DrawMethodDfog(true));
        app.insert_resource(MapFrameFog::new(ExpFog {
            start_dist: 1.0,
            halfway_dist: 2.0,
            color_rgb: [0.0; 3],
            max_opacity: 1.0,
            transition_time: 0.0,
            sun: None,
            volumetric: None,
        }));
        app.world_mut().resource_mut::<DrawSurfList>().keys = 5;
        app
    }

    fn tear_down(app: &mut App) {
        app.world_mut().write_message(MatchTornDown {
            reason: TeardownReason::MatchEnded,
            world_generation: WorldGeneration(Some(1)),
            match_key: MatchKey::default(),
            match_epoch: 0,
        });
    }

    #[test]
    fn nothing_changes_without_a_teardown() {
        let mut app = app();
        app.update();
        assert!(app.world().resource::<DrawMethodDfog>().0);
        assert_eq!(app.world().resource::<DrawSurfList>().keys, 5);
        assert!(app.world().contains_resource::<MapFrameFog>());
    }

    #[test]
    fn a_teardown_clears_the_fog_and_every_plan() {
        let mut app = app();
        tear_down(&mut app);
        app.update();
        assert!(!app.world().resource::<DrawMethodDfog>().0);
        assert_eq!(app.world().resource::<DrawSurfList>().keys, 0);
        assert!(!app.world().contains_resource::<MapFrameFog>());
        assert!(app.world().contains_resource::<ScriptFogPresentation>());
    }

    #[test]
    fn several_teardowns_in_a_frame_clear_once_and_cleanly() {
        let mut app = app();
        tear_down(&mut app);
        tear_down(&mut app);
        app.update();
        assert_eq!(app.world().resource::<DrawSurfList>().keys, 0);
        app.update();
        assert_eq!(app.world().resource::<DrawSurfList>().keys, 0);
    }
}
