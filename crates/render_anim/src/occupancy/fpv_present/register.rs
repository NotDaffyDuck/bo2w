//! Wiring: which resources the first-person viewmodel needs and the order its
//! systems run in each frame.

use bevy::prelude::*;
use frame::{LifeFrontPublished, PresentedPublished, WorkerCmdSet};
use net::{ClientSet, LocalPresentClient, PresentedSnapshot, ViewweaponAim};

use super::geometry::{publish_fpv_dobj_pose, skin_fpv_geometry, stamp_fpv_placement_matrix};
use super::occupy::occupy_fpv_scene;
use super::placement::apply_fpv_placement;
use super::session::{FpvStatusGap, LocalSpawnArmed, SessionViewmodel};
use super::spawn::spawn_pending_fpv;
use super::tick::tick_fpv_viewmodel;
use super::{FpvGeometrySet, FpvPlacementSet};
use crate::anim::fpv_host::{PendingFpvNotetracks, PendingFpvSpawn};
use crate::anim::fpv_prepared::PreparedFpv;
use crate::anim::scene_submission::AnimSceneSubmit;
use crate::gaps::RenderPresentationGaps;
use crate::occupancy::camera_shake;
use crate::occupancy::view_kick::{
    GunOffset, PendingViewHurt, SessionViewKick, log_death_cam_jumps,
    reset_view_kick_on_life_started, sync_camera_from_presented, tick_session_view_kick,
};

/// Run condition: a viewmodel spawn is waiting to be served.
fn fpv_spawn_queued(pending: Res<PendingFpvSpawn>) -> bool {
    pending.0.is_some()
}

/// Applies the deferred commands of the spawn system so the tick sees them.
fn flush_fpv_spawn(world: &mut World) {
    world.flush();
}

/// Forwards the notetracks the viewmodel animation fired this frame to audio.
fn publish_fpv_notetracks(
    generation: Res<frame::WorldGeneration>,
    presented: Res<PresentedSnapshot>,
    local: Res<LocalPresentClient>,
    pending: Res<PendingFpvNotetracks>,
    mut notes: MessageWriter<audio::ViewmodelNotetracks>,
) {
    if pending.weapon == 0 || pending.names.is_empty() {
        return;
    }
    let Some(meta) = presented
        .snapshot()
        .and_then(|snapshot| snapshot.meta.for_client(local.0))
    else {
        return;
    };
    notes.write(audio::ViewmodelNotetracks {
        generation: *generation,
        client: local.0,
        life: meta.life_sequence,
        weapon: pending.weapon,
        names: pending.names.clone(),
    });
}

/// Installs the viewmodel's resources, camera-shake observers and per-frame
/// systems.
pub fn register_fpv_present_systems(app: &mut App) {
    app.init_resource::<LocalSpawnArmed>()
        .init_resource::<SessionViewmodel>()
        .init_resource::<PreparedFpv>()
        .init_resource::<crate::anim::model_materials::PreparedModelMaterials>()
        .init_resource::<SessionViewKick>()
        .init_resource::<GunOffset>()
        .init_resource::<ViewweaponAim>()
        .init_resource::<PendingViewHurt>()
        .init_resource::<camera_shake::CameraShakes>()
        .init_resource::<FpvStatusGap>()
        .init_resource::<RenderPresentationGaps>()
        .add_observer(camera_shake::remote_missile_boost)
        .add_observer(camera_shake::earthquake)
        .add_observer(camera_shake::explosion)
        .add_observer(camera_shake::weapon_fire)
        .add_observer(camera_shake::shield_hit)
        .add_observer(camera_shake::shield_blast);

    app.add_systems(
        Update,
        reset_view_kick_on_life_started.in_set(LifeFrontPublished),
    );
    app.add_systems(
        Update,
        occupy_fpv_scene
            .after(PresentedPublished)
            .in_set(render_scene::GfxSceneAdd)
            .in_set(AnimSceneSubmit),
    );
    app.add_systems(
        Update,
        (
            tick_session_view_kick.after(reset_view_kick_on_life_started),
            sync_camera_from_presented.after(tick_session_view_kick),
            log_death_cam_jumps.after(sync_camera_from_presented),
            spawn_pending_fpv
                .run_if(fpv_spawn_queued)
                .after(sync_camera_from_presented),
            flush_fpv_spawn.after(spawn_pending_fpv),
            tick_fpv_viewmodel.after(flush_fpv_spawn),
            skin_fpv_geometry
                .after(tick_fpv_viewmodel)
                .in_set(FpvGeometrySet),
            publish_fpv_notetracks.after(tick_fpv_viewmodel),
            apply_fpv_placement
                .after(tick_fpv_viewmodel)
                .before(WorkerCmdSet::CellSceneEnt),
            // Neither placement nor bone publication reads a vertex.
            stamp_fpv_placement_matrix
                .after(apply_fpv_placement)
                .in_set(FpvPlacementSet),
            publish_fpv_dobj_pose
                .after(stamp_fpv_placement_matrix)
                .after(crate::anim::dobj_pose::begin_dobj_pose_frame),
        )
            .in_set(ClientSet::Present),
    );
}
