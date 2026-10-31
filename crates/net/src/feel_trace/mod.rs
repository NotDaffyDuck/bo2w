//! Per-frame diagnostics lines for the netcode: how the local player's own movement feels and
//! where the other players' bodies are drawn against where the host says they are.

mod feel;
mod remote;

use bevy::prelude::*;

use crate::ClientStep;
use crate::host_tick::peer_mail::PlayerMoveIntake;
use crate::client::presentation::shown_frame::HostViewClient;

/// How many commands from the local player were waiting on the host this frame, latched right
/// after the frame's receive so the feel line reports what the host saw.
#[derive(Resource, Default)]
struct InboundBacklogTally(Option<i64>);

pub(crate) fn register(host: &mut App) {
    host.init_resource::<InboundBacklogTally>().add_systems(
        Update,
        (
            freeze_inbound_queue.after(ClientStep::Receive).before(ClientStep::Input),
            (feel::emit_feel, remote::emit_remote).in_set(ClientStep::Diag),
        ),
    );
}

fn freeze_inbound_queue(
    inbox: Option<Res<PlayerMoveIntake>>,
    local: Res<HostViewClient>,
    mut census: ResMut<InboundBacklogTally>,
) {
    census.0 = inbox.map(|inbox| inbox.queued_depth(local.0) as i64);
}

#[cfg(test)]
mod tests {
    use sim::ClientId;

    use super::*;

    fn app_with_local(client: u32) -> App {
        let mut app = App::new();
        app.insert_resource(HostViewClient(ClientId(client)))
            .init_resource::<InboundBacklogTally>()
            .add_systems(Update, freeze_inbound_queue);
        app
    }

    #[test]
    fn without_a_host_inbox_there_is_no_queue_to_report() {
        let mut app = app_with_local(1);
        app.world_mut().resource_mut::<InboundBacklogTally>().0 = Some(9);
        app.update();
        assert_eq!(app.world().resource::<InboundBacklogTally>().0, None);
    }

    #[test]
    fn an_empty_inbox_reports_zero_waiting() {
        let mut app = app_with_local(1);
        app.init_resource::<PlayerMoveIntake>();
        app.update();
        assert_eq!(app.world().resource::<InboundBacklogTally>().0, Some(0));
    }
}
