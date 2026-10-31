//! Puts the netcode into an app. What goes in depends on which of the four runtime roles the process plays.

mod wiring;

use bevy::prelude::{App, Plugin};
use frame::ProcessDuty;

use crate::host_tick::tick_systems::{HostLoadingPause, enroll_hosted_systems};
use crate::client::drive::enroll_hosted_forecast_hookup;
use crate::{HoldTransferInstalled, HoldMapLoadedSignal, DirectoryLaunchRequest, feel_trace};

/// The netcode for one role: the authority's inboxes and fixed clock always, plus the client's input, prediction and
/// presentation for a role that has a client.
pub struct NetworkingPlugin {
    /// Which of the four runtime roles this process plays.
    pub role: ProcessDuty,
}

impl NetworkingPlugin {
    const fn for_role(role: ProcessDuty) -> Self {
        Self { role }
    }

    pub const fn listen() -> Self {
        Self::for_role(ProcessDuty::Listen)
    }

    pub const fn dedicated() -> Self {
        Self::for_role(ProcessDuty::Dedicated)
    }

    pub const fn client() -> Self {
        Self::for_role(ProcessDuty::Client)
    }

    pub const fn replay() -> Self {
        Self::for_role(ProcessDuty::Replay)
    }
}

impl Plugin for NetworkingPlugin {
    fn build(&self, app: &mut App) {
        let role = self.role;
        let via_master = app
            .world()
            .get_resource::<DirectoryLaunchRequest>()
            .is_some_and(DirectoryLaunchRequest::enabled);

        wiring::shared(app, role);
        wiring::authority_state(app);
        if role.hosts_player_view() {
            wiring::client_side(app, role);
        }

        enroll_hosted_systems(app);
        if matches!(role, ProcessDuty::Listen | ProcessDuty::Client | ProcessDuty::Replay) {
            enroll_hosted_forecast_hookup(app);
        }
        if matches!(role, ProcessDuty::Replay) {
            app.insert_resource(HostLoadingPause(true));
        }

        wiring::match_flow(app, via_master);
        feel_trace::register(app);
        app.insert_resource(HoldMapLoadedSignal::read_env_switch());
        app.insert_resource(HoldTransferInstalled::read_env_switch());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::presentation::shown_frame::{HostViewClient, ShownWorldFrame};

    #[test]
    fn each_constructor_picks_its_role() {
        let wanted = [
            (NetworkingPlugin::listen(), ProcessDuty::Listen),
            (NetworkingPlugin::dedicated(), ProcessDuty::Dedicated),
            (NetworkingPlugin::client(), ProcessDuty::Client),
            (NetworkingPlugin::replay(), ProcessDuty::Replay),
        ];
        for (plugin, role) in wanted {
            assert_eq!(plugin.role, role);
        }
    }

    #[test]
    fn the_shared_part_stores_the_role_for_everyone_else() {
        let mut app = App::new();
        wiring::shared(&mut app, ProcessDuty::Replay);
        let world = app.world();
        assert_eq!(*world.resource::<ProcessDuty>(), ProcessDuty::Replay);
        assert!(world.contains_resource::<ShownWorldFrame>());
        assert!(world.contains_resource::<HostViewClient>());
    }
}
