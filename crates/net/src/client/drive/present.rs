//! The frame's picture of the match. What the screen shows this frame is put together here
//! from the snapshot just taken in, the delayed poses of the other players and the local
//! prediction, then handed to [`PresentedSnapshot`] and to the bodies in the world.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;

use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use frame::{HasWorld, RuntimeRole};
use playerstate_t6::game::PlayerState;
use sim::{ClientId, Snapshot, Tick};

use super::killcam_log::{self, KillcamLog};
use super::stores::{ClientPhaseTrace, ClientPredictionState, LastAdoptedSnapshot, RemoteProxyState, note_phase};
use super::ticker::ClientClock;
use crate::CEntityRuntime;
use crate::authority::inbox::AUTHORITY_MS;
use crate::client::frame_clock::FrameClock;
use crate::client::presentation::presented::{
    LocalPresentClient, PresentLocalCensus, PresentedSnapshot, interpolate_player_state,
};
use crate::client::presentation::projectiles::merge_presented_projectiles;
use crate::client::proxy::{
    PROXY_DELAY_MS, PresentationSampleOutcome, PresentationSampleProvenance, ProxySample, RemoteProxy,
};

/// What the frame already knows, none of it changed here.
#[derive(SystemParam)]
pub struct PresentFacts<'w> {
    clock: Res<'w, ClientClock>,
    cg_clock: Res<'w, FrameClock>,
    role: Res<'w, RuntimeRole>,
    prediction: Res<'w, ClientPredictionState>,
    last_adopted: Res<'w, LastAdoptedSnapshot>,
    proxy: Res<'w, RemoteProxyState>,
    local: Res<'w, LocalPresentClient>,
    has_world: Option<Res<'w, HasWorld>>,
}

/// Where the picture is put.
#[derive(SystemParam)]
pub struct PresentSinks<'w> {
    presented: ResMut<'w, PresentedSnapshot>,
    census: ResMut<'w, PresentLocalCensus>,
    phase: Option<ResMut<'w, crate::UpdatePhaseCensus>>,
    trace: Option<ResMut<'w, ClientPhaseTrace>>,
}

/// Writes how long the system took into the frame census when it is dropped, whichever way the
/// system returns.
struct PublishStamp<'w> {
    started: Instant,
    phase: Option<ResMut<'w, crate::UpdatePhaseCensus>>,
}

impl Drop for PublishStamp<'_> {
    fn drop(&mut self) {
        let ms = self.started.elapsed().as_secs_f32() * 1000.0;
        if let Some(phase) = self.phase.as_mut() {
            phase.publish_presented_ms = Some(ms);
        }
    }
}

/// The two moments a frame is drawn at: the one the other players' poses are sampled for, and
/// the one the bodies already in the world are moved to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct DrawTimes {
    poses_ms: i32,
    bodies_ms: i32,
}

/// A replay runs on the clock of its snapshots: the poses are a step ahead of the bodies, and
/// both move on by the part of a step that has built up. Otherwise the poses are drawn at the
/// game time, and the bodies one step behind it, or the full proxy delay behind it while the
/// local player is watching a replay of the match.
fn draw_times(replay: bool, archived: bool, tick: Tick, built_up_ms: f32, game_ms: i32) -> DrawTimes {
    let tick_ms = tick.0 as i32 * AUTHORITY_MS;
    if replay {
        return DrawTimes {
            poses_ms: tick_ms + AUTHORITY_MS + built_up_ms as i32,
            bodies_ms: tick_ms - AUTHORITY_MS + built_up_ms as i32,
        };
    }
    DrawTimes {
        poses_ms: game_ms,
        bodies_ms: game_ms.saturating_sub(if archived { PROXY_DELAY_MS } else { AUTHORITY_MS }),
    }
}

