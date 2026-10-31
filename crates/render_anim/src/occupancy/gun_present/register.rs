//! Wiring for the first-person viewmodel: the resources it needs, and the
//! order its systems run in every frame.

use bevy::prelude::*;
use frame::{LifeStatePosted, ShownStatePosted, JobQueueStage};
use net::{ClientStep, HostViewClient, ShownWorldFrame, ViewweaponAim};

use super::gun_mesh::publish_dobj_pose;
use super::gun_mesh::skin_geometry;
use super::gun_mesh::stamp_matrix;
use super::occupy::populate_gun_scene;
use super::gun_place::place_viewmodel;
use super::gun_status::{GunStatusShortfall, OwnSpawnPrimed, ActiveGunMeshes};
use super::spawn::spawn_viewmodel;
use super::tick::tick_viewmodel;
use super::{FpvGeometrySet, GunMountStage};
use crate::motion::puppet_pose::start_skeleton_pose_tick;
use super::super::super::motion::gun_host::{QueuedGunNotes, WaitingFpvSpawn};
use crate::motion::fpv_prepared::PreparedFpv;
use crate::motion::scene_feed::SkeletonSceneStage;
use crate::holes::ShortfallLedger;
use crate::occupancy::camera_shake;
use crate::occupancy::eye_kick::{GunOffset, QueuedHurtFlinch, CameraRecoilState};
use crate::occupancy::eye_kick::{log_death_cam_jumps, clear_recoil_at_respawn};
use crate::occupancy::eye_kick::{sync_camera_from_shown, advance_session_view_kick};

/// Run condition: someone asked for a viewmodel and it has not been spawned.
fn gun_spawn_waiting(queued: Res<WaitingFpvSpawn>) -> bool {
    queued.0.is_some()
}

/// Applies the commands the spawn system queued, so the tick can see them.
fn run_gun_spawn(world_mats: &mut World) {
    world_mats.flush();
}

/// Hands the notetracks that the viewmodel animation fired this frame over to
/// audio.
fn emit_gun_notes(
    generation: Res<frame::LevelGeneration>,
    presented: Res<ShownWorldFrame>,
    local: Res<HostViewClient>,
    fired: Res<QueuedGunNotes>,
    mut to_audio: MessageWriter<audio::FirstPersonAnimCues>,
) {
    let weapon = fired.weap;
    if weapon == 0 || fired.labels.is_empty() {
        return;
    }
    let client = local.0;
    let Some(life) = presented
        .snapshot()
        .and_then(|snap_in| snap_in.meta.for_client(client))
        .map(|meta| meta.life_sequence)
    else {
        return;
    };
    to_audio.write(audio::FirstPersonAnimCues {
        cue_list: fired.labels.clone(),
        generation: *generation,
        cue_wpn: weapon,
        client,
        life,
    });
}

/// Adds the viewmodel's resources, its camera-shake observers and the systems
/// that run on every frame.
pub fn wire_gun_display_systems(application: &mut App) {
    let present = ClientStep::Present;
    application.init_resource::<OwnSpawnPrimed>();
    application.init_resource::<ActiveGunMeshes>();
    application.init_resource::<PreparedFpv>();
    application.init_resource::<crate::motion::fpv_prepared::FpvHandsOverride>();
    application.init_resource::<crate::motion::model_materials::PreparedModelMaterials>();
    application.init_resource::<CameraRecoilState>();
    application.init_resource::<crate::occupancy::platform_carry::PlatformCarry>();
    application.init_resource::<GunOffset>();
    application.init_resource::<ViewweaponAim>();
    application.init_resource::<QueuedHurtFlinch>();
    application.init_resource::<camera_shake::CameraShakes>();
    application.init_resource::<GunStatusShortfall>();
    application.init_resource::<ShortfallLedger>();
    application.add_observer(camera_shake::remote_missile_boost);
    application.add_observer(camera_shake::earthquake);
    application.add_observer(camera_shake::explosion);
    application.add_observer(camera_shake::weapon_fire);
    application.add_observer(camera_shake::shield_hit);
    application.add_observer(camera_shake::shield_blast);

    let life_started = clear_recoil_at_respawn;
    application.add_systems(Update, life_started.in_set(LifeStatePosted));
    application.add_systems(
        Update,
        populate_gun_scene
            .in_set(SkeletonSceneStage)
            .in_set(render_scene::RenderSceneInsert)
            .after(ShownStatePosted),
    );

    // The camera comes first, then the viewmodel is spawned and ticked.
    application.add_systems(
        Update,
        advance_session_view_kick
            .after(life_started)
            .in_set(present),
    );
    // bo2zm: the platform under the player (Tranzit's bus) carries the camera with it.
    let carry = crate::occupancy::platform_carry::track_platform_carry;
    application.add_systems(
        Update,
        carry.after(advance_session_view_kick).in_set(present),
    );
    application.add_systems(
        Update,
        sync_camera_from_shown
            .after(advance_session_view_kick)
            .after(carry)
            .in_set(present),
    );
    application.add_systems(
        Update,
        log_death_cam_jumps
            .after(sync_camera_from_shown)
            .in_set(present),
    );
    application.add_systems(
        Update,
        spawn_viewmodel
            .run_if(gun_spawn_waiting)
            .after(sync_camera_from_shown)
            .in_set(present),
    );
    application.add_systems(
        Update,
        run_gun_spawn
            .after(spawn_viewmodel)
            .in_set(present),
    );
    application.add_systems(
        Update,
        tick_viewmodel
            .after(run_gun_spawn)
            .in_set(present),
    );

    // Everything below hangs off the tick.
    application.add_systems(
        Update,
        skin_geometry
            .in_set(FpvGeometrySet)
            .after(tick_viewmodel)
            .in_set(present),
    );
    application.add_systems(
        Update,
        emit_gun_notes
            .after(tick_viewmodel)
            .in_set(present),
    );
    application.add_systems(
        Update,
        place_viewmodel
            .before(JobQueueStage::CellSceneEnt)
            .after(tick_viewmodel)
            .in_set(present),
    );
    // Neither placement nor bone publication reads a vertex.
    application.add_systems(
        Update,
        stamp_matrix
            .in_set(GunMountStage)
            .after(place_viewmodel)
            .in_set(present),
    );
    application.add_systems(
        Update,
        publish_dobj_pose
            .after(stamp_matrix)
            .after(start_skeleton_pose_tick)
            .in_set(present),
    );
}
