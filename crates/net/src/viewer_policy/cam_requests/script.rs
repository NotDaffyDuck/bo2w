//! Turning the scripts' spectator seats into running killcam sessions.

use playerstate_t6::game::SpectatorTarget;
use sim::{ClientId, ScriptSeat};

use super::follow::settle_on_recorded_focus;
use crate::viewer_policy::cam_view::{LiveDeathReplays, DeathReplayRun};
use crate::net_links::snapshot_history::{VAULT_STEP_MS, SnapshotVault};

/// Bring the running killcams in line with this tick's script seats: viewers the scripts no
/// longer list are dropped, the rest get a session placed on a recorded frame.
///
/// Returns the viewers whose replay time had to move off the time the script asked for,
/// with the time it moved to, so the scripts can read it back. A seat keeps its original start
/// time for as long as it asks for the same moment and the same player.
pub(crate) fn play_script_seats(
    seats: &mut LiveDeathReplays,
    store: &SnapshotVault,
    clock_millis: i32,
    scripted: &[(ClientId, ScriptSeat)],
) -> Vec<(ClientId, i32)> {
    for onlooker in seats.watching() {
        if !scripted.iter().any(|(id, _)| *id == onlooker) {
            seats.stop(onlooker);
        }
    }

    let mut moved = Vec::new();
    for (onlooker, seat) in scripted {
        let watched = ClientId(seat.spectator_client as u32);
        let began_millis = start_time(seats, *onlooker, seat, watched, clock_millis);
        if began_millis == clock_millis {
            diag::info!(
                WireLink,
                "killcam: script seat viewer={} spectating={} entity={} archive_ms={} length_ms={}",
                onlooker.0,
                seat.spectator_client,
                seat.death_replay_target,
                seat.archive_ms,
                seat.length_ms
            );
        }

        let mut run = new_session(seat, watched, began_millis);
        let found = settle_on_recorded_focus(store, &mut run, *onlooker, seat);

        // The archive only holds whole 50 ms ticks, so only a step further back than the
        // tick the request rounds to counts as the replay time having moved.
        let asked_tick_ms = seat.archive_ms / VAULT_STEP_MS * VAULT_STEP_MS;
        if run.history_window_millis < asked_tick_ms {
            diag::info!(
                WireLink,
                "killcam: viewer={} archive_ms {} -> {} (spectating {} has no saved state){}",
                onlooker.0,
                seat.archive_ms,
                run.history_window_millis,
                seat.spectator_client,
                if found { "" } else { ", none left: the killcam ends" }
            );
            run.killcamoffset_ms = run.history_window_millis;
            moved.push((*onlooker, run.history_window_millis));
        }

        if found {
            seats.launch(*onlooker, run);
        } else {
            seats.stop(*onlooker);
        }
    }
    moved
}

/// When this seat's killcam began: `clock_millis` for a new one, else the running one's start when
/// it is still the same moment of the same player (or an entity killcam).
fn start_time(
    seats: &LiveDeathReplays,
    onlooker: ClientId,
    seat: &ScriptSeat,
    watched: ClientId,
    clock_millis: i32,
) -> i32 {
    seats
        .run_for(onlooker)
        .filter(|running| {
            running.killcamoffset_ms == seat.archive_ms
                && (running.spectated_receiver == watched || seat.death_replay_target >= 0)
        })
        .map_or(clock_millis, |running| running.began_millis)
}

/// The session a seat asks for, before its replay time is settled on a recorded frame.
fn new_session(seat: &ScriptSeat, watched: ClientId, began_millis: i32) -> DeathReplayRun {
    let mut focus = SpectatorTarget::hitscan(seat.spectator_client);
    if seat.death_replay_target >= 0 {
        focus.death_replay_target = seat.death_replay_target;
    }
    if seat.look_at_entity >= 0 {
        focus.death_replay_focus = seat.look_at_entity;
    }
    DeathReplayRun {
        history_window_millis: seat.archive_ms,
        spectated_receiver: watched,
        focus,
        expires_at_millis: i32::MAX,
        ends_match: seat.last_kill,
        began_millis,
        killcamoffset_ms: seat.archive_ms,
        predelay_ms: 0,
        postdelay_ms: 0,
        recalc_pending: false,
    }
}

#[cfg(test)]
mod tests {
    use super::super::test_support::archive_with;
    use super::*;

    const TICK: i32 = VAULT_STEP_MS;
    const VIEWER: ClientId = ClientId(2);

    fn seat_for(spectator: i32, archive_ms: i32) -> ScriptSeat {
        ScriptSeat { spectator_client: spectator, archive_ms, length_ms: 4000, ..ScriptSeat::default() }
    }

    fn play(
        seats: &mut LiveDeathReplays,
        store: &SnapshotVault,
        clock_millis: i32,
        scripted: &[(ClientId, ScriptSeat)],
    ) -> Vec<(ClientId, i32)> {
        play_script_seats(seats, store, clock_millis, scripted)
    }

    #[test]
    fn a_seat_on_a_recorded_player_arms_a_session_at_the_requested_time() {
        let store = archive_with(10, |_| vec![1]);
        let mut seats = LiveDeathReplays::new();
        let scripted = [(VIEWER, ScriptSeat { last_kill: true, ..seat_for(1, 4 * TICK) })];
        let moved = play(&mut seats, &store, 1000, &scripted);
        assert!(moved.is_empty());

        let run = seats.run_for(VIEWER).expect("the killcam is running");
        assert_eq!(run.history_window_millis, 4 * TICK);
        assert_eq!(run.killcamoffset_ms, 4 * TICK);
        assert_eq!(run.spectated_receiver, ClientId(1));
        assert_eq!(run.focus, SpectatorTarget::hitscan(1));
        assert_eq!(run.began_millis, 1000);
        assert_eq!(run.expires_at_millis, i32::MAX);
        assert!(run.ends_match);
        assert_eq!((run.predelay_ms, run.postdelay_ms, run.recalc_pending), (0, 0, false));
    }

