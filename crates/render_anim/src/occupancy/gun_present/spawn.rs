//! Turns a queued request into a spawned viewmodel: finds the weapon in the
//! prepared table, hangs a placement root off the camera, and records what the
//! animation side will need.

use std::sync::Arc;
use std::time::Instant;

use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use net::{HostViewClient, ShownWorldFrame};
use render_scene::{FreeRoamCamera, StitchMaterials};

use super::gun_status::{ViewmodelStatus, GunStatusShortfall, GunMeshHandleSet, ActiveGunMeshes};
use super::gun_status::same_material_catalog;
use super::{GunMountAnchor, GunViewLayer};
use crate::motion::fpv::HeldGunLoadout;
use super::super::super::motion::gun_host::{FpvShownCursor, WaitingFpvSpawn, QueuedGunSpawn};
use crate::motion::fpv_prepared::{FpvWeaponSlot, FpvWeaponTable, FpvWeaponView, PreparedFpv};
use crate::motion::gun_rig_ctl::FirstPersonGunAnimator;
use crate::holes::{PresentationShortfall, Cause, ShortfallLedger};

/// The inputs [`spawn_queued_first_person`] only reads. They are grouped so the system
/// stays within Bevy's parameter limit.
#[derive(SystemParam)]
pub struct GunSpawnSystemParams<'w> {
    table_source: Res<'w, PreparedFpv>,
    mat_list: Res<'w, StitchMaterials>,
    snapshot: Res<'w, ShownWorldFrame>,
    client: Res<'w, HostViewClient>,
}

/// Records a failed spawn: the cause goes into the status first, then onto the
/// gap ledger.
fn report_refusal(ledger: &ShortfallLedger, status: &mut GunStatusShortfall, cause: Cause) {
    let gun_state = ViewmodelStatus::Blocked(cause.clone());
    status.0 = Some(gun_state);
    ledger.open_gap(cause);
}

/// Whether the local player is on the team that wears the Axis hands.
fn client_on_axis(presented: &ShownWorldFrame, own_client: &HostViewClient) -> bool {
    let (ffa_team, team) = presented
        .snapshot()
        .and_then(|s| s.meta.for_client(own_client.0))
        .map(|m| (m.ffa_team, m.client_state_team)).unwrap_or((None, 0));
    asset_model::kit_belongs_to_axis(team, ffa_team)
}

/// Joins names with commas; an empty list gives nothing.
fn comma_list(names: &[String]) -> Option<String> {
    (!names.is_empty()).then(|| names.join(","))
}

/// Fills the shared draw plan with the hints that a prepared view carries.
fn seed_draw_plan(plan: &mut crate::FirstPersonPaintPlan, view: &FpvWeaponView) {
    crate::clear_fpv_draw_plan(plan, 0);
    let census = &view.census;
    plan.gun_index_skipped = Some(0);
    plan.gun_tint_skipped = Some(census.gun_tint_skipped);
    plan.gun_tint_skip_list = comma_list(&census.gun_tint_skip_list);
    plan.batch_material_notes = comma_list(&census.mat_hints);
}

/// What the log says about the left hand's clips, if the view has a left hand.
fn left_hand_note(view: &FpvWeaponView) -> String {
    view.left.as_ref().map_or_else(String::new, |left| {
        format!(" R, {} clips L (dual DObj)", left.linked_total())
    })
}

/// Picks the prepared view for a request, or says why there is none.
fn pick_view(
    table: &FpvWeaponTable,
    request: &QueuedGunSpawn,
    along: bool,
) -> Result<Arc<FpvWeaponView>, Cause> {
    let unresolved = Cause::FpvGunXModelUnresolved {
        gun_catalog_no: request.gun_catalog_no,
    };
    if table.catalog_id() != request.catalog_id
        || table.gun_index(request.gun_catalog_no) != Some(request.gun_index)
    {
        return Err(unresolved);
    }
    match table.gun_slot(request.gun_catalog_no, request.parent_weapon, along) {
        FpvWeaponSlot::Ready(view) => Ok(Arc::clone(view)),
        FpvWeaponSlot::Declined(cause) => Err(cause.clone()),
        FpvWeaponSlot::NothingThere => Err(unresolved),
    }
}

