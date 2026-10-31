//! Clearing the world's draw plans when a match ends, so nothing from the old
//! world is drawn in the next one.

use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use frame::GameSessionClosed;

use super::draw_items::fog::ScriptFogPresentation;
use super::draw_items::DrawMethodDepthHaze;
use super::draw_items::FacetDrawList;
use super::draw_items::FxCodeDrawPlan;
use super::draw_items::FxModelDrawList;
use super::draw_items::FxCloudDrawPlan;
use super::draw_items::VisualStainMeshPlan;
use super::draw_items::GlassTable;
use super::draw_items::MapFrameHaze;

/// Every plan built from the world that is being torn down.
#[derive(SystemParam)]
pub(crate) struct WorldDrawPlans<'w> {
    depth_haze: ResMut<'w, DrawMethodDepthHaze>,
    draw_list: ResMut<'w, FacetDrawList>,
    glass: ResMut<'w, GlassTable>,
    fx_code: ResMut<'w, FxCodeDrawPlan>,
    particles: ResMut<'w, FxCloudDrawPlan>,
    marks: ResMut<'w, VisualStainMeshPlan>,
    effect_meshes: ResMut<'w, FxModelDrawList>,
}

impl WorldDrawPlans<'_> {
    fn clear(&mut self) {
        self.depth_haze.0 = false;
        *self.draw_list = FacetDrawList::default();
        self.glass.forget_panes();
        *self.fx_code = FxCodeDrawPlan::default();
        *self.particles = FxCloudDrawPlan::default();
        *self.marks = VisualStainMeshPlan::default();
        *self.effect_meshes = FxModelDrawList::without_geometry(0);
    }
}

/// On any teardown message: drop the map fog and everything planned from the
/// old world.
pub(crate) fn reset_level_draw_plans_on_teardown(
    mut ripped: MessageReader<GameSessionClosed>,
    mut commands: Commands,
    mut plans: WorldDrawPlans,
) {
    if ripped.read().next().is_none() {
        return;
    }
    commands.remove_resource::<MapFrameHaze>();
    commands.insert_resource(ScriptFogPresentation::default());
    plans.clear();
}

pub(crate) fn register_match_reset_steps(app: &mut App) {
    app.add_systems(
        Update,
        reset_level_draw_plans_on_teardown
            .after(frame::MatchSwapDone)
            .after(crate::prepare::scene::spawn::reset_level_spawn_on_teardown)
            .before(crate::prepare::scene::spawn::shutdown_level_on_teardown)
            .in_set(net::ClientStep::Load),
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use asset_world::ExpHaze;
    use frame::{GameSessionStamp, ShutdownCause, LevelGeneration};

    fn app() -> App {
        let mut client = App::new();
        client.add_message::<GameSessionClosed>()
            .init_resource::<DrawMethodDepthHaze>()
            .init_resource::<FacetDrawList>()
            .init_resource::<GlassTable>()
            .init_resource::<FxCodeDrawPlan>()
            .init_resource::<FxCloudDrawPlan>()
            .init_resource::<VisualStainMeshPlan>()
            .init_resource::<FxModelDrawList>();
        client.add_systems(Update, reset_level_draw_plans_on_teardown);
        client.insert_resource(DrawMethodDepthHaze(true));
        let volumetric = None;
        let opacity_cap = 1.0;
        client.insert_resource(MapFrameHaze::new(ExpHaze {
            near_edge: 1.0,
            half_density_at: 2.0,
            tint_rgb: Vec3::ZERO.to_array(),
            opacity_cap,
            fade_duration: 0.0,
            sun: None,
            volumetric,
        }));
        client.world_mut().resource_mut::<FacetDrawList>().keys = 5;
        client
    }

    fn tear_down(app: &mut App) {
        app.world_mut().write_message(GameSessionClosed {
            reason: ShutdownCause::RoundOver,
            level_generation: LevelGeneration(Some(1)),
            game_session_key: GameSessionStamp::default(),
            game_generation: 0,
        });
    }

    #[test]
    fn nothing_changes_without_a_teardown() {
        let mut app = app();
        app.update();
        assert!(app.world().resource::<DrawMethodDepthHaze>().0);
        assert_eq!(app.world().resource::<FacetDrawList>().keys, 5);
        assert!(app.world().contains_resource::<MapFrameHaze>());
    }

    #[test]
    fn a_teardown_clears_the_fog_and_every_plan() {
        let mut app = app();
        tear_down(&mut app);
        app.update();
        assert!(!app.world().resource::<DrawMethodDepthHaze>().0);
        assert_eq!(app.world().resource::<FacetDrawList>().keys, 0);
        assert!(!app.world().contains_resource::<MapFrameHaze>());
        assert!(app.world().contains_resource::<ScriptFogPresentation>());
    }

    #[test]
    fn several_teardowns_in_a_frame_clear_once_and_cleanly() {
        let mut app = app();
        tear_down(&mut app);
        tear_down(&mut app);
        app.update();
        assert_eq!(app.world().resource::<FacetDrawList>().keys, 0);
        app.update();
        assert_eq!(app.world().resource::<FacetDrawList>().keys, 0);
    }
}
