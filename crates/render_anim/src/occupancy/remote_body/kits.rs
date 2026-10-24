use super::*;

pub(super) fn prepare_remote_kits(
    bodies: Option<Res<PreparedBodies>>,
    weapons: Option<Res<PreparedWeapons>>,
    world_weapons: Option<Res<PreparedWorldWeapons>>,
    mut kits: ResMut<PreparedRemoteKits>,
) {
    let (Some(bodies), Some(weapons), Some(world)) = (bodies, weapons, world_weapons) else {
        return;
    };
    if kits.owned_by(&bodies, &weapons, &world) {
        return;
    }
    let started = std::time::Instant::now();
    *kits = PreparedRemoteKits::prepare(&bodies, &weapons, &world);
    diag::info!(
        World,
        "remote kits: prepared for {} weapons in {:.1}ms",
        weapons.0.len(),
        started.elapsed().as_secs_f64() * 1000.0
    );
}

pub(super) fn finish_body_draw_plan(mut plan: ResMut<RemoteBodyDrawPlan>) {
    finish_remote_body_draw_plan(&mut plan);
}

/// The entity number the player is looking out of: whoever they are
/// spectating from a seat, else themself.
pub(super) fn viewing_entity(local: &LocalPresentClient, subject: Option<&ViewSubject>) -> i32 {
    let number = match subject {
        Some(ViewSubject::Seat {
            focus: Some(focus), ..
        }) => i32::try_from(*focus),
        _ => i32::try_from(local.0.0),
    };
    number.unwrap_or(0)
}

/// What decides whether a remote entity's body is drawn this frame.
pub(super) fn draw_gate(
    local: &LocalPresentClient,
    presented: &PresentedSnapshot,
    subject: Option<&ViewSubject>,
) -> PlayerDrawGate {
    let in_killcam = subject.is_some_and(|s| s.in_killcam());
    PlayerDrawGate {
        eyes_entity_num: viewing_entity(local, subject),
        other_flags: presented
            .player(local.0)
            .map(|ps| ps.other_flags)
            .unwrap_or(0),
        rendering_third_person: presented_is_third_person(presented, local.0, in_killcam),
    }
}

/// The team a client plays on, as the snapshot's metadata gives it.
pub(super) fn client_team(
    presented: &PresentedSnapshot,
    client: sim::ClientId,
) -> (Option<u8>, i32) {
    let meta = presented
        .snapshot()
        .and_then(|snapshot| snapshot.meta.for_client(client));
    (
        meta.and_then(|m| m.ffa_team),
        meta.map_or(0, |m| m.client_state_team),
    )
}
