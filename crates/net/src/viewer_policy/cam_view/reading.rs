use playerstate_t6::game::{AvatarState, apply_killcam_seat, rebase_archived_timers};
use sim::{ClientId, Snapshot};

use super::runs::DeathReplayRun;
use crate::net_links::snapshot_history::{VAULT_STEP_MS, HistoryQueryResult, SnapshotVault};

/// The player state a killcam beholder is given, and where in the archive it came from.
#[derive(Clone, Debug, PartialEq)]
pub struct ViewerSeatReading {
    pub player_state: AvatarState,
    pub lookup: HistoryQueryResult,
    /// How far the archived timers were moved forward to sit on the live clock.
    pub clock_shift_millis: i32,
}

fn row_of(snapshot: &Snapshot, client: ClientId) -> Option<AvatarState> {
    snapshot
        .players
        .iter()
        .find(|(who, _)| *who == client)
        .map(|(_, st)| *st)
}

/// The beholder's seat in the archived frame the killcam points at. Nothing if the archive has no
/// frame that far back or the focused player had no state in it.
pub fn read_deathreplay_seat(
    vault: &SnapshotVault,
    world_now: &Snapshot,
    beholder: ClientId,
    run: &DeathReplayRun,
    clock_millis: i32,
) -> Option<ViewerSeatReading> {
    let lookup = vault.seek_back(run.history_window_millis);
    let at_tick = lookup.found_tick?;
    let stored = vault.held_at(at_tick)?;
    if !stored.player_state_exists(run.spectated_receiver) {
        return None;
    }
    let focused = row_of(&stored.state, run.spectated_receiver)?;
    let beholder_now = row_of(world_now, beholder).unwrap_or(AvatarState::ZERO);

    let frame_start_ms = i32::try_from(at_tick.0).ok()?.saturating_mul(VAULT_STEP_MS);
    let clock_shift_millis = clock_millis.saturating_sub(frame_start_ms);

    let mut moved = focused;
    rebase_archived_timers(&mut moved, clock_shift_millis);
    Some(ViewerSeatReading {
        player_state: apply_killcam_seat(&beholder_now, &moved, run.focus),
        lookup,
        clock_shift_millis,
    })
}

/// Put the sampled seat into `world` as the beholder's own row, adding the row if they had none.
pub fn stamp_seat_into_world_state(
    vault: &SnapshotVault,
    world: &mut Snapshot,
    beholder: ClientId,
    run: &DeathReplayRun,
    clock_millis: i32,
) -> Option<ViewerSeatReading> {
    let reading = read_deathreplay_seat(vault, world, beholder, run, clock_millis)?;
    match world.players.iter_mut().find(|(who, _)| *who == beholder) {
        Some((_, row)) => *row = reading.player_state,
        None => world.players.push((beholder, reading.player_state)),
    }
    Some(reading)
}

#[cfg(test)]
mod tests {
    use playerstate_t6::game::SpectatorTarget;
    use sim::Tick;

    use super::*;
    use crate::net_links::snapshot_history::StoredHistoryFrame;

    const BEHOLDER: ClientId = ClientId(1);
    const KILLER: ClientId = ClientId(2);

    fn session(history_window_millis: i32) -> DeathReplayRun {
        DeathReplayRun {
            history_window_millis,
            spectated_receiver: KILLER,
            focus: SpectatorTarget::hitscan(2),
            expires_at_millis: 10_000,
            ends_match: false,
            began_millis: 0,
            killcamoffset_ms: 0,
            predelay_ms: 0,
            postdelay_ms: 0,
            recalc_pending: false,
        }
    }

    fn frame(tick: u32, with_killer: bool) -> StoredHistoryFrame {
        let mut snapshot = Snapshot::unpublished(Tick(tick));
        if with_killer {
            snapshot.players.push((KILLER, AvatarState::ZERO));
        }
        StoredHistoryFrame {
            at_tick: Tick(tick),
            state: snapshot,
            no_player_state: Vec::new(),
        }
    }

    fn archive_of(frames: impl IntoIterator<Item = StoredHistoryFrame>) -> SnapshotVault {
        let mut vault = SnapshotVault::new();
        for frame in frames {
            assert!(vault.append(frame));
        }
        vault
    }

    #[test]
    fn an_empty_archive_gives_no_seat() {
        let live = Snapshot::unpublished(Tick(50));
        assert!(
            read_deathreplay_seat(&SnapshotVault::new(), &live, BEHOLDER, &session(1000), 2500)
                .is_none()
        );
    }

    #[test]
    fn a_frame_without_the_focused_player_gives_no_seat() {
        let vault = archive_of((0..40).map(|tick| frame(tick, false)));
        let live = Snapshot::unpublished(Tick(40));
        assert!(read_deathreplay_seat(&vault, &live, BEHOLDER, &session(1000), 2000).is_none());
    }

    #[test]
    fn a_player_marked_as_having_no_state_gives_no_seat() {
        let mut stateless = frame(0, true);
        stateless.no_player_state.push(KILLER);
        let vault = archive_of([stateless]);
        let live = Snapshot::unpublished(Tick(1));
        assert!(read_deathreplay_seat(&vault, &live, BEHOLDER, &session(VAULT_STEP_MS), 50).is_none());
    }

    #[test]
    fn the_sample_reports_where_it_looked_and_how_far_it_moved_the_timers() {
        let vault = archive_of((0..40).map(|tick| frame(tick, true)));
        let live = Snapshot::unpublished(Tick(40));
        let clock_millis = 40 * VAULT_STEP_MS;
        let reading = read_deathreplay_seat(&vault, &live, BEHOLDER, &session(1000), clock_millis)
            .expect("the vault reaches back that far");
        let tick = reading.lookup.found_tick.expect("a frame was found");
        assert_eq!(reading.clock_shift_millis, clock_millis - tick.0 as i32 * VAULT_STEP_MS);
        assert_eq!(reading.lookup, vault.seek_back(1000));
    }

    #[test]
    fn applying_a_seat_adds_the_viewers_row_when_they_had_none() {
        let vault = archive_of((0..40).map(|tick| frame(tick, true)));
        let mut live = Snapshot::unpublished(Tick(40));
        assert!(live.players.is_empty());
        let reading = stamp_seat_into_world_state(&vault, &mut live, BEHOLDER, &session(1000), 2000)
            .expect("a seat");
        assert_eq!(live.players, [(BEHOLDER, reading.player_state)]);
    }

    #[test]
    fn applying_a_seat_replaces_the_row_the_viewer_already_had() {
        let vault = archive_of((0..40).map(|tick| frame(tick, true)));
        let mut live = Snapshot::unpublished(Tick(40));
        live.players.push((BEHOLDER, AvatarState::ZERO));
        let reading = stamp_seat_into_world_state(&vault, &mut live, BEHOLDER, &session(1000), 2000)
            .expect("a seat");
        assert_eq!(live.players.len(), 1);
        assert_eq!(live.players[0], (BEHOLDER, reading.player_state));
    }
}
