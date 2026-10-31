//! The frame's picture of the match. What the screen shows this frame is put together here
//! from the snapshot just taken in, the delayed poses of the other players and the local
//! prediction, then handed to [`ShownWorldFrame`] and to the bodies in the world.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;

use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use frame::{LevelPresent, ProcessDuty};
use playerstate_t6::game::AvatarState;
use sim::{ClientId, Snapshot, Tick};

use super::killcam_log::{self, KillcamLog};
use super::stores::{ReceiverStageLog, ClientGuessState, NewestTakenSnapshot, OtherPlayerSmootherHolder, note_phase};
use super::ticker::PlayerTimeStepper;
use crate::ClientEntityPoseState;
use crate::host_tick::peer_mail::HOST_STEP_MILLIS;
use crate::client::frame_clock::FrameClock;
use crate::client::presentation::shown_frame::{
    HostViewClient, OwnViewerTally, ShownWorldFrame, blend_participant_state,
};
use crate::client::presentation::seen_throws::combine_shown_missiles;
use crate::client::remote_buffer::{
    STANDIN_LAG_MILLIS, SmoothingSampleVerdict, SmoothingSampleOrigin, SmootherReading, OtherPlayerSmoother,
};

/// What the frame already knows, none of it changed here.
#[derive(SystemParam)]
pub struct PresentFacts<'w> {
    clock: Res<'w, PlayerTimeStepper>,
    game_clock: Res<'w, FrameClock>,
    duty: Res<'w, ProcessDuty>,
    forecast: Res<'w, ClientGuessState>,
    prior_adopt: Res<'w, NewestTakenSnapshot>,
    proxy: Res<'w, OtherPlayerSmootherHolder>,
    own_id: Res<'w, HostViewClient>,
    level_is_up: Option<Res<'w, LevelPresent>>,
}

/// Where the picture is put.
#[derive(SystemParam)]
pub struct PresentSinks<'w> {
    world_view: ResMut<'w, ShownWorldFrame>,
    census: ResMut<'w, OwnViewerTally>,
    stage: Option<ResMut<'w, crate::FrameUpdateTally>>,
    trace: Option<ResMut<'w, ReceiverStageLog>>,
}

/// Writes how long the system took into the frame census when it is dropped, whichever way the
/// system returns.
struct AnnounceMark<'w> {
    began: Instant,
    stage: Option<ResMut<'w, crate::FrameUpdateTally>>,
}