/// The time a body was last sampled for: the time of its snapshot when a whole snapshot was
/// used, otherwise the time the blend was taken at.
fn sample_time(provenance: &PresentationSampleProvenance) -> i32 {
    match provenance.outcome {
        PresentationSampleOutcome::Exact { snapshot }
        | PresentationSampleOutcome::Starved { held_snapshot: Some(snapshot), .. } => {
            snapshot.0 as i32 * AUTHORITY_MS
        }
        PresentationSampleOutcome::Interpolated { .. } => provenance.effective_time.0,
        PresentationSampleOutcome::Starved { held_snapshot: None, .. } => {
            unreachable!("a proxy pose always has a source snapshot")
        }
    }
}

/// The world a replay draws. It comes from the delayed snapshot, but the viewer's own screen
/// (the killcam bands, the killer card) is taken from the newest one: the game sends the archived
/// state and the viewer's HUD together, so the bands are up on the replay's first frame.
fn replay_view(newest: Arc<Snapshot>, delayed: Option<Arc<Snapshot>>, local: ClientId) -> Arc<Snapshot> {
    let Some(delayed) = delayed else {
        return newest;
    };
    if delayed.tick == newest.tick {
        return delayed;
    }
    let mut view = (*delayed).clone();
    if let (Some(own), Some((_, meta))) = (
        newest.meta.for_client(local),
        view.meta.clients.iter_mut().find(|(id, _)| *id == local),
    ) {
        meta.client_dvars = own.client_dvars.clone();
        meta.killcam_hud = own.killcam_hud.clone();
    }
    Arc::new(view)
}

/// The other players as the proxy draws them, and how each pose was got.
#[derive(Default)]
struct RemoteSamples {
    poses: HashMap<ClientId, PlayerState>,
    provenance: HashMap<ClientId, PresentationSampleProvenance>,
}

/// Samples every player in the snapshot except the local one, who is drawn from prediction. A
/// replay has no prediction, so the local seat is sampled like the rest.
fn remote_samples(
    proxy: &RemoteProxy,
    snapshot: &Snapshot,
    local: ClientId,
    archived: bool,
    at_ms: i32,
) -> RemoteSamples {
    let mut samples = RemoteSamples::default();
    for (id, _) in &snapshot.players {
        if *id == local && !archived {
            continue;
        }
        let (pose, provenance) = match proxy.interpolate_at(*id, at_ms) {
            ProxySample::Pose { ps, provenance } => (Some(ps), provenance),
            ProxySample::Starved { held, provenance } => (held, provenance),
        };
        if let Some(pose) = pose {
            samples.poses.insert(*id, pose);
        }
        samples.provenance.insert(*id, provenance);
    }
    samples
}

/// The local player's state from the two newest snapshots, for a replay. Both have to be live
/// frames: an archived one is already the state to show.
fn replay_local(
    earlier: Option<&Snapshot>,
    later_local: Option<&PlayerState>,
    local: ClientId,
    built_up: f32,
) -> Option<PlayerState> {
    let earlier_local = earlier?.players.iter().find(|(id, _)| *id == local).map(|(_, ps)| ps)?;
    let later_local = later_local?;
    (earlier_local.is_live_frame() && later_local.is_live_frame())
        .then(|| interpolate_player_state(earlier_local, later_local, built_up))
}

/// What the local player's state is made from this frame.
struct LocalSources<'a> {
    snapshot: Option<&'a PlayerState>,
    predicted: Option<PlayerState>,
    previous: Option<PlayerState>,
    replayed: Option<PlayerState>,
    armed: bool,
}

/// The state the local player is drawn from, and the name of where it came from.
fn pick_local(from: LocalSources) -> (PlayerState, &'static str) {
    if let Some(replayed) = from.replayed {
        return (replayed, "interpolated");
    }
    if from.armed {
        let choice = PresentedSnapshot::presentation_local_choice(
            from.snapshot,
            from.predicted.as_ref(),
            from.previous.as_ref(),
        );
        let state = PresentedSnapshot::presentation_local_player_state(from.snapshot, from.predicted, from.previous);
        (state, choice)
    } else {
        (
            PresentedSnapshot::presentation_local_player_state_unarmed(from.snapshot),
            PresentedSnapshot::presentation_local_choice_unarmed(from.snapshot),
        )
    }
}

