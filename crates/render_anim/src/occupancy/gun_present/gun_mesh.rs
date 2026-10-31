//! What happens once the viewmodel pose is known: its vertices are skinned,
//! its place in the world is stamped, and the bone poses that effects attach
//! to are published.

use assets::ReadiedFpvMeshes;
use bevy::prelude::*;
use render_scene::{FreeRoamCamera, FirstPersonLens, LevelScriptModelInstance};

use super::GunMountAnchor;
use super::gun_status::{ViewmodelStatus, GunStatusShortfall, ActiveGunMeshes, same_material_catalog};
use crate::motion::puppet_pose::HostSkeletonFrame;
use crate::motion::gun_host::{AttachTagGoals, ViewmodelPoseVariety, GunPoseOutput, GunPoseDecline};
use crate::holes::Cause;
use crate::holes::{PresentationShortfall, ShortfallLedger};
use crate::occupancy::peer_body::OtherPlayerBody;

/// Turns a refused pose into the gap that gets reported for it.
pub(super) fn decline_to_reason(refuse_in: GunPoseDecline) -> Cause {
    match refuse_in {
        GunPoseDecline::TableAbsent => Cause::GunTableAbsent,
        GunPoseDecline::NothingPlaying => Cause::GunNoPlayingClips,
        GunPoseDecline::EyePoseFailed { weapon_mesh } => Cause::FpvEyePoseFailed { weapon_mesh },
        GunPoseDecline::DependencyUnresolved {
            gun_catalog_no,
            purpose: role,
            name,
        } => Cause::FpvDependencyUnresolved {
            gun_catalog_no,
            part_role: role,
            name,
        },
    }
}

/// Empties the viewmodel's draw plan but keeps its lighting handle.
fn blank_plan(plan: &mut crate::FirstPersonPaintPlan) {
    let lighting = plan.illum_handle;
    crate::clear_fpv_draw_plan(plan, lighting);
}

/// Skins this frame's vertices into the buffer the rig published, and nothing
/// more. Indices, surface ranges, materials and draws belong to the composition
/// and are only installed when the rig is replaced.
pub(super) fn skin_geometry(
    mut lenses: Query<&mut Transform, (Without<LevelScriptModelInstance>, Without<OtherPlayerBody>, With<FirstPersonLens>)>,
    gaps: Res<ShortfallLedger>,
    first_person_meshes: Option<Res<ReadiedFpvMeshes>>,
    mut status_gap: ResMut<GunStatusShortfall>,
    product: Res<GunPoseOutput>,
    tess: Option<Res<render_scene::StitchMaterials>>,
    mut plan: ResMut<crate::FirstPersonPaintPlan>,
    session_vm: Option<Res<ActiveGunMeshes>>,
) {
    plan.viewmodel_pass_flag = product.viewmodel_pass_flag;
    let lighting = plan.illum_handle;
    let ViewmodelPoseVariety::Posed(frame) = &product.kind else {
        blank_plan(&mut plan);
        if let ViewmodelPoseVariety::Refuse(refuse) = &product.kind {
            let cause_in = decline_to_reason(refuse.clone());
            gaps.open_gap(cause_in.clone());
            status_gap.0 = Some(ViewmodelStatus::Blocked(cause_in));
        }
        return;
    };

    // From here on a live session is needed, on the current catalog and with
    // a rig.
    let live = session_vm
        .as_ref()
        .and_then(|held| held.0.as_ref())
        .filter(|session| same_material_catalog(&session.material_catalog, tess.as_deref()));
    let (Some(session), Some(meshes)) = (live, first_person_meshes.as_ref()) else {
        blank_plan(&mut plan);
        return;
    };
    let Some(rig) = session
        .active_rig
        .as_ref()
        .filter(|_| session.catalog_id == meshes.0.identity())
    else {
        blank_plan(&mut plan);
        return;
    };

    let generation = rig.generation();
    if plan.rig_generation != generation {
        crate::install_prepared_fpv_plan(&mut plan, &rig.geometry, lighting);
        plan.rig_generation = generation;
    }
    if let Some(rows) = plan.packed_rows_mut() {
        rig.skin_into(&meshes.0, &frame.poses, rows);
    }
    plan.revisions.advance_vertexdata_stamp();
    // A guest weapon draws itself and hides whatever stands in for it.
    plan.geometry_ok = !plan.draws().is_empty() && !crate::guest_weapons::hides_viewmodel();
    plan.settle_visible();

    gaps.close_gap(PresentationShortfall::GunViewLayer);
    let rest_pose_taken = frame.rest_pose_taken;
    status_gap.0 = Some(ViewmodelStatus::Drawn { rest_pose_taken });
    let lens = Transform::from_matrix(frame.optic);
    for mut place in &mut lenses {
        *place = lens;
    }
}

