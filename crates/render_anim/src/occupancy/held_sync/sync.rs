//! The per-frame decision: does the first-person view match the held weapon?

use bevy::prelude::*;
use frame::{LaunchReport, LifeEnded, LifeStarted, ViewSubject};
use net::{LocalPresentClient, PresentedSnapshot};
use playerstate_t6::game::{PlayerState, get_viewmodel_weapon_index};

use super::slots::FpvHeldSlots;
use crate::anim::fpv_prepared::{FpvWeaponTable, PreparedFpv};
use crate::gaps::{RenderGap, RenderGapCause, RenderPresentationGaps};
use crate::occupancy::fpv_present::{
    FpvState, PendingFpvSpawnRequest, fpv_viewmodel_weapon,
};

/// Make the first-person view follow the weapon the local player holds.
///
/// In the order it is checked:
/// 1. A life ending for the local player (without a new one starting in the
///    same frame) clears the view and forgets the life; a life starting
///    records the new life number.
/// 2. During the intermission, unless this is a killcam, the view is cleared
///    and nothing more is done.
/// 3. With no player to show a view weapon for, the view is cleared and the
///    life forgotten.
/// 4. A fresh life drops whatever was left from the last one.
/// 5. An empty hand settles on "weapon 0": the view stays cleared.
/// 6. If the held weapon is already drawn, queued, or deliberately left as
///    is, nothing changes.
/// 7. Otherwise the weapon table decides: a known gun is queued for drawing;
///    an unknown one is recorded as unresolved; no table yet means wait.
pub fn sync_fpv_to_held_weapon(
    mut slots: FpvHeldSlots,
    presented: Res<PresentedSnapshot>,
    local: Res<LocalPresentClient>,
    view: Res<ViewSubject>,
    prepared: Res<PreparedFpv>,
    gaps: Res<RenderPresentationGaps>,
    mut report: Option<ResMut<LaunchReport>>,
    mut started: MessageReader<LifeStarted>,
    mut ended: MessageReader<LifeEnded>,
) {
    let me = local.0.0;

    // Read both queues to the end every frame, whatever happens below.
    let life_ended = ended.read().filter(|event| event.client == me).count() > 0;
    let new_life = started
        .read()
        .filter(|event| event.client == me)
        .map(|event| event.life)
        .last();
    if let Some(life) = new_life {
        slots.life.0 = Some(life);
    }
    if life_ended && new_life.is_none() {
        slots.retire_not_alive();
        slots.life.0 = None;
    }

    let between_rounds = presented
        .snapshot()
        .is_some_and(|snapshot| snapshot.meta.phase == sim::MatchPhase::Intermission);
    if between_rounds && !view.in_killcam() {
        slots.retire_not_alive();
        return;
    }

    let Some(ps) = presented.viewweapon_player(local.0) else {
        slots.retire_not_alive();
        slots.life.0 = None;
        return;
    };
    if new_life.is_some() {
        slots.wipe_if_occupied();
    }

    let table = prepared.table();
    let held = match table {
        Some(table) => fpv_viewmodel_weapon(ps, table),
        None => get_viewmodel_weapon_index(ps),
    };

    if held == 0 {
        settle_empty_hand(&mut slots, &mut report);
        return;
    }
    if is_up_to_date(&mut slots, held, ps) {
        return;
    }

    let Some(table) = table else {
        // The weapon catalog has not been built yet; try again next frame.
        let waiting = FpvState::Blocked(RenderGapCause::FpvCatalogMissing);
        if slots.status.0.as_ref() != Some(&waiting) {
            slots.status.0 = Some(waiting);
        }
        return;
    };

    let outcome = request_weapon(&mut slots, table, held, ps);

    match outcome.cause() {
        Some(cause) => gaps.raise(cause.clone()),
        None => gaps.clear(RenderGap::FpvViewmodel),
    }
    diag::info!(Fpv, "fpv held weapon {held}: {outcome}");
    post_to_report(&mut report, &outcome);
    slots.status.0 = Some(outcome);
}

/// The hand is empty: show nothing, and remember that was decided so it is not
/// redone every frame.
fn settle_empty_hand(slots: &mut FpvHeldSlots, report: &mut Option<ResMut<LaunchReport>>) {
    let already_settled = slots.session.0.is_none()
        && slots.pending.0.is_none()
        && slots.settled.0 == Some(0);
    if already_settled {
        return;
    }
    slots.wipe();
    slots.settled.0 = Some(0);
    let outcome = FpvState::ClearedNoWeapon;
    post_to_report(report, &outcome);
    slots.status.0 = Some(outcome);
}

/// True when the held weapon needs no new request: it is already queued, already
/// drawn, or was decided on earlier and nothing is live.
fn is_up_to_date(slots: &mut FpvHeldSlots, held: u32, ps: &PlayerState) -> bool {
    let parent = ps.weapon_primary;

    let queued = slots
        .pending
        .0
        .as_ref()
        .is_some_and(|request| request.weapon_id == held && request.parent_weapon == parent);
    if queued {
        return true;
    }

    let drawn = slots
        .session
        .0
        .as_ref()
        .is_some_and(|drawn| drawn.weapon_id == held && drawn.parent_weapon == parent);
    if drawn {
        slots.settled.0 = Some(held);
        return true;
    }

    slots.settled.0 == Some(held) && slots.session.0.is_none() && slots.pending.0.is_none()
}

/// Replace the view with one for `held`, using the weapon table. Returns the
/// state to report: queued for drawing, or blocked because the weapon has no
/// gun model.
fn request_weapon(
    slots: &mut FpvHeldSlots,
    table: &FpvWeaponTable,
    held: u32,
    ps: &PlayerState,
) -> FpvState {
    slots.despawn_roots();
    slots.session.0 = None;

    match table.gun_index(held) {
        Some(gun_index) => {
            slots.pending.0 = Some(PendingFpvSpawnRequest {
                gun_index,
                catalog_id: table.catalog_id(),
                weapon_id: held,
                parent_weapon: ps.weapon_primary,
            });
            slots.settled.0 = None;
            FpvState::Queued
        }
        None => {
            slots.pending.0 = None;
            slots.settled.0 = Some(held);
            FpvState::Blocked(RenderGapCause::FpvGunXModelUnresolved { weapon_id: held })
        }
    }
}

/// Copy a state's label into the launch report, when there is one.
fn post_to_report(report: &mut Option<ResMut<LaunchReport>>, state: &FpvState) {
    if let Some(report) = report.as_mut() {
        report.sim_gap = state.label();
    }
}