/// Moves a body to where the proxy says the player it shows is, or lets it follow its own
/// trajectory when it is not a player the proxy knows. Says whether the proxy placed it.
fn place_body(body: &mut CEntityRuntime, samples: &RemoteSamples, bodies_ms: i32) -> bool {
    body.presented_player = None;
    let player = ClientId(u32::from(body.next_state.client_num));
    if i16::from(body.pose_e_type) == entity_t6::et::PLAYER
        && let Some(pose) = samples.poses.get(&player)
    {
        body.origin = pose.origin;
        body.angles = pose.viewangles;
        let at = sample_time(&samples.provenance[&player]);
        body.presented_player = Some((crate::player_state_to_entity_state(player, pose), at));
        true
    } else {
        body.present_pose(bodies_ms);
        false
    }
}

/// What the census says when there is nothing to show.
fn nothing_to_show(prediction: &ClientPredictionState) -> PresentLocalCensus {
    PresentLocalCensus {
        choice: Some("no_next"),
        snapshot_delta_time: None,
        predicted_delta_time: prediction.0.predicted_local().map(|ps| ps.delta_time),
    }
}

pub fn publish_presented(
    facts: PresentFacts,
    sinks: PresentSinks,
    mut bodies: Query<&mut CEntityRuntime>,
    mut killcam: Local<KillcamLog>,
) {
    let PresentSinks { mut presented, mut census, phase, trace } = sinks;
    let _stamp = PublishStamp { started: Instant::now(), phase };
    note_phase(trace, "Present");

    let PresentFacts { clock, cg_clock, role, prediction, last_adopted, proxy, local, has_world } = facts;
    let local = local.0;
    let replay = *role == RuntimeRole::Replay;

    if has_world.is_some_and(|world| !world.0) {
        presented.clear();
        *census = nothing_to_show(&prediction);
        return;
    }
    let Some(newest) = last_adopted.next_snap.clone() else {
        *census = nothing_to_show(&prediction);
        return;
    };

    let archived = newest.players.iter().any(|(id, ps)| *id == local && !ps.is_live_frame());
    let shown = if archived {
        replay_view(newest, proxy.0.snapshot_at(cg_clock.time()), local)
    } else {
        newest
    };
    let snapshot = shown.as_ref();
    let snapshot_local = snapshot.players.iter().find(|(id, _)| *id == local).map(|(_, ps)| ps);

    let built_up = clock.accumulator_ms / AUTHORITY_MS as f32;
    let earlier = if replay {
        last_adopted.snap.clone().filter(|earlier| earlier.tick.0 < snapshot.tick.0)
    } else {
        None
    };
    let (mut local_state, choice) = pick_local(LocalSources {
        snapshot: snapshot_local,
        predicted: prediction.0.predicted_local().copied(),
        previous: presented.player(local).copied(),
        replayed: replay_local(earlier.as_deref(), snapshot_local, local, built_up),
        armed: prediction.0.is_armed(),
    });
    *census = PresentLocalCensus {
        choice: Some(choice),
        snapshot_delta_time: snapshot_local.map(|ps| ps.delta_time),
        predicted_delta_time: prediction.0.predicted_local().map(|ps| ps.delta_time),
    };

    let times = draw_times(replay, archived, snapshot.tick, clock.accumulator_ms, cg_clock.time());
    if killcam_log::enabled() {
        killcam.note_frame(archived, cg_clock.time(), times.bodies_ms, &proxy.0);
    }

    let mut samples = remote_samples(&proxy.0, snapshot, local, archived, times.poses_ms);
    if archived && let Some(seat) = samples.poses.remove(&local) {
        local_state = seat;
    }
    for mut body in &mut bodies {
        let via_proxy = place_body(&mut body, &samples, times.bodies_ms);
        if killcam_log::enabled() {
            killcam.note_body(
                body.next_state.number,
                i32::from(body.next_state.client_num),
                body.pose_e_type,
                body.origin,
                via_proxy,
            );
        }
    }

    let mut predicted_projectiles = Vec::new();
    prediction.0.world().visit_projectiles(|projectile| predicted_projectiles.push(*projectile));
    let own_projectiles: &[_] = if archived { &[] } else { &predicted_projectiles };
    let projectiles = merge_presented_projectiles(snapshot.projectiles.as_slice(), own_projectiles, local);
    let view_offset = if archived {
        [0.0; 3]
    } else {
        prediction.0.predicted_error().offset_for_pm_type(local_state.pm_type)
    };

    presented.publish_proxied(shown.clone(), local, local_state, samples.poses, samples.provenance);
    // The gun's view reads the predicted player state whole, so with the predicted local state its
    // view values come from the predicted gun too. A held previous state keeps the ones it had.
    match census.choice {
        Some("predicted") if !archived => presented.set_local_t6_view(
            prediction.0.world().client_meta(local).and_then(|meta| meta.t6_predicted_view()),
        ),
        Some("previous") if !archived => {}
        _ => presented.set_local_t6_view(None),
    }
    presented.set_presented_projectiles(projectiles);
    presented.set_view_offset(view_offset);
    // The camera eases over a stair step.
    presented.set_view_z_drop(if archived { 0.0 } else { prediction.0.view_z().camera_drop(cg_clock.time()) });
    presented.set_snapshot_interpolation(earlier, built_up);
    presented.set_trajectory_sample(
        archived.then_some(times.bodies_ms),
        archived.then(|| proxy.0.snapshot_after(cg_clock.time())).flatten(),
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::proxy::{FIXED_DELAY_POLICY_REVISION, PresentationSampleTime, ProxyStarvationReason};

    const ME: ClientId = ClientId(1);
    const THEM: ClientId = ClientId(2);

    fn state_at(x: f32) -> PlayerState {
        PlayerState { origin: [x, 0.0, 0.0], ..PlayerState::ZERO }
    }

    fn archived_state() -> PlayerState {
        PlayerState { delta_time: 50, ..PlayerState::ZERO }
    }

    fn snapshot_of(tick: u32, players: &[(ClientId, PlayerState)]) -> Snapshot {
        let mut snapshot = Snapshot::unpublished(Tick(tick));
        snapshot.players.extend_from_slice(players);
        snapshot
    }

    fn provenance(outcome: PresentationSampleOutcome, effective_ms: i32) -> PresentationSampleProvenance {
        PresentationSampleProvenance {
            policy_revision: FIXED_DELAY_POLICY_REVISION,
            effective_time: PresentationSampleTime(effective_ms),
            outcome,
        }
    }

    // --- the two draw times ---

    #[test]
    fn a_live_match_draws_poses_now_and_bodies_one_step_behind() {
        let times = draw_times(false, false, Tick(9), 20.0, 1000);
        assert_eq!(times, DrawTimes { poses_ms: 1000, bodies_ms: 1000 - AUTHORITY_MS });
    }

    #[test]
    fn watching_a_replay_of_the_match_puts_bodies_the_whole_proxy_delay_behind() {
        let times = draw_times(false, true, Tick(9), 20.0, 1000);
        assert_eq!(times, DrawTimes { poses_ms: 1000, bodies_ms: 1000 - PROXY_DELAY_MS });
    }

    #[test]
    fn a_replay_file_runs_on_the_clock_of_its_snapshots() {
        let times = draw_times(true, false, Tick(10), 20.0, 123_456);
        assert_eq!(times.poses_ms, 11 * AUTHORITY_MS + 20);
        assert_eq!(times.bodies_ms, 9 * AUTHORITY_MS + 20);
        // Being archived makes no difference to a replay file.
        assert_eq!(draw_times(true, true, Tick(10), 20.0, 0), times);
    }

    #[test]
    fn the_built_up_part_of_a_step_is_cut_to_whole_milliseconds() {
        let times = draw_times(true, false, Tick(0), 19.9, 0);
        assert_eq!(times.poses_ms, AUTHORITY_MS + 19);
    }

    #[test]
    fn a_game_clock_near_the_bottom_of_the_range_does_not_wrap() {
        let times = draw_times(false, false, Tick(0), 0.0, i32::MIN + 1);
        assert_eq!(times.bodies_ms, i32::MIN);
    }

    // --- when a body was sampled ---

    #[test]
    fn a_whole_snapshot_is_dated_by_its_tick() {
        let exact = provenance(PresentationSampleOutcome::Exact { snapshot: Tick(7) }, 999);
        assert_eq!(sample_time(&exact), 7 * AUTHORITY_MS);
        let held = provenance(
            PresentationSampleOutcome::Starved {
                reason: ProxyStarvationReason::AfterNewest,
                held_snapshot: Some(Tick(4)),
            },
            999,
        );
        assert_eq!(sample_time(&held), 4 * AUTHORITY_MS);
    }

    #[test]
    fn a_blend_is_dated_by_the_time_it_was_taken_at() {
        let blend = provenance(
            PresentationSampleOutcome::Interpolated { left: Tick(3), right: Tick(4), alpha: 0.5 },
            175,
        );
        assert_eq!(sample_time(&blend), 175);
    }

    // --- the world a replay draws ---

    fn with_screen(tick: u32, dvar: &str, final_kill: bool) -> Arc<Snapshot> {
        let mut snapshot = snapshot_of(tick, &[(ME, archived_state())]);
        snapshot.meta.clients.push((
            ME,
            sim::ClientSnapshotMeta {
                client_dvars: vec![(dvar.to_owned(), "1".to_owned())],
                killcam_hud: Some(sim::KillcamHud { final_kill }),
                ..Default::default()
            },
        ));
        Arc::new(snapshot)
    }

    #[test]
    fn without_a_delayed_snapshot_the_newest_is_shown() {
        let newest = with_screen(9, "new", false);
        assert!(Arc::ptr_eq(&replay_view(newest.clone(), None, ME), &newest));
    }

    #[test]
    fn when_the_delayed_snapshot_is_the_newest_it_is_shown_as_it_is() {
        let newest = with_screen(9, "new", false);
        let delayed = with_screen(9, "old", true);
        assert!(Arc::ptr_eq(&replay_view(newest, Some(delayed.clone()), ME), &delayed));
    }

    #[test]
    fn an_older_world_keeps_the_viewers_screen_from_the_newest_snapshot() {
        let newest = with_screen(9, "new", true);
        let delayed = with_screen(6, "old", false);
        let view = replay_view(newest, Some(delayed), ME);
        assert_eq!(view.tick, Tick(6));
        let meta = view.meta.for_client(ME).unwrap();
        assert_eq!(meta.client_dvars, [("new".to_owned(), "1".to_owned())]);
        assert!(meta.killcam_hud.as_ref().unwrap().final_kill);
    }

    #[test]
    fn the_delayed_snapshot_itself_is_not_changed() {
        let newest = with_screen(9, "new", true);
        let delayed = with_screen(6, "old", false);
        let _ = replay_view(newest, Some(delayed.clone()), ME);
        assert_eq!(delayed.meta.for_client(ME).unwrap().client_dvars[0].0, "old");
    }

    #[test]
    fn a_viewer_missing_from_the_newest_snapshot_leaves_the_older_screen_alone() {
        let newest = Arc::new(snapshot_of(9, &[]));
        let delayed = with_screen(6, "old", false);
        let view = replay_view(newest, Some(delayed), ME);
        assert_eq!(view.meta.for_client(ME).unwrap().client_dvars[0].0, "old");
    }

    // --- the other players ---

    #[test]
    fn the_local_player_is_not_sampled_unless_a_replay_is_being_watched() {
        let snapshot = snapshot_of(1, &[(ME, state_at(1.0)), (THEM, state_at(2.0))]);
        let proxy = RemoteProxy::default();
        let live = remote_samples(&proxy, &snapshot, ME, false, 0);
        assert_eq!(live.provenance.keys().copied().collect::<Vec<_>>(), [THEM]);
        let watching = remote_samples(&proxy, &snapshot, ME, true, 0);
        assert_eq!(watching.provenance.len(), 2);
    }

    #[test]
    fn a_player_the_proxy_cannot_place_still_gets_a_provenance_but_no_pose() {
        let snapshot = snapshot_of(1, &[(THEM, state_at(2.0))]);
        let samples = remote_samples(&RemoteProxy::default(), &snapshot, ME, false, 0);
        assert!(samples.poses.is_empty());
        assert!(matches!(
            samples.provenance[&THEM].outcome,
            PresentationSampleOutcome::Starved { held_snapshot: None, .. }
        ));
    }

    // --- the local player ---

    fn sources<'a>(snapshot: Option<&'a PlayerState>) -> LocalSources<'a> {
        LocalSources { snapshot, predicted: None, previous: None, replayed: None, armed: true }
    }

    #[test]
    fn a_replayed_state_beats_everything() {
        let snapshot = state_at(7.0);
        let from = LocalSources { replayed: Some(state_at(5.0)), predicted: Some(state_at(6.0)), ..sources(Some(&snapshot)) };
        let (state, choice) = pick_local(from);
        assert_eq!((state.origin[0], choice), (5.0, "interpolated"));
    }

    #[test]
    fn an_armed_client_draws_its_prediction() {
        let snapshot = state_at(1.0);
        let from = LocalSources { predicted: Some(state_at(2.0)), previous: Some(state_at(3.0)), ..sources(Some(&snapshot)) };
        let (state, choice) = pick_local(from);
        assert_eq!((state.origin[0], choice), (2.0, "predicted"));
    }

    #[test]
    fn without_a_prediction_the_last_picture_is_held() {
        let snapshot = state_at(1.0);
        let (state, choice) = pick_local(LocalSources { previous: Some(state_at(3.0)), ..sources(Some(&snapshot)) });
        assert_eq!((state.origin[0], choice), (3.0, "previous"));
    }

    #[test]
    fn an_unarmed_client_draws_the_snapshot_and_a_missing_one_draws_zero() {
        let snapshot = state_at(1.0);
        let (state, choice) = pick_local(LocalSources {
            armed: false,
            predicted: Some(state_at(2.0)),
            ..sources(Some(&snapshot))
        });
        assert_eq!((state.origin[0], choice), (1.0, "snapshot"));
        let (state, choice) = pick_local(LocalSources { armed: false, ..sources(None) });
        assert_eq!((state, choice), (PlayerState::ZERO, "zero"));
    }

    #[test]
    fn an_archived_snapshot_state_beats_the_prediction() {
        let snapshot = archived_state();
        let from = LocalSources { predicted: Some(state_at(2.0)), ..sources(Some(&snapshot)) };
        let (state, choice) = pick_local(from);
        assert_eq!((state.delta_time, choice), (50, "archived"));
    }

    #[test]
    fn a_replay_blends_the_two_snapshots_by_the_built_up_part_of_a_step() {
        let earlier = snapshot_of(1, &[(ME, state_at(0.0))]);
        let later = state_at(10.0);
        let blended = replay_local(Some(&earlier), Some(&later), ME, 0.25).unwrap();
        assert_eq!(blended.origin[0], 2.5);
    }

    #[test]
    fn nothing_is_blended_across_an_archived_frame_or_with_a_snapshot_missing() {
        let earlier = snapshot_of(1, &[(ME, state_at(0.0))]);
        assert!(replay_local(Some(&earlier), Some(&archived_state()), ME, 0.5).is_none());
        let archived_earlier = snapshot_of(1, &[(ME, archived_state())]);
        assert!(replay_local(Some(&archived_earlier), Some(&state_at(1.0)), ME, 0.5).is_none());
        assert!(replay_local(None, Some(&state_at(1.0)), ME, 0.5).is_none());
        assert!(replay_local(Some(&earlier), None, ME, 0.5).is_none());
        let elsewhere = snapshot_of(1, &[(THEM, state_at(0.0))]);
        assert!(replay_local(Some(&elsewhere), Some(&state_at(1.0)), ME, 0.5).is_none());
    }

    // --- the bodies ---

    fn a_player_body(client: u8) -> CEntityRuntime {
        let mut body = CEntityRuntime::default();
        body.pose_e_type = entity_t6::et::PLAYER as u8;
        body.next_state.client_num = client;
        body
    }

    fn samples_for(id: ClientId, pose: PlayerState) -> RemoteSamples {
        let mut samples = RemoteSamples::default();
        samples.poses.insert(id, pose);
        samples
            .provenance
            .insert(id, provenance(PresentationSampleOutcome::Exact { snapshot: Tick(7) }, 0));
        samples
    }

    #[test]
    fn a_player_body_is_moved_to_the_pose_the_proxy_gave() {
        let mut pose = state_at(40.0);
        pose.viewangles = [1.0, 2.0, 3.0];
        let mut body = a_player_body(2);
        assert!(place_body(&mut body, &samples_for(THEM, pose), 0));
        assert_eq!(body.origin, [40.0, 0.0, 0.0]);
        assert_eq!(body.angles, [1.0, 2.0, 3.0]);
        let (_, at) = body.presented_player.expect("the pose is recorded on the body");
        assert_eq!(at, 7 * AUTHORITY_MS);
    }

    #[test]
    fn a_body_the_proxy_has_no_pose_for_follows_its_own_trajectory() {
        let mut body = a_player_body(3);
        body.origin = [9.0; 3];
        body.presented_player = Some((Default::default(), 1));
        assert!(!place_body(&mut body, &samples_for(THEM, state_at(40.0)), 0));
        assert_eq!(body.origin, [9.0; 3]);
        assert_eq!(body.presented_player, None);
    }

    #[test]
    fn a_body_that_is_not_a_player_ignores_the_proxy() {
        let mut body = a_player_body(2);
        body.pose_e_type = 0;
        assert!(!place_body(&mut body, &samples_for(THEM, state_at(40.0)), 0));
        assert_eq!(body.origin, [0.0; 3]);
    }

    // --- the whole system ---

    fn present_app(role: RuntimeRole) -> App {
        let mut app = App::new();
        app.insert_resource(role)
            .init_resource::<ClientClock>()
            .init_resource::<FrameClock>()
            .init_resource::<ClientPredictionState>()
            .init_resource::<LastAdoptedSnapshot>()
            .init_resource::<RemoteProxyState>()
            .init_resource::<PresentedSnapshot>()
            .init_resource::<PresentLocalCensus>()
            .insert_resource(LocalPresentClient(ME))
            .add_systems(Update, publish_presented);
        app
    }

    fn adopt(app: &mut App, earlier: Option<Snapshot>, newest: Snapshot) {
        let mut adopted = app.world_mut().resource_mut::<LastAdoptedSnapshot>();
        adopted.snap = earlier.map(Arc::new);
        adopted.next_snap = Some(Arc::new(newest));
    }

    fn census(app: &App) -> PresentLocalCensus {
        *app.world().resource::<PresentLocalCensus>()
    }

    #[test]
    fn with_no_snapshot_taken_in_the_picture_is_left_alone_and_the_census_says_so() {
        let mut app = present_app(RuntimeRole::Client);
        app.update();
        assert_eq!(census(&app).choice, Some("no_next"));
        assert_eq!(census(&app).snapshot_delta_time, None);
        assert!(app.world().resource::<PresentedSnapshot>().snapshot().is_none());
    }

    #[test]
    fn without_a_world_the_picture_is_cleared() {
        let mut app = present_app(RuntimeRole::Client);
        adopt(&mut app, None, snapshot_of(1, &[(ME, state_at(1.0))]));
        app.update();
        assert!(app.world().resource::<PresentedSnapshot>().snapshot().is_some());
        app.insert_resource(HasWorld(false));
        app.update();
        assert!(app.world().resource::<PresentedSnapshot>().snapshot().is_none());
        assert_eq!(census(&app).choice, Some("no_next"));
    }

    #[test]
    fn a_snapshot_without_a_prediction_is_shown_as_it_is() {
        let mut app = present_app(RuntimeRole::Client);
        adopt(&mut app, None, snapshot_of(1, &[(ME, state_at(1.0)), (THEM, state_at(2.0))]));
        app.update();
        let presented = app.world().resource::<PresentedSnapshot>();
        assert_eq!(presented.snapshot().map(|snapshot| snapshot.tick), Some(Tick(1)));
        assert_eq!(presented.player(ME).map(|ps| ps.origin[0]), Some(1.0));
        assert_eq!(census(&app).choice, Some("snapshot"));
        assert_eq!(census(&app).snapshot_delta_time, Some(0));
    }

    #[test]
    fn the_time_taken_and_the_phase_are_recorded() {
        let mut app = present_app(RuntimeRole::Client);
        app.init_resource::<crate::UpdatePhaseCensus>().init_resource::<ClientPhaseTrace>();
        app.update();
        assert!(app.world().resource::<crate::UpdatePhaseCensus>().publish_presented_ms.is_some());
        assert_eq!(app.world().resource::<ClientPhaseTrace>().0, ["Present"]);
    }

    #[test]
    fn watching_a_replay_of_the_match_samples_trajectories_at_the_body_time() {
        let mut app = present_app(RuntimeRole::Client);
        adopt(&mut app, None, snapshot_of(1, &[(ME, archived_state())]));
        app.update();
        let presented = app.world().resource::<PresentedSnapshot>();
        assert_eq!(census(&app).choice, Some("archived"));
        assert_eq!(presented.trajectory_time_ms(12_345), 0 - PROXY_DELAY_MS);
    }

    #[test]
    fn a_live_picture_leaves_trajectories_on_the_render_time() {
        let mut app = present_app(RuntimeRole::Client);
        adopt(&mut app, None, snapshot_of(1, &[(ME, state_at(1.0))]));
        app.update();
        assert_eq!(app.world().resource::<PresentedSnapshot>().trajectory_time_ms(12_345), 12_345);
    }

    #[test]
    fn a_replay_file_blends_the_local_player_between_its_two_snapshots() {
        let mut app = present_app(RuntimeRole::Replay);
        app.world_mut().resource_mut::<ClientClock>().accumulator_ms = AUTHORITY_MS as f32 / 2.0;
        adopt(&mut app, Some(snapshot_of(1, &[(ME, state_at(0.0))])), snapshot_of(2, &[(ME, state_at(10.0))]));
        app.update();
        assert_eq!(census(&app).choice, Some("interpolated"));
        let presented = app.world().resource::<PresentedSnapshot>();
        assert_eq!(presented.player(ME).map(|ps| ps.origin[0]), Some(5.0));
        assert_eq!(presented.interpolation_pair().map(|(_, _, fraction)| fraction), Some(0.5));
    }

    #[test]
    fn only_a_replay_file_blends_between_snapshots() {
        let mut app = present_app(RuntimeRole::Client);
        adopt(&mut app, Some(snapshot_of(1, &[(ME, state_at(0.0))])), snapshot_of(2, &[(ME, state_at(10.0))]));
        app.update();
        assert_eq!(census(&app).choice, Some("snapshot"));
        assert!(app.world().resource::<PresentedSnapshot>().interpolation_pair().is_none());
    }

    #[test]
    fn player_bodies_in_the_world_are_placed_each_frame() {
        let mut app = present_app(RuntimeRole::Client);
        adopt(&mut app, None, snapshot_of(1, &[(ME, state_at(1.0)), (THEM, state_at(2.0))]));
        let body = app.world_mut().spawn(a_player_body(2)).id();
        app.world_mut().get_mut::<CEntityRuntime>(body).unwrap().presented_player = Some((Default::default(), 5));
        app.update();
        // The proxy holds nothing for them yet, so the body is left on its own trajectory.
        assert_eq!(app.world().get::<CEntityRuntime>(body).unwrap().presented_player, None);
    }
}
