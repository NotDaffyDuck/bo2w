//! The two clocks the client moves forward every frame: the wall clock the connection runs on
//! and the game clock the presentation reads.

use bevy::prelude::*;
use frame::RuntimeRole;

use super::stores::{ClientPredictionState, LastAdoptedSnapshot};
use crate::ServerTime;
use crate::client::frame_clock::{FrameClock, GameActive, GameJoinCensus};
use crate::client::realtime::ClientRealtime;

pub fn advance_cls_realtime(mut cls: ResMut<ClientRealtime>, time: Res<Time>) {
    cls.advance_listen(time.delta_secs());
}

/// Moves the game clock to where the host says the match is. A listen host that is already
/// playing simply copies its own authority clock plus the part of the fixed step that has gone
/// by; every other case lets the clock run on frame time and lean toward the newest server time.
pub fn advance_cg_frame_clock(
    mut clock: ResMut<FrameClock>,
    mut prediction: ResMut<ClientPredictionState>,
    mut join: ResMut<GameJoinCensus>,
    time: Res<Time>,
    cgame_active: Res<GameActive>,
    authority: Option<Res<crate::AuthorityClock>>,
    role: Res<RuntimeRole>,
    fixed: Res<Time<Fixed>>,
    adopted: Option<Res<LastAdoptedSnapshot>>,
) {
    let started_before = clock.started();
    let newest_adopted_tick = adopted
        .as_deref()
        .and_then(LastAdoptedSnapshot::next)
        .map(|snap| snap.tick);

    // Only a runtime that hosts the match has a clock of its own to follow.
    let own_clock = authority.as_deref().filter(|_| role.runs_authority());
    let server_time = own_clock
        .map(|own| ServerTime::from_ms(own.time_ms))
        .or_else(|| newest_adopted_tick.map(ServerTime::from_tick));

    let follows_host_clock = *role == RuntimeRole::Listen && cgame_active.0 && started_before;
    if follows_host_clock {
        let host_ms = authority.as_deref().expect("listen authority clock").time_ms;
        let into_step_ms = fixed.overstep().as_millis() as i32;
        clock.assign_server_time(host_ms.wrapping_add(into_step_ms));
    } else if *role == RuntimeRole::Client {
        clock.advance_remote(time.delta_secs(), cgame_active.0, server_time);
    } else {
        clock.advance_listen(time.delta_secs(), cgame_active.0, server_time);
    }

    if clock.started() && !started_before {
        let join_tick = own_clock
            .map(|own| own.tick)
            .or_else(|| newest_adopted_tick.map(|tick| tick.0))
            .unwrap_or(0);
        join.latch(
            server_time.map_or(0, ServerTime::ms),
            join_tick,
            (time.elapsed_secs() * 1000.0) as i32,
        );
    }
    prediction.0.predicted_error_mut().sync_frame(&clock);
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::Duration;

    use super::*;

    fn clock_app(role: RuntimeRole, active: bool) -> App {
        let mut app = App::new();
        app.insert_resource(role)
            .insert_resource(GameActive(active))
            .init_resource::<FrameClock>()
            .init_resource::<GameJoinCensus>()
            .init_resource::<ClientPredictionState>()
            .init_resource::<ClientRealtime>()
            .init_resource::<LastAdoptedSnapshot>()
            .insert_resource(Time::<Fixed>::default())
            .insert_resource(Time::<()>::default())
            .add_systems(Update, (advance_cls_realtime, advance_cg_frame_clock).chain());
        app
    }

    fn run_frame(app: &mut App, ms: u64) {
        app.world_mut()
            .resource_mut::<Time>()
            .advance_by(Duration::from_millis(ms));
        app.update();
    }

    fn adopt_tick(app: &mut App, tick: u32) {
        let mut snap = sim::Snapshot::unpublished(sim::Tick(0));
        snap.tick = sim::Tick(tick);
        app.world_mut().resource_mut::<LastAdoptedSnapshot>().next_snap = Some(Arc::new(snap));
    }

    #[test]
    fn the_realtime_clock_starts_at_zero_and_then_follows_frame_time() {
        let mut app = clock_app(RuntimeRole::Client, false);
        run_frame(&mut app, 16);
        assert!(app.world().resource::<ClientRealtime>().started());
        assert_eq!(app.world().resource::<ClientRealtime>().realtime(), 0);
        run_frame(&mut app, 16);
        run_frame(&mut app, 20);
        // Frame seconds are floats, so a millisecond can stay in the remainder.
        let realtime = app.world().resource::<ClientRealtime>().realtime();
        assert!((35..=36).contains(&realtime), "realtime was {realtime}");
    }

    #[test]
    fn a_remote_client_does_not_start_its_game_clock_before_the_game_is_active() {
        let mut app = clock_app(RuntimeRole::Client, false);
        adopt_tick(&mut app, 10);
        run_frame(&mut app, 16);
        assert!(!app.world().resource::<FrameClock>().started());
        assert_eq!(app.world().resource::<GameJoinCensus>().level_ms, None);
    }

    #[test]
    fn a_remote_client_starts_on_the_newest_adopted_snapshot_time() {
        let mut app = clock_app(RuntimeRole::Client, true);
        adopt_tick(&mut app, 10);
        run_frame(&mut app, 16);
        let clock = *app.world().resource::<FrameClock>();
        assert!(clock.started());
        assert_eq!(clock.time(), 10 * crate::authority::inbox::AUTHORITY_MS);
        let join = *app.world().resource::<GameJoinCensus>();
        assert_eq!(join.level_ms, Some(10 * crate::authority::inbox::AUTHORITY_MS));
        assert_eq!(join.tick, Some(10));
    }

    #[test]
    fn the_join_is_only_latched_once() {
        let mut app = clock_app(RuntimeRole::Client, true);
        adopt_tick(&mut app, 4);
        run_frame(&mut app, 16);
        adopt_tick(&mut app, 400);
        run_frame(&mut app, 16);
        assert_eq!(app.world().resource::<GameJoinCensus>().tick, Some(4));
    }

    #[test]
    fn a_listen_host_that_is_playing_copies_its_authority_clock() {
        let mut app = clock_app(RuntimeRole::Listen, true);
        app.insert_resource(crate::AuthorityClock { tick: 6, time_ms: 300 });
        run_frame(&mut app, 16);
        assert!(app.world().resource::<FrameClock>().started());
        assert_eq!(app.world().resource::<GameJoinCensus>().tick, Some(6));

        app.world_mut().resource_mut::<crate::AuthorityClock>().time_ms = 450;
        run_frame(&mut app, 16);
        assert_eq!(app.world().resource::<FrameClock>().time(), 450);
    }

    #[test]
    fn a_dedicated_runtime_keeps_to_its_own_clock_when_it_has_one() {
        let mut app = clock_app(RuntimeRole::Dedicated, true);
        app.insert_resource(crate::AuthorityClock { tick: 2, time_ms: 100 });
        adopt_tick(&mut app, 90);
        run_frame(&mut app, 16);
        assert_eq!(app.world().resource::<GameJoinCensus>().level_ms, Some(100));
        assert_eq!(app.world().resource::<GameJoinCensus>().tick, Some(2));
    }
}
