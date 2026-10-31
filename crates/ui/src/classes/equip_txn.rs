//! Sending a class choice to the host and following it to its answer.

use bevy::prelude::*;
use net::{PlayerActionIntake, ClientStep, HostViewClient, ShownWorldFrame, JoinProgressRecord};
use sim::{ClassId, PlayerIntent, PlayerSessionStage, MatchOccurrence};

use crate::classes::select::{LoadoutSwapPermit, LoadoutSwapDenial};
use crate::classes::select::{LoadoutPickerShown, LoadoutPickerStage, LoadoutPickerReport};
use crate::classes::select::{QueuedLoadoutArm, grant_loadout_arm, refuse_loadout_arm};
use crate::classes::store::MatchLoadoutVault;

/// The request whose answer is still awaited.
#[derive(Resource, Debug, Default)]
pub struct LoadoutArmTracker {
    awaited_id: Option<u32>,
}

/// Why signon forbids picking a class right now, or `None` when it allows it
/// (or there is no signon to consult).
fn signon_refusal(signon: Option<&JoinProgressRecord>, fallback: &str) -> Option<String> {
    let signon = signon?;
    if signon.can_choose_loadout() {
        return None;
    }
    let line = signon.phase.summary_banner();
    Some(line.unwrap_or_else(|| fallback.to_owned()))
}

/// Turns the pending equip request into a `SelectClass` action for the host.
pub fn enact_queued_loadout_arm(
    local: Res<HostViewClient>,
    classes: Res<MatchLoadoutVault>,
    signon_state: Option<Res<JoinProgressRecord>>,
    prepared: Option<Res<assets::StagedWeapons>>,
    mut pending: ResMut<QueuedLoadoutArm>,
    mut intake: ResMut<PlayerActionIntake>,
    mut inflight: ResMut<LoadoutArmTracker>,
    mut phase: ResMut<LoadoutPickerStage>,
    mut words: ResMut<LoadoutPickerReport>,
) {
    let ask = match pending.0.take() {
        Some(ask) => ask,
        None => return,
    };
    let id = ask.ask_number;
    let mut refuse = |why: String| {
        refuse_loadout_arm(&mut phase, &mut words, id, why);
    };

    if let Some(why) = signon_refusal(signon_state.as_deref(), "signon not Active") {
        diag::info!(Ui, "class equip: dropped ask {id}, {why}");
        return refuse(why);
    }

    let build_loadout = || -> Result<_, String> {
        let slot = classes
            .slots
            .get(ask.category_slot)
            .ok_or("Class is unavailable")?;
        let kits = prepared.as_deref().ok_or("Weapon catalog is not ready")?;
        let row = session::ClassRow::from(&frame::ServerKitSlot::from(slot));
        session::loadout::resolve_personal_class(&row, &kits.0)
    };
    let loadout = match build_loadout() {
        Ok(loadout) => loadout,
        Err(why) => return refuse(why),
    };

    let revision = 1;
    let select = PlayerIntent::PickLoadout {
        request_id: ask.ask_number,
        class_id: ClassId(ask.category_slot as u32),
        revision,
        loadout,
    };
    match intake.push(local.0, select) {
        Ok(_) => {}
        Err(error) => {
            diag::info!(Ui, "class equip: ask {id} could not be queued, {error}");
            return refuse(format!("not queued — {error}"));
        }
    }
    inflight.awaited_id = Some(id);
    diag::info!(
        Ui,
        "class select: sent ask {id} for class {} rev=1",
        ask.category_slot
    );
}

/// Why the match, as presented, will not let him change class at this
/// stage of his session; `None` when it will.
fn stage_refusal(now: PlayerSessionStage) -> Option<String> {
    match now {
        PlayerSessionStage::Connecting
        | PlayerSessionStage::Spectating
        | PlayerSessionStage::PickingLoadout
        | PlayerSessionStage::Dead
        | PlayerSessionStage::Alive => None,
        _ => Some(format!("class change not allowed (lifecycle={now:?})")),
    }
}

