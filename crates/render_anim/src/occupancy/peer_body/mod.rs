//! Bodies for the other players: deciding which remote entities get one this
//! frame, putting them in the scene, and drawing all the seated skinned
//! meshes in one batch.

use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use bevy::prelude::*;

use crate::motion::puppet_pose::{HostSkeletonFrame, AnimatedBodyFrame};
use crate::motion::puppet_pose::start_skeleton_pose_tick;
use crate::motion::model_materials::PreparedModelMaterials;
use crate::motion::other_body::{PreparedRemoteKits, BodySkinJob, BodySkinJobs};
use crate::motion::other_body::{kit_hide_part_bits, choose_lod_byte, build_body_kit_dobj};
use crate::motion::scene_feed::{SceneBonePoseSet, SkeletonSceneHandoff};
use crate::motion::scene_feed::SkeletonSceneStage;
use crate::holes::{PresentationShortfall, Cause, ShortfallLedger};
use crate::occupancy::gun_present::populate_gun_scene;
use crate::occupancy::script_prop::ScriptPropSkinStage;
use crate::occupancy::t6_body::SlowGuard;
use crate::occupancy::other_view::shown_in_third_person;
use crate::OtherBodyPaintPlan;
use crate::{add_other_body_software_mesh, seal_other_body_batch};
use crate::{begin_body_packed_session, queue_other_body_cpu_paint};
use crate::{end_body_packed_session, ix_layout_digest};
use lighting_t6::illum_query_half_size;
use asset_model::{MeshRig, kit_belongs_to_axis};
use assets::{ReadiedCharacterBodies, StagedWeapons, ReadiedWorldWeapons};
use frame::{ModelLightPlaced, EyeSubject, JobQueueStage};
use net::{ClientEntityRecord, ClientEntityPoseState, ClientStep, HostViewClient, PlayerDrawGate};
use net::{ShownStatePosted, ShownWorldFrame, remote_body_draws, other_player_pose};
use render_scene::{LooseAtSpotLookup, FreeRoamCamera, FirstPersonLens, RenderSceneInsert, DetailRampArgs};
use render_scene::{DetailRampSkinnedTunable, ModelIllumOwner, ModelIllumRequest};
use render_scene::{ModelIllumRequests, ResolvedModelIllum, ResolvedModelIllumTable};
use render_scene::{StitchMaterials, LevelModelIllumAtlas, LevelScriptModelInstance};
use render_scene::prop_camera_detail;

mod kits;
mod occupy;
mod seated;
mod submit;
use self::kits::*;
use self::occupy::*;
use self::seated::*;
use self::submit::*;

mod tests;

/// Tags an entity that represents another player, with the team they are on.
#[derive(Component, Debug, Clone, Copy)]
#[require(OtherBodyEffectTags)]
pub struct OtherPlayerBody {
    pub ffa_side: Option<u8>,
    pub player_state_team: i32,
}

/// The spots on a remote player's body where their weapon's effects attach
/// this frame.
#[derive(Component, Default)]
pub struct OtherBodyEffectTags {
    pub flash: Option<fx::EffectBoltTarget>,
    pub brass: Option<fx::EffectBoltTarget>,
    pub knife: Option<fx::EffectBoltTarget>,
    pub laser: Option<fx::EffectBoltTarget>,
}

/// This frame's lighting request for every drawn client, looked up by number.
#[derive(Resource, Default)]
struct BodyLightBinds {
    per_client: HashMap<u32, ModelIllumRequest>,
}

/// A body whose light is seated, so it can be drawn.
struct SeatedBody {
    item: BodySkinJob,
    hnd: u32,
    stage_light: u8,
    probe: u8,
}

/// Adds the resources for remote bodies and the systems that build, light and
/// draw them each frame.
pub fn wire_remote_body_systems(application: &mut App) {
    let present = ClientStep::Present;
    application.init_resource::<ShortfallLedger>();
    application.init_resource::<OtherBodyPaintPlan>();
    application.init_resource::<BodySkinJobs>();
    application.init_resource::<BodyLightBinds>();
    application.init_resource::<PreparedRemoteKits>();
    application.init_resource::<HostSkeletonFrame>();
    application.init_resource::<AnimatedBodyFrame>();

    application.add_systems(Update, prepare_remote_kits.in_set(ClientStep::Load));

    // Bodies are synced first, then entered into the scene behind the
    // viewmodel's entry.
    application.add_systems(
        Update,
        refresh_body_entities
            .in_set(RenderSceneInsert)
            .after(ShownStatePosted)
            .before(claim_body_scene_ents),
    );
    application.add_systems(
        Update,
        claim_body_scene_ents
            .in_set(SkeletonSceneStage)
            .in_set(RenderSceneInsert)
            .after(refresh_body_entities)
            .after(populate_gun_scene)
            .after(ShownStatePosted),
    );

    application.add_systems(
        Update,
        clear_remote_body_frame
            .in_set(JobQueueStage::SkinModel)
            .after(refresh_body_entities)
            .after(start_skeleton_pose_tick)
            .after(JobQueueStage::CellStatic)
            .after(JobQueueStage::CellSceneEnt)
            .after(JobQueueStage::DpvsEnt)
            .after(ShownStatePosted),
    );
    application.add_systems(
        Update,
        start_skeleton_pose_tick
            .in_set(present)
            .after(ShownStatePosted),
    );
    application.add_systems(
        Update,
        queue_body_lighting
            .in_set(present)
            .after(JobQueueStage::SkinModel)
            .after(ScriptPropSkinStage)
            .after(ShownStatePosted)
            .before(JobQueueStage::CellDynModel),
    );

    let draw = submit_remote_bodies.after(ModelLightPlaced);
    let finish = finish_body_draw_plan.after(submit_remote_bodies);
    application.add_systems(Update, (draw, finish).in_set(JobQueueStage::AddSceneEnt));
}
