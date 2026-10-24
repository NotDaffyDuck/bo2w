//! Running a queued request: starting its walk once nothing else holds the
//! resources, watching it, and settling it into a result, or into nothing if
//! the player gave up on it.

use asset_transport::progress::LoadProgress;
use bevy::prelude::*;
use bevy::tasks::{Task, futures_lite::future};
use frame::ClientSet;

use crate::load_process::MapLoadProcess;
use crate::match_load_admit::approve_map_load;
use crate::match_load_state::{
    MapLoadApproval, MatchLoadAbort, MatchLoadAccepted, MatchLoadBusy, MatchLoadDispatch,
    MatchLoadRequest, PreparedMatchReady, PreparedMatchSound,
};
use crate::session_load::{MatchLoadOutcome, load_pool, load_prepared_match};

/// The walk in flight. `None` when it was cancelled.
#[derive(Resource)]
struct MatchLoadTask {
    walk: Task<Option<PreparedMatchReady>>,
    request_id: u64,
    progress: LoadProgress,
}

fn start_match_load(
    mut commands: Commands,
    request: Option<Res<MatchLoadRequest>>,
    running: Option<Res<MatchLoadTask>>,
    retiring: Option<Res<frame::Retiring>>,
    abort: Option<Res<MatchLoadAbort>>,
    mut parked_on: Local<Option<u64>>,
    mut busy: ResMut<MatchLoadBusy>,
) {
    let Some(request) = request else {
        return;
    };
    let id = request.request_id;

    if MatchLoadAbort::covers(abort.as_deref(), id) {
        *parked_on = None;
        commands.remove_resource::<MatchLoadRequest>();
        if running.is_none() {
            busy.0 = false;
        }
        diag::info!(World, "match load: request #{id} aborted before its walk started");
        return;
    }
    if running.is_some() {
        return;
    }
    if retiring.is_some_and(|retiring| retiring.busy()) {
        if *parked_on != Some(id) {
            diag::info!(World, "match load: request #{id} waits for retired resources");
            *parked_on = Some(id);
        }
        return;
    }
    if parked_on.take() == Some(id) {
        diag::info!(World, "match load: request #{id} goes on after the retirement");
    }

    let MatchLoadRequest {
        load_key,
        zone,
        zone_ff,
        common_mp,
        progress,
        ..
    } = &*request;
    let (load_key, zone) = (*load_key, zone.clone());
    // The process outlives the overlay and every task started here: the table
    // reads it, admission finishes it, and the bench still has it after the
    // screen is gone.
    commands.insert_resource(MapLoadProcess::new(id, load_key, zone.clone(), progress.clone()));
    commands.insert_resource(MatchLoadAccepted {
        request_id: id,
        load_key,
        zone: zone.clone(),
    });

    let (zone_ff, common_mp, walk_progress) = (zone_ff.clone(), common_mp.clone(), progress.clone());
    let walk = load_pool().spawn(async move {
        match load_prepared_match(zone_ff, common_mp, walk_progress).await {
            MatchLoadOutcome::Ready(prepared) => Some(PreparedMatchReady {
                request_id: id,
                load_key,
                zone,
                prepared,
            }),
            MatchLoadOutcome::Canceled => None,
        }
    });
    commands.insert_resource(MatchLoadTask {
        walk,
        request_id: id,
        progress: progress.clone(),
    });
    commands.remove_resource::<MatchLoadRequest>();
    busy.0 = true;
}

fn poll_match_load(
    mut commands: Commands,
    task: Option<ResMut<MatchLoadTask>>,
    mut busy: ResMut<MatchLoadBusy>,
    abort: Option<Res<MatchLoadAbort>>,
) {
    let Some(mut task) = task else {
        return;
    };
    let id = task.request_id;

    if MatchLoadAbort::covers(abort.as_deref(), id) && !task.progress.is_canceled() {
        task.progress.cancel();
        diag::info!(
            World,
            "match load: cancelling the walk of request #{id}; it stops at its next checkpoint"
        );
    }
    let Some(finished) = future::block_on(future::poll_once(&mut task.walk)) else {
        return;
    };
    busy.0 = false;
    commands.remove_resource::<MatchLoadTask>();
    commands.remove_resource::<MatchLoadAccepted>();

    let Some(mut ready) = finished else {
        diag::info!(World, "match load: the walk of request #{id} was cancelled; nothing prepared");
        return;
    };
    if MatchLoadAbort::covers(abort.as_deref(), ready.request_id) {
        diag::info!(
            World,
            "match load: dropped the walk of aborted request #{} (`{}`)",
            ready.request_id,
            ready.zone
        );
        return;
    }
    commands.insert_resource(PreparedMatchSound {
        load_key: ready.load_key,
        zone: ready.zone.clone(),
        common_profile_id: ready.prepared.materials.common_profile_id,
        products_id: ready.prepared.materials.products_id,
        sound: ready
            .prepared
            .sound
            .take()
            .unwrap_or_else(|| Err("the map zone never opened".to_owned())),
    });
    commands.insert_resource(ready);
}

/// An abort has done its work once no walk, request or result is left for it.
fn expire_match_load_abort(
    mut commands: Commands,
    abort: Option<Res<MatchLoadAbort>>,
    task: Option<Res<MatchLoadTask>>,
    request: Option<Res<MatchLoadRequest>>,
    ready: Option<Res<PreparedMatchReady>>,
) {
    let leftover = task.is_some() || request.is_some() || ready.is_some();
    if abort.is_some() && !leftover {
        commands.remove_resource::<MatchLoadAbort>();
    }
}

pub fn register_match_load_systems(app: &mut App) {
    let dispatch = (
        start_match_load,
        poll_match_load.after(start_match_load),
        expire_match_load_abort.after(poll_match_load),
    )
        .in_set(MatchLoadDispatch)
        .after(MapLoadApproval);
    app.init_resource::<MatchLoadBusy>().add_systems(
        Update,
        (approve_map_load.in_set(MapLoadApproval), dispatch)
            .after(frame::SessionSwapApplied)
            .in_set(ClientSet::Load),
    );
}
