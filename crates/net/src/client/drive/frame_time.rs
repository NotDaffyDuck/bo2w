//! The two clocks the client moves forward every frame: the wall clock the connection runs on
//! and the game clock the presentation reads.

use bevy::prelude::*;
use frame::ProcessDuty;

use super::stores::{ClientGuessState, NewestTakenSnapshot};
use crate::HostInstant;
use crate::client::frame_clock::{FrameClock, GameActive, GameJoinCensus};
use crate::client::realtime::ClientRealtime;

pub fn advance_receiver_wall_clock(mut cls: ResMut<ClientRealtime>, frame_dt: Res<Time>) {
    cls.advance_host_clock(frame_dt.delta_secs());
}

/// Moves the game clock to where the host says the match is. A listen host that is already
/// playing simply copies its own authority clock plus the part of the fixed step that has gone
/// by; every other case lets the clock run on frame time and lean toward the newest server time.
pub fn step_presentation_clock(
    mut clock: ResMut<FrameClock>,
    mut forecast: ResMut<ClientGuessState>,
    mut join_gate: ResMut<GameJoinCensus>,
    frame_dt: Res<Time>,
    cg_live: Res<GameActive>,
    authority: Option<Res<crate::HostTimeKeeper>>,
    role: Res<ProcessDuty>,
    step_time: Res<Time<Fixed>>,
    adopted: Option<Res<NewestTakenSnapshot>>,
) {
    let started_before = clock.started();
    let newest_adopted_tick = adopted
        .as_deref()
        .and_then(NewestTakenSnapshot::next)
        .map(|snap| snap.tick);

    // Only a runtime that hosts the match has a clock of its own to follow.
    let own_clock = authority.as_deref().filter(|_| role.hosts_simulation());
    let authority_clock_ms = own_clock
        .map(|own| HostInstant::from_millis_value(own.clock_stamp_ms))
        .or_else(|| newest_adopted_tick.map(HostInstant::from_step_number));

    let follows_host_clock = *role == ProcessDuty::Listen && cg_live.0 && started_before;
    if follows_host_clock {
        let host_ms = authority.as_deref().expect("listen authority clock").clock_stamp_ms;
        let into_step_ms = step_time.overstep().as_millis() as i32;
        clock.set_host_ms(host_ms.wrapping_add(into_step_ms));
    } else if *role == ProcessDuty::Client {
        clock.advance_remote(frame_dt.delta_secs(), cg_live.0, authority_clock_ms);
    } else {
        clock.step_local_host_clock(frame_dt.delta_secs(), cg_live.0, authority_clock_ms);
    }

    if clock.started() && !started_before {
        let join_tick = own_clock
            .map(|own| own.tick)
            .or_else(|| newest_adopted_tick.map(|tick| tick.0))
            .unwrap_or(0);
        join_gate.note_join(
            authority_clock_ms.map_or(0, HostInstant::as_millis),
            join_tick,
            (frame_dt.elapsed_secs() * 1000.0) as i32,
        );
    }
    forecast.0.forecast_miss_mut().follow_clock(&clock);
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::Duration;

    use super::*;

    fn clock_app(role: ProcessDuty, active: bool) -> App {
        let mut app = App::new();
        app.insert_resource(role)
            .insert_resource(GameActive(active))
            .init_resource::<FrameClock>()
            .init_resource::<GameJoinCensus>()
            .init_resource::<ClientGuessState>()
            .init_resource::<ClientRealtime>()
            .init_resource::<NewestTakenSnapshot>()
            .insert_resource(Time::<Fixed>::default())
            .insert_resource(Time::<()>::default())
            .add_systems(Update, (advance_receiver_wall_clock, step_presentation_clock).chain());
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
        app.world_mut().resource_mut::<NewestTakenSnapshot>().upcoming_world_state = Some(Arc::new(snap));
    }

    #[test]
    fn the_realtime_clock_starts_at_zero_and_then_follows_frame_time() {
        let mut app = clock_app(ProcessDuty::Client, false);
        run_frame(&mut app, 16);
        assert!(app.world().resource::<ClientRealtime>().has_begun());
        assert_eq!(app.world().resource::<ClientRealtime>().now_ms(), 0);
        run_frame(&mut app, 16);
        run_frame(&mut app, 20);
        // Frame seconds are floats, so a millisecond can stay in the remainder.
        let realtime = app.world().resource::<ClientRealtime>().now_ms();
        assert!((35..=36).contains(&realtime), "realtime was {realtime}");
    }

    #[test]
    fn a_remote_client_does_not_start_its_game_clock_before_the_game_is_active() {
        let mut app = clock_app(ProcessDuty::Client, false);
        adopt_tick(&mut app, 10);
        run_frame(&mut app, 16);
        assert!(!app.world().resource::<FrameClock>().started());
        assert_eq!(app.world().resource::<GameJoinCensus>().buffered_depth_millis, None);
    }

    #[test]
    fn a_remote_client_starts_on_the_newest_adopted_snapshot_time() {
        let mut app = clock_app(ProcessDuty::Client, true);
        adopt_tick(&mut app, 10);
        run_frame(&mut app, 16);
        let clock = *app.world().resource::<FrameClock>();
        assert!(clock.started());
        assert_eq!(clock.time(), 10 * crate::host_tick::peer_mail::HOST_STEP_MILLIS);
        let join_gate = *app.world().resource::<GameJoinCensus>();
        assert_eq!(join_gate.buffered_depth_millis, Some(10 * crate::host_tick::peer_mail::HOST_STEP_MILLIS));
        assert_eq!(join_gate.joined_tick, Some(10));
    }

    #[test]
    fn the_join_is_only_latched_once() {
        let mut app = clock_app(ProcessDuty::Client, true);
        adopt_tick(&mut app, 4);
        run_frame(&mut app, 16);
        adopt_tick(&mut app, 400);
        run_frame(&mut app, 16);
        assert_eq!(app.world().resource::<GameJoinCensus>().joined_tick, Some(4));
    }

    #[test]
    fn a_listen_host_that_is_playing_copies_its_authority_clock() {
        let mut app = clock_app(ProcessDuty::Listen, true);
        app.insert_resource(crate::HostTimeKeeper { tick: 6, clock_stamp_ms: 300 });
        run_frame(&mut app, 16);
        assert!(app.world().resource::<FrameClock>().started());
        assert_eq!(app.world().resource::<GameJoinCensus>().joined_tick, Some(6));

        app.world_mut().resource_mut::<crate::HostTimeKeeper>().clock_stamp_ms = 450;
        run_frame(&mut app, 16);
        assert_eq!(app.world().resource::<FrameClock>().time(), 450);
    }

    #[test]
    fn a_dedicated_runtime_keeps_to_its_own_clock_when_it_has_one() {
        let mut app = clock_app(ProcessDuty::Dedicated, true);
        app.insert_resource(crate::HostTimeKeeper { tick: 2, clock_stamp_ms: 100 });
        adopt_tick(&mut app, 90);
        run_frame(&mut app, 16);
        assert_eq!(app.world().resource::<GameJoinCensus>().buffered_depth_millis, Some(100));
        assert_eq!(app.world().resource::<GameJoinCensus>().joined_tick, Some(2));
    }
}
