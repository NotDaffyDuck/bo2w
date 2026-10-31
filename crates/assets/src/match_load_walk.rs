//! Running a queued request: starting its walk once nothing else holds the
//! resources, watching it, and settling it into a result, or into nothing if
//! the player gave up on it.

use asset_transport::progress::LoadCompletion;
use bevy::prelude::*;
use bevy::tasks::{Task, futures_lite::future};
use frame::ClientStep;

use crate::load_process::MapLoadProcess;
use crate::match_load_admit::approve_map_load;
use crate::match_load_state::{
    MapLoadApproval, SessionLoadCancel, SessionLoadApproved, SessionLoadOccupied, MatchLoadDispatch,
    SessionLoadOrder, ReadiedSessionSignal, PreparedMatchSound,
};
use crate::load_stages::{MatchLoadOutcome, loader_pool, read_staged_round};

/// A walk that has been started and not yet collected. It yields `None`
/// when the player cancelled it part-way.
#[derive(Resource)]
struct WalkInFlight {
    id: u64,
    cancel: LoadCompletion,
    job: Task<Option<ReadiedSessionSignal>>,
}

/// What the starter does with the queued request on this frame.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Verdict {
    /// The player aborted it: throw it away unstarted.
    Discard,
    /// Another walk still runs; ask again next frame.
    Wait,
    /// The previous session is still handing its resources back.
    Park,
    /// Nothing is in the way: start the walk.
    Launch,
}

fn verdict(aborted: bool, walk_running: bool, retiring: bool) -> Verdict {
    match (aborted, walk_running, retiring) {
        (true, _, _) => Verdict::Discard,
        (false, true, _) => Verdict::Wait,
        (false, false, true) => Verdict::Park,
        (false, false, false) => Verdict::Launch,
    }
}

/// Queues the removal of resource `R`.
fn retire<R: Resource>(commands: &mut Commands) {
    commands.remove_resource::<R>();
}

fn begin_walk(
    mut commands: Commands,
    request: Option<Res<SessionLoadOrder>>,
    running: Option<Res<WalkInFlight>>,
    retiring: Option<Res<frame::Retiring>>,
    abort: Option<Res<SessionLoadCancel>>,
    mut parked_on: Local<Option<u64>>,
    mut busy: ResMut<SessionLoadOccupied>,
) {
    let Some(queued) = request.as_deref() else { return };
    let ticket = queued.ticket;
    let retired_busy = retiring.as_deref().map_or(false, frame::Retiring::busy);
    let call = verdict(SessionLoadCancel::covers(abort.as_deref(), ticket), running.is_some(), retired_busy);

    if call == Verdict::Discard {
        parked_on.take();
        retire::<SessionLoadOrder>(&mut commands);
        // A walk that is still going keeps the flag up until it is collected.
        if running.is_none() {
            busy.0 = false;
        }
        diag::info!(World, "match load: request #{ticket} aborted before its walk started");
    } else if call == Verdict::Park {
        let before = parked_on.replace(ticket);
        if before != Some(ticket) {
            diag::info!(World, "match load: request #{ticket} waits for retired resources");
        }
    } else if call == Verdict::Launch {
        let resumed = *parked_on == Some(ticket);
        *parked_on = None;
        if resumed {
            diag::info!(World, "match load: request #{ticket} goes on after the retirement");
        }
        launch_walk(&mut commands, queued);
        busy.0 = true;
    }
}

/// Opens the process record, announces the request as taken, and hands the
/// walk to the load pool.
fn launch_walk(commands: &mut Commands, queued: &SessionLoadOrder) {
    let load_req_id = queued.ticket;
    let decrypt_secret = queued.decrypt_secret;
    let tracker = queued.progress.clone();
    // The process outlives the overlay and every task started here: the table
    // reads it, admission finishes it, and the bench still has it after the
    // screen is gone.
    let process = MapLoadProcess::new(load_req_id, decrypt_secret, queued.zone.clone(), tracker.clone());
    commands.insert_resource(process);
    let wanted_zone = queued.zone.clone();
    commands.insert_resource(SessionLoadApproved { ticket: load_req_id, decrypt_secret, zone: wanted_zone.clone() });

    let inputs = (queued.zone_archive_file.clone(), queued.common_zone.clone(), tracker.clone());
    let job = loader_pool().spawn(async move {
        let (map_zone, common_zone, walk_tracker) = inputs;
        let outcome = read_staged_round(map_zone, common_zone, walk_tracker).await;
        let MatchLoadOutcome::Ready(prepared) = outcome else { return None };
        Some(ReadiedSessionSignal { ticket: load_req_id, decrypt_secret, zone: wanted_zone, prepared })
    });
    commands.insert_resource(WalkInFlight { id: load_req_id, cancel: tracker, job });
    retire::<SessionLoadOrder>(commands);
}

