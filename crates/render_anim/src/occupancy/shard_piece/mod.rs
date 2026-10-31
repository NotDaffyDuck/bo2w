//! Loose pieces (dynamic entities): the cells and primary lights each piece
//! touches, whether it is on screen this frame, and the draw rows it adds.

use bevy::prelude::*;
use dpvs_t6::detach_moving_entity_from_main_lights;
use dpvs_t6::{Bounds, VisibilityCellPlanes};
use dpvs_t6::{debris_cell_mask_len, debris_viewer_words, debris_marked_in_cell};
use dpvs_t6::{moving_entity_main_light_visibility_words, filter_debris_into_rooms};
use dpvs_t6::{attach_dynamic_prop_key_light_bit, unfilter_dynamic_ent_from_cells};
use render_scene::{LooseAtSpotLookup, DynamicPropModelEntity, FirstPersonLens, FrustumShape};
use render_scene::{ModelIllumOwner, ModelIllumRequest, ModelIllumRequests};
use render_scene::{ExportedCellVisibility, PropPassMaterial, StitchMaterials, LevelSightRooms};
use render_scene::{LevelDebrisInstance, LevelModelIllumAtlas, LevelPresentFacts};
use render_scene::{XModelCasterBound, ModelTintRejection, FigureFacetDraw};
use render_scene::{perspective_frustum_planes, prop_camera_detail};

use crate::motion::body_cull::CellVisAdmit;
use crate::motion::model_materials::PreparedModelMaterials;
use crate::motion::figure_pose::SkinnedMeshFacet;
use crate::occupancy::debris_phys;
use crate::occupancy::script_prop::{DetailLevelWindow, pose_script_skeleton_and_materials};
use crate::occupancy::t6_body::SlowGuard;
use lighting_t6::illum_query_half_size;
use crate::{DynEntPaintPlan, ModelOwnerPaint, PAINT_ID_FIRST_PROP};
use crate::{append_debris_asset, bump_plan_mesh_rev, ix_layout_digest};

pub use debris_phys::{PropPhysicsShape, PropPhysicsNudge, DebrisPhysLevel};

mod bounds;
mod lights;
mod link;
mod pose;
pub use self::bounds::*;
pub use self::lights::*;
use self::link::*;
use self::pose::*;

mod tests;

// ---------------------------------------------------------------------------
// Per-piece cell membership and primary-light links
// ---------------------------------------------------------------------------

/// One row of bits per cell, one bit per piece slot; a set bit means the
/// piece's box reaches that cell.
#[derive(Resource, Clone, Debug, Default)]
pub struct DebrisRoomBits {
    pub dword_tally: usize,
    pub slot_cell_count: usize,
    pub player_count: u32,
    pub bits: Vec<u32>,
}

impl DebrisRoomBits {
    pub fn sized(&self) -> bool {
        if self.dword_tally == 0 || self.slot_cell_count == 0 {
            return false;
        }
        self.bits.len() == debris_cell_mask_len(self.dword_tally, self.slot_cell_count)
    }

    /// Resizes (and clears) the table when the piece or cell count changed.
    /// Returns true when it was rebuilt, so the caller must relink everything.
    pub fn ensure(&mut self, client_total: u32, cell_total: usize) -> bool {
        let words = debris_viewer_words(client_total);
        let wanted = debris_cell_mask_len(words, cell_total);
        let shape = (self.dword_tally, self.slot_cell_count, self.player_count);
        if shape == (words, cell_total, client_total) && self.bits.len() == wanted {
            return false;
        }
        *self = Self {
            bits: vec![0; wanted],
            player_count: client_total,
            slot_cell_count: cell_total,
            dword_tally: words,
        };
        true
    }
}

#[derive(Resource, Clone, Debug, Default)]
pub struct PropMainLightVisibility {
    pub tokens: Vec<u32>,
    pub nearest: Vec<u8>,
    pub sunlight_primary_flag_index: u32,
    pub main_light_total: u32,
}

#[derive(Resource, Clone, Debug, Default)]
pub struct DebrisSolidKeyLightVis {
    pub words: Vec<u32>,
    pub sunlight_primary_flag_index: u32,
    pub main_light_total: u32,
}

