use bevy::prelude::*;

use crate::anim::fpv_host::{
    FpvBoltTargets, FpvHeldLife, FpvHeldSettled, FpvPoseProduct, FpvPresentCursor,
    PendingFpvNotetracks, PendingFpvSpawn,
};
use crate::anim::remote_body::RemoteBodySkinnedQueue;
use crate::anim::scene_submission::{AnimDObjSceneSkels, AnimDObjSceneSubmission};
use crate::draw::{
    DynEntDrawPlan, FpvDrawPlan, ItemDrawPlan, MissileDrawPlan, RemoteBodyDrawPlan,
    ScriptModelDrawPlan,
};
use crate::{gaps, guest_weapons, occupancy};

/// Registers the producers of every presented model kind and the draw plans
/// they fill.
pub struct RenderAnimPlugin;

fn init_posing_state(app: &mut App) {
    app.init_resource::<AnimDObjSceneSkels>()
        .init_resource::<RemoteBodySkinnedQueue>()
        .add_message::<AnimDObjSceneSubmission>();
}

fn init_viewmodel_state(app: &mut App) {
    app.init_resource::<FpvPresentCursor>()
        .init_resource::<PendingFpvSpawn>()
        .init_resource::<PendingFpvNotetracks>()
        .init_resource::<FpvHeldSettled>()
        .init_resource::<FpvHeldLife>()
        .init_resource::<FpvBoltTargets>()
        .init_resource::<FpvPoseProduct>();
}

fn init_draw_plans(app: &mut App) {
    app.init_resource::<FpvDrawPlan>()
        .init_resource::<RemoteBodyDrawPlan>()
        .init_resource::<ScriptModelDrawPlan>()
        .init_resource::<MissileDrawPlan>()
        .init_resource::<ItemDrawPlan>()
        .init_resource::<DynEntDrawPlan>();
}

impl Plugin for RenderAnimPlugin {
    fn build(&self, app: &mut App) {
        init_posing_state(app);
        init_viewmodel_state(app);
        init_draw_plans(app);

        occupancy::fpv_present::register_fpv_present_systems(app);
        occupancy::held_sync::register_held_sync_systems(app);
        occupancy::match_reset::register_match_reset_systems(app);
        occupancy::remote_body::register_remote_body_systems(app);
        occupancy::script_model::register_script_model_systems(app);
        occupancy::missile::register_missile_systems(app);
        occupancy::item::register_item_systems(app);
        occupancy::t6_body::register_t6_body_systems(app);
        occupancy::dyn_ent::register_dyn_ent_systems(app);
        gaps::register_render_gaps(app);
        guest_weapons::register(app);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn state_groups_register_their_resources() {
        let mut app = App::new();
        init_posing_state(&mut app);
        init_viewmodel_state(&mut app);
        init_draw_plans(&mut app);
        let world = app.world();
        assert!(world.contains_resource::<AnimDObjSceneSkels>());
        assert!(world.contains_resource::<RemoteBodySkinnedQueue>());
        assert!(world.contains_resource::<FpvPresentCursor>());
        assert!(world.contains_resource::<FpvPoseProduct>());
        assert!(world.contains_resource::<FpvDrawPlan>());
        assert!(world.contains_resource::<DynEntDrawPlan>());
    }
}
