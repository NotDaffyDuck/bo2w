//! The two systems that keep the entity table current: the client one follows the snapshot on
//! screen, the authority one follows the tick the server just produced.

use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use playerstate_t6::game::AvatarState;
use sim::{ClientId, PlayerCorpsePool, Snapshot, Tick};

use super::reconcile::{Pass, advance_blends, retire_unclaimed, settle};
use super::{ClientEntityRecord, EntitySpawnTally, EntityTableCells, EntitySyncBranch, claims};
use crate::client::presentation::entity_records::ClientEntityPoseState;
use crate::identity_slips::LinkIdentityDeficits;
use crate::{NewestTakenSnapshot, HostStepNumber};

/// Everything a run needs to write into the entity table.
#[derive(SystemParam)]
pub struct EntityTableAccess<'w, 's> {
    commands: Commands<'w, 's>,
    census: ResMut<'w, EntitySpawnTally>,
    pose_states: Query<'w, 's, &'static mut ClientEntityPoseState>,
    identities: Query<'w, 's, &'static ClientEntityRecord>,
    gaps: ResMut<'w, LinkIdentityDeficits>,
    cells: ResMut<'w, EntityTableCells>,
}

/// The rows a snapshot puts forward for the table, and the time they belong to.
struct Intake<'a> {
    for_instant_millis: i32,
    entities: &'a [entity_t6::EntitySnapshot],
    corpses: &'a PlayerCorpsePool,
    players: &'a [(ClientId, AvatarState)],
}

impl<'a> Intake<'a> {
    /// A missing snapshot puts nothing forward, at time zero; `no_corpses` is the empty pool to
    /// borrow for that case.
    fn from_world_state(snapshot: Option<&'a Snapshot>, no_corpses: &'a PlayerCorpsePool) -> Self {
        match snapshot {
            None => Intake {
                for_instant_millis: 0,
                entities: &[],
                corpses: no_corpses,
                players: &[],
            },
            Some(frame) => Intake {
                for_instant_millis: crate::HostInstant::from_step_number(frame.tick).as_millis(),
                entities: &frame.meta.entities,
                corpses: &frame.meta.corpses,
                players: &frame.players,
            },
        }
    }
}

/// What differs between the client run and the authority run.
struct Mode {
    side: EntitySyncBranch,
    reset_all: bool,
    /// The viewer whose own body stays out of the table, if any.
    seated_viewer: Option<ClientId>,
    /// Two snapshots are available to blend between.
    blend_ready: bool,
}

fn reconcile_table(table: &mut EntityTableAccess, intake: &Intake<'_>, mode: &Mode) {
    if mode.blend_ready {
        advance_blends(&mut table.pose_states);
    }
    let pass = Pass {
        arm: mode.side,
        reset_all: mode.reset_all,
        for_instant_millis: intake.for_instant_millis,
    };
    let queued = claims::collect(
        intake.players,
        intake.corpses,
        intake.entities,
        mode.seated_viewer,
        &mut table.gaps,
    );
    settle(
        &mut table.commands,
        &queued,
        &mut table.cells,
        &table.identities,
        &mut table.pose_states,
        &pass,
        &mut table.census,
    );
    retire_unclaimed(
        &mut table.commands,
        &queued,
        &mut table.cells,
        &mut table.pose_states,
    );
    table.gaps.log_counts();
}

/// True when this frame has nothing new to put in the table. A replay snapshot is used once; a
/// live one is used when it was adopted this frame, or when there is no snapshot at all.
fn nothing_new(
    archived: bool,
    remembered: Option<(Tick, bool)>,
    now: Option<(Tick, bool)>,
    adopted_this_frame: bool,
    has_snapshot: bool,
) -> bool {
    let same = remembered == now;
    let live_and_stale = !adopted_this_frame && has_snapshot;
    same && (archived || live_and_stale)
}

/// True when the viewer moved into or out of a replay since the previous run.
fn replay_flipped(remembered: Option<(Tick, bool)>, archived: bool) -> bool {
    matches!(remembered, Some((_, was_archived)) if was_archived != archived)
}

