//! After the pose is known: skin the rig's vertices, stamp where the viewmodel
//! sits in the world, and publish the bone poses that effects attach to.

use assets::PreparedFpvMeshes;
use bevy::prelude::*;
use render_scene::{FlyCamera, FpvLens, WorldScriptModelInstance};

use super::session::{FpvState, FpvStatusGap, SessionViewmodel, same_material_catalog};
use super::FpvPlacementRoot;
use crate::anim::fpv_host::{FpvBoltTargets, FpvPoseKind, FpvPoseProduct, FpvPoseRefuse};
use crate::gaps::{RenderGap, RenderGapCause, RenderPresentationGaps};
use crate::occupancy::remote_body::RemotePlayer;

/// The gap a refused pose is reported as.
pub(super) fn refuse_gap_cause(refuse: FpvPoseRefuse) -> RenderGapCause {
    use FpvPoseRefuse::*;
    match refuse {
        CatalogMissing => RenderGapCause::FpvCatalogMissing,
        NoActiveClips => RenderGapCause::FpvNoActiveClips,
        EyePoseFailed { gun_xmodel } => RenderGapCause::FpvEyePoseFailed { gun_xmodel },
        DependencyUnresolved {
            weapon_id,
            role,
            name,
        } => RenderGapCause::FpvDependencyUnresolved {
            weapon_id,
            role,
            name,
        },
    }
}

/// Writes this frame's skinned vertices into the buffer the rig published. It
/// touches nothing else: indices, surface ranges, materials and draws belong to
/// the composition and are installed only when the rig changes.
pub(super) fn skin_fpv_geometry(
    product: Res<FpvPoseProduct>,
    session_vm: Option<Res<SessionViewmodel>>,
    tess: Option<Res<render_scene::TessMaterials>>,
    fpv_meshes: Option<Res<PreparedFpvMeshes>>,
    mut fpv_plan: ResMut<crate::FpvDrawPlan>,
    mut status: ResMut<FpvStatusGap>,
    gaps: Res<RenderPresentationGaps>,
    mut lenses: Query<
        &mut Transform,
        (
            With<FpvLens>,
            Without<RemotePlayer>,
            Without<WorldScriptModelInstance>,
        ),
    >,
) {
    fpv_plan.drawgun = product.drawgun;
    let lighting = fpv_plan.lighting_handle;
    let frame = match &product.kind {
        FpvPoseKind::Hide => {
            crate::clear_fpv_draw_plan(&mut fpv_plan, lighting);
            return;
        }
        FpvPoseKind::Refuse(refuse) => {
            let cause = refuse_gap_cause(refuse.clone());
            crate::clear_fpv_draw_plan(&mut fpv_plan, lighting);
            gaps.raise(cause.clone());
            status.0 = Some(FpvState::Blocked(cause));
            return;
        }
        FpvPoseKind::Posed(frame) => frame,
    };

    // Everything below needs a live session on the current catalog with a rig.
    let session = session_vm
        .as_ref()
        .and_then(|vm| vm.0.as_ref())
        .filter(|s| same_material_catalog(&s.material_catalog, tess.as_deref()));
    let (Some(session), Some(meshes)) = (session, fpv_meshes.as_ref()) else {
        crate::clear_fpv_draw_plan(&mut fpv_plan, lighting);
        return;
    };
    let rig = match &session.active_rig {
        Some(rig) if session.catalog_id == meshes.0.identity() => rig,
        _ => {
            crate::clear_fpv_draw_plan(&mut fpv_plan, lighting);
            return;
        }
    };

    if fpv_plan.rig_generation != rig.generation() {
        crate::install_prepared_fpv_plan(&mut fpv_plan, &rig.geometry, lighting);
        fpv_plan.rig_generation = rig.generation();
    }
    if let Some(rows) = fpv_plan.packed_rows_mut() {
        rig.skin_into(&meshes.0, &frame.poses, rows);
    }
    fpv_plan.revisions.bump_vertices();
    // A guest weapon draws itself, and whatever stands in under it is hidden.
    fpv_plan.geometry_ok =
        !crate::guest_weapons::hides_viewmodel() && !fpv_plan.draws().is_empty();
    fpv_plan.settle_visible();

    gaps.clear(RenderGap::FpvViewmodel);
    status.0 = Some(FpvState::Drawn {
        idle_sampled: frame.idle_sampled,
    });
    let lens = Transform::from_matrix(frame.lens);
    for mut transform in &mut lenses {
        *transform = lens;
    }
}