/// Spawns the viewmodel that a [`PendingFpvSpawn`] asks for, once the prepared
/// table is ready and was built for the live material catalog. A request that
/// cannot be served yet goes back in the queue, and one that never can be
/// served turns into a gap.
pub fn spawn_viewmodel(
    mut cmds: Commands,
    inputs: GunSpawnSystemParams,
    cameras_in: Query<Entity, With<FreeRoamCamera>>,
    existing_fpv_in: Query<Entity, With<GunMountAnchor>>,
    presentation_gaps: Res<ShortfallLedger>,
    mut queue: ResMut<WaitingFpvSpawn>,
    mut draw_plan: ResMut<crate::FirstPersonPaintPlan>,
    mut outcome: ResMut<GunStatusShortfall>,
    mut anim_cursor: ResMut<FpvShownCursor>,
    mut kept: ResMut<ActiveGunMeshes>,
) {
    let request_in = match queue.0.take() {
        Some(req) => req,
        None => return,
    };
    // Keep the request until a table exists for this catalog.
    let table = inputs
        .table_source
        .table()
        .filter(|t| same_material_catalog(t.material_catalog(), Some(&*inputs.mat_list)));
    let Some(table) = table else {
        queue.0 = Some(request_in);
        return;
    };
    let t0 = Instant::now();
    anim_cursor.0.drop_gun_anim_memory();
    existing_fpv_in
        .iter()
        .for_each(|root| cmds.entity(root).try_despawn());
    kept.0 = None;

    let axis = client_on_axis(&inputs.snapshot, &inputs.client);
    let view = match pick_view(table, &request_in, axis) {
        Ok(view) => view,
        Err(cause) => return report_refusal(&presentation_gaps, &mut outcome, cause),
    };
    let camera = match cameras_in.single() {
        Ok(camera) => camera,
        Err(_) => {
            diag::info!(ViewModelPerspective, "fpv: the viewmodel has no FlyCamera to hang under, queued again");
            queue.0 = Some(request_in);
            report_refusal(&presentation_gaps, &mut outcome, Cause::GunNoViewer);
            return;
        }
    };

    let placement = cmds
        .spawn((
            Visibility::Visible,
            Transform::IDENTITY,
            GunMountAnchor,
            GunViewLayer,
        ))
        .id();
    cmds.entity(camera).add_child(placement);
    seed_draw_plan(&mut draw_plan, &view);

    let controllers = (
        FirstPersonGunAnimator::new(view.right.clone()),
        view.left.clone().map(FirstPersonGunAnimator::new),
    );
    log_controller(&view);
    let rest_pose_taken = view.idle_name.is_some();
    log_equipped(&view, t0);
    let (right, left) = controllers;
    let mut equipped = HeldGunLoadout::new(
        view.gun_name.clone(),
        view.gun_index,
        view.hands_index,
        view.namespace,
        view.hands.clone(),
        right,
        left,
    );
    // With the arms-and-gun skeleton, a smoothed run loop can swing the arms in
    // turn and a knife held in the left hand stays in the left fist.
    equipped.view_skeleton = Some(crate::motion::t6_loop_seam::view_skeleton(
        &view.assemblies.bare.joint_tree,
    ));

    let QueuedGunSpawn {
        gun_catalog_no,
        parent_weapon,
        catalog_id,
        ..
    } = request_in;
    let handles = GunMeshHandleSet {
        material_catalog: Arc::clone(table.material_catalog()),
        active_rig: None,
        axes: axis,
        catalog_id,
        parent_weapon,
        gun_catalog_no,
        view,
        fpv: equipped,
    };
    kept.0 = Some(handles);

    presentation_gaps.close_gap(PresentationShortfall::GunViewLayer);
    outcome.0 = Some(ViewmodelStatus::Drawn { rest_pose_taken });
}

/// Logs the controller that was built for a view.
fn log_controller(view: &FpvWeaponView) {
    diag::info!(
        ViewModelPerspective,
        "fpv: controller for `{}` has {} clips{}; fireTime={}ms, holdFireTime={}ms, raiseTime={}ms (the action comes from weapAnim); pull-back clip {:?}s",
        view.right.name,
        view.right.linked_total(),
        left_hand_note(view),
        view.right.fire_time_ms,
        view.right.trigger_hold_millis,
        view.right.raise_time_ms,
        view.right.reel(asset_game::GunMoveRole::HoldFire).map(|c| c.duration()),
    );
}

/// Logs that a gun is equipped, with how its idle was found and how long the spawn took.
fn log_equipped(view: &FpvWeaponView, started: Instant) {
    let idle_note = view.idle_name.as_deref().map_or_else(
        || "no szXAnims[IDLE], and guessing is forbidden".to_owned(),
        |name| format!("idle `{name}` (szXAnims[IDLE])"),
    );
    diag::info!(
        ViewModelPerspective,
        "fpv: `{}` equipped ({}; prepared before it was Ready), built in {:.2}ms",
        view.gun_name,
        idle_note,
        started.elapsed().as_secs_f64() * 1000.0,
    );
}

mod tests {
    #![cfg(test)]
    use super::*;

    #[test]
    fn comma_list_is_none_when_empty() {
        assert_eq!(comma_list(&[]), None);
        let names = ["a".to_owned(), "b".to_owned()];
        assert_eq!(comma_list(&names).as_deref(), Some("a,b"));
    }

    #[test]
    fn refusal_lands_in_status_and_ledger() {
        let ledger = ShortfallLedger::default();
        let mut status_in = GunStatusShortfall::default();
        report_refusal(&ledger, &mut status_in, Cause::GunNoViewer);
        assert_eq!(status_in.0, Some(ViewmodelStatus::Blocked(Cause::GunNoViewer)));
        assert!(ledger.still_counting(PresentationShortfall::GunViewLayer));
    }
}
