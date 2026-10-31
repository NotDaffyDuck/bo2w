//! The "remote" line: where each other player's body is drawn against the newest snapshot.

use bevy::prelude::*;

use crate::FrameClock;
use crate::client::drive::NewestTakenSnapshot;
use crate::client::presentation::entity_records::ClientEntityPoseState;
use crate::client::presentation::entity_slots::ClientEntityRecord;
use crate::client::presentation::shown_frame::{HostViewClient, ShownWorldFrame};

pub(super) fn emit_remote(
    timebase: Option<Res<FrameClock>>,
    shown: Res<ShownWorldFrame>,
    local: Res<HostViewClient>,
    last_adopted: Option<Res<NewestTakenSnapshot>>,
    bodies: Query<(&ClientEntityRecord, &ClientEntityPoseState)>,
) {
    let Some(timebase) = timebase else {
        return;
    };
    // The newest snapshot there is: the one just adopted, else the one on screen.
    let newest = last_adopted
        .as_ref()
        .and_then(|adopted| adopted.next())
        .or_else(|| shown.snapshot());

    for (record, pose) in &bodies {
        let Some(client) = record.client() else {
            continue;
        };
        if client == local.0 {
            continue;
        }
        let verdict = shown
            .peer_reading_origin(client)
            .map(|sample_origin_tag| sample_origin_tag.outcome.dump_variety_name());
        let state_position = newest
            .and_then(|snapshot| snapshot.players.iter().find(|(id, _)| *id == client))
            .map(|(_, state)| state.origin);
        perf::remote(
            client.0,
            timebase.time(),
            i32::from(pose.pose_entity_kind),
            pose.origin,
            state_position,
            verdict,
        );
    }
}