pub fn mirror_entities(
    adopted: Res<NewestTakenSnapshot>,
    local: Option<Res<crate::HostViewClient>>,
    proxy: Option<Res<crate::OtherPlayerSmootherHolder>>,
    clock: Option<Res<crate::FrameClock>>,
    mut remembered: Local<Option<(Tick, bool)>>,
    mut table: EntityTableAccess,
) {
    let me = local.map(|resource| resource.0);
    // Watching a replay means the local player's own state is not a live frame.
    let replaying = adopted.next().is_some_and(|frame| {
        frame
            .players
            .iter()
            .any(|(id, ps)| Some(*id) == me && !ps.is_live_frame())
    });
    // A replay blends from the snapshot the clock already passed to the one after it, so the
    // clock sits inside the pair the way it does live. Using the passed one would leave the blend
    // finished and every body would hop once per snapshot.
    let replay_frame = match (replaying, proxy.as_ref(), clock.as_ref()) {
        (true, Some(proxy), Some(clock)) => {
            let at = clock.time();
            proxy
                .0
                .snapshot_after(at)
                .or_else(|| proxy.0.blended_frame_at(at))
        }
        _ => None,
    };
    let shown = match replay_frame.as_deref() {
        Some(frame) => Some(frame),
        None => adopted.next(),
    };
    let now = shown.map(|frame| (frame.tick, replaying));
    if nothing_new(
        replaying,
        *remembered,
        now,
        adopted.taken_in_current_step,
        shown.is_some(),
    ) {
        return;
    }
    let mode = Mode {
        side: EntitySyncBranch::Client,
        reset_all: replay_flipped(*remembered, replaying),
        seated_viewer: if replaying { me } else { None },
        blend_ready: adopted.frame_state.is_some(),
    };
    *remembered = now;

    let no_corpses = PlayerCorpsePool::default();
    let intake = Intake::from_world_state(shown, &no_corpses);
    reconcile_table(&mut table, &intake, &mode);
}

pub(crate) fn sync_authority_entities(host_step_from_raw: Res<HostStepNumber>, mut table: EntityTableAccess) {
    let no_corpses = PlayerCorpsePool::default();
    let newest = host_step_from_raw.0.as_ref().map(|produced| &produced.snapshot);
    let mode = Mode {
        side: EntitySyncBranch::Authority,
        reset_all: false,
        seated_viewer: None,
        blend_ready: false,
    };
    let intake = Intake::from_world_state(newest, &no_corpses);
    reconcile_table(&mut table, &intake, &mode);
}

#[cfg(test)]
mod tests {
    use super::*;

    const LIVE_10: Option<(Tick, bool)> = Some((Tick(10), false));
    const REPLAY_10: Option<(Tick, bool)> = Some((Tick(10), true));

    #[test]
    fn replay_frames_are_used_one_time_only() {
        // (remembered, now, adopted this frame, nothing new) for a replay run.
        let runs = [
            (REPLAY_10, REPLAY_10, true, true),
            (REPLAY_10, REPLAY_10, false, true),
            (Some((Tick(9), true)), REPLAY_10, false, false),
        ];
        for (remembered, now, adopted, expected) in runs {
            assert_eq!(nothing_new(true, remembered, now, adopted, true), expected);
        }
    }

    #[test]
    fn live_frames_are_new_once_adopted_and_stale_otherwise() {
        let ninth = Some((Tick(9), false));
        // (archived, remembered, now, adopted this frame, has a snapshot, nothing new)
        let cases = [
            (false, LIVE_10, LIVE_10, true, true, false),
            (false, LIVE_10, LIVE_10, false, true, true),
            (false, None, None, false, false, false),
            (false, LIVE_10, LIVE_10, false, false, false),
            (false, ninth, LIVE_10, false, true, false),
            (false, REPLAY_10, LIVE_10, false, true, false),
        ];
        for (archived, remembered, now, adopted, has, expected) in cases {
            let got = nothing_new(archived, remembered, now, adopted, has);
            assert_eq!(got, expected, "{remembered:?} -> {now:?}");
        }
    }

    #[test]
    fn moving_between_live_and_replay_flips_the_reset() {
        let table = [
            (LIVE_10, true, true),
            (REPLAY_10, false, true),
            (LIVE_10, false, false),
            (REPLAY_10, true, false),
            (None, true, false),
        ];
        for (remembered, archived, flipped) in table {
            assert_eq!(replay_flipped(remembered, archived), flipped);
        }
    }

    #[test]
    fn a_snapshot_puts_forward_its_rows_at_its_own_time_and_none_puts_nothing() {
        let no_corpses = PlayerCorpsePool::default();
        let nothing = Intake::from_world_state(None, &no_corpses);
        assert_eq!(nothing.for_instant_millis, 0);
        assert!(nothing.players.is_empty() && nothing.entities.is_empty());

        let mut frame = Snapshot::unpublished(Tick(0));
        frame.tick = Tick(25);
        frame.players = vec![(ClientId(2), AvatarState::ZERO)];
        let intake = Intake::from_world_state(Some(&frame), &no_corpses);
        assert_eq!(intake.players.len(), 1);
        assert_eq!(
            intake.for_instant_millis,
            crate::HostInstant::from_step_number(Tick(25)).as_millis()
        );
    }
}
