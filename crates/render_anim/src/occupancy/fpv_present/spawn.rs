//! Turning a queued request into a spawned viewmodel: look the weapon up in the
//! prepared table, parent a placement root under the camera and record what
//! the animation side needs.

use std::sync::Arc;

use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use net::{LocalPresentClient, PresentedSnapshot};
use render_scene::FlyCamera;

use super::session::{FpvState, FpvStatusGap, SessionFpvMeshesHandles, SessionViewmodel, same_material_catalog};
use super::{FpvPlacementRoot, FpvViewmodel};
use crate::anim::fpv::EquippedFpv;
use crate::anim::fpv_host::{FpvPresentCursor, PendingFpvSpawn, PendingFpvSpawnRequest};
use crate::anim::fpv_prepared::{FpvWeaponSlot, FpvWeaponView, PreparedFpv};
use crate::anim::viewmodel_controller::ViewmodelController;
use crate::gaps::{RenderGap, RenderGapCause, RenderPresentationGaps};

/// The read-only inputs of [`spawn_pending_fpv`], bundled to keep its
/// parameter list inside Bevy's limit.
#[derive(SystemParam)]
pub struct SpawnPendingFpvInputs<'w> {
    prepared: Res<'w, PreparedFpv>,
    tess: Res<'w, render_scene::TessMaterials>,
    presented: Res<'w, PresentedSnapshot>,
    local: Res<'w, LocalPresentClient>,
}

/// Records a failed spawn: raises the gap and mirrors it into the status.
fn block(gaps: &RenderPresentationGaps, status: &mut FpvStatusGap, cause: RenderGapCause) {
    gaps.raise(cause.clone());
    status.0 = Some(FpvState::Blocked(cause));
}

/// Whether the local player's team puts them in the Axis hands variant.
fn local_is_axis(presented: &PresentedSnapshot, local: &LocalPresentClient) -> bool {
    let meta = presented
        .snapshot()
        .and_then(|snap| snap.meta.for_client(local.0));
    let ffa_team = meta.and_then(|m| m.ffa_team);
    let team = meta.map_or(0, |m| m.client_state_team);
    asset_model::kit_assignment_is_axis(team, ffa_team)
}

/// Copies the draw-plan hints a prepared view carries into the shared plan.
fn seed_draw_plan(plan: &mut crate::FpvDrawPlan, view: &FpvWeaponView) {
    crate::clear_fpv_draw_plan(plan, 0);
    let census = &view.census;
    let joined = |names: &[String]| (!names.is_empty()).then(|| names.join(","));
    plan.gun_colormap_skip_n = Some(census.gun_colormap_skip_n);
    plan.gun_ordinal_skip_n = Some(0);
    plan.gun_colormap_skip_names = joined(&census.gun_colormap_skip_names);
    plan.plan_mat_hints = joined(&census.mat_hints);
}