/// Where the viewmodel sits this frame: the camera's transform composed with
/// the placement root's. Only those two are read, never the rig's output, so
/// this does not have to wait for the geometry.
pub fn stamp_fpv_placement_matrix(
    mut fpv_plan: ResMut<crate::FpvDrawPlan>,
    cameras: Query<&Transform, (With<FlyCamera>, Without<FpvPlacementRoot>)>,
    roots: Query<&Transform, With<FpvPlacementRoot>>,
) {
    match (cameras.single(), roots.single()) {
        (Ok(camera), Ok(root)) => {
            fpv_plan.world_from_local = camera.to_matrix() * root.to_matrix();
            fpv_plan.placement_ok = true;
        }
        _ => fpv_plan.placement_ok = false,
    }
    fpv_plan.settle_visible();
}

/// Hands this frame's viewmodel bone poses to the shared dobj pose store and
/// resolves the effect attachment points (muzzle flash, brass ejection, laser
/// and so on) against them.
pub fn publish_fpv_dobj_pose(
    fpv_plan: Res<crate::FpvDrawPlan>,
    roots: Query<(), With<FpvPlacementRoot>>,
    mut bolts: ResMut<FpvBoltTargets>,
    mut dobj_poses: ResMut<crate::anim::dobj_pose::HostDObjPoseFrame>,
) {
    if roots.single().is_err() {
        bolts.clear();
        return;
    }
    bolts.tracker_screen = None;
    bolts.tracker_light = None;
    bolts.hud = None;
    let world_from_local = fpv_plan.world_from_local;
    for hand in 0..2usize {
        let Some(frame) = bolts.pose[hand].take() else {
            continue;
        };
        let dobj = fx_t6::runtime::FX_BOLT_VIEWMODEL_DOBJ_BASE + hand as u32;
        if dobj_poses
            .publish(dobj, true, 0, world_from_local, &frame.bones)
            .is_err()
        {
            continue;
        }
        // An attachment for `bone`, or nothing when the bone does not resolve.
        let attach = |bone: Option<u16>| -> Option<fx::FxBoltTarget> {
            let bone = bone?;
            let orientation = dobj_poses.resolve(dobj, i32::from(bone)).ok()?;
            Some(fx::FxBoltTarget {
                dobj,
                bone,
                centity_teleport: false,
                orientation,
            })
        };

        // The right hand's gun carries the tracker and HUD points.
        if hand == 0 && fpv_plan.placement_ok {
            bolts.tracker_light = attach(frame.tags.tracker_light);
            bolts.hud = attach(frame.tags.hud);
            let corners = frame.tags.tracker_screen.map(|bone| {
                let bone = usize::from(bone?);
                let position = frame.bones.get(bone)?.w_axis.truncate();
                Some(world_from_local.transform_point3(position))
            });
            bolts.tracker_screen = match corners {
                [Some(a), Some(b), Some(c)] => Some([a, b, c]),
                _ => None,
            };
        }
        bolts.flash[hand] = attach(frame.tags.flash);
        bolts.brass[hand] = attach(frame.tags.brass);
        bolts.knife[hand] = attach(frame.tags.knife);
        bolts.laser[hand] = attach(frame.tags.laser);
        bolts.target[hand] = attach(frame.tags.target);
        // Dual wield on one set of arms: the left gun's flash and brass tags
        // sit on the right hand's skeleton.
        if hand == 0 && frame.tags.flash_left.is_some() {
            bolts.flash[1] = attach(frame.tags.flash_left);
            bolts.brass[1] = attach(frame.tags.brass_left);
        }
    }
}
