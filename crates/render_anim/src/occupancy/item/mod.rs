//! Weapons lying on the ground: each dropped item is a world-weapon model with
//! its attachments, posed once per weapon and drawn wherever one lies.

use std::collections::HashMap;
use std::sync::Arc;

use bevy::prelude::*;

use crate::motion::model_materials::PreparedModelMaterials;
use crate::motion::scene_feed::{SceneBonePoseSet, SkeletonSceneHandoff, SkeletonSceneStage};
use crate::motion::figure_pose::SkinnedMeshFacet;
use crate::occupancy::script_prop::pose_script_skeleton_and_materials;
use crate::occupancy::t6_body::SlowGuard;
use crate::{
    PickupMeshPaint, PickupPaintPlan, ModelOwnerPaint, PAINT_ID_FIRST_PICKUP, push_pickup_surfaces,
    bump_plan_mesh_rev, ix_layout_digest,
};
use lighting_t6::illum_query_half_size;
use entity_t6::{EntitySnapshot, et, evaluate_trajectory};
use frame::{DrawPassStage, JobQueueStage};
use playerstate_t6::perks::PERK_SCAVENGER;
use render_scene::{
    VisualStage, HostVisualStage, ModelIllumOwner, ModelIllumRequest, ModelIllumRequests,
    PropPassMaterial, StitchMaterials, LevelModelIllumAtlas, LevelPresentFacts,
    FigureFacetDraw, orientation_quat_from_euler,
};

mod item_build;
mod draw_out;
mod occupy;
mod place;
mod pose;
use self::item_build::*;
use self::draw_out::*;
use self::occupy::*;
pub use self::place::*;
use self::pose::*;

mod tests;

/// How far above an item its light sample is taken.
pub const ITEM_LIGHT_LIFT: f32 = 4.0;

/// Entity flag: the item is not shown.
const FLAG_ITEM_HIDDEN: i32 = 0x20;

/// A dropped item's weapon model and attachments, assembled into one object.
struct ItemComposition {
    /// The compiled object the attachments hang off.
    pub(super) dobj: xmodel_runtime::PosedSkeleton,
    /// Model and attachment names joined, identifying the posed result.
    pub(super) key: String,
    attachments: Vec<assets::WorldWeaponSlot>,
    mdl: assets::WorldWeaponSlot,
    name: String,
}

/// Compositions for every weapon, valid for one loaded weapon set.
#[derive(Resource, Default)]
struct PreparedItemCompositions {
    built_for: Option<(usize, u64)>,
    by_weapon: HashMap<u32, Arc<ItemComposition>>,
}

impl PreparedItemCompositions {
    fn built_for(
        weapons: &assets::StagedWeapons,
        world: &assets::ReadiedWorldWeapons,
    ) -> (usize, u64) {
        let weapon_set = Arc::as_ptr(&weapons.0) as usize;
        let world_set = world.0.identity();
        (weapon_set, world_set)
    }

    fn matches(
        &self,
        weapons: &assets::StagedWeapons,
        world: &assets::ReadiedWorldWeapons,
    ) -> bool {
        let Some(stamp) = self.built_for else {
            return false;
        };
        stamp == Self::built_for(weapons, world)
    }
}

/// Items submitted to the scene this frame.
#[derive(Resource, Default)]
pub(super) struct ItemRows {
    pub(super) rows: Vec<ItemRow>,
}

pub(super) struct ItemRow {
    composition: Arc<ItemComposition>,
    pub(super) entnum: i32,
    /// Position among this frame's item entities.
    seat: usize,
    illum_origin: [f32; 3],
    angs: [f32; 3],
    pos: [f32; 3],
}

/// Posed surfaces for each distinct composition, and who uses them.
#[derive(Resource, Default)]
pub(super) struct ItemPosed {
    pub(super) owners: Vec<ItemPosedOwner>,
    pub(super) assets: Vec<ItemPosedAsset>,
}

pub(super) struct ItemPosedAsset {
    surfaces: Vec<SkinnedMeshFacet>,
    models: Vec<assets::WorldWeaponSlot>,
    pub(super) key: String,
}

pub(super) struct ItemPosedOwner {
    illum_origin: [f32; 3],
    mdl: assets::WorldWeaponSlot,
    name: String,
    rot: [f32; 3],
    spot: [f32; 3],
    ent_num: u32,
    ix: usize,
    pub(super) draw_asset_slot: usize,
}

pub fn add_item_systems(application: &mut App) {
    application.init_resource::<PreparedItemCompositions>();
    application.init_resource::<ItemRows>();
    application.init_resource::<ItemPosed>();
    application.init_resource::<PickupPaintPlan>();

    let dyn_model_cmds = JobQueueStage::CellDynModel;
    application.add_systems(Update, build_item_compositions.in_set(net::ClientStep::Load));
    let occupy_stage = occupy_item_scene_ents
        .in_set(render_scene::RenderSceneInsert)
        .in_set(SkeletonSceneStage)
        .in_set(DrawPassStage::Anim);
    application.add_systems(Update, occupy_stage.before(dyn_model_cmds));
    let scene_ready = JobQueueStage::CellSceneEnt;
    let pose_then_draw = (pose_items, append_item_draws).chain();
    application.add_systems(
        Update,
        pose_then_draw
            .in_set(JobQueueStage::SkinModel)
            .after(JobQueueStage::DpvsEnt)
            .after(scene_ready)
            .before(dyn_model_cmds),
    );
}

/// The scene state of one item's entity slot.
struct SceneSlot<'a> {
    concealed: bool,
    skin_jobs: &'a [dpvs_t6::EntitySkinSlotRecord],
}

fn scene_slot(scene: &VisualStage, entnum: u32) -> SceneSlot<'_> {
    let skipped = scene.entity_not_drawn(entnum);
    let rows: &[dpvs_t6::EntitySkinSlotRecord] = match scene.entity_bone_surface_set(entnum) {
        Some(surfs) => surfs.entries.as_slice(),
        None => &[],
    };
    SceneSlot {
        skin_jobs: rows,
        concealed: skipped,
    }
}
