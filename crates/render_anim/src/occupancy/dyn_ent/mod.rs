//! Loose pieces (dynamic entities): the cells and primary lights each piece
//! touches, whether it is on screen this frame, and the draw rows it adds.

use bevy::prelude::*;
use dpvs_t6::{
    Bounds, DpvsPlanes, dyn_ent_cell_bits_len, dyn_ent_client_word_count, dyn_ent_in_cell,
    dyn_ent_primary_light_vis_word_count, filter_dyn_ent_into_cells,
    link_dyn_ent_primary_light_bit, unfilter_dyn_ent_from_cells,
    unlink_dyn_ent_from_primary_lights,
};
use render_scene::{
    DynAtPointLookup, DynEntModelEntity, FpvLens, FrustumSpec, ModelLightingOwner,
    ModelLightingRequest, ModelLightingRequests, PublishedCellVis, SmodelPassMaterial,
    TessMaterials, WorldDpvsCells, WorldDynEntInstance, WorldModelLightingAtlas, WorldPresentFacts,
    XModelCasterBound, XModelColourRefusal, XModelSurfaceDraw, perspective_frustum_planes,
    smodel_camera_lod,
};

use crate::anim::body_frustum::AdmittedCellVis;
use crate::anim::model_materials::PreparedModelMaterials;
use crate::anim::xmodel_pose::PosedModelSurface;
use crate::occupancy::dyn_ent_phys;
use crate::occupancy::script_model::{DObjLodView, pose_script_dobj_with_materials};
use crate::occupancy::t6_body::SlowGuard;
use crate::radius_lighting_box_half;
use crate::{
    DynEntDrawPlan, DynEntOwnerDraw, XMODEL_OBJECT_ID_DYNENT_BASE, append_dynent_asset,
    stamp_plan_geometry, topology_fingerprint,
};

pub use dyn_ent_phys::{DynEntPhysClip, DynEntPhysImpulse, DynEntPhysWorld};

mod bounds;
mod lights;
mod link;
mod pose;
pub use self::bounds::*;
pub use self::lights::*;
use self::link::*;
use self::pose::*;

#[cfg(test)]
mod tests;

// ---------------------------------------------------------------------------
// Per-piece cell membership and primary-light links
// ---------------------------------------------------------------------------

/// One row of bits per cell, one bit per piece slot; a set bit means the
/// piece's box reaches that cell.
#[derive(Resource, Clone, Debug, Default)]
pub struct DynEntCellBits {
    pub word_count: usize,
    pub cell_count: usize,
    pub client_count: u32,
    pub bits: Vec<u32>,
}

impl DynEntCellBits {
    pub fn is_ready(&self) -> bool {
        let expected = dyn_ent_cell_bits_len(self.word_count, self.cell_count);
        self.word_count != 0 && self.cell_count != 0 && self.bits.len() == expected
    }

    /// Resizes (and clears) the table when the piece or cell count changed.
    /// Returns true when it was rebuilt, so the caller must relink everything.
    pub fn ensure(&mut self, client_count: u32, cell_count: usize) -> bool {
        let words = dyn_ent_client_word_count(client_count);
        let wanted = dyn_ent_cell_bits_len(words, cell_count);
        let same_shape = self.word_count == words
            && self.cell_count == cell_count
            && self.client_count == client_count;
        if same_shape && self.bits.len() == wanted {
            return false;
        }
        *self = Self {
            word_count: words,
            cell_count,
            client_count,
            bits: vec![0; wanted],
        };
        true
    }
}

#[derive(Resource, Clone, Debug, Default)]
pub struct DynEntPrimaryLightVis {
    pub words: Vec<u32>,
    pub closest: Vec<u8>,
    pub sun_primary: u32,
    pub primary_count: u32,
}

#[derive(Resource, Clone, Debug, Default)]
pub struct DynEntBrushPrimaryLightVis {
    pub words: Vec<u32>,
    pub sun_primary: u32,
    pub primary_count: u32,
}

