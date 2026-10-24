//! Other players' bodies: which remote entities get a body this frame, their
//! scene entries, and the batched draw of every seated body's skinned mesh.

use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use bevy::prelude::*;

use crate::anim::dobj_pose::{HostDObjPoseFrame, PosedPlayerFrame, begin_dobj_pose_frame};
use crate::anim::model_materials::PreparedModelMaterials;
use crate::anim::remote_body::{
    PreparedRemoteKits, RemoteBodySkinnedItem, RemoteBodySkinnedQueue, kit_hide_part_bits,
    occupy_lod_byte, occupy_remote_kit_dobj,
};
use crate::anim::scene_submission::{AnimDObjSceneSkels, AnimDObjSceneSubmission, AnimSceneSubmit};
use crate::gaps::{RenderGap, RenderGapCause, RenderPresentationGaps};
use crate::occupancy::fpv_present::occupy_fpv_scene;
use crate::occupancy::script_model::ScriptModelSkinSet;
use crate::occupancy::t6_body::SlowGuard;
use crate::occupancy::third_person::presented_is_third_person;
use crate::{
    RemoteBodyDrawPlan, append_remote_body_cpu_blob, finish_remote_body_draw_plan,
    install_body_packed_session, push_remote_body_cpu_draw, radius_lighting_box_half,
    take_body_packed_session, topology_fingerprint,
};
use asset_model::{ModelSkel, kit_assignment_is_axis};
use assets::{PreparedBodies, PreparedWeapons, PreparedWorldWeapons};
use frame::{ModelLightingSeated, ViewSubject, WorkerCmdSet};
use net::{
    CEntity, CEntityRuntime, ClientSet, LocalPresentClient, PlayerDrawGate, PresentedPublished,
    PresentedSnapshot, remote_body_submits, remote_pose_sample,
};
use render_scene::{
    DynAtPointLookup, FlyCamera, FpvLens, GfxSceneAdd, LodRampArgs, LodRampSkinnedDvar,
    ModelLightingOwner, ModelLightingRequest, ModelLightingRequests, ResolvedModelLighting,
    ResolvedModelLightingTable, TessMaterials, WorldModelLightingAtlas, WorldScriptModelInstance,
    smodel_camera_lod,
};

mod kits;
mod occupy;
mod seated;
mod submit;
use self::kits::*;
use self::occupy::*;
use self::seated::*;
use self::submit::*;

#[cfg(test)]
mod tests;

/// Marks an entity that stands for another player and carries their team.
#[derive(Component, Debug, Clone, Copy)]
#[require(RemoteFxBolts)]
pub struct RemotePlayer {
    pub ffa_team: Option<u8>,
    pub client_state_team: i32,
}

/// Where the effects of a remote player's weapon attach on their body this
/// frame.
#[derive(Component, Default)]
pub struct RemoteFxBolts {
    pub flash: Option<fx::FxBoltTarget>,
    pub brass: Option<fx::FxBoltTarget>,
    pub knife: Option<fx::FxBoltTarget>,
    pub laser: Option<fx::FxBoltTarget>,
}

/// Each drawn client's lighting request, by client number, for this frame.
#[derive(Resource, Default)]
struct RemoteBodyLightingBinds {
    by_client: HashMap<u32, ModelLightingRequest>,
}

/// A body that has its light seated and is ready to draw.
struct SeatedBody {
    item: RemoteBodySkinnedItem,
    handle: u32,
    scene_light: u8,
    probe: u8,
}

type RemoteQueryFilter = (
    Without<WorldScriptModelInstance>,
    Without<FpvLens>,
    Without<FlyCamera>,
);

pub fn register_remote_body_systems(app: &mut App) {
    app.init_resource::<RenderPresentationGaps>()
        .init_resource::<RemoteBodyDrawPlan>()
        .init_resource::<RemoteBodySkinnedQueue>()
        .init_resource::<RemoteBodyLightingBinds>()
        .init_resource::<PreparedRemoteKits>()
        .init_resource::<HostDObjPoseFrame>()
        .init_resource::<PosedPlayerFrame>()
        .add_systems(Update, prepare_remote_kits.in_set(ClientSet::Load))
        .add_systems(
            Update,
            sync_remote_bodies
                .after(PresentedPublished)
                .before(occupy_remote_scene_ents)
                .in_set(GfxSceneAdd),
        )
        .add_systems(
            Update,
            occupy_remote_scene_ents
                .after(sync_remote_bodies)
                .after(PresentedPublished)
                .after(occupy_fpv_scene)
                .in_set(GfxSceneAdd)
                .in_set(AnimSceneSubmit),
        )
        .add_systems(
            Update,
            (clear_remote_body_frame
                .after(sync_remote_bodies)
                .after(begin_dobj_pose_frame)
                .after(WorkerCmdSet::CellStatic)
                .after(WorkerCmdSet::CellSceneEnt)
                .after(WorkerCmdSet::DpvsEnt),)
                .after(PresentedPublished)
                .in_set(WorkerCmdSet::SkinModel),
        )
        .add_systems(
            Update,
            begin_dobj_pose_frame
                .after(PresentedPublished)
                .in_set(ClientSet::Present),
        )
        .add_systems(
            Update,
            enqueue_remote_body_lighting
                .after(WorkerCmdSet::SkinModel)
                .after(ScriptModelSkinSet)
                .before(WorkerCmdSet::CellDynModel)
                .after(PresentedPublished)
                .in_set(ClientSet::Present),
        )
        .add_systems(
            Update,
            (
                submit_remote_bodies.after(ModelLightingSeated),
                finish_body_draw_plan.after(submit_remote_bodies),
            )
                .in_set(WorkerCmdSet::AddSceneEnt),
        );
}
