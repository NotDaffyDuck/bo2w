//! The "feel" line: the local player's position and command timebase as the host, the last
//! adopted snapshot and the prediction each have it.

use bevy::prelude::*;
use sim::PlayerSessionStage;

use super::InboundBacklogTally;
use crate::FrameClock;
use crate::host_tick::tick_systems::{HostSimulationWorld, LocalHostFanoutTally};
use crate::client::drive::{PlayerTimeStepper, ClientGuessState, NewestTakenSnapshot};
use crate::client::presentation::shown_frame::{
    HostViewClient, OwnViewerTally, ShownWorldFrame,
};

pub(super) fn emit_feel(
    timebase: Option<Res<FrameClock>>,
    presented: Res<ShownWorldFrame>,
    local: Res<HostViewClient>,
    client_clock: Option<Res<PlayerTimeStepper>>,
    queue: Res<InboundBacklogTally>,
    last_adopted: Option<Res<NewestTakenSnapshot>>,
    prediction: Option<Res<ClientGuessState>>,
    authority_world: Option<Res<HostSimulationWorld>>,
    fanout: Option<Res<LocalHostFanoutTally>>,
    present_census: Option<Res<OwnViewerTally>>,
) {
    let Some(timebase) = timebase else {
        return;
    };
    let host_state = authority_world.as_ref().and_then(|world| world.0.player(local.0));
    let adopted_state = last_adopted
        .as_ref()
        .and_then(|adopted| adopted.next())
        .and_then(|snapshot| snapshot.players.iter().find(|(id, _)| *id == local.0))
        .map(|(_, st)| st);
    let predicted_state = prediction.as_ref().and_then(|p| p.0.predicted_local());
    let stage_label = presented
        .snapshot()
        .and_then(|got_snap| got_snap.meta.for_client(local.0))
        .map(|meta| lifecycle_label(meta.lifecycle));

    perf::feel(
        timebase.time(),
        stage_label,
        fanout.as_ref().and_then(|f| f.viewer_seat_installed),
        fanout.as_ref().and_then(|f| f.seat_query_tick),
        present_census.as_ref().and_then(|cen| cen.picked_view),
        present_census.as_ref().and_then(|c| c.frame_gap_millis),
        host_state.map(|state| state.origin),
        adopted_state.map(|state| state.origin),
        client_clock.as_ref().map(|c| c.owed_millis()),
        queue.0,
        host_state.map(|state| state.last_cmd_ms),
        predicted_state.map(|state| state.last_cmd_ms),
    );
}

/// The lifecycle's name as the feel line prints it.
fn lifecycle_label(life: PlayerSessionStage) -> &'static str {
    match life {
        PlayerSessionStage::Connecting => "Connecting",
        PlayerSessionStage::PickingLoadout => "PickingLoadout",
        PlayerSessionStage::AwaitingFirstLife => "AwaitingFirstLife",
        PlayerSessionStage::Alive => "Alive",
        PlayerSessionStage::Dead => "Dead",
        PlayerSessionStage::AwaitingNextLife => "AwaitingNextLife",
        PlayerSessionStage::Spectating => "Spectating",
        PlayerSessionStage::Intermission => "Intermission",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_lifecycle_prints_as_its_variant_name() {
        let all = [
            PlayerSessionStage::Connecting,
            PlayerSessionStage::PickingLoadout,
            PlayerSessionStage::AwaitingFirstLife,
            PlayerSessionStage::Alive,
            PlayerSessionStage::Dead,
            PlayerSessionStage::AwaitingNextLife,
            PlayerSessionStage::Spectating,
            PlayerSessionStage::Intermission,
        ];
        for life in all {
            assert_eq!(lifecycle_label(life), format!("{life:?}"));
        }
    }
}
