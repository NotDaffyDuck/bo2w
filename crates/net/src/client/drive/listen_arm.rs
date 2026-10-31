//! Arms the local player's prediction from the host's own world when this runtime hosts the
//! match, or shares its content, instead of waiting for a snapshot to say what the map is.

use bevy::prelude::*;
use frame::ProcessDuty;

use super::stores::ClientGuessState;
use crate::host_tick::tick_systems::HostSimulationWorld;
use crate::client::run_ahead::LocalForecast;
use crate::tick_sets::ClientStep;

/// Whether the prediction still has to be (re)armed from `authority`: there must be a world to
/// copy, and the prediction either has none, has a different map, or lacks the penetration
/// table the host has since loaded.
pub fn forecast_needs_rearm(pred: &LocalForecast, host_world: &sim::SimWorld) -> bool {
    if !host_world.has_static_collision() {
        return false;
    }
    let ours = pred.world();
    if !pred.is_forecast_live() || !ours.has_static_collision() {
        return true;
    }
    ours.content_fingerprint() != host_world.content_fingerprint()
        || (host_world.wall_pierce_table_ready() && !ours.wall_pierce_table_ready())
}

pub fn enable_local_host_forecast(
    role: Res<ProcessDuty>,
    host_world: Option<Res<HostSimulationWorld>>,
    mut forecast: ResMut<ClientGuessState>,
) {
    let shares_host_content = matches!(*role, ProcessDuty::Listen | ProcessDuty::Client);
    let Some(host_world) = host_world.filter(|_| shares_host_content) else {
        return;
    };
    if forecast_needs_rearm(&forecast.0, &host_world.0) {
        forecast.0.prime_from_assets(&host_world.0);
    }
}

pub fn enroll_hosted_forecast_hookup(app: &mut App) {
    app.add_systems(Update, enable_local_host_forecast.in_set(ClientStep::Load));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_host_without_a_world_clip_gives_nothing_to_arm_from() {
        let forecast = ClientGuessState::default();
        assert!(!forecast_needs_rearm(&forecast.0, &sim::SimWorld::new()));
    }

    #[test]
    fn a_runtime_without_an_authority_world_is_left_alone() {
        let mut app = App::new();
        app.insert_resource(ProcessDuty::Listen)
            .init_resource::<ClientGuessState>()
            .add_systems(Update, enable_local_host_forecast);
        app.update();
        assert!(!app.world().resource::<ClientGuessState>().0.is_forecast_live());
    }

    #[test]
    fn the_roles_that_do_not_share_host_content_never_arm_from_it() {
        for role in [ProcessDuty::Dedicated, ProcessDuty::Replay] {
            let mut app = App::new();
            app.insert_resource(role)
                .insert_resource(HostSimulationWorld(sim::SimWorld::new()))
                .init_resource::<ClientGuessState>()
                .add_systems(Update, enable_local_host_forecast);
            app.update();
            assert!(!app.world().resource::<ClientGuessState>().0.is_forecast_live(), "{role:?}");
        }
    }

    #[test]
    fn registering_puts_the_arm_system_in_the_load_phase() {
        let mut app = App::new();
        app.insert_resource(ProcessDuty::Client)
            .init_resource::<ClientGuessState>()
            .configure_sets(Update, ClientStep::Load);
        enroll_hosted_forecast_hookup(&mut app);
        app.update();
    }
}