impl Drop for AnnounceMark<'_> {
    fn drop(&mut self) {
        let ms = self.began.elapsed().as_secs_f32() * 1000.0;
        if let Some(stage) = self.stage.as_mut() {
            stage.announce_shown_millis = Some(ms);
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
    let tick_ms = tick.0 as i32 * HOST_STEP_MILLIS;
    if replay {
        return DrawTimes {
            poses_ms: tick_ms + HOST_STEP_MILLIS + built_up_ms as i32,
            bodies_ms: tick_ms - HOST_STEP_MILLIS + built_up_ms as i32,
        };
    }
    DrawTimes {
        poses_ms: game_ms,
        bodies_ms: game_ms.saturating_sub(if archived { STANDIN_LAG_MILLIS } else { HOST_STEP_MILLIS }),
    }
}

/// The time a body was last sampled for: the time of its snapshot when a whole snapshot was
/// used, otherwise the time the blend was taken at.
fn sample_time(sample_origin_tag: &SmoothingSampleOrigin) -> i32 {
    match sample_origin_tag.outcome {
        SmoothingSampleVerdict::Exact { whole_frame }
        | SmoothingSampleVerdict::RanDry { retained_world_state: Some(whole_frame), .. } => {
            whole_frame.0 as i32 * HOST_STEP_MILLIS
        }
        SmoothingSampleVerdict::SmoothedBetweenFrames { .. } => sample_origin_tag.resolved_clock.0,
        SmoothingSampleVerdict::RanDry { retained_world_state: None, .. } => {
            unreachable!("a proxy pose always has a source snapshot")
        }
    }
}

/// The world a replay draws. It comes from the delayed snapshot, but the viewer's own screen
/// (the killcam bands, the killer card) is taken from the newest one: the game sends the archived
/// state and the viewer's HUD together, so the bands are up on the replay's first frame.
fn replay_view(newest: Arc<Snapshot>, delayed: Option<Arc<Snapshot>>, own_id: ClientId) -> Arc<Snapshot> {
    let Some(delayed) = delayed else {
        return newest;
    };
    if delayed.tick == newest.tick {
        return delayed;
    }
    let mut view = (*delayed).clone();
    if let (Some(own), Some((_, meta))) = (
        newest.meta.for_client(own_id),
        view.meta.clients.iter_mut().find(|(id, _)| *id == own_id),
    ) {
        meta.client_dvars = own.client_dvars.clone();
        meta.killcam_hud = own.killcam_hud.clone();
    }
    Arc::new(view)
}

/// The other players as the proxy draws them, and how each pose was got.
#[derive(Default)]
struct RemoteSamples {
    poses: HashMap<ClientId, AvatarState>,
    sample_origin_tag: HashMap<ClientId, SmoothingSampleOrigin>,
}

/// Samples every player in the snapshot except the local one, who is drawn from prediction. A
/// replay has no prediction, so the local seat is sampled like the rest.
fn remote_samples(
    proxy: &OtherPlayerSmoother,
    frame_snap: &Snapshot,
    own_id: ClientId,
    archived: bool,
    at_ms: i32,
) -> RemoteSamples {
    let mut samples = RemoteSamples::default();
    for (id, _) in &frame_snap.players {
        if *id == own_id && !archived {
            continue;
        }
        let (pose, sample_origin_tag) = match proxy.pose_for(*id, at_ms) {
            SmootherReading::Pose { ps, sample_origin_tag } => (Some(ps), sample_origin_tag),
            SmootherReading::RanDry { held, sample_origin_tag } => (held, sample_origin_tag),
        };
        if let Some(pose) = pose {
            samples.poses.insert(*id, pose);
        }
        samples.sample_origin_tag.insert(*id, sample_origin_tag);
    }
    samples
}

/// The local player's state from the two newest snapshots, for a replay. Both have to be live
/// frames: an archived one is already the state to show.
fn replay_local(
    earlier: Option<&Snapshot>,
    later_local: Option<&AvatarState>,
    own_id: ClientId,
    built_up: f32,
) -> Option<AvatarState> {
    let earlier_local = earlier?.players.iter().find(|(id, _)| *id == own_id).map(|(_, ps)| ps)?;
    let later_local = later_local?;
    (earlier_local.is_live_frame() && later_local.is_live_frame())
        .then(|| blend_participant_state(earlier_local, later_local, built_up))
}

/// What the local player's state is made from this frame.
struct LocalSources<'a> {
    snapshot: Option<&'a AvatarState>,
    predicted: Option<AvatarState>,
    previous: Option<AvatarState>,
    replayed: Option<AvatarState>,
    engaged: bool,
}

/// The state the local player is drawn from, and the name of where it came from.
fn choose_local_pose(from: LocalSources) -> (AvatarState, &'static str) {
    if let Some(replayed) = from.replayed {
        return (replayed, "interpolated");
    }
    if from.engaged {
        let choice = ShownWorldFrame::display_self_pick(
            from.snapshot,
            from.predicted.as_ref(),
            from.previous.as_ref(),
        );
        let state = ShownWorldFrame::display_self_participant_state(from.snapshot, from.predicted, from.previous);
        (state, choice)
    } else {
        (
            ShownWorldFrame::display_self_participant_state_bare(from.snapshot),
            ShownWorldFrame::presentation_local_choice_unarmed(from.snapshot),
        )
    }
}

