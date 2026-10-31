//! Walking a killcam's replay time to a moment the watched player was recorded.

use sim::{ClientId, ScriptSeat};

use crate::viewer_policy::cam_view::DeathReplayRun;
use crate::net_links::snapshot_history::{VAULT_STEP_MS, SnapshotVault};

/// Highest client number a script can name as the entity killcam's fallback target.
const CLIENT_SLOTS: i32 = 18;

/// Settle `run.history_window_millis` on a frame where `run.spectated_receiver` has a saved player
/// state, and say whether one was found.
///
/// While the watched client has no state at the replay time: an entity killcam first turns to
/// the seat's target client (or the viewer when the seat names none), and once that is already
/// the focus, or for any other killcam, the replay time moves 50 ms towards the present. When
/// it reaches the present there is nothing left to show: the time is 0 and the answer is no.
pub(super) fn settle_on_recorded_focus(
    store: &SnapshotVault,
    run: &mut DeathReplayRun,
    onlooker: ClientId,
    seat: &ScriptSeat,
) -> bool {
    loop {
        let lookup = store.seek_back(run.history_window_millis.max(0));
        let frame = lookup.found_tick.and_then(|tick| store.held_at(tick));
        run.history_window_millis = if frame.is_some() { lookup.reached_millis } else { 0 };

        if frame.is_some_and(|f| f.player_state_exists(run.spectated_receiver)) {
            return true;
        }
        if run.history_window_millis < 1 {
            run.history_window_millis = 0;
            return false;
        }
        if seat.death_replay_target >= 0 {
            let target = if (0..CLIENT_SLOTS).contains(&seat.target_client) {
                ClientId(seat.target_client as u32)
            } else {
                onlooker
            };
            if run.spectated_receiver != target {
                run.spectated_receiver = target;
                run.focus.death_replay_player = target.0 as i32;
                continue;
            }
        }
        run.history_window_millis = (run.history_window_millis - VAULT_STEP_MS).max(0);
    }
}

#[cfg(test)]
mod tests {
    use playerstate_t6::game::SpectatorTarget;

    use super::super::test_support::archive_with;
    use super::*;

    const TICK: i32 = VAULT_STEP_MS;

    fn session_on(spectated_receiver: u32, history_window_millis: i32) -> DeathReplayRun {
        DeathReplayRun {
            history_window_millis,
            spectated_receiver: ClientId(spectated_receiver),
            focus: SpectatorTarget::hitscan(spectated_receiver as i32),
            expires_at_millis: i32::MAX,
            ends_match: false,
            began_millis: 0,
            killcamoffset_ms: history_window_millis,
            predelay_ms: 0,
            postdelay_ms: 0,
            recalc_pending: false,
        }
    }

    fn plain_seat() -> ScriptSeat {
        ScriptSeat { spectator_client: 5, ..ScriptSeat::default() }
    }

    fn entity_seat(target_client: i32) -> ScriptSeat {
        ScriptSeat { spectator_client: 5, death_replay_target: 40, target_client, ..ScriptSeat::default() }
    }

    #[test]
    fn a_focus_recorded_at_the_requested_time_is_found_where_it_stands() {
        let store = archive_with(10, |_| vec![5]);
        let mut run = session_on(5, 4 * TICK);
        assert!(settle_on_recorded_focus(&store, &mut run, ClientId(1), &plain_seat()));
        assert_eq!(run.history_window_millis, 4 * TICK);
        assert_eq!(run.spectated_receiver, ClientId(5));
    }

    #[test]
    fn the_time_moves_towards_the_present_until_the_focus_was_recorded() {
        // Client 5 is only in the last two frames: 2 and 1 ticks back from the end.
        let store = archive_with(10, |tick| if tick >= 8 { vec![5] } else { Vec::new() });
        let mut run = session_on(5, 6 * TICK);
        assert!(settle_on_recorded_focus(&store, &mut run, ClientId(1), &plain_seat()));
        assert_eq!(run.history_window_millis, 2 * TICK);
    }

    #[test]
    fn a_focus_never_recorded_runs_out_at_the_present() {
        let store = archive_with(10, |_| vec![9]);
        let mut run = session_on(5, 4 * TICK);
        assert!(!settle_on_recorded_focus(&store, &mut run, ClientId(1), &plain_seat()));
        assert_eq!(run.history_window_millis, 0);
    }

    #[test]
    fn an_empty_archive_or_a_negative_time_finds_nothing() {
        let empty = archive_with(0, |_| Vec::new());
        let mut run = session_on(5, 3 * TICK);
        assert!(!settle_on_recorded_focus(&empty, &mut run, ClientId(1), &plain_seat()));
        assert_eq!(run.history_window_millis, 0);

        let store = archive_with(10, |_| vec![5]);
        let mut run = session_on(5, -500);
        assert!(!settle_on_recorded_focus(&store, &mut run, ClientId(1), &plain_seat()));
        assert_eq!(run.history_window_millis, 0);
    }

    #[test]
    fn an_entity_killcam_turns_to_the_target_client_without_moving_the_time() {
        let store = archive_with(10, |_| vec![7]);
        let mut run = session_on(5, 4 * TICK);
        assert!(settle_on_recorded_focus(&store, &mut run, ClientId(1), &entity_seat(7)));
        assert_eq!(run.spectated_receiver, ClientId(7));
        assert_eq!(run.focus.death_replay_player, 7);
        assert_eq!(run.history_window_millis, 4 * TICK);
    }

    #[test]
    fn a_target_outside_the_client_slots_means_the_viewer() {
        for target in [-1, 18, 400] {
            let store = archive_with(10, |_| vec![3]);
            let mut run = session_on(5, 4 * TICK);
            assert!(settle_on_recorded_focus(&store, &mut run, ClientId(3), &entity_seat(target)));
            assert_eq!(run.spectated_receiver, ClientId(3), "target {target}");
        }
    }

    #[test]
    fn once_on_the_target_an_entity_killcam_steps_the_time_like_any_other() {
        let store = archive_with(10, |_| vec![9]);
        let mut run = session_on(5, 4 * TICK);
        assert!(!settle_on_recorded_focus(&store, &mut run, ClientId(1), &entity_seat(7)));
        assert_eq!(run.spectated_receiver, ClientId(7));
        assert_eq!(run.history_window_millis, 0);
    }

    #[test]
    fn an_ordinary_killcam_never_changes_its_focus() {
        let store = archive_with(10, |_| vec![7]);
        let mut run = session_on(5, 4 * TICK);
        let seat = ScriptSeat { target_client: 7, ..plain_seat() };
        assert!(!settle_on_recorded_focus(&store, &mut run, ClientId(1), &seat));
        assert_eq!(run.spectated_receiver, ClientId(5));
    }
}
