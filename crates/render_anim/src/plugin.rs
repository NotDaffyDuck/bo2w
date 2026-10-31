use bevy::prelude::*;

use crate::motion::gun_host::{
    AttachTagGoals, HoldLifeStamp, HoldSettledLife, GunPoseOutput, FpvShownCursor,
    QueuedGunNotes, WaitingFpvSpawn,
};
use crate::motion::other_body::BodySkinJobs;
use crate::motion::scene_feed::{SceneBonePoseSet, SkeletonSceneHandoff};
use crate::strokes::{
    DynEntPaintPlan, FirstPersonPaintPlan, PickupPaintPlan, ThrownPaintPlan, OtherBodyPaintPlan,
    ScriptedModelPaintPlan,
};
use crate::{holes, guest_weapons, occupancy};

/// Registers the producers of every presented model kind and the draw plans
/// they fill.
pub struct BodyAnimationPlugin;

fn init_posing_state(app: &mut App) {
    app.init_resource::<SceneBonePoseSet>()
        .init_resource::<BodySkinJobs>()
        .add_message::<SkeletonSceneHandoff>();
}

fn init_viewmodel_state(app: &mut App) {
    app.init_resource::<FpvShownCursor>()
        .init_resource::<WaitingFpvSpawn>()
        .init_resource::<QueuedGunNotes>()
        .init_resource::<HoldSettledLife>()
        .init_resource::<HoldLifeStamp>()
        .init_resource::<AttachTagGoals>()
        .init_resource::<GunPoseOutput>();
}

fn init_draw_plans(app: &mut App) {
    app.init_resource::<FirstPersonPaintPlan>()
        .init_resource::<OtherBodyPaintPlan>()
        .init_resource::<ScriptedModelPaintPlan>()
        .init_resource::<ThrownPaintPlan>()
        .init_resource::<PickupPaintPlan>()
        .init_resource::<DynEntPaintPlan>();
}

impl Plugin for BodyAnimationPlugin {
    fn build(&self, app_in: &mut App) {
        init_posing_state(app_in);
        init_viewmodel_state(app_in);
        init_draw_plans(app_in);

        occupancy::gun_present::wire_gun_display_systems(app_in);
        occupancy::hold_sync::add_held_sync_systems(app_in);
        occupancy::round_reset::register_match_reset_steps(app_in);
        occupancy::peer_body::wire_remote_body_systems(app_in);
        occupancy::script_prop::wire_script_prop_systems(app_in);
        occupancy::missile::wire_rocket_systems(app_in);
        occupancy::item::add_item_systems(app_in);
        occupancy::t6_body::register_t6_body_systems(app_in);
        occupancy::shard_piece::add_dyn_ent_systems(app_in);
        holes::add_render_gaps(app_in);
        guest_weapons::register(app_in);
    }
}

mod tests {
    #![cfg(test)]
    use super::*;

    #[test]
    fn state_groups_register_their_resources() {
        let mut application = App::new();
        init_posing_state(&mut application);
        init_viewmodel_state(&mut application);
        init_draw_plans(&mut application);
        let world = application.world();
        assert!(world.contains_resource::<SceneBonePoseSet>());
        assert!(world.contains_resource::<BodySkinJobs>());
        assert!(world.contains_resource::<FpvShownCursor>());
        assert!(world.contains_resource::<GunPoseOutput>());
        assert!(world.contains_resource::<FirstPersonPaintPlan>());
        assert!(world.contains_resource::<DynEntPaintPlan>());
    }
}