    #[test]
    fn entity_and_look_at_choices_replace_the_hitscan_defaults_only_when_given() {
        let store = archive_with(10, |_| vec![1]);
        let mut seats = LiveDeathReplays::new();
        let seat = ScriptSeat { death_replay_target: 33, look_at_entity: 12, ..seat_for(1, 3 * TICK) };
        play(&mut seats, &store, 0, &[(VIEWER, seat)]);
        let focus = seats.run_for(VIEWER).unwrap().focus;
        assert_eq!((focus.death_replay_target, focus.death_replay_focus), (33, 12));

        let mut seats = LiveDeathReplays::new();
        play(&mut seats, &store, 0, &[(VIEWER, seat_for(1, 3 * TICK))]);
        assert_eq!(seats.run_for(VIEWER).unwrap().focus, SpectatorTarget::hitscan(1));
    }

    #[test]
    fn a_viewer_the_scripts_stop_listing_loses_their_killcam() {
        let store = archive_with(10, |_| vec![1]);
        let mut seats = LiveDeathReplays::new();
        play(&mut seats, &store, 0, &[(VIEWER, seat_for(1, 3 * TICK)), (ClientId(4), seat_for(1, 3 * TICK))]);
        assert_eq!(seats.watching().len(), 2);

        play(&mut seats, &store, 50, &[(ClientId(4), seat_for(1, 3 * TICK))]);
        assert!(seats.run_for(VIEWER).is_none());
        assert!(seats.run_for(ClientId(4)).is_some());

        play(&mut seats, &store, 100, &[]);
        assert!(seats.none_running());
    }

    #[test]
    fn the_start_time_holds_while_the_same_moment_of_the_same_player_is_asked_for() {
        let store = archive_with(10, |_| vec![1, 6]);
        let mut seats = LiveDeathReplays::new();
        play(&mut seats, &store, 1000, &[(VIEWER, seat_for(1, 4 * TICK))]);
        play(&mut seats, &store, 2000, &[(VIEWER, seat_for(1, 4 * TICK))]);
        assert_eq!(seats.run_for(VIEWER).unwrap().began_millis, 1000);

        play(&mut seats, &store, 3000, &[(VIEWER, seat_for(1, 5 * TICK))]);
        assert_eq!(seats.run_for(VIEWER).unwrap().began_millis, 3000, "a different moment starts over");

        play(&mut seats, &store, 4000, &[(VIEWER, seat_for(6, 5 * TICK))]);
        assert_eq!(seats.run_for(VIEWER).unwrap().began_millis, 4000, "a different player starts over");
    }

    #[test]
    fn an_entity_killcam_keeps_its_start_time_even_when_the_player_changes() {
        let store = archive_with(10, |_| vec![1, 6]);
        let mut seats = LiveDeathReplays::new();
        let entity_on = |spectator| ScriptSeat { death_replay_target: 20, ..seat_for(spectator, 4 * TICK) };
        play(&mut seats, &store, 1000, &[(VIEWER, entity_on(1))]);
        play(&mut seats, &store, 2000, &[(VIEWER, entity_on(6))]);
        assert_eq!(seats.run_for(VIEWER).unwrap().began_millis, 1000);
    }

    #[test]
    fn a_player_recorded_only_late_moves_the_time_and_is_reported() {
        let store = archive_with(10, |tick| if tick >= 8 { vec![1] } else { Vec::new() });
        let mut seats = LiveDeathReplays::new();
        let moved = play(&mut seats, &store, 0, &[(VIEWER, seat_for(1, 6 * TICK))]);
        assert_eq!(moved, [(VIEWER, 2 * TICK)]);
        let run = seats.run_for(VIEWER).unwrap();
        assert_eq!((run.history_window_millis, run.killcamoffset_ms), (2 * TICK, 2 * TICK));
    }

    #[test]
    fn a_request_between_ticks_is_not_a_move() {
        let store = archive_with(10, |_| vec![1]);
        let mut seats = LiveDeathReplays::new();
        let moved = play(&mut seats, &store, 0, &[(VIEWER, seat_for(1, 4 * TICK + 30))]);
        assert!(moved.is_empty());
        assert_eq!(seats.run_for(VIEWER).unwrap().history_window_millis, 4 * TICK);
        assert_eq!(seats.run_for(VIEWER).unwrap().killcamoffset_ms, 4 * TICK + 30);
    }

    #[test]
    fn a_player_never_recorded_ends_the_killcam_and_reports_the_move_to_zero() {
        let store = archive_with(10, |_| vec![9]);
        let mut seats = LiveDeathReplays::new();
        play(&mut seats, &store, 0, &[(VIEWER, seat_for(9, 2 * TICK))]);
        assert!(seats.run_for(VIEWER).is_some());

        let moved = play(&mut seats, &store, 50, &[(VIEWER, seat_for(1, 2 * TICK))]);
        assert_eq!(moved, [(VIEWER, 0)]);
        assert!(seats.run_for(VIEWER).is_none(), "nothing left to show: the killcam is dropped");
    }

    #[test]
    fn a_request_at_the_present_is_never_a_move() {
        let store = archive_with(10, |_| vec![9]);
        let mut seats = LiveDeathReplays::new();
        let moved = play(&mut seats, &store, 0, &[(VIEWER, seat_for(1, 0))]);
        assert!(moved.is_empty());
        assert!(seats.none_running());
    }
}
