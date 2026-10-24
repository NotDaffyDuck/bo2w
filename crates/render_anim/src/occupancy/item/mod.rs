//! Weapons lying on the ground: each dropped item is a world-weapon model with
//! its attachments, posed once per weapon and drawn wherever one lies.

use std::collections::HashMap;
use std::sync::Arc;

use bevy::prelude::*;

use crate::anim::model_materials::PreparedModelMaterials;
use crate::anim::scene_submission::{AnimDObjSceneSkels, AnimDObjSceneSubmission, AnimSceneSubmit};
use crate::anim::xmodel_pose::PosedModelSurface;
use crate::occupancy::script_model::pose_script_dobj_with_materials;
use crate::occupancy::t6_body::SlowGuard;
use crate::{
    ItemAssetDraw, ItemDrawPlan, ItemOwnerDraw, XMODEL_OBJECT_ID_ITEM_BASE, append_item_surfaces,
    radius_lighting_box_half, stamp_plan_geometry, topology_fingerprint,
};
use entity_t6::{EntityState, et, evaluate_trajectory};
use frame::{RenderSet, WorkerCmdSet};
use playerstate_t6::game::PERK_SCAVENGER;
use render_scene::{
    GfxScene, HostGfxScene, ModelLightingOwner, ModelLightingRequest, ModelLightingRequests,
    SmodelPassMaterial, TessMaterials, WorldModelLightingAtlas, WorldPresentFacts,
    XModelSurfaceDraw, scene_quat_from_angles,
};

mod compose;
mod draw;
mod occupy;
mod place;
mod pose;
use self::compose::*;
use self::draw::*;
use self::occupy::*;
pub use self::place::*;
use self::pose::*;

#[cfg(test)]
mod tests;

/// How far above an item its light sample is taken.
pub const ITEM_LIGHTING_Z_OFS: f32 = 4.0;

/// Entity flag: the item is not shown.
const FLAG_ITEM_HIDDEN: i32 = 0x20;

/// A dropped item's weapon model and attachments, assembled into one object.
struct ItemComposition {
    name: String,
    model: assets::WorldWeaponIndex,
    attachments: Vec<assets::WorldWeaponIndex>,
    /// Model and attachment names joined, identifying the posed result.
    key: String,
    dobj: xmodel_runtime::DObj,
}

/// Compositions for every weapon, valid for one loaded weapon set.
#[derive(Resource, Default)]
struct PreparedItemCompositions {
    built_for: Option<(usize, u64)>,
    by_weapon: HashMap<u32, Arc<ItemComposition>>,
}

impl PreparedItemCompositions {
    fn built_for(
        weapons: &assets::PreparedWeapons,
        world: &assets::PreparedWorldWeapons,
    ) -> (usize, u64) {
        (Arc::as_ptr(&weapons.0) as usize, world.0.identity())
    }

    fn matches(
        &self,
        weapons: &assets::PreparedWeapons,
        world: &assets::PreparedWorldWeapons,
    ) -> bool {
        self.built_for == Some(Self::built_for(weapons, world))
    }
}

/// Items submitted to the scene this frame.
#[derive(Resource, Default)]
struct ItemOccupancy {
    rows: Vec<OccupiedItem>,
}

struct OccupiedItem {
    /// Position among this frame's item entities.
    slot: usize,
    entnum: i32,
    origin: [f32; 3],
    angles: [f32; 3],
    composition: Arc<ItemComposition>,
    lighting_origin: [f32; 3],
}

/// Posed surfaces for each distinct composition, and who uses them.
#[derive(Resource, Default)]
struct ItemPoseProduct {
    assets: Vec<ItemPosedAsset>,
    owners: Vec<ItemPosedOwner>,
}

struct ItemPosedAsset {
    key: String,
    models: Vec<assets::WorldWeaponIndex>,
    surfaces: Vec<PosedModelSurface>,
}

struct ItemPosedOwner {
    asset_index: usize,
    slot: usize,
    entnum: u32,
    origin: [f32; 3],
    angles: [f32; 3],
    name: String,
    model: assets::WorldWeaponIndex,
    lighting_origin: [f32; 3],
}

pub fn register_item_systems(app: &mut App) {
    app.init_resource::<ItemDrawPlan>()
        .init_resource::<ItemOccupancy>()
        .init_resource::<PreparedItemCompositions>()
        .init_resource::<ItemPoseProduct>()
        .add_systems(Update, build_item_compositions.in_set(net::ClientSet::Load))
        .add_systems(
            Update,
            occupy_item_scene_ents
                .in_set(RenderSet::Anim)
                .before(WorkerCmdSet::CellDynModel)
                .in_set(render_scene::GfxSceneAdd)
                .in_set(AnimSceneSubmit),
        )
        .add_systems(
            Update,
            (pose_items, append_item_draws)
                .chain()
                .after(WorkerCmdSet::CellSceneEnt)
                .after(WorkerCmdSet::DpvsEnt)
                .before(WorkerCmdSet::CellDynModel)
                .in_set(WorkerCmdSet::SkinModel),
        );
}

/// The scene state of one item's entity slot.
struct SceneSlot<'a> {
    hidden: bool,
    skin_entries: &'a [dpvs_t6::SceneEntSkinEntry],
}

fn scene_slot(scene: &GfxScene, entnum: u32) -> SceneSlot<'_> {
    SceneSlot {
        hidden: scene.scene_ent_skips_draw(entnum),
        skin_entries: scene
            .scene_ent_skinned_surfs(entnum)
            .map_or(&[][..], |surfs| surfs.entries.as_slice()),
    }
}
