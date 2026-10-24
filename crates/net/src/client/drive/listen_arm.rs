//! Arms the local player's prediction from the host's own world when this runtime hosts the
//! match, or shares its content, instead of waiting for a snapshot to say what the map is.

use bevy::prelude::*;
use frame::RuntimeRole;

use super::stores::ClientPredictionState;
use crate::authority::runtime::AuthorityWorld;
use crate::client::predict::ClientPrediction;
use crate::schedule::ClientSet;

/// Whether the prediction still has to be (re)armed from `authority`: there must be a world to
/// copy, and the prediction either has none, has a different map, or lacks the penetration
/// table the host has since loaded.
pub fn listen_prediction_needs_content(pred: &ClientPrediction, authority: &sim::SimWorld) -> bool {
    if !authority.has_world_clip() {
        return false;
    }
    let ours = pred.world();
    if !pred.is_armed() || !ours.has_world_clip() {
        return true;
    }
    ours.content_digest() != authority.content_digest()
        || (authority.pen_table_loaded() && !ours.pen_table_loaded())
}

pub fn arm_listen_prediction(
    role: Res<RuntimeRole>,
    authority: Option<Res<AuthorityWorld>>,
    mut prediction: ResMut<ClientPredictionState>,
) {
    let shares_host_content = matches!(*role, RuntimeRole::Listen | RuntimeRole::Client);
    let Some(authority) = authority.filter(|_| shares_host_content) else {
        return;
    };
    if listen_prediction_needs_content(&prediction.0, &authority.0) {
        prediction.0.arm_from_content(&authority.0);
    }
}

pub fn register_listen_prediction_arm(app: &mut App) {
    app.add_systems(Update, arm_listen_prediction.in_set(ClientSet::Load));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_host_without_a_world_clip_gives_nothing_to_arm_from() {
        let prediction = ClientPredictionState::default();
        assert!(!listen_prediction_needs_content(&prediction.0, &sim::SimWorld::new()));
    }

    #[test]
    fn a_runtime_without_an_authority_world_is_left_alone() {
        let mut app = App::new();
        app.insert_resource(RuntimeRole::Listen)
            .init_resource::<ClientPredictionState>()
            .add_systems(Update, arm_listen_prediction);
        app.update();
        assert!(!app.world().resource::<ClientPredictionState>().0.is_armed());
    }

    #[test]
    fn the_roles_that_do_not_share_host_content_never_arm_from_it() {
        for role in [RuntimeRole::Dedicated, RuntimeRole::Replay] {
            let mut app = App::new();
            app.insert_resource(role)
                .insert_resource(AuthorityWorld(sim::SimWorld::new()))
                .init_resource::<ClientPredictionState>()
                .add_systems(Update, arm_listen_prediction);
            app.update();
            assert!(!app.world().resource::<ClientPredictionState>().0.is_armed(), "{role:?}");
        }
    }

    #[test]
    fn registering_puts_the_arm_system_in_the_load_phase() {
        let mut app = App::new();
        app.insert_resource(RuntimeRole::Client)
            .init_resource::<ClientPredictionState>()
            .configure_sets(Update, ClientSet::Load);
        register_listen_prediction_arm(&mut app);
        app.update();
    }
}
