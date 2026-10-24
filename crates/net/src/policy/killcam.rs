use bevy::prelude::*;
use playerstate_t6::game::SeatFocus;
use sim::ClientId;

use crate::policy::seat::{ActiveKillcams, KillcamSession};
use crate::transport::archive::{ARCHIVE_TICK_MS, FrameArchive};

/// Arms each script seat for this tick. Returns the seats whose
/// `archivetime` the engine moved, for the scripts to read back: real
/// SpectatorClientEndFrame keeps the stepped `sess.archiveTime`, so the
/// replay plays on from where the watched player has a saved state (and
/// `_killcam.gsc` drops a killcam whose time fell to the death or to 0).
pub(crate) fn play_script_seats(
    seats: &mut ActiveKillcams,
    archive: &FrameArchive,
    now_ms: i32,
    scripted: &[(ClientId, sim::ScriptSeat)],
) -> Vec<(ClientId, i32)> {
    let mut stepped = Vec::new();
    for viewer in seats.viewers() {
        if !scripted.iter().any(|(id, _)| *id == viewer) {
            seats.clear(viewer);
        }
    }
    for (viewer, seat) in scripted {
        let script_focus = ClientId(seat.spectator_client as u32);
        let mut focus = SeatFocus::hitscan(seat.spectator_client);
        if seat.kill_cam_entity >= 0 {
            focus.kill_cam_entity = seat.kill_cam_entity;
        }
        if seat.look_at_entity >= 0 {
            focus.kill_cam_look_at_entity = seat.look_at_entity;
        }
        let started_at_ms = seats
            .get(*viewer)
            .filter(|s| {
                s.killcamoffset_ms == seat.archive_ms
                    && (s.focus_client == script_focus || seat.kill_cam_entity >= 0)
            })
            .map_or(now_ms, |s| s.started_at_ms);
        if started_at_ms == now_ms {
            diag::info!(
                Net,
                "killcam: script seat viewer={} spectating={} entity={} archive_ms={} length_ms={}",
                viewer.0,
                seat.spectator_client,
                seat.kill_cam_entity,
                seat.archive_ms,
                seat.length_ms
            );
        }
        let mut session = KillcamSession {
            archivetime_ms: seat.archive_ms,
            focus_client: script_focus,
            focus,
            ends_at_ms: i32::MAX,
            final_kill: seat.final_kill,
            started_at_ms,
            killcamoffset_ms: seat.archive_ms,
            predelay_ms: 0,
            postdelay_ms: 0,
            recalc_pending: false,
        };
        let found = follow_archived_focus(archive, &mut session, *viewer, seat);
        // The lookup lands on 50 ms ticks; only a real step is kept.
        if session.archivetime_ms < seat.archive_ms / ARCHIVE_TICK_MS * ARCHIVE_TICK_MS {
            diag::info!(
                Net,
                "killcam: viewer={} archive_ms {} -> {} (spectating {} has no saved state){}",
                viewer.0,
                seat.archive_ms,
                session.archivetime_ms,
                seat.spectator_client,
                if found { "" } else { ", none left: the killcam ends" }
            );
            session.killcamoffset_ms = session.archivetime_ms;
            stepped.push((*viewer, session.archivetime_ms));
        }
        if found {
            seats.arm(*viewer, session);
        } else {
            seats.clear(*viewer);
        }
    }
    stepped
}

/// Real SpectatorClientEndFrame: while the watched client has no saved
/// state at `archiveTime`, an entity killcam first turns to
/// `killCamTargetEntity` (or the viewer), otherwise `archiveTime` moves
/// 50 ms nearer the present; below 1 ms the seat lets go.
pub(crate) fn follow_archived_focus(
    archive: &FrameArchive,
    session: &mut KillcamSession,
    viewer: ClientId,
    seat: &sim::ScriptSeat,
) -> bool {
    loop {
        let lookup = archive.lookup(session.archivetime_ms.max(0));
        let found = lookup.tick.and_then(|tick| archive.frame(tick));
        session.archivetime_ms = if found.is_some() {
            lookup.attained_ms
        } else {
            0
        };
        if found.is_some_and(|f| f.player_state_exists(session.focus_client)) {
            return true;
        }
        if session.archivetime_ms < 1 {
            session.archivetime_ms = 0;
            return false;
        }
        if seat.kill_cam_entity >= 0 {
            let target = if (0..18).contains(&seat.target_client) {
                ClientId(seat.target_client as u32)
            } else {
                viewer
            };
            if session.focus_client != target {
                session.focus_client = target;
                session.focus.kill_cam_client_num = target.0 as i32;
                continue;
            }
        }
        session.archivetime_ms = (session.archivetime_ms - ARCHIVE_TICK_MS).max(0);
    }
}
