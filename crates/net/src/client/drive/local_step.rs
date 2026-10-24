//! The frame's own move: steps the client clock, then turns the finished input template into a
//! predicted command and queues it for the host.

use bevy::prelude::*;
use frame::RuntimeRole;
use sim::{ClientId, Snapshot};

use super::outbox::PendingClientSends;
use super::stores::{ClientCmdTemplate, ClientPhaseTrace, ClientPredictionState, RemoteProxyState, note_phase};
use super::ticker::ClientClock;
use crate::ServerTime;
use crate::authority::runtime::AuthorityInputGate;
use crate::client::frame_clock::FrameClock;
use crate::client::input::ClientActionInput;
use crate::client::presentation::presented::{LocalPresentClient, PresentedSnapshot};
use crate::client::proxy::{ProxySample, RemoteProxy};
use crate::client::realtime::ClientRealtime;

/// Where each living opponent is drawn right now, with his velocity. A predicted knife lunge
/// goes at him there, which is also the moment the command's shot sample names, so the host's
/// rewind finds him at the same spot.
fn drawn_opponents<'a>(
    snapshot: &'a Snapshot,
    me: ClientId,
    proxy: &'a RemoteProxy,
    time: i32,
) -> impl Iterator<Item = (ClientId, [f32; 3], [f32; 3])> + 'a {
    let alive = move |id: ClientId| {
        snapshot
            .meta
            .for_client(id)
            .is_some_and(|meta| meta.lifecycle == sim::ClientLifecycle::Alive)
    };
    snapshot
        .players
        .iter()
        .filter(move |(id, _)| *id != me && alive(*id))
        .map(move |(id, adopted)| {
            let shown = match proxy.interpolate_at(*id, time) {
                ProxySample::Pose { ps, .. } => ps,
                ProxySample::Starved { held: Some(ps), .. } => ps,
                ProxySample::Starved { held: None, .. } => *adopted,
            };
            (*id, shown.origin, shown.velocity)
        })
}

pub fn predict_local_move(
    time: Res<Time>,
    mut clock: ResMut<ClientClock>,
    role: Res<RuntimeRole>,
    mut prediction: ResMut<ClientPredictionState>,
    template: Res<ClientCmdTemplate>,
    mut actions: ResMut<ClientActionInput>,
    gate: Res<AuthorityInputGate>,
    mut pending: ResMut<PendingClientSends>,
    proxy: Res<RemoteProxyState>,
    cls: Res<ClientRealtime>,
    cg_clock: Res<FrameClock>,
    presented: Res<PresentedSnapshot>,
    local: Res<LocalPresentClient>,
    trace: Option<ResMut<ClientPhaseTrace>>,
) {
    note_phase(trace, "Predict");

    if *role != RuntimeRole::Replay {
        clock.tick(time.delta_secs() * 1000.0);
    }
    let ready = gate.local_cmds_enabled
        && prediction.0.is_armed()
        && template.ready
        && cg_clock.started()
        && pending.has_command_capacity(cg_clock.time());
    if !ready || cls.frametime() <= 0 {
        return;
    }

    let now = cg_clock.time();
    if let Some(snapshot) = presented.snapshot() {
        prediction
            .0
            .set_lunge_seen(drawn_opponents(snapshot, local.0, &proxy.0, now));
    }
    let sample = proxy.0.shot_sample(now);
    if let Some((seq, cmd)) = prediction.0.predict(template.cmd, ServerTime::from_ms(now)) {
        actions.consume_edges();
        pending.push(seq, cmd, sample);
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;
    use crate::client::presentation::presented::PresentedSnapshot;

    fn local_app(role: RuntimeRole) -> App {
        let mut app = App::new();
        app.insert_resource(role)
            .insert_resource(Time::<()>::default())
            .init_resource::<ClientClock>()
            .init_resource::<ClientPredictionState>()
            .init_resource::<ClientCmdTemplate>()
            .init_resource::<ClientActionInput>()
            .insert_resource(AuthorityInputGate { local_cmds_enabled: true })
            .init_resource::<PendingClientSends>()
            .init_resource::<RemoteProxyState>()
            .init_resource::<ClientRealtime>()
            .init_resource::<FrameClock>()
            .init_resource::<PresentedSnapshot>()
            .insert_resource(LocalPresentClient(ClientId(1)))
            .add_systems(Update, predict_local_move);
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
        let mut app = local_app(RuntimeRole::Client);
        app.world_mut().resource_mut::<ClientCmdTemplate>().ready = true;
        frame(&mut app, 16);
        assert!(app.world().resource::<PendingClientSends>().is_empty());
    }

    #[test]
    fn the_phase_is_traced_even_when_nothing_is_predicted() {
        let mut app = local_app(RuntimeRole::Client);
        app.insert_resource(ClientPhaseTrace::default());
        frame(&mut app, 16);
        assert_eq!(app.world().resource::<ClientPhaseTrace>().0, ["Predict"]);
    }

    #[test]
    fn a_replay_does_not_step_the_client_clock() {
        let mut replay = local_app(RuntimeRole::Replay);
        let mut client = local_app(RuntimeRole::Client);
        frame(&mut replay, 5);
        frame(&mut client, 5);
        let owed = |app: &App| app.world().resource::<ClientClock>().debt_ms();
        assert_eq!(owed(&replay), 0.0);
        assert!(owed(&client) > 0.0, "the client clock did not move");
    }

    #[test]
    fn opponents_are_taken_where_they_are_drawn_and_only_while_alive() {
        let mut snap = Snapshot::unpublished(sim::Tick(0));
        let mut at = |id: u32, x: f32, lifecycle| {
            let mut ps = playerstate_t6::game::PlayerState::ZERO;
            ps.origin = [x, 0.0, 0.0];
            ps.velocity = [0.0, x, 0.0];
            snap.players.push((ClientId(id), ps));
            snap.meta.clients.push((
                ClientId(id),
                sim::ClientSnapshotMeta { lifecycle, ..Default::default() },
            ));
        };
        at(1, 1.0, sim::ClientLifecycle::Alive);
        at(2, 2.0, sim::ClientLifecycle::Alive);
        at(3, 3.0, sim::ClientLifecycle::Dead);
        let proxy = RemoteProxy::default();
        let seen: Vec<_> = drawn_opponents(&snap, ClientId(1), &proxy, 0).collect();
        // The proxy has nothing buffered, so each one falls back to the adopted state.
        assert_eq!(seen, [(ClientId(2), [2.0, 0.0, 0.0], [0.0, 2.0, 0.0])]);
    }
}