/// Keeps `LoadoutSwapPermit` and its reason in step with signon and the
/// presented match. Resources are written only when the value changes.
pub fn refresh_loadout_swap_permit(
    local: Res<HostViewClient>,
    presented: Res<ShownWorldFrame>,
    overlay_open: Res<LoadoutPickerShown>,
    signon_state: Option<Res<JoinProgressRecord>>,
    mut may_change: ResMut<LoadoutSwapPermit>,
    mut why_blocked: ResMut<LoadoutSwapDenial>,
) {
    // Each refusal is the line shown; none means he may change class.
    let refusal = if let Some(line) = signon_refusal(
        signon_state.as_deref(),
        "class change not allowed (signon not Active)",
    ) {
        Some(line)
    } else if let Some(me) = presented
        .snapshot()
        .and_then(|view| view.meta.for_client(local.0))
    {
        stage_refusal(me.lifecycle)
    } else if overlay_open.0 {
        None
    } else {
        Some(String::from("class change not allowed (no match loaded)"))
    };

    set_if_changed(&mut may_change.0, refusal.is_none());
    set_if_changed(&mut why_blocked.0, refusal);
}

/// Writes `fresh` only when it differs, so change detection stays quiet.
fn set_if_changed<T: PartialEq>(slot: &mut T, fresh: T) {
    if *slot != fresh {
        *slot = fresh;
    }
}

/// The host's answer to an equip request, as far as this transaction cares.
enum EquipAnswer {
    Rejected(String),
    Accepted,
}

/// Settles the awaited request when the host's reliable channel answers it.
pub fn settle_loadout_arm_deal(
    mut control: MessageReader<net::GuaranteedSimEvent>,
    mut inflight: ResMut<LoadoutArmTracker>,
    mut store: ResMut<MatchLoadoutVault>,
    mut phase: ResMut<LoadoutPickerStage>,
    mut menu_open: ResMut<LoadoutPickerShown>,
    mut words: ResMut<LoadoutPickerReport>,
) {
    let awaited = match inflight.awaited_id.or_else(|| phase.queued_ask_number()) {
        Some(awaited) => awaited,
        None => {
            control.clear();
            return;
        }
    };

    let answer = control.read().find_map(|event| match event.0 {
        MatchOccurrence::LoadoutTaken { request_id, .. } if request_id == awaited => {
            Some(EquipAnswer::Accepted)
        }
        MatchOccurrence::LoadoutRefused {
            request_id, reason, ..
        } if request_id == awaited => Some(EquipAnswer::Rejected(reason.as_str().to_owned())),
        _ => None,
    });
    let Some(answer) = answer else {
        return;
    };
    inflight.awaited_id = None;
    if let EquipAnswer::Rejected(why) = &answer {
        if refuse_loadout_arm(&mut phase, &mut words, awaited, why.as_str()) {
            diag::info!(
                Ui,
                "class select: host said no to ask {awaited}: {why}"
            );
        }
    } else if grant_loadout_arm(&mut store, &mut phase, &mut menu_open, awaited) {
        diag::info!(Ui, "class select: host said yes to ask {awaited}");
    }
}

pub(crate) fn install_loadout_arm_steps(app: &mut App) {
    app.init_resource::<frame::KitPickerPassover>();
    app.init_resource::<LoadoutArmTracker>();
    let equip_system = enact_queued_loadout_arm;
    let swap_applied = frame::MatchSwapDone;
    let resolved = frame::KitEquipDone;
    let settle = settle_loadout_arm_deal;
    let intake = (take_class_offers, show_signon_progress).chain();
    let verdict_then_equip = (refresh_loadout_swap_permit, equip_system).chain();
    app.add_systems(
        Update,
        (intake, verdict_then_equip)
            .chain()
            .in_set(ClientStep::Present),
    );
    app.add_systems(
        Update,
        forget_equip_state.after(swap_applied).before(equip_system),
    );
    app.add_systems(
        Update,
        settle.after(equip_system).in_set(resolved),
    );
}

