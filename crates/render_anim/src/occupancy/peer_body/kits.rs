use super::*;

pub(super) fn prepare_remote_kits(
    bodies: Option<Res<ReadiedCharacterBodies>>,
    weapons: Option<Res<StagedWeapons>>,
    level_placed_weapons: Option<Res<ReadiedWorldWeapons>>,
    mut kits: ResMut<PreparedRemoteKits>,
) {
    let (Some(bodies), Some(weapons), Some(world)) = (bodies, weapons, level_placed_weapons) else {
        return;
    };
    if kits.owned_by(&bodies, &weapons, &world) {
        return;
    }
    let clock = std::time::Instant::now();
    *kits = PreparedRemoteKits::stage(&bodies, &weapons, &world);
    let millis = clock.elapsed().as_secs_f64() * 1e3;
    diag::info!(
        World,
        "remote kits: prepared for {} weapons in {:.1}ms",
        weapons.0.len(),
        millis
    );
}

pub(super) fn finish_body_draw_plan(mut plan: ResMut<OtherBodyPaintPlan>) {
    seal_other_body_batch(&mut plan);
}

/// The entity number the player is looking out of: whoever they are
/// spectating from a seat, else themself.
pub(super) fn viewing_entity(local: &HostViewClient, subject: Option<&EyeSubject>) -> i32 {
    let looking_from = match subject {
        Some(EyeSubject::ChairSlot { focus, .. }) => focus.unwrap_or(local.0.0),
        _ => local.0.0,
    };
    i32::try_from(looking_from).unwrap_or_default()
}

/// What decides whether a remote entity's body is drawn this frame.
pub(super) fn draw_gate(
    local: &HostViewClient,
    presented: &ShownWorldFrame,
    subject: Option<&EyeSubject>,
) -> PlayerDrawGate {
    let killcam = subject.is_some_and(|view| view.replaying_final_death());
    let secondary_state_bits = presented.player(local.0).map_or(0, |ps| ps.secondary_state_bits);
    PlayerDrawGate {
        showing_outside_camera: shown_in_third_person(presented, local.0, killcam),
        secondary_state_bits,
        viewer_entity: viewing_entity(local, subject),
    }
}

/// The team a client plays on, as the snapshot's metadata gives it.
pub(super) fn client_team(
    presented: &ShownWorldFrame,
    client_num: sim::ClientId,
) -> (Option<u8>, i32) {
    let Some(meta) = presented.snapshot().and_then(|s| s.meta.for_client(client_num)) else {
        return (None, 0);
    };
    (meta.ffa_team, meta.client_state_team)
}
