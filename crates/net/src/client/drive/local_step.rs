//! The frame's own move: steps the client clock, then turns the finished input template into a
//! predicted command and queues it for the host.

use bevy::prelude::*;
use frame::ProcessDuty;
use sim::{ClientId, Snapshot};

use super::outbox::QueuedOutgoingPlayerMessages;
use super::stores::{PlayerMoveBlueprint, ReceiverStageLog, ClientGuessState, OtherPlayerSmootherHolder, note_phase};
use super::ticker::PlayerTimeStepper;
use crate::HostInstant;
use crate::host_tick::tick_systems::HostInputValve;
use crate::client::frame_clock::FrameClock;
use crate::client::local_input::PlayerControlSnapshot;
use crate::client::presentation::shown_frame::{HostViewClient, ShownWorldFrame};
use crate::client::remote_buffer::{SmootherReading, OtherPlayerSmoother};
use crate::client::realtime::ClientRealtime;

/// Where each living opponent is drawn right now, with his velocity. A predicted knife lunge
/// goes at him there, which is also the moment the command's shot sample names, so the host's
/// rewind finds him at the same spot.
fn drawn_opponents<'a>(
    snapshot: &'a Snapshot,
    me: ClientId,
    proxy: &'a OtherPlayerSmoother,
    time: i32,
) -> impl Iterator<Item = (ClientId, [f32; 3], [f32; 3])> + 'a {
    let alive = move |id: ClientId| {
        snapshot
            .meta
            .for_client(id)
            .is_some_and(|meta| meta.lifecycle == sim::PlayerSessionStage::Alive)
    };
    snapshot
        .players
        .iter()
        .filter(move |(id, _)| *id != me && alive(*id))
        .map(move |(id, adopted)| {
            let shown = match proxy.pose_for(*id, time) {
                SmootherReading::Pose { ps, .. } => ps,
                SmootherReading::RanDry { held: Some(ps), .. } => ps,
                SmootherReading::RanDry { held: None, .. } => *adopted,
            };
            (*id, shown.origin, shown.velocity)
        })
}

pub fn forecast_own_move(
    time: Res<Time>,
    mut clock: ResMut<PlayerTimeStepper>,
    role: Res<ProcessDuty>,
    mut forecast: ResMut<ClientGuessState>,
    template: Res<PlayerMoveBlueprint>,
    mut ctl: ResMut<PlayerControlSnapshot>,
    gate: Res<HostInputValve>,
    mut pending: ResMut<QueuedOutgoingPlayerMessages>,
    proxy: Res<OtherPlayerSmootherHolder>,
    cls: Res<ClientRealtime>,
    cg_clock: Res<FrameClock>,
    world_view: Res<ShownWorldFrame>,
    local: Res<HostViewClient>,
    trace: Option<ResMut<ReceiverStageLog>>,
) {
    note_phase(trace, "Predict");

    if *role != ProcessDuty::Replay {
        clock.advance_by(time.delta_secs() * 1000.0);
    }
    let primed = gate.local_cmds_enabled
        && forecast.0.is_forecast_live()
        && template.prefilled
        && cg_clock.started()
        && pending.has_command_capacity(cg_clock.time());
    if !primed || cls.step_millis_elapsed() <= 0 {
        return;
    }

    let now = cg_clock.time();
    if let Some(snapshot) = world_view.snapshot() {
        forecast
            .0
            .set_lunge_seen(drawn_opponents(snapshot, local.0, &proxy.0, now));
    }
    let sample = proxy.0.shot_bracket(now);
    if let Some((move_seq, cmd)) = forecast.0.run_ahead(template.cmd, HostInstant::from_millis_value(now)) {
        ctl.take_transitions();
        pending.enqueue(move_seq, cmd, sample);
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;
    use crate::client::presentation::shown_frame::ShownWorldFrame;

    fn local_app(role: ProcessDuty) -> App {
        let mut app = App::new();
        app.insert_resource(role)
            .insert_resource(Time::<()>::default())
            .init_resource::<PlayerTimeStepper>()
            .init_resource::<ClientGuessState>()
            .init_resource::<PlayerMoveBlueprint>()
            .init_resource::<PlayerControlSnapshot>()
            .insert_resource(HostInputValve { local_cmds_enabled: true })
            .init_resource::<QueuedOutgoingPlayerMessages>()
            .init_resource::<OtherPlayerSmootherHolder>()
            .init_resource::<ClientRealtime>()
            .init_resource::<FrameClock>()
            .init_resource::<ShownWorldFrame>()
            .insert_resource(HostViewClient(ClientId(1)))
            .add_systems(Update, forecast_own_move);
        app
    }

    fn frame(app: &mut App, ms: u64) {
        app.world_mut()
            .resource_mut::<Time>()
            .advance_by(Duration::from_millis(ms));
        app.update();
    }

    #[test]
    fn nothing_is_queued_while_the_prediction_is_not_armed() {
        let mut app = local_app(ProcessDuty::Client);
        app.world_mut().resource_mut::<PlayerMoveBlueprint>().prefilled = true;
        frame(&mut app, 16);
        assert!(app.world().resource::<QueuedOutgoingPlayerMessages>().is_empty());
    }

    #[test]
    fn the_phase_is_traced_even_when_nothing_is_predicted() {
        let mut app = local_app(ProcessDuty::Client);
        app.insert_resource(ReceiverStageLog::default());
        frame(&mut app, 16);
        assert_eq!(app.world().resource::<ReceiverStageLog>().0, ["Predict"]);
    }

    #[test]
    fn a_replay_does_not_step_the_client_clock() {
        let mut replay = local_app(ProcessDuty::Replay);
        let mut client = local_app(ProcessDuty::Client);
        frame(&mut replay, 5);
        frame(&mut client, 5);
        let owed = |app: &App| app.world().resource::<PlayerTimeStepper>().owed_millis();
        assert_eq!(owed(&replay), 0.0);
        assert!(owed(&client) > 0.0, "the client clock did not move");
    }

    #[test]
    fn opponents_are_taken_where_they_are_drawn_and_only_while_alive() {
        let mut snap = Snapshot::unpublished(sim::Tick(0));
        let mut at = |id: u32, x: f32, lifecycle| {
            let mut own_state = playerstate_t6::game::AvatarState::ZERO;
            own_state.origin = [x, 0.0, 0.0];
            own_state.velocity = [0.0, x, 0.0];
            snap.players.push((ClientId(id), own_state));
            snap.meta.clients.push((
                ClientId(id),
                sim::ClientSnapshotMeta { lifecycle, ..Default::default() },
            ));
        };
        at(1, 1.0, sim::PlayerSessionStage::Alive);
        at(2, 2.0, sim::PlayerSessionStage::Alive);
        at(3, 3.0, sim::PlayerSessionStage::Dead);
        let proxy = OtherPlayerSmoother::default();
        let seen: Vec<_> = drawn_opponents(&snap, ClientId(1), &proxy, 0).collect();
        // The proxy has nothing buffered, so each one falls back to the adopted state.
        assert_eq!(seen, [(ClientId(2), [2.0, 0.0, 0.0], [0.0, 2.0, 0.0])]);
    }
}