/// Takes the host's class offers into the catalog, building the catalog from
/// the prepared weapons first if it has no registry yet.
fn take_class_offers(
    prepared: Option<Res<assets::StagedWeapons>>,
    mut offers: ResMut<frame::KitPickerPassover>,
    mut loadouts: ResMut<crate::LoadoutRoster>,
    mut store: ResMut<MatchLoadoutVault>,
) {
    if !offers.pending {
        return;
    }
    if loadouts.resolver.0.is_none() {
        let Some(prepared) = prepared else {
            return;
        };
        *loadouts = crate::LoadoutRoster::from_weapon_registry(prepared.0.clone());
    }

    // The whole offer is taken, which also leaves it no longer pending.
    let incoming = std::mem::take(&mut *offers);
    store.enforce_cover_pins(incoming.locked_because);
    (loadouts.primary, loadouts.secondary) = (incoming.primary, incoming.secondary);
    (loadouts.lethal, loadouts.tactical) = (incoming.lethal, incoming.tactical);
    loadouts.left_out = incoming.excluded;
    let next_revision = loadouts.swap_count.wrapping_add(1);
    loadouts.swap_count = next_revision;
}

/// True for the signon progress lines that stop meaning anything once signon
/// has moved on.
fn is_waiting_line(shown: &str) -> bool {
    ["waiting for host", "handshake rejected"]
        .iter()
        .any(|prefix| shown.starts_with(prefix))
}

/// Shows the signon progress line in the class menu while no equip request is
/// in flight, and clears it again once signon has moved past the waiting
/// states.
fn show_signon_progress(
    signon_state: Option<Res<JoinProgressRecord>>,
    menu_phase: Res<LoadoutPickerStage>,
    mut status: ResMut<LoadoutPickerReport>,
) {
    if menu_phase.queued_ask_number().is_some() {
        return;
    }
    let Some(signon_state) = signon_state else {
        return;
    };
    let line = signon_state.phase.summary_banner();
    if status.0 == line {
        return;
    }
    let stale = line.is_none() && status.0.as_deref().is_some_and(is_waiting_line);
    if line.is_some() || stale {
        status.0 = line;
    }
}

/// Forgets everything about the old match once it has been torn down.
fn forget_equip_state(
    mut ended: MessageReader<frame::GameSessionClosed>,
    mut queued: ResMut<QueuedLoadoutArm>,
    mut store: ResMut<MatchLoadoutVault>,
    mut inflight: ResMut<LoadoutArmTracker>,
    mut phase: ResMut<LoadoutPickerStage>,
    mut status: ResMut<LoadoutPickerReport>,
) {
    if ended.is_empty() {
        return;
    }
    ended.clear();
    store.equipped = None;
    queued.0 = None;
    *status = Default::default();
    *phase = Default::default();
    *inflight = LoadoutArmTracker::default();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_live_lifecycles_may_change_class() {
        for lifecycle in [
            PlayerSessionStage::Alive,
            PlayerSessionStage::Dead,
            PlayerSessionStage::PickingLoadout,
            PlayerSessionStage::Spectating,
            PlayerSessionStage::Connecting,
        ] {
            assert_eq!(stage_refusal(lifecycle), None);
        }
        let line = stage_refusal(PlayerSessionStage::Intermission);
        assert!(line.is_some_and(|line| line.contains("Intermission")));
    }

    #[test]
    fn only_the_wait_lines_go_stale() {
        assert!(is_waiting_line("waiting for host to answer"));
        assert!(is_waiting_line("handshake rejected: full"));
        assert!(!is_waiting_line("Class is unavailable"));
    }

    #[test]
    fn no_signon_never_refuses() {
        assert_eq!(signon_refusal(None, "fallback"), None);
    }
}
