//! Sending a class choice to the host and following it to its answer.

use bevy::prelude::*;
use net::{ClientActionInbox, ClientSet, LocalPresentClient, PresentedSnapshot, SignonState};
use sim::{ClassId, ClientAction, ClientLifecycle, SimEvent};

use crate::classes::select::{
    ClassChangeAllowed, ClassChangeBlockReason, ClassSelectOverlayOpen, ClassSelectPhase,
    ClassSelectStatus, PendingClassEquip, accept_class_equip, reject_class_equip,
};
use crate::classes::store::SessionClassStore;

/// The request whose answer is still awaited.
#[derive(Resource, Debug, Default)]
pub struct EquipTxnWatch {
    request_id: Option<u32>,
}

/// Why signon forbids picking a class right now, or `None` when it allows it
/// (or there is no signon to consult).
fn signon_refusal(signon: Option<&SignonState>, fallback: &str) -> Option<String> {
    let signon = signon?;
    if signon.may_select_class() {
        return None;
    }
    Some(signon.phase.status_line().unwrap_or_else(|| fallback.to_owned()))
}

/// Turns the pending equip request into a `SelectClass` action for the host.
pub fn apply_pending_class_equip(
    mut pending: ResMut<PendingClassEquip>,
    store: Res<SessionClassStore>,
    weapons: Option<Res<assets::PreparedWeapons>>,
    mut actions: ResMut<ClientActionInbox>,
    mut watch: ResMut<EquipTxnWatch>,
    mut phase: ResMut<ClassSelectPhase>,
    mut status: ResMut<ClassSelectStatus>,
    local: Res<LocalPresentClient>,
    signon: Option<Res<SignonState>>,
) {
    let Some(request) = pending.0.take() else {
        return;
    };
    let request_id = request.request_id;

    if let Some(reason) = signon_refusal(signon.as_deref(), "signon not Active") {
        diag::info!(Ui, "class equip: dropped request_id={request_id} — {reason}");
        reject_class_equip(&mut phase, &mut status, request_id, reason);
        return;
    }

    let Some(slot) = store.slots.get(request.class_index) else {
        reject_class_equip(&mut phase, &mut status, request_id, "Class is unavailable");
        return;
    };
    let loadout = match weapons.as_deref() {
        None => Err("Weapon catalog is not ready".to_owned()),
        Some(weapons) => {
            let row = session::ClassRow::from(&frame::HostClassSlot::from(slot));
            session::loadout::resolve_personal_class(&row, &weapons.0)
        }
    };
    let loadout = match loadout {
        Ok(loadout) => loadout,
        Err(reason) => {
            reject_class_equip(&mut phase, &mut status, request_id, reason);
            return;
        }
    };

    let action = ClientAction::SelectClass {
        request_id,
        class_id: ClassId(request.class_index as u32),
        revision: 1,
        loadout,
    };
    match actions.push(local.0, action) {
        Ok(()) => {
            watch.request_id = Some(request_id);
            diag::info!(
                Ui,
                "class select: queued SelectClass request_id={request_id} id={} rev=1 (queued→gather→step→codec→present)",
                request.class_index
            );
        }
        Err(error) => {
            diag::info!(Ui, "class equip: request_id={request_id} not queued — {error}");
            reject_class_equip(
                &mut phase,
                &mut status,
                request_id,
                format!("not queued — {error}"),
            );
        }
    }
}

/// What the match, as presented, says about changing class.
fn lifecycle_verdict(lifecycle: ClientLifecycle) -> (bool, Option<String>) {
    use ClientLifecycle::*;
    match lifecycle {
        Alive | Dead | ChoosingClass | Spectating | Connecting => (true, None),
        other => (
            false,
            Some(format!("class change not allowed (lifecycle={other:?})")),
        ),
    }
}

/// Keeps `ClassChangeAllowed` and its reason in step with signon and the
/// presented match. Resources are written only when the value changes.
pub fn sync_class_change_allowed(
    presented: Res<PresentedSnapshot>,
    local: Res<LocalPresentClient>,
    overlay: Res<ClassSelectOverlayOpen>,
    mut allowed: ResMut<ClassChangeAllowed>,
    mut block: ResMut<ClassChangeBlockReason>,
    signon: Option<Res<SignonState>>,
) {
    let refused = signon_refusal(
        signon.as_deref(),
        "class change not allowed (signon not Active)",
    );
    let (now_allowed, reason) = if let Some(reason) = refused {
        (false, Some(reason))
    } else {
        let meta = presented
            .snapshot()
            .and_then(|snapshot| snapshot.meta.for_client(local.0));
        match meta {
            Some(meta) => lifecycle_verdict(meta.lifecycle),
            None if overlay.0 => (true, None),
            None => (
                false,
                Some("class change not allowed (no match loaded)".to_owned()),
            ),
        }
    };

    if allowed.0 != now_allowed {
        allowed.0 = now_allowed;
    }
    if block.0 != reason {
        block.0 = reason;
    }
}

