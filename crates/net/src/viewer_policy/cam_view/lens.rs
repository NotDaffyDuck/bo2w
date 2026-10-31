use sim::{ClientId, PlayerSessionStage, Snapshot};

use super::hud::draw_deathreplay_overlay;
use super::reading::{ViewerSeatReading, stamp_seat_into_world_state};
use super::runs::LiveDeathReplays;
use super::world::overlay_archived_world;
use crate::net_links::snapshot_history::SnapshotVault;

/// What `beholder` is shown this tick.
pub fn state_for_viewer(
    vault: &SnapshotVault,
    seats: &LiveDeathReplays,
    present: &Snapshot,
    beholder: ClientId,
    clock_millis: i32,
) -> Snapshot {
    state_and_reading_for_viewer(vault, seats, present, beholder, clock_millis).0
}

/// What `beholder` is shown this tick, and the killcam seat in it if they are in a killcam.
pub fn state_and_reading_for_viewer(
    vault: &SnapshotVault,
    seats: &LiveDeathReplays,
    present: &Snapshot,
    beholder: ClientId,
    clock_millis: i32,
) -> (Snapshot, Option<ViewerSeatReading>) {
    let (mut shown, reading) = killcam_or_live(vault, seats, present, beholder, clock_millis);
    super::follow::follow_teammate(&mut shown, beholder);
    show_movers_to(&mut shown, beholder);
    // BO2's `setenemymodel`: what is not on the beholder's side wears the other side's look. A
    // killcam's frames carry those rows too.
    shown.meta.swap_enemy_models(beholder);
    (shown, reading)
}

/// Script movers that were created hidden and are shown to chosen clients: reveal them to the
/// beholder if chosen, otherwise take their models out.
fn show_movers_to(shown: &mut Snapshot, beholder: ClientId) {
    let Some(beholder_bit) = 1u64.checked_shl(beholder.0) else {
        return;
    };
    let meta = &mut shown.meta;
    for mover in &mut meta.script_movers {
        // bo2zm: a brush that does not stop this player goes to his prediction nonsolid (the
        // others' still clip against it).
        if mover.passes_for & beholder_bit != 0 {
            mover.nonsolid = true;
        }
        let hidden_for_all = mover.shown_to != 0
            && mover.state.lerp.e_flags & entity_t6::missile::EF_NODRAW != 0;
        if !hidden_for_all {
            continue;
        }
        if mover.shown_to & beholder_bit == 0 {
            meta.entity_dobjs
                .retain(|(holder, _)| holder.scripted_model() != Some(mover.id));
            continue;
        }
        let as_hidden = mover.state.clone();
        mover.state.lerp.e_flags &= !entity_t6::missile::EF_NODRAW;
        for row in meta.entities.iter_mut().filter(|row| **row == as_hidden) {
            *row = mover.state.clone();
        }
    }
}

/// The live world, unless the beholder is in a killcam that applies: then the archived one with
/// their seat and HUD put in.
fn killcam_or_live(
    vault: &SnapshotVault,
    seats: &LiveDeathReplays,
    present: &Snapshot,
    beholder: ClientId,
    clock_millis: i32,
) -> (Snapshot, Option<ViewerSeatReading>) {
    let Some(active) = seats.run_for(beholder) else {
        return (present.clone(), None);
    };
    // A killcam that is not for the last kill of the match waits until the beholder is dead.
    let alive = present
        .meta
        .for_client(beholder)
        .is_some_and(|meta| meta.lifecycle == PlayerSessionStage::Alive);
    if alive && !active.ends_match {
        return (present.clone(), None);
    }

    let lookup = vault.seek_back(active.history_window_millis);
    let archived = lookup.found_tick.and_then(|tick| vault.held_at(tick));
    let mut shown = match archived {
        Some(past) => overlay_archived_world(present, &past.state, beholder, active.spectated_receiver),
        None => present.clone(),
    };
    let reading = stamp_seat_into_world_state(vault, &mut shown, beholder, active, clock_millis);
    let clock_shift_millis = reading.as_ref().map_or(0, |reading| reading.clock_shift_millis);
    draw_deathreplay_overlay(
        &mut shown,
        present,
        archived.map(|past| &past.state),
        beholder,
        active,
        clock_shift_millis,
    );
    (shown, reading)
}

#[cfg(test)]
mod tests {
    use playerstate_t6::game::SpectatorTarget;
    use sim::Tick;

    use super::*;
    use crate::viewer_policy::cam_view::DeathReplayRun;

    const BEHOLDER: ClientId = ClientId(1);

    #[test]
    fn a_viewer_with_no_killcam_is_shown_the_live_world() {
        let present = Snapshot::unpublished(Tick(5));
        let (shown, reading) = state_and_reading_for_viewer(
            &SnapshotVault::new(),
            &LiveDeathReplays::new(),
            &present,
            BEHOLDER,
            250,
        );
        assert_eq!(shown.tick, present.tick);
        assert!(reading.is_none());
    }

    #[test]
    fn a_killcam_with_nothing_archived_falls_back_to_the_live_world() {
        let mut seats = LiveDeathReplays::new();
        seats.launch(
            BEHOLDER,
            DeathReplayRun {
                history_window_millis: 1000,
                spectated_receiver: ClientId(2),
                focus: SpectatorTarget::hitscan(2),
                expires_at_millis: 9000,
                ends_match: true,
                began_millis: 0,
                killcamoffset_ms: 0,
                predelay_ms: 0,
                postdelay_ms: 0,
                recalc_pending: false,
            },
        );
        let present = Snapshot::unpublished(Tick(5));
        let (shown, reading) =
            state_and_reading_for_viewer(&SnapshotVault::new(), &seats, &present, BEHOLDER, 250);
        assert_eq!(shown.tick, present.tick);
        assert!(reading.is_none());
    }

    #[test]
    fn a_viewer_number_too_big_for_a_bit_mask_is_left_alone() {
        let mut world = Snapshot::unpublished(Tick(1));
        show_movers_to(&mut world, ClientId(64));
        assert!(world.meta.script_movers.is_empty());
    }
}
