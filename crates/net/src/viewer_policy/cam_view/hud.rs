use playerstate_t6::game::HITSCAN_KILL_CAM_ENTITY;
use sim::{ClientId, Snapshot};

use super::runs::DeathReplayRun;

/// Give the beholder's client row a killcam HUD: their own current elements from the live game, and
/// the focused player's archived ones moved onto the live clock. A killcam that follows something
/// other than a hitscan shot shows no archived elements.
pub(super) fn draw_deathreplay_overlay(
    shown: &mut Snapshot,
    present: &Snapshot,
    recorded: Option<&Snapshot>,
    beholder: ClientId,
    session: &DeathReplayRun,
    clock_shift_millis: i32,
) {
    let own_hud = present
        .meta
        .for_client(beholder)
        .map(|meta| meta.hud_current.clone())
        .unwrap_or_default();

    let follows_a_hitscan_shot = session.focus.death_replay_target == HITSCAN_KILL_CAM_ENTITY;
    let mut recorded_hud = match recorded.and_then(|snap| snap.meta.for_client(session.spectated_receiver)) {
        Some(meta) if follows_a_hitscan_shot => meta.hud_archival.clone(),
        _ => Vec::new(),
    };
    sim::shift_replayed_clock(&mut recorded_hud, clock_shift_millis);

    if let Some(entry) = shown.meta.clients.iter_mut().find(|(who, _)| *who == beholder) {
        let row = &mut entry.1;
        let hud = sim::KillcamHud {
            final_kill: session.ends_match,
        };
        row.killcam_hud = Some(hud);
        row.hud_current = own_hud;
        row.hud_archival = recorded_hud;
    }
}