/// Where a loose piece's size comes from. Some maps list no models of their
/// own; their pieces draw from the prepared T6 model list instead, so the size
/// must come from there too. Without it every piece was size 0: shots missed
/// and pushes passed straight through.
#[derive(Clone, Copy, Default)]
pub struct PieceModels<'a> {
    pub map: Option<&'a asset_world::LevelModelPlacementRoster>,
    pub t6: Option<&'a assets::ReadiedFpvMeshes>,
}

impl PieceModels<'_> {
    fn bone_rig(&self, key: &asset_world::LevelModelAssetId) -> Option<&asset_model::MeshRig> {
        let listed = self.map.and_then(|scene| scene.get(key));
        let from_map = match listed {
            Some(asset_world::LevelModelPlacementAsset::T5(bone_rig)) => Some(bone_rig.as_ref()),
            Some(asset_world::LevelModelPlacementAsset::Unavailable { .. }) | None => None,
        };
        if from_map.is_some() {
            return from_map;
        }
        let entry = self.t6?.0.by_label(asset_core::AssetCategoryScope::T6, &key.0)?;
        Some(entry.bone_rig.as_ref())
    }

    pub fn radius(&self, key: &asset_world::LevelModelAssetId) -> f32 {
        self.bone_rig(key).and_then(|bone_rig| bone_rig.radius).unwrap_or(0.0)
    }

    /// Local-space box a piece collides with: the model's own bounds when it
    /// has usable ones, else a cube of its radius (at least 1).
    pub fn phys_hull(&self, key: &asset_world::LevelModelAssetId) -> ([f32; 3], [f32; 3]) {
        let from_bounds = self
            .bone_rig(key)
            .and_then(|bone_rig| hull_from_bounds(bone_rig.bounds));
        if let Some(hull) = from_bounds {
            return hull;
        }
        let reach = self.radius(key).max(1.0);
        ([-reach; 3], [reach; 3])
    }
}

#[derive(Resource, Default)]
struct DynEntPosed {
    posed_owners: Vec<DynEntPosedHolder>,
    posed_assets: Vec<DynEntPosedModel>,
}

/// A model posed once for a camera level of detail, shared by the pieces that use it.
struct DynEntPosedModel {
    lod: Option<u8>,
    mesh_asset_name: asset_world::LevelModelAssetId,
    authored_materials: Vec<Option<assets::SurfaceMaterialId>>,
    drawn: Vec<SkinnedMeshFacet>,
}

/// One piece, and which posed asset it draws from.
struct DynEntPosedHolder {
    concealed: bool,
    reach: Option<f32>,
    piece: Entity,
    model_name: String,
    lit_sample_point: [f32; 3],
    asset_at: usize,
    world_mat: Mat4,
}

impl DynEntPosed {
    fn draw_asset_slot(&self, key: &asset_world::LevelModelAssetId, lod: Option<u8>) -> Option<usize> {
        let same_asset = |posed: &DynEntPosedModel| posed.mesh_asset_name == *key && posed.lod == lod;
        self.posed_assets.iter().position(same_asset)
    }
}

// ---------------------------------------------------------------------------
// Registration
// ---------------------------------------------------------------------------

pub fn add_dyn_ent_systems(application: &mut App) {
    use frame::JobQueueStage;

    debris_phys::add_dyn_ent_physics(application);
    application.init_resource::<PropMainLightVisibility>();
    application.init_resource::<DebrisRoomBits>();
    application.init_resource::<DynEntPosed>();
    application.init_resource::<DynEntPaintPlan>();
    let phys_step = debris_phys::step_phys_first;
    application.add_systems(Update, phys_step.in_set(JobQueueStage::Physics));
    application.add_systems(
        Update,
        (
            bind_dyn_ents_to_cells.after(phys_step),
            cull_dyn_ents_by_cell
                .after(bind_dyn_ents_to_cells)
                .after(JobQueueStage::CellStatic),
            place_dyn_ent_models.after(cull_dyn_ents_by_cell),
            push_dyn_ent_draws.after(place_dyn_ent_models),
        )
            .in_set(frame::DrawPassStage::Anim)
            .in_set(JobQueueStage::CellDynModel),
    );
}

// ---------------------------------------------------------------------------
// Systems
// ---------------------------------------------------------------------------