/// Spawns the viewmodel a [`PendingFpvSpawn`] asks for, once the prepared table
/// is ready and built against the live material catalog. Requests that cannot
/// be served yet are put back; requests that can never be served become a gap.
pub fn spawn_pending_fpv(
    mut commands: Commands,
    mut pending: ResMut<PendingFpvSpawn>,
    inputs: SpawnPendingFpvInputs,
    mut session_vm: ResMut<SessionViewmodel>,
    mut cursor: ResMut<FpvPresentCursor>,
    mut fpv_plan: ResMut<crate::FpvDrawPlan>,
    cameras: Query<Entity, With<FlyCamera>>,
    existing_fpv: Query<Entity, With<FpvPlacementRoot>>,
    mut status: ResMut<FpvStatusGap>,
    gaps: Res<RenderPresentationGaps>,
) {
    let SpawnPendingFpvInputs {
        prepared,
        tess,
        presented,
        local,
    } = inputs;
    let Some(request) = pending.0.take() else {
        return;
    };
    // Wait (keeping the request) until a table exists for this catalog.
    let Some(table) = prepared
        .table()
        .filter(|t| same_material_catalog(t.material_catalog(), Some(&*tess)))
    else {
        pending.0 = Some(request);
        return;
    };
    let started = std::time::Instant::now();
    cursor.0.forget_weap_anim();
    existing_fpv
        .iter()
        .for_each(|root| commands.entity(root).try_despawn());
    session_vm.0 = None;

    let unresolved = || RenderGapCause::FpvGunXModelUnresolved {
        weapon_id: request.weapon_id,
    };
    let table_matches = table.catalog_id() == request.catalog_id
        && table.gun_index(request.weapon_id) == Some(request.gun_index);
    if !table_matches {
        block(&gaps, &mut status, unresolved());
        return;
    }
    let axis = local_is_axis(&presented, &local);
    let view = match table.slot(request.weapon_id, request.parent_weapon, axis) {
        FpvWeaponSlot::Ready(view) => Arc::clone(view),
        FpvWeaponSlot::Refused(cause) => {
            block(&gaps, &mut status, cause.clone());
            return;
        }
        FpvWeaponSlot::Absent => {
            block(&gaps, &mut status, unresolved());
            return;
        }
    };
    let Ok(camera) = cameras.single() else {
        diag::info!(
            Fpv,
            "fpv: no FlyCamera to parent viewmodel under — re-queue"
        );
        pending.0 = Some(request);
        block(&gaps, &mut status, RenderGapCause::FpvNoCamera);
        return;
    };

    commands.entity(camera).with_children(|children| {
        children.spawn((
            FpvViewmodel,
            FpvPlacementRoot,
            Transform::IDENTITY,
            Visibility::Visible,
        ));
    });
    seed_draw_plan(&mut fpv_plan, &view);

    let right = ViewmodelController::new(view.right.clone());
    let left = view.left.clone().map(ViewmodelController::new);
    let left_note = view.left.as_ref().map_or_else(String::new, |l| {
        format!(" R, {} clips L (dual DObj)", l.resolved_count())
    });
    diag::info!(
        Fpv,
        "fpv: viewmodel controller `{}` — {} clips{}, fireTime={}ms raiseTime={}ms (action from weapAnim)",
        view.right.name,
        view.right.resolved_count(),
        left_note,
        view.right.fire_time_ms,
        view.right.raise_time_ms,
    );
    let idle_sampled = view.idle_name.is_some();
    let idle_note = view.idle_name.as_deref().map_or_else(
        || "no szXAnims[IDLE] — guess forbidden".to_owned(),
        |name| format!("idle `{name}` (szXAnims[IDLE])"),
    );
    diag::info!(
        Fpv,
        "fpv: equipped `{}` ({}; prepared before Ready) — instance {:.2}ms",
        view.gun_name,
        idle_note,
        started.elapsed().as_secs_f64() * 1000.0,
    );
    let mut equipped = EquippedFpv::new(
        view.gun_name.clone(),
        view.gun_index,
        view.hands_index,
        view.namespace,
        view.hands.clone(),
        right,
        left,
    );
    // The arms-and-gun skeleton lets a smoothed run loop swing the arms in turn
    // and keep a left-held knife in the left fist.
    equipped.view_skeleton = Some(crate::anim::t6_loop_seam::view_skeleton(
        &view.assemblies.bare.dobj,
    ));
    let PendingFpvSpawnRequest {
        weapon_id,
        parent_weapon,
        catalog_id,
        ..
    } = request;
    session_vm.0 = Some(SessionFpvMeshesHandles {
        weapon_id,
        parent_weapon,
        catalog_id,
        axis,
        fpv: equipped,
        view,
        active_rig: None,
        material_catalog: Arc::clone(table.material_catalog()),
    });

    gaps.clear(RenderGap::FpvViewmodel);
    status.0 = Some(FpvState::Drawn { idle_sampled });
}