/// Settles the awaited request when the host's reliable channel answers it.
pub fn resolve_class_equip_transaction(
    mut store: ResMut<SessionClassStore>,
    mut phase: ResMut<ClassSelectPhase>,
    mut overlay: ResMut<ClassSelectOverlayOpen>,
    mut status: ResMut<ClassSelectStatus>,
    mut watch: ResMut<EquipTxnWatch>,
    mut reliable: MessageReader<net::ReliableControlEvent>,
) {
    let Some(awaited) = watch.request_id.or_else(|| phase.pending_request_id()) else {
        reliable.clear();
        return;
    };

    for event in reliable.read() {
        match event.0 {
            SimEvent::ClassRejected {
                request_id, reason, ..
            } if request_id == awaited => {
                let reason = reason.as_str();
                if reject_class_equip(&mut phase, &mut status, awaited, reason) {
                    diag::info!(
                        Ui,
                        "class select: reliable Reject request_id={awaited} reason={reason}"
                    );
                }
                watch.request_id = None;
                return;
            }
            SimEvent::ClassAccepted { request_id, .. } if request_id == awaited => {
                if accept_class_equip(&mut store, &mut phase, &mut overlay, awaited) {
                    diag::info!(
                        Ui,
                        "class select: reliable Accept request_id={awaited} (overlay closed)"
                    );
                }
                watch.request_id = None;
                return;
            }
            _ => {}
        }
    }
}

pub(crate) fn register_equip_systems(app: &mut App) {
    app.init_resource::<EquipTxnWatch>()
        .init_resource::<frame::ClassSelectHandoff>()
        .add_systems(
            Update,
            reset_equip_transaction
                .after(frame::SessionSwapApplied)
                .before(apply_pending_class_equip),
        )
        .add_systems(
            Update,
            (
                consume_class_select_handoff,
                publish_signon_class_status,
                sync_class_change_allowed,
                apply_pending_class_equip,
            )
                .chain()
                .in_set(ClientSet::Present),
        )
        .add_systems(
            Update,
            resolve_class_equip_transaction
                .after(apply_pending_class_equip)
                .in_set(frame::ClassEquipResolved),
        );
}

/// Takes the host's class offers into the catalog, building the catalog from
/// the prepared weapons first if it has no registry yet.
fn consume_class_select_handoff(
    mut handoff: ResMut<frame::ClassSelectHandoff>,
    mut catalog: ResMut<crate::ClassLoadoutCatalog>,
    mut store: ResMut<SessionClassStore>,
    weapons: Option<Res<assets::PreparedWeapons>>,
) {
    if !handoff.pending {
        return;
    }
    if catalog.resolver.0.is_none() {
        let Some(weapons) = weapons else {
            return;
        };
        *catalog = crate::ClassLoadoutCatalog::from_weapon_registry(weapons.0.clone());
    }

    catalog.revision = catalog.revision.wrapping_add(1);
    catalog.primary = std::mem::take(&mut handoff.primary);
    catalog.secondary = std::mem::take(&mut handoff.secondary);
    catalog.lethal = std::mem::take(&mut handoff.lethal);
    catalog.tactical = std::mem::take(&mut handoff.tactical);
    catalog.excluded = std::mem::take(&mut handoff.excluded);
    store.apply_coverage_locks(std::mem::take(&mut handoff.lock_reasons));
    handoff.pending = false;
}

/// Shows the signon progress line in the class menu while no equip request is
/// in flight, and clears it again once signon has moved past the waiting
/// states.
fn publish_signon_class_status(
    signon: Option<Res<SignonState>>,
    phase: Res<ClassSelectPhase>,
    mut status: ResMut<ClassSelectStatus>,
) {
    if phase.is_pending() {
        return;
    }
    let Some(signon) = signon else {
        return;
    };
    let line = signon.phase.status_line();
    if status.0 == line {
        return;
    }
    match line {
        Some(line) => status.0 = Some(line),
        None => {
            let stale = status.0.as_deref().is_some_and(|shown| {
                shown.starts_with("waiting for host") || shown.starts_with("handshake rejected")
            });
            if stale {
                status.0 = None;
            }
        }
    }
}

/// Forgets everything about the old match once it has been torn down.
fn reset_equip_transaction(
    mut store: ResMut<SessionClassStore>,
    mut torn: MessageReader<frame::MatchTornDown>,
    mut watch: ResMut<EquipTxnWatch>,
    mut pending: ResMut<PendingClassEquip>,
    mut phase: ResMut<ClassSelectPhase>,
    mut status: ResMut<ClassSelectStatus>,
) {
    if torn.read().next().is_none() {
        return;
    }
    torn.clear();
    store.equipped = None;
    *watch = EquipTxnWatch::default();
    pending.0 = None;
    *phase = ClassSelectPhase::default();
    status.0 = None;
}