/// Works out where the viewmodel sits this frame: the camera's transform
/// composed with the placement root's. Neither the rig nor its output is
/// read, so the geometry need not be ready.
pub fn stamp_matrix(
    mut plan: ResMut<crate::FirstPersonPaintPlan>,
    eye: Query<&Transform, (With<FreeRoamCamera>, Without<GunMountAnchor>)>,
    root: Query<&Transform, With<GunMountAnchor>>,
) {
    if let (Ok(eye), Ok(root)) = (eye.single(), root.single()) {
        plan.model_to_world = eye.to_matrix() * root.to_matrix();
        plan.placement_ok = true;
    } else {
        plan.placement_ok = false;
    }
    plan.settle_visible();
}

/// The effect attachment for `bone` of the published `dobj`, or nothing when
/// there is no bone or it cannot be resolved.
fn bolt_target(store: &HostSkeletonFrame, dobj: u32, bone: Option<u16>) -> Option<fx::EffectBoltTarget> {
    let joint = bone?;
    store
        .find(dobj, i32::from(joint))
        .ok()
        .map(|orient| fx::EffectBoltTarget {
            bone: joint,
            dobj,
            orientation: orient,
            client_actor_teleport: false,
        })
}

/// Publishes this frame's viewmodel bone poses to the shared dobj pose store,
/// then resolves the effect attachment points (muzzle flash, brass ejection,
/// laser and the rest) against them.
pub fn publish_dobj_pose(
    plan: Res<crate::FirstPersonPaintPlan>,
    placement_root: Query<(), With<GunMountAnchor>>,
    mut targets: ResMut<AttachTagGoals>,
    mut store: ResMut<HostSkeletonFrame>,
) {
    if placement_root.single().is_err() {
        targets.clear();
        return;
    }
    (targets.tracker_screen, targets.tracker_light, targets.hud) = (None, None, None);
    (targets.wrist, targets.weapon) = ([None, None], None);
    let scene_from_local = plan.model_to_world;
    for hand in [0usize, 1] {
        let Some(frame) = targets.pose[hand].take() else {
            continue;
        };
        let dobj = u32::try_from(hand).unwrap_or_default() + fx_t6::runtime::ATTACH_HELD_GUN_SKELETON_START;
        if store.publish(dobj, true, 0, scene_from_local, &frame.joints).is_err() {
            continue;
        }
        let at = |tag: Option<u16>| bolt_target(&store, dobj, tag);
        let tags = &frame.tags;

        // Only the right hand's gun carries the tracker and HUD points.
        if hand == 0 && plan.placement_ok {
            targets.tracker_light = at(tags.tracker_light);
            targets.hud = at(tags.hud);
            // bo2zm: the arms' wrists and the gun's tag (Mob's Afterlife hands and ghost fx).
            targets.wrist = tags.wrist.map(at);
            targets.weapon = at(tags.weapon);
            let corner = |tag: Option<u16>| {
                let at_rest = frame.joints.get(usize::from(tag?))?.w_axis.truncate();
                Some(scene_from_local.transform_point3(at_rest))
            };
            let [first, second, third] = tags.tracker_screen.map(corner);
            targets.tracker_screen = first.zip(second).zip(third).map(|((a, b), c)| [a, b, c]);
        }
        targets.flash[hand] = at(tags.flash_tag);
        targets.brass[hand] = at(tags.brass_tag);
        targets.knife[hand] = at(tags.knife_tag);
        targets.laser[hand] = at(tags.laser_tag);
        targets.target[hand] = at(tags.target);
        // With dual wield on a single set of arms, the left gun's flash and
        // brass tags are on the right hand's skeleton.
        if hand == 0 && tags.flash_left.is_some() {
            targets.flash[1] = at(tags.flash_left);
            targets.brass[1] = at(tags.brass_left);
        }
    }
}