/// Where a loose piece's size comes from. Some maps list no models of their
/// own; their pieces draw from the prepared T6 model list instead, so the size
/// must come from there too. Without it every piece was size 0: shots missed
/// and pushes passed straight through.
#[derive(Clone, Copy, Default)]
pub struct PieceModels<'a> {
    pub map: Option<&'a asset_world::MapXModelSceneCatalog>,
    pub t6: Option<&'a assets::PreparedFpvMeshes>,
}

impl PieceModels<'_> {
    fn skel(&self, key: &asset_world::MapXModelAssetKey) -> Option<&asset_model::ModelSkel> {
        let listed = self.map.and_then(|catalog| match catalog.get(key)? {
            asset_world::MapXModelSceneAsset::T5(skel) => Some(skel.as_ref()),
            asset_world::MapXModelSceneAsset::Unavailable { .. } => None,
        });
        listed.or_else(|| {
            let prepared = self.t6?;
            let entry = prepared.0.get(asset_core::AssetNamespace::T6, &key.0)?;
            Some(entry.skel.as_ref())
        })
    }

    pub fn radius(&self, key: &asset_world::MapXModelAssetKey) -> f32 {
        self.skel(key).and_then(|skel| skel.radius).unwrap_or(0.0)
    }

    /// Local-space box a piece collides with: the model's own bounds when it
    /// has usable ones, else a cube of its radius (at least 1).
    pub fn phys_hull(&self, key: &asset_world::MapXModelAssetKey) -> ([f32; 3], [f32; 3]) {
        if let Some(hull) = self
            .skel(key)
            .and_then(|skel| hull_from_bounds(skel.bounds))
        {
            return hull;
        }
        let reach = self.radius(key).max(1.0);
        ([-reach; 3], [reach; 3])
    }
}

#[derive(Resource, Default)]
struct DynEntPoseProduct {
    assets: Vec<DynEntPosedAsset>,
    owners: Vec<DynEntPosedOwner>,
}

struct DynEntPosedAsset {
    key: asset_world::MapXModelAssetKey,
    camera_lod: Option<u8>,
    surfaces: Vec<PosedModelSurface>,
    authored: Vec<Option<assets::MaterialIndex>>,
}

struct DynEntPosedOwner {
    entity: Entity,
    asset_index: usize,
    model: String,
    world_from_local: Mat4,
    lighting_origin: [f32; 3],
    camera_hidden: bool,
    radius: Option<f32>,
}

impl DynEntPoseProduct {
    fn asset_index(
        &self,
        key: &asset_world::MapXModelAssetKey,
        camera_lod: Option<u8>,
    ) -> Option<usize> {
        self.assets
            .iter()
            .position(|posed| posed.camera_lod == camera_lod && &posed.key == key)
    }
}

// ---------------------------------------------------------------------------
// Registration
// ---------------------------------------------------------------------------

pub fn register_dyn_ent_systems(app: &mut App) {
    dyn_ent_phys::register_dyn_ent_phys(app);
    app.init_resource::<DynEntDrawPlan>()
        .init_resource::<DynEntPoseProduct>()
        .init_resource::<DynEntCellBits>()
        .init_resource::<DynEntPrimaryLightVis>();
    app.add_systems(
        Update,
        dyn_ent_phys::step_phys_world0.in_set(frame::WorkerCmdSet::Physics),
    );
    app.add_systems(
        Update,
        (
            link_dyn_ent_cells.after(dyn_ent_phys::step_phys_world0),
            cull_dyn_ent_cell_models
                .after(link_dyn_ent_cells)
                .after(frame::WorkerCmdSet::CellStatic),
            pose_dyn_ents.after(cull_dyn_ent_cell_models),
            append_dynent_draws.after(pose_dyn_ents),
        )
            .in_set(frame::RenderSet::Anim)
            .in_set(frame::WorkerCmdSet::CellDynModel),
    );
}

// ---------------------------------------------------------------------------
// Systems
// ---------------------------------------------------------------------------

type MovedPieces = Or<(
    Changed<Transform>,
    Changed<WorldDynEntInstance>,
    Added<WorldDynEntInstance>,
)>;