/// Moves a body to where the proxy says the player it shows is, or lets it follow its own
/// trajectory when it is not a player the proxy knows. Says whether the proxy placed it.
fn place_body(body: &mut ClientEntityPoseState, samples: &RemoteSamples, bodies_ms: i32) -> bool {
    body.shown_participant = None;
    let player = ClientId(u32::from(body.upcoming_condition.client_num));
    if i16::from(body.pose_entity_kind) == entity_t6::et::PLAYER
        && let Some(pose) = samples.poses.get(&player)
    {
        body.origin = pose.origin;
        body.angles = pose.look_angles;
        let at = sample_time(&samples.sample_origin_tag[&player]);
        body.shown_participant = Some((crate::avatar_entity_row(player, pose), at));
        true
    } else {
        body.settle_pose(bodies_ms);
        false
    }
}

/// What the census says when there is nothing to show.
fn empty_display(forecast: &ClientGuessState) -> OwnViewerTally {
    OwnViewerTally {
        picked_view: Some("no_next"),
        frame_gap_millis: None,
        forecast_step_gap: forecast.0.predicted_local().map(|ps| ps.time_slice),
    }
}

pub fn expose_shown_frame(
    facts: PresentFacts,
    sinks: PresentSinks,
    mut bodies: Query<&mut ClientEntityPoseState>,
    mut killcam: Local<KillcamLog>,
) {
    let PresentSinks { mut world_view, mut census, stage, trace } = sinks;
    let _stamp = AnnounceMark { began: Instant::now(), stage };
    note_phase(trace, "Present");

    let PresentFacts { clock, game_clock, duty, forecast, prior_adopt, proxy, own_id, level_is_up } = facts;
    let own_id = own_id.0;
    let replay = *duty == ProcessDuty::Replay;

    if level_is_up.is_some_and(|world| !world.0) {
        world_view.clear();
        *census = empty_display(&forecast);
        return;
    }
    let Some(newest) = prior_adopt.upcoming_world_state.clone() else {
        *census = empty_display(&forecast);
        return;
    };

    let archived = newest.players.iter().any(|(id, ps)| *id == own_id && !ps.is_live_frame());
    let shown = if archived {
        replay_view(newest, proxy.0.blended_frame_at(game_clock.time()), own_id)
    } else {
        newest
    };
    let snapshot = shown.as_ref();
    let world_state_of_self = snapshot.players.iter().find(|(id, _)| *id == own_id).map(|(_, ps)| ps);

    let built_up = clock.carryover_millis / HOST_STEP_MILLIS as f32;
    let earlier = if replay {
        prior_adopt.frame_state.clone().filter(|earlier| earlier.tick.0 < snapshot.tick.0)
    } else {
        None
    };
    let (mut local_state, choice) = choose_local_pose(LocalSources {
        snapshot: world_state_of_self,
        predicted: forecast.0.predicted_local().copied(),
        previous: world_view.player(own_id).copied(),
        replayed: replay_local(earlier.as_deref(), world_state_of_self, own_id, built_up),
        engaged: forecast.0.is_forecast_live(),
    });
    *census = OwnViewerTally {
        picked_view: Some(choice),
        frame_gap_millis: world_state_of_self.map(|ps| ps.time_slice),
        forecast_step_gap: forecast.0.predicted_local().map(|ps| ps.time_slice),
    };

    let times = draw_times(replay, archived, snapshot.tick, clock.carryover_millis, game_clock.time());
    if killcam_log::is_switched_on() {
        killcam.note_frame(archived, game_clock.time(), times.bodies_ms, &proxy.0);
    }

    let mut samples = remote_samples(&proxy.0, snapshot, own_id, archived, times.poses_ms);
    if archived && let Some(seat) = samples.poses.remove(&own_id) {
        local_state = seat;
    }
    for mut body in &mut bodies {
        let via_proxy = place_body(&mut body, &samples, times.bodies_ms);
        if killcam_log::is_switched_on() {
            killcam.note_body(
                body.upcoming_condition.slot_index,
                i32::from(body.upcoming_condition.client_num),
                body.pose_entity_kind,
                body.origin,
                via_proxy,
            );
        }
    }

    let mut forecast_missiles = Vec::new();
    forecast.0.world().visit_projectiles(|projectile| forecast_missiles.push(*projectile));
    let own_projectiles: &[_] = if archived { &[] } else { &forecast_missiles };
    let projectiles = combine_shown_missiles(snapshot.projectiles.as_slice(), own_projectiles, own_id);
    let eye_displacement = if archived {
        [0.0; 3]
    } else {
        forecast.0.predicted_error().correction_for_move_mode(local_state.movement_mode)
    };

    world_view.announce_standin(shown.clone(), own_id, local_state, samples.poses, samples.sample_origin_tag);
    // The gun's view reads the predicted player state whole, so with the predicted local state its
    // view values come from the predicted gun too. A held previous state keeps the ones it had.
    match census.picked_view {
        Some("predicted") if !archived => world_view.set_local_t6_view(
            forecast.0.world().client_meta(own_id).and_then(|meta| meta.t6_predicted_view()),
        ),
        Some("previous") if !archived => {}
        _ => world_view.set_local_t6_view(None),
    }
    world_view.store_projectile_rows(projectiles);
    world_view.store_eye_shift(eye_displacement);
    // The camera eases over a stair step.
    world_view.set_view_z_drop(if archived { 0.0 } else { forecast.0.view_z().camera_drop(game_clock.time()) });
    world_view.assign_state_blending(earlier, built_up);
    world_view.set_trajectory_sample(
        archived.then_some(times.bodies_ms),
        archived.then(|| proxy.0.snapshot_after(game_clock.time())).flatten(),
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::remote_buffer::{CONSTANT_LAG_POLICY_GENERATION, SmoothingInstant, RemoteBufferShortfall};

    const ME: ClientId = ClientId(1);
    const THEM: ClientId = ClientId(2);

    fn state_at(x: f32) -> AvatarState {
        AvatarState { origin: [x, 0.0, 0.0], ..AvatarState::ZERO }
    }

    fn archived_state() -> AvatarState {
        AvatarState { time_slice: 50, ..AvatarState::ZERO }
    }

    fn snapshot_of(tick: u32, roster: &[(ClientId, AvatarState)]) -> Snapshot {
        let mut snapshot = Snapshot::unpublished(Tick(tick));
        snapshot.players.extend_from_slice(roster);
        snapshot
    }

    fn sample_origin_tag(outcome: SmoothingSampleVerdict, effective_ms: i32) -> SmoothingSampleOrigin {
        SmoothingSampleOrigin {
            policy_revision: CONSTANT_LAG_POLICY_GENERATION,
            resolved_clock: SmoothingInstant(effective_ms),
            outcome,
        }
    }

    // --- the two draw times ---

    #[test]
    fn a_live_match_draws_poses_now_and_bodies_one_step_behind() {
        let times = draw_times(false, false, Tick(9), 20.0, 1000);
        assert_eq!(times, DrawTimes { poses_ms: 1000, bodies_ms: 1000 - HOST_STEP_MILLIS });
    }

    #[test]
    fn watching_a_replay_of_the_match_puts_bodies_the_whole_proxy_delay_behind() {
        let times = draw_times(false, true, Tick(9), 20.0, 1000);
        assert_eq!(times, DrawTimes { poses_ms: 1000, bodies_ms: 1000 - STANDIN_LAG_MILLIS });
    }

    #[test]
    fn a_replay_file_runs_on_the_clock_of_its_snapshots() {
        let times = draw_times(true, false, Tick(10), 20.0, 123_456);
        assert_eq!(times.poses_ms, 11 * HOST_STEP_MILLIS + 20);
        assert_eq!(times.bodies_ms, 9 * HOST_STEP_MILLIS + 20);
        // Being archived makes no difference to a replay file.
        assert_eq!(draw_times(true, true, Tick(10), 20.0, 0), times);
    }

    #[test]
    fn the_built_up_part_of_a_step_is_cut_to_whole_milliseconds() {
        let times = draw_times(true, false, Tick(0), 19.9, 0);
        assert_eq!(times.poses_ms, HOST_STEP_MILLIS + 19);
    }

    #[test]
    fn a_game_clock_near_the_bottom_of_the_range_does_not_wrap() {
        let times = draw_times(false, false, Tick(0), 0.0, i32::MIN + 1);
        assert_eq!(times.bodies_ms, i32::MIN);
    }

    // --- when a body was sampled ---

    #[test]
    fn a_whole_snapshot_is_dated_by_its_tick() {
        let exact = sample_origin_tag(SmoothingSampleVerdict::Exact { whole_frame: Tick(7) }, 999);
        assert_eq!(sample_time(&exact), 7 * HOST_STEP_MILLIS);
        let held = sample_origin_tag(
            SmoothingSampleVerdict::RanDry {
                reason: RemoteBufferShortfall::PastNewestFrame,
                retained_world_state: Some(Tick(4)),
            },
            999,
        );
        assert_eq!(sample_time(&held), 4 * HOST_STEP_MILLIS);
    }

    #[test]
    fn a_blend_is_dated_by_the_time_it_was_taken_at() {
        let blend = sample_origin_tag(
            SmoothingSampleVerdict::SmoothedBetweenFrames { from_tick: Tick(3), to_tick: Tick(4), alpha: 0.5 },
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
        let proxy = OtherPlayerSmoother::default();
        let live = remote_samples(&proxy, &snapshot, ME, false, 0);
        assert_eq!(live.sample_origin_tag.keys().copied().collect::<Vec<_>>(), [THEM]);
        let watching = remote_samples(&proxy, &snapshot, ME, true, 0);
        assert_eq!(watching.sample_origin_tag.len(), 2);
    }

    #[test]
    fn a_player_the_proxy_cannot_place_still_gets_a_provenance_but_no_pose() {
        let snapshot = snapshot_of(1, &[(THEM, state_at(2.0))]);
        let samples = remote_samples(&OtherPlayerSmoother::default(), &snapshot, ME, false, 0);
        assert!(samples.poses.is_empty());
        assert!(matches!(
            samples.sample_origin_tag[&THEM].outcome,
            SmoothingSampleVerdict::RanDry { retained_world_state: None, .. }
        ));
    }

    // --- the local player ---

    fn sources<'a>(snapshot: Option<&'a AvatarState>) -> LocalSources<'a> {
        LocalSources { snapshot, predicted: None, previous: None, replayed: None, engaged: true }
    }

    #[test]
    fn a_replayed_state_beats_everything() {
        let snapshot = state_at(7.0);
        let from = LocalSources { replayed: Some(state_at(5.0)), predicted: Some(state_at(6.0)), ..sources(Some(&snapshot)) };
        let (state, choice) = choose_local_pose(from);
        assert_eq!((state.origin[0], choice), (5.0, "interpolated"));
    }

    #[test]
    fn an_armed_client_draws_its_prediction() {
        let snapshot = state_at(1.0);
        let from = LocalSources { predicted: Some(state_at(2.0)), previous: Some(state_at(3.0)), ..sources(Some(&snapshot)) };
        let (state, choice) = choose_local_pose(from);
        assert_eq!((state.origin[0], choice), (2.0, "predicted"));
    }

    #[test]
    fn without_a_prediction_the_last_picture_is_held() {
        let snapshot = state_at(1.0);
        let (state, choice) = choose_local_pose(LocalSources { previous: Some(state_at(3.0)), ..sources(Some(&snapshot)) });
        assert_eq!((state.origin[0], choice), (3.0, "previous"));
    }

    #[test]
    fn an_unarmed_client_draws_the_snapshot_and_a_missing_one_draws_zero() {
        let snapshot = state_at(1.0);
        let (state, choice) = choose_local_pose(LocalSources {
            engaged: false,
            predicted: Some(state_at(2.0)),
            ..sources(Some(&snapshot))
        });
        assert_eq!((state.origin[0], choice), (1.0, "snapshot"));
        let (state, choice) = choose_local_pose(LocalSources { engaged: false, ..sources(None) });
        assert_eq!((state, choice), (AvatarState::ZERO, "zero"));
    }

    #[test]
    fn an_archived_snapshot_state_beats_the_prediction() {
        let snapshot = archived_state();
        let from = LocalSources { predicted: Some(state_at(2.0)), ..sources(Some(&snapshot)) };
        let (state, choice) = choose_local_pose(from);
        assert_eq!((state.time_slice, choice), (50, "archived"));
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

    fn a_player_body(client: u8) -> ClientEntityPoseState {
        let mut body = ClientEntityPoseState::default();
        body.pose_entity_kind = entity_t6::et::PLAYER as u8;
        body.upcoming_condition.client_num = client;
        body
    }

    fn samples_for(id: ClientId, pose: AvatarState) -> RemoteSamples {
        let mut samples = RemoteSamples::default();
        samples.poses.insert(id, pose);
        samples
            .sample_origin_tag
            .insert(id, sample_origin_tag(SmoothingSampleVerdict::Exact { whole_frame: Tick(7) }, 0));
        samples
    }

    #[test]
    fn a_player_body_is_moved_to_the_pose_the_proxy_gave() {
        let mut pose = state_at(40.0);
        pose.look_angles = [1.0, 2.0, 3.0];
        let mut body = a_player_body(2);
        assert!(place_body(&mut body, &samples_for(THEM, pose), 0));
        assert_eq!(body.origin, [40.0, 0.0, 0.0]);
        assert_eq!(body.angles, [1.0, 2.0, 3.0]);
        let (_, at) = body.shown_participant.expect("the pose is recorded on the body");
        assert_eq!(at, 7 * HOST_STEP_MILLIS);
    }

    #[test]
    fn a_body_the_proxy_has_no_pose_for_follows_its_own_trajectory() {
        let mut body = a_player_body(3);
        body.origin = [9.0; 3];
        body.shown_participant = Some((Default::default(), 1));
        assert!(!place_body(&mut body, &samples_for(THEM, state_at(40.0)), 0));
        assert_eq!(body.origin, [9.0; 3]);
        assert_eq!(body.shown_participant, None);
    }

    #[test]
    fn a_body_that_is_not_a_player_ignores_the_proxy() {
        let mut body = a_player_body(2);
        body.pose_entity_kind = 0;
        assert!(!place_body(&mut body, &samples_for(THEM, state_at(40.0)), 0));
        assert_eq!(body.origin, [0.0; 3]);
    }

    // --- the whole system ---

    fn present_app(duty: ProcessDuty) -> App {
        let mut app = App::new();
        app.insert_resource(duty)
            .init_resource::<PlayerTimeStepper>()
            .init_resource::<FrameClock>()
            .init_resource::<ClientGuessState>()
            .init_resource::<NewestTakenSnapshot>()
            .init_resource::<OtherPlayerSmootherHolder>()
            .init_resource::<ShownWorldFrame>()
            .init_resource::<OwnViewerTally>()
            .insert_resource(HostViewClient(ME))
            .add_systems(Update, expose_shown_frame);
        app
    }

    fn adopt(app: &mut App, earlier: Option<Snapshot>, newest: Snapshot) {
        let mut adopted = app.world_mut().resource_mut::<NewestTakenSnapshot>();
        adopted.frame_state = earlier.map(Arc::new);
        adopted.upcoming_world_state = Some(Arc::new(newest));
    }

    fn census(app: &App) -> OwnViewerTally {
        *app.world().resource::<OwnViewerTally>()
    }

    #[test]
    fn with_no_snapshot_taken_in_the_picture_is_left_alone_and_the_census_says_so() {
        let mut app = present_app(ProcessDuty::Client);
        app.update();
        assert_eq!(census(&app).picked_view, Some("no_next"));
        assert_eq!(census(&app).frame_gap_millis, None);
        assert!(app.world().resource::<ShownWorldFrame>().snapshot().is_none());
    }

    #[test]
    fn without_a_world_the_picture_is_cleared() {
        let mut app = present_app(ProcessDuty::Client);
        adopt(&mut app, None, snapshot_of(1, &[(ME, state_at(1.0))]));
        app.update();
        assert!(app.world().resource::<ShownWorldFrame>().snapshot().is_some());
        app.insert_resource(LevelPresent(false));
        app.update();
        assert!(app.world().resource::<ShownWorldFrame>().snapshot().is_none());
        assert_eq!(census(&app).picked_view, Some("no_next"));
    }

    #[test]
    fn a_snapshot_without_a_prediction_is_shown_as_it_is() {
        let mut app = present_app(ProcessDuty::Client);
        adopt(&mut app, None, snapshot_of(1, &[(ME, state_at(1.0)), (THEM, state_at(2.0))]));
        app.update();
        let world_view = app.world().resource::<ShownWorldFrame>();
        assert_eq!(world_view.snapshot().map(|snapshot| snapshot.tick), Some(Tick(1)));
        assert_eq!(world_view.player(ME).map(|ps| ps.origin[0]), Some(1.0));
        assert_eq!(census(&app).picked_view, Some("snapshot"));
        assert_eq!(census(&app).frame_gap_millis, Some(0));
    }

    #[test]
    fn the_time_taken_and_the_phase_are_recorded() {
        let mut app = present_app(ProcessDuty::Client);
        app.init_resource::<crate::FrameUpdateTally>().init_resource::<ReceiverStageLog>();
        app.update();
        assert!(app.world().resource::<crate::FrameUpdateTally>().announce_shown_millis.is_some());
        assert_eq!(app.world().resource::<ReceiverStageLog>().0, ["Present"]);
    }

    #[test]
    fn watching_a_replay_of_the_match_samples_trajectories_at_the_body_time() {
        let mut app = present_app(ProcessDuty::Client);
        adopt(&mut app, None, snapshot_of(1, &[(ME, archived_state())]));
        app.update();
        let world_view = app.world().resource::<ShownWorldFrame>();
        assert_eq!(census(&app).picked_view, Some("archived"));
        assert_eq!(world_view.trajectory_time_ms(12_345), 0 - STANDIN_LAG_MILLIS);
    }

    #[test]
    fn a_live_picture_leaves_trajectories_on_the_render_time() {
        let mut app = present_app(ProcessDuty::Client);
        adopt(&mut app, None, snapshot_of(1, &[(ME, state_at(1.0))]));
        app.update();
        assert_eq!(app.world().resource::<ShownWorldFrame>().trajectory_time_ms(12_345), 12_345);
    }

    #[test]
    fn a_replay_file_blends_the_local_player_between_its_two_snapshots() {
        let mut app = present_app(ProcessDuty::Replay);
        app.world_mut().resource_mut::<PlayerTimeStepper>().carryover_millis = HOST_STEP_MILLIS as f32 / 2.0;
        adopt(&mut app, Some(snapshot_of(1, &[(ME, state_at(0.0))])), snapshot_of(2, &[(ME, state_at(10.0))]));
        app.update();
        assert_eq!(census(&app).picked_view, Some("interpolated"));
        let world_view = app.world().resource::<ShownWorldFrame>();
        assert_eq!(world_view.player(ME).map(|ps| ps.origin[0]), Some(5.0));
        assert_eq!(world_view.blend_endpoints().map(|(_, _, fraction)| fraction), Some(0.5));
    }

    #[test]
    fn only_a_replay_file_blends_between_snapshots() {
        let mut app = present_app(ProcessDuty::Client);
        adopt(&mut app, Some(snapshot_of(1, &[(ME, state_at(0.0))])), snapshot_of(2, &[(ME, state_at(10.0))]));
        app.update();
        assert_eq!(census(&app).picked_view, Some("snapshot"));
        assert!(app.world().resource::<ShownWorldFrame>().blend_endpoints().is_none());
    }

    #[test]
    fn player_bodies_in_the_world_are_placed_each_frame() {
        let mut app = present_app(ProcessDuty::Client);
        adopt(&mut app, None, snapshot_of(1, &[(ME, state_at(1.0)), (THEM, state_at(2.0))]));
        let body = app.world_mut().spawn(a_player_body(2)).id();
        app.world_mut().get_mut::<ClientEntityPoseState>(body).unwrap().shown_participant = Some((Default::default(), 5));
        app.update();
        // The proxy holds nothing for them yet, so the body is left on its own trajectory.
        assert_eq!(app.world().get::<ClientEntityPoseState>(body).unwrap().shown_participant, None);
    }
}