fn collect_walk(
    mut commands: Commands,
    task: Option<ResMut<WalkInFlight>>,
    mut busy: ResMut<SessionLoadOccupied>,
    abort: Option<Res<SessionLoadCancel>>,
) {
    let Some(mut flight) = task else { return };
    let aborted = |ticket: u64| SessionLoadCancel::covers(abort.as_deref(), ticket);

    let ticket = flight.id;
    if aborted(ticket) && !flight.cancel.was_aborted() {
        flight.cancel.cancel();
        diag::info!(
            World,
            "match load: cancelling the walk of request #{ticket}; it stops at its next checkpoint"
        );
    }
    let polled = future::block_on(future::poll_once(&mut flight.job));
    let Some(finished) = polled else { return };

    busy.0 = false;
    retire::<WalkInFlight>(&mut commands);
    retire::<SessionLoadApproved>(&mut commands);
    match finished {
        None => {
            diag::info!(World, "match load: the walk of request #{ticket} was cancelled; nothing prepared");
        }
        Some(done) if aborted(done.ticket) => {
            let (dropped, zone) = (done.ticket, &done.zone);
            diag::info!(World, "match load: dropped the walk of aborted request #{dropped} (`{zone}`)");
        }
        Some(done) => publish(&mut commands, done),
    }
}

/// Splits the sound catalog off a finished walk and publishes both halves.
fn publish(commands: &mut Commands, mut done: ReadiedSessionSignal) {
    let sound = done.prepared.sound.take();
    let materials = &done.prepared.materials;
    let half = PreparedMatchSound {
        sound: sound.unwrap_or_else(|| Err("the map zone never opened".to_owned())),
        products_id: materials.products_id,
        common_profile_id: materials.common_profile_id,
        zone: done.zone.clone(),
        decrypt_secret: done.decrypt_secret,
    };
    commands.insert_resource(half);
    commands.insert_resource(done);
}

/// An abort has done its work once no walk, request or result is left for it.
fn retire_spent_abort(
    mut commands: Commands,
    abort: Option<Res<SessionLoadCancel>>,
    task: Option<Res<WalkInFlight>>,
    request: Option<Res<SessionLoadOrder>>,
    ready: Option<Res<ReadiedSessionSignal>>,
) {
    let settled = [task.is_none(), request.is_none(), ready.is_none()].into_iter().all(|gone| gone);
    if settled && abort.is_some() {
        retire::<SessionLoadCancel>(&mut commands);
    }
}

pub fn add_match_load_systems(app: &mut App) {
    // `chain` orders the three exactly as `after` did, with the same sync
    // points between them, so each sees the previous one's commands.
    let walk = (begin_walk, collect_walk, retire_spent_abort)
        .chain()
        .in_set(MatchLoadDispatch)
        .after(MapLoadApproval);
    let approve = approve_map_load.in_set(MapLoadApproval);
    app.init_resource::<SessionLoadOccupied>();
    app.add_systems(
        Update,
        (approve, walk).after(frame::MatchSwapDone).in_set(ClientStep::Load),
    );
}

#[cfg(test)]
mod tests {
    use super::{Verdict, verdict};

    #[test]
    fn an_abort_wins_over_everything() {
        for running in [false, true] {
            for retiring in [false, true] {
                assert_eq!(verdict(true, running, retiring), Verdict::Discard);
            }
        }
    }

    #[test]
    fn a_running_walk_comes_before_retirement() {
        assert_eq!(verdict(false, true, true), Verdict::Wait);
        assert_eq!(verdict(false, true, false), Verdict::Wait);
        assert_eq!(verdict(false, false, true), Verdict::Park);
        assert_eq!(verdict(false, false, false), Verdict::Launch);
    }
}
