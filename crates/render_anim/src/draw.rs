//! The draw plans: what each presented model kind publishes for the renderer
//! each frame, together with the revision counters that tell a consumer which
//! part of it moved.

use asset_t6::T6_PACKED_VERTEX;
use asset_world::{MapXModelAssetKey, PackedVertexPayload};
use bevy::prelude::*;
use render_frame::{SmodelVertex, publish_rows};
use render_scene::model_lighting::{
    ModelLightingOwner, ResolvedModelLighting, ResolvedModelLightingTable,
};
use render_scene::{SmodelPassMaterial, XModelSurfaceDraw};

pub use render_frame::SourceRevisions;

pub const FPV_PACKED_UNAVAILABLE: &str =
    "fpv GfxPackedVertex missing or count-mismatched; decoded float is not packed VB";
pub const FPV_PACKED_EMPTY_PLAN: &str = "fpv plan has no vertices";
pub const BODY_PACKED_UNAVAILABLE: &str = "remote body GfxPackedVertex not retained";

/// Object ids of each kind of presented model start at their own base, so a
/// failed lighting lookup names the plan it came from.
pub const XMODEL_OBJECT_ID_BODY_BASE: u16 = 2;
pub const XMODEL_OBJECT_ID_MISSILE_BASE: u16 = 0x400;
pub const XMODEL_OBJECT_ID_ITEM_BASE: u16 = 0x500;
pub const XMODEL_OBJECT_ID_DYNENT_BASE: u16 = 0x600;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FpvSurfaceDraw {
    pub surface: u32,
    pub material: u32,
    pub is_scope: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RemoteBodySurfaceDraw {
    pub surface: u32,
    pub material: u32,
    pub world_from_local: Mat4,
    pub lighting_handle: u32,
    pub scene_light_index: u8,
    pub reflection_probe_index: u8,
    pub scene_entnum: Option<u32>,
}

/// One script-model asset in the plan: its draw state and the
/// `(surface, material)` pairs it owns.
#[derive(Clone, Debug)]
pub struct ScriptModelAssetDraw {
    pub key: MapXModelAssetKey,
    pub dobj_state: xmodel_runtime::DObjSemanticState,
    pub camera_lods: Vec<Option<u8>>,
    pub surfaces: Vec<(u32, u32)>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ScriptModelOwnerDraw {
    pub entity: Entity,
    pub current_model: MapXModelAssetKey,
    pub object_id: u16,
}

/// The owner row of a plan whose owners are told apart by object id alone.
#[derive(Clone, Debug, PartialEq)]
pub struct ModelOwnerDraw {
    pub object_id: u16,
    pub model: String,
}

pub type MissileOwnerDraw = ModelOwnerDraw;
pub type ItemOwnerDraw = ModelOwnerDraw;
pub type DynEntOwnerDraw = ModelOwnerDraw;

#[derive(Clone, Debug)]
pub struct ItemAssetDraw {
    pub model: String,
    pub surfaces: Vec<(u32, u32)>,
}

#[derive(Clone, Debug)]
pub struct DynEntAssetDraw {
    pub key: MapXModelAssetKey,
    pub camera_lod: Option<u8>,
    pub surfaces: Vec<(u32, u32)>,
}

/// Declares a draw plan resource: the geometry, draw rows and revision fields
/// every plan shares, then the fields only this plan has, plus read-only views
/// of what the plan published. A consumer that could write the rows would make
/// the published revision less than the whole truth.
macro_rules! draw_plan {
    (
        $(#[$meta:meta])*
        $name:ident, draw: $draw:ty,
        $($(#[$field_meta:meta])* $field_vis:vis $field:ident: $field_ty:ty),* $(,)?
    ) => {
        $(#[$meta])*
        #[derive(Resource, Clone, Debug, Default)]
        pub struct $name {
            pub(crate) vertices: Vec<SmodelVertex>,
            pub(crate) indices: Vec<u32>,
            pub(crate) surface_ranges: Vec<(u32, u32)>,
            pub(crate) materials: Vec<SmodelPassMaterial>,
            pub(crate) draws: Vec<$draw>,
            pub(crate) packed_vertices: PackedVertexPayload,
            pub revision: u64,
            pub generation: u64,
            pub revisions: SourceRevisions,
            $($(#[$field_meta])* $field_vis $field: $field_ty,)*
        }

        impl $name {
            pub fn draws(&self) -> &[$draw] {
                &self.draws
            }

            pub fn materials(&self) -> &[SmodelPassMaterial] {
                &self.materials
            }

            pub fn vertices(&self) -> &[SmodelVertex] {
                &self.vertices
            }

            pub fn indices(&self) -> &[u32] {
                &self.indices
            }

            pub fn surface_ranges(&self) -> &[(u32, u32)] {
                &self.surface_ranges
            }

            pub fn packed_vertices(&self) -> &PackedVertexPayload {
                &self.packed_vertices
            }
        }
    };
}

/// The rows a producer rebuilds every frame can be handed to their plan, which
/// answers whether anything moved; no consumer re-hashes the rows to find out.
macro_rules! frame_published {
    ($plan:ty, $owner:ty) => {
        impl $plan {
            pub fn publish_frame_rows(
                &mut self,
                draws: &mut Vec<XModelSurfaceDraw>,
                owners: &mut Vec<$owner>,
            ) {
                let draws_moved = publish_rows(&mut self.draws, draws);
                let owners_moved = publish_rows(&mut self.owners, owners);
                if draws_moved || owners_moved {
                    note_rows_moved(&mut self.revisions, &mut self.revision);
                }
            }

            /// The producer found nothing to draw this frame. An early return
            /// may not leave last frame's rows published under this frame's
            /// revision, so this counts like a rebuild that came back empty.
            pub fn publish_no_rows(&mut self) {
                if self.draws.is_empty() && self.owners.is_empty() {
                    return;
                }
                self.draws.clear();
                self.owners.clear();
                self.revisions.bump_admission();
                note_rows_moved(&mut self.revisions, &mut self.revision);
            }
        }
    };
}

/// Turns the lookups the lighting pass answered into the plan's draw rows.
macro_rules! lit_from_table {
    ($plan:ty, $owner_id:expr) => {
        impl $plan {
            /// Seats the lighting every draw asked for. Draws whose lookup
            /// failed are dropped together with their owners; their object ids
            /// are returned.
            pub fn finalize_lighting(&mut self, table: &ResolvedModelLightingTable) -> Vec<u16> {
                seat_lighting(
                    table,
                    &mut self.draws,
                    &mut self.owners,
                    &mut self.revisions,
                    $owner_id,
                )
            }
        }
    };
}

fn note_rows_moved(revisions: &mut SourceRevisions, revision: &mut u64) {
    revisions.bump_draws();
    *revision = revision.wrapping_add(1);
}

draw_plan!(
    RemoteBodyDrawPlan, draw: RemoteBodySurfaceDraw,
    /// How many vertices the packed rows carry (the float vertices are not
    /// kept for bodies).
    pub(crate) decoded_n: usize,
    pub last_packed_id: Option<u64>,
    pub last_draw_id: Option<u64>,
);

draw_plan!(
    ScriptModelDrawPlan, draw: XModelSurfaceDraw,
    pub(crate) assets: Vec<ScriptModelAssetDraw>,
    pub(crate) owners: Vec<ScriptModelOwnerDraw>,
);

draw_plan!(
    MissileDrawPlan, draw: XModelSurfaceDraw,
    pub(crate) owners: Vec<MissileOwnerDraw>,
);

draw_plan!(
    ItemDrawPlan, draw: XModelSurfaceDraw,
    pub(crate) assets: Vec<ItemAssetDraw>,
    pub(crate) owners: Vec<ItemOwnerDraw>,
);

draw_plan!(
    DynEntDrawPlan, draw: XModelSurfaceDraw,
    pub(crate) assets: Vec<DynEntAssetDraw>,
    pub(crate) owners: Vec<DynEntOwnerDraw>,
);

frame_published!(ScriptModelDrawPlan, ScriptModelOwnerDraw);
frame_published!(ItemDrawPlan, ItemOwnerDraw);
frame_published!(DynEntDrawPlan, DynEntOwnerDraw);

lit_from_table!(ScriptModelDrawPlan, |owner: &ScriptModelOwnerDraw| owner
    .object_id);
lit_from_table!(MissileDrawPlan, |owner: &MissileOwnerDraw| owner.object_id);
lit_from_table!(ItemDrawPlan, |owner: &ItemOwnerDraw| owner.object_id);
lit_from_table!(DynEntDrawPlan, |owner: &DynEntOwnerDraw| owner.object_id);

impl RemoteBodyDrawPlan {
    pub fn decoded_n(&self) -> usize {
        self.decoded_n
    }

    /// Drops everything this plan published. Dropping rows is a change like any
    /// other: a consumer that kept last frame's revision would go on drawing
    /// bodies this plan no longer has.
    pub fn clear_geometry(&mut self) {
        let had_rows = !self.draws.is_empty() || self.decoded_n != 0;
        self.vertices.clear();
        self.indices.clear();
        self.surface_ranges.clear();
        self.materials.clear();
        self.draws.clear();
        self.decoded_n = 0;
        self.last_packed_id = None;
        self.last_draw_id = None;
        self.packed_vertices = PackedVertexPayload::Unavailable {
            source_layout: BODY_PACKED_UNAVAILABLE,
        };
        if !had_rows {
            return;
        }
        self.revisions.set_topology_from(&[], &[], 0);
        self.revisions.bump_surfaces();
        self.revisions.bump_admission();
        note_rows_moved(&mut self.revisions, &mut self.revision);
    }
}

impl ScriptModelDrawPlan {
    /// The asset already in the plan with this key, draw state and camera lods.
    pub fn asset_index(
        &self,
        key: &MapXModelAssetKey,
        dobj_state: &xmodel_runtime::DObjSemanticState,
        camera_lods: &[Option<u8>],
    ) -> Option<usize> {
        self.assets.iter().position(|asset| {
            &asset.key == key && &asset.dobj_state == dobj_state && asset.camera_lods == camera_lods
        })
    }
}

impl DynEntDrawPlan {
    pub fn asset(
        &self,
        key: &MapXModelAssetKey,
        camera_lod: Option<u8>,
    ) -> Option<&DynEntAssetDraw> {
        self.assets
            .iter()
            .find(|asset| &asset.key == key && asset.camera_lod == camera_lod)
    }
}

impl MissileDrawPlan {
    /// Empties a staging plan for this frame's rebuild, keeping its allocations.
    pub fn clear_rebuild(&mut self) {
        self.vertices.clear();
        self.indices.clear();
        self.surface_ranges.clear();
        self.materials.clear();
        self.packed_vertices = PackedVertexPayload::Unavailable {
            source_layout: crate::XMODEL_PACKED_UNAVAILABLE,
        };
    }

    /// Takes the rebuild and reports what moved. Vertex bytes are not compared:
    /// these models are posed by `world_from_local`, so a different mesh always
    /// shows up in the index layout, the surface ranges, the materials or the
    /// owner list.
    pub fn publish_rebuild(
        &mut self,
        staged: &mut Self,
        draws: &mut Vec<XModelSurfaceDraw>,
        owners: &mut Vec<MissileOwnerDraw>,
    ) {
        // Every comparison runs: each one hands its rows over when they differ.
        let materials_moved = publish_rows(&mut self.materials, &mut staged.materials);
        let layout_moved = publish_rows(&mut self.indices, &mut staged.indices)
            | publish_rows(&mut self.surface_ranges, &mut staged.surface_ranges)
            | (self.vertices.len() != staged.vertices.len());
        let geometry_moved = layout_moved || materials_moved;

        if materials_moved {
            self.revisions.bump_materials();
        }
        if geometry_moved {
            std::mem::swap(&mut self.vertices, &mut staged.vertices);
            self.packed_vertices = std::mem::replace(
                &mut staged.packed_vertices,
                PackedVertexPayload::Unavailable {
                    source_layout: crate::XMODEL_PACKED_UNAVAILABLE,
                },
            );
            self.revisions.bump_vertices();
            self.revisions.set_topology_from(
                &self.indices,
                &self.surface_ranges,
                self.vertices.len(),
            );
        }
        staged.vertices.clear();

        let draws_moved = publish_rows(&mut self.draws, draws);
        let owners_moved = publish_rows(&mut self.owners, owners);
        if geometry_moved || draws_moved || owners_moved {
            note_rows_moved(&mut self.revisions, &mut self.revision);
        }
    }
}

/// A hash of the index layout, which a consumer compares to learn whether the
/// draw ranges changed.
pub fn topology_fingerprint(indices: &[u32], ranges: &[(u32, u32)], decoded_n: usize) -> u64 {
    let mut scratch = SourceRevisions::default();
    scratch.set_topology_from(indices, ranges, decoded_n);
    scratch.topology
}

/// Records a plan's new topology. A changed topology counts as a draw change;
/// the returned revision is the plan's, advanced when it moved.
pub fn stamp_plan_geometry(revisions: &mut SourceRevisions, revision: u64, topology: u64) -> u64 {
    if revisions.topology == topology {
        return revision;
    }
    revisions.topology = topology;
    revisions.bump_draws();
    revision.wrapping_add(1)
}

fn seat_lighting<O>(
    table: &ResolvedModelLightingTable,
    draws: &mut Vec<XModelSurfaceDraw>,
    owners: &mut Vec<O>,
    revisions: &mut SourceRevisions,
    owner_id: impl Fn(&O) -> u16,
) -> Vec<u16> {
    let mut failed = Vec::new();
    let mut moved = false;
    for draw in draws.iter_mut() {
        let Some(request) = draw.pending_lighting.take() else {
            continue;
        };
        match table.get(request.owner) {
            Some(ResolvedModelLighting::Seated {
                handle,
                scene_light_index,
                reflection_probe_index,
                packed_lighting,
            }) => {
                let seated = (
                    handle,
                    scene_light_index,
                    reflection_probe_index,
                    packed_lighting,
                );
                let held = (
                    draw.lighting_handle,
                    draw.scene_light_index,
                    draw.reflection_probe_index,
                    draw.packed_lighting,
                );
                moved |= seated != held;
                (
                    draw.lighting_handle,
                    draw.scene_light_index,
                    draw.reflection_probe_index,
                    draw.packed_lighting,
                ) = seated;
            }
            Some(ResolvedModelLighting::Failed) | None => failed.push(draw.object_id),
        }
    }
    if !failed.is_empty() {
        draws.retain(|draw| !failed.contains(&draw.object_id));
        owners.retain(|owner| !failed.contains(&owner_id(owner)));
        revisions.bump_admission();
        moved = true;
    }
    if moved {
        revisions.bump_draws();
    }
    failed
}

/// bo2zm: one frame of the first-person gun and arms, as the Black Ops II
/// fallback draw reads it: skinned packed rows (legacy layout, local space),
/// indices, and per draw (first index, index count, the material's catalog
/// index).
pub struct FpvFallbackRows<'a> {
    pub rows: &'a [[u8; T6_PACKED_VERTEX]],
    pub indices: &'a [u32],
    pub draws: Vec<(u32, u32, Option<u32>)>,
}

#[derive(Resource, Clone, Debug, Default)]
pub struct FpvDrawPlan {
    /// How many vertices the packed rows carry.
    pub(crate) decoded_n: usize,
    pub(crate) indices: Vec<u32>,
    pub(crate) surface_ranges: Vec<(u32, u32)>,
    pub(crate) materials: Vec<SmodelPassMaterial>,
    pub(crate) draws: Vec<FpvSurfaceDraw>,
    pub(crate) packed_vertices: PackedVertexPayload,

    pub world_from_local: Mat4,
    pub lighting_handle: u32,
    pub visible: bool,

    pub hands_plan_n: Option<u32>,
    pub gun_plan_n: Option<u32>,
    pub gun_colormap_skip_n: Option<u32>,
    pub gun_ordinal_skip_n: Option<u32>,
    pub gun_colormap_skip_names: Option<String>,
    pub scope_plan_n: Option<u32>,
    pub scope_house_plan_n: Option<u32>,
    pub scope_lens_plan_n: Option<u32>,
    pub plan_draw_n: Option<u32>,
    pub plan_mat_hints: Option<String>,
    pub plan_skip_n: Option<u32>,
    pub drawgun: Option<i32>,

    pub scene_light_index: u8,
    pub reflection_probe_index: u8,

    pub revision: u64,
    pub generation: u64,
    pub revisions: SourceRevisions,

    /// Whether this frame's placement matrix is real. Placement and geometry
    /// are published by two independent systems, each stating its own half for
    /// `settle_visible` to read.
    pub placement_ok: bool,

    /// Whether the rig skinned rows into this plan this frame.
    pub geometry_ok: bool,

    /// Which prepared rig the rows came from. The rig publishes them once; a
    /// frame that still answers this generation publishes vertices.
    pub rig_generation: u64,
}

impl FpvDrawPlan {
    pub fn draws(&self) -> &[FpvSurfaceDraw] {
        &self.draws
    }

    pub fn materials(&self) -> &[SmodelPassMaterial] {
        &self.materials
    }

    pub fn indices(&self) -> &[u32] {
        &self.indices
    }

    pub fn surface_ranges(&self) -> &[(u32, u32)] {
        &self.surface_ranges
    }

    pub fn packed_vertices(&self) -> &PackedVertexPayload {
        &self.packed_vertices
    }

    pub fn decoded_n(&self) -> usize {
        self.decoded_n
    }

    /// bo2zm: see [`FpvFallbackRows`]; `None` while the plan has no skinned
    /// rows this frame.
    pub fn fallback_rows(&self) -> Option<FpvFallbackRows<'_>> {
        let PackedVertexPayload::T6(rows) = &self.packed_vertices else {
            return None;
        };
        if rows.is_empty() || self.indices.is_empty() {
            return None;
        }
        let draws = self
            .draws
            .iter()
            .filter_map(|draw| {
                let &(first, count) = self.surface_ranges.get(draw.surface as usize)?;
                let material = self
                    .materials
                    .get(draw.material as usize)
                    .and_then(|material| material.t6_material);
                Some((first, count, material))
            })
            .collect();
        Some(FpvFallbackRows {
            rows,
            indices: &self.indices,
            draws,
        })
    }

    /// The rows this frame skinned into. `None` once the plan has nothing
    /// packed to write — an empty plan, or a composition whose models never
    /// carried packed vertices.
    pub fn packed_rows_mut(&mut self) -> Option<&mut [[u8; T6_PACKED_VERTEX]]> {
        match &mut self.packed_vertices {
            PackedVertexPayload::T6(rows) => Some(rows),
            PackedVertexPayload::Unavailable { .. } => None,
        }
    }

    #[must_use]
    pub fn admits_colour(&self) -> bool {
        self.visible && self.lighting_handle != 0 && self.drawgun != Some(0)
    }

    /// Both halves have to hold: a plan with rows nobody can place is no more
    /// drawable than a placement with nothing to draw.
    pub fn settle_visible(&mut self) {
        self.visible = self.geometry_ok && self.placement_ok && self.drawgun != Some(0);
    }

    /// Takes the eye's lighting, or hides the viewmodel when it has none.
    pub fn finalize_lighting(&mut self, table: &ResolvedModelLightingTable) {
        let was_visible = self.visible;
        let held = (
            self.lighting_handle,
            self.scene_light_index,
            self.reflection_probe_index,
        );
        match table.get(ModelLightingOwner::Eye) {
            Some(ResolvedModelLighting::Seated {
                handle,
                scene_light_index,
                reflection_probe_index,
                ..
            }) => {
                self.lighting_handle = handle;
                self.scene_light_index = scene_light_index;
                self.reflection_probe_index = reflection_probe_index;
                if !self.draws.is_empty() {
                    self.settle_visible();
                }
            }
            Some(ResolvedModelLighting::Failed) | None => {
                self.lighting_handle = 0;
                self.scene_light_index = 0;
                self.reflection_probe_index = 0;
                self.visible = false;
            }
        }
        let visibility_moved = was_visible != self.visible;
        if visibility_moved {
            self.revisions.bump_admission();
        }
        let seated = (
            self.lighting_handle,
            self.scene_light_index,
            self.reflection_probe_index,
        );
        if visibility_moved || held != seated {
            self.revisions.bump_draws();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use render_scene::model_lighting::ModelLightingRequest;

    fn pass_material(sort_key: u8) -> SmodelPassMaterial {
        SmodelPassMaterial {
            model_lighting_required: false,
            color: None,
            specular: None,
            probe: None,
            atlas: None,
            alpha_mode: AlphaMode::Opaque,
            draw_mode: None,
            cull_mode: None,
            env_map_parms: [0.0; 4],
            lighting_lookup_scale: [0.0; 4],
            atlas_lookup: [0.0; 4],
            sort_key,
            material_sorted_index: None,
            t6_material: None,
        }
    }

    fn surface_draw(object_id: u16, asks: Option<ModelLightingOwner>) -> XModelSurfaceDraw {
        XModelSurfaceDraw {
            surface: 0,
            material: 0,
            world_from_local: Mat4::IDENTITY,
            lighting_handle: 0,
            pending_lighting: asks.map(|owner| ModelLightingRequest {
                owner,
                origin: [0.0; 3],
                lookup_fallback: 0,
            }),
            colour_refusal: None,
            object_id,
            scene_light_index: 0,
            reflection_probe_index: 0,
            packed_lighting: None,
            is_scope: false,
            scene_entnum: None,
            caster_bound: None,
        }
    }

    fn owner(object_id: u16) -> ModelOwnerDraw {
        ModelOwnerDraw {
            object_id,
            model: format!("model{object_id}"),
        }
    }

    fn seated(handle: u32) -> ResolvedModelLighting {
        ResolvedModelLighting::Seated {
            handle,
            scene_light_index: 3,
            reflection_probe_index: 4,
            packed_lighting: Some([1, 2, 3, 4]),
        }
    }

    fn payload_is_unavailable(payload: &PackedVertexPayload, why: &str) -> bool {
        matches!(payload, PackedVertexPayload::Unavailable { source_layout } if *source_layout == why)
    }

    #[test]
    fn a_topology_fingerprint_follows_every_input() {
        let base = topology_fingerprint(&[0, 1, 2], &[(0, 3)], 3);
        assert_eq!(base, topology_fingerprint(&[0, 1, 2], &[(0, 3)], 3));
        assert_ne!(base, topology_fingerprint(&[0, 2, 1], &[(0, 3)], 3));
        assert_ne!(base, topology_fingerprint(&[0, 1, 2], &[(0, 2)], 3));
        assert_ne!(base, topology_fingerprint(&[0, 1, 2], &[(0, 3)], 4));
    }

    #[test]
    fn a_known_topology_leaves_the_plan_alone() {
        let mut revisions = SourceRevisions {
            topology: 9,
            ..Default::default()
        };
        assert_eq!(stamp_plan_geometry(&mut revisions, 5, 9), 5);
        assert_eq!(
            revisions,
            SourceRevisions {
                topology: 9,
                ..Default::default()
            }
        );
    }

    #[test]
    fn a_new_topology_counts_as_a_draw_change() {
        let mut revisions = SourceRevisions::default();
        assert_eq!(stamp_plan_geometry(&mut revisions, 5, 9), 6);
        assert_eq!((revisions.topology, revisions.draws), (9, 1));
        assert_eq!(stamp_plan_geometry(&mut revisions, u64::MAX, 10), 0);
    }

    #[test]
    fn clearing_an_empty_body_plan_publishes_nothing() {
        let mut plan = RemoteBodyDrawPlan::default();
        plan.clear_geometry();
        assert_eq!(plan.revision, 0);
        assert_eq!(plan.revisions, SourceRevisions::default());
        assert!(payload_is_unavailable(
            plan.packed_vertices(),
            BODY_PACKED_UNAVAILABLE
        ));
    }

    #[test]
    fn clearing_a_body_plan_with_rows_is_a_change() {
        let mut plan = RemoteBodyDrawPlan {
            decoded_n: 4,
            indices: vec![0, 1, 2],
            last_packed_id: Some(7),
            last_draw_id: Some(8),
            ..Default::default()
        };
        plan.clear_geometry();
        assert_eq!(plan.decoded_n(), 0);
        assert!(plan.indices().is_empty());
        assert_eq!((plan.last_packed_id, plan.last_draw_id), (None, None));
        assert_eq!(plan.revision, 1);
        assert_eq!(plan.revisions.draws, 1);
        assert_eq!(plan.revisions.admission, 1);
        assert_eq!(plan.revisions.topology, topology_fingerprint(&[], &[], 0));
    }

    #[test]
    fn script_model_assets_are_found_by_key_state_and_lods() {
        let key = MapXModelAssetKey("crate".to_owned());
        let state = xmodel_runtime::DObjSemanticState::bind_pose("crate".to_owned(), 1, 1);
        let mut plan = ScriptModelDrawPlan::default();
        plan.assets.push(ScriptModelAssetDraw {
            key: key.clone(),
            dobj_state: state.clone(),
            camera_lods: vec![Some(1), None],
            surfaces: vec![(0, 0)],
        });
        assert_eq!(plan.asset_index(&key, &state, &[Some(1), None]), Some(0));
        assert_eq!(plan.asset_index(&key, &state, &[Some(1)]), None);
        let other = MapXModelAssetKey("barrel".to_owned());
        assert_eq!(plan.asset_index(&other, &state, &[Some(1), None]), None);
    }

    #[test]
    fn dynent_assets_are_found_by_key_and_lod() {
        let key = MapXModelAssetKey("door".to_owned());
        let mut plan = DynEntDrawPlan::default();
        plan.assets.push(DynEntAssetDraw {
            key: key.clone(),
            camera_lod: Some(2),
            surfaces: vec![(1, 0)],
        });
        assert_eq!(
            plan.asset(&key, Some(2)).map(|a| a.surfaces.clone()),
            Some(vec![(1, 0)])
        );
        assert!(plan.asset(&key, None).is_none());
    }

    #[test]
    fn unchanged_frame_rows_publish_nothing() {
        let mut plan = ItemDrawPlan::default();
        plan.publish_frame_rows(&mut vec![surface_draw(1, None)], &mut vec![owner(1)]);
        assert_eq!((plan.revision, plan.revisions.draws), (1, 1));

        let mut draws = vec![surface_draw(1, None)];
        let mut owners = vec![owner(1)];
        plan.publish_frame_rows(&mut draws, &mut owners);
        assert_eq!((plan.revision, plan.revisions.draws), (1, 1));
        assert!(
            draws.is_empty() && owners.is_empty(),
            "rebuild buffers come back empty"
        );
    }

    #[test]
    fn a_changed_owner_alone_moves_the_frame() {
        let mut plan = DynEntDrawPlan::default();
        plan.publish_frame_rows(&mut vec![surface_draw(1, None)], &mut vec![owner(1)]);
        plan.publish_frame_rows(&mut vec![surface_draw(1, None)], &mut vec![owner(2)]);
        assert_eq!(plan.revision, 2);
        assert_eq!(plan.owners, [owner(2)]);
    }

    #[test]
    fn no_rows_is_a_change_only_once() {
        let mut plan = ScriptModelDrawPlan::default();
        plan.publish_no_rows();
        assert_eq!(plan.revision, 0);

        plan.draws.push(surface_draw(1, None));
        plan.publish_no_rows();
        assert_eq!(
            (
                plan.revision,
                plan.revisions.draws,
                plan.revisions.admission
            ),
            (1, 1, 1)
        );
        assert!(plan.draws().is_empty());

        plan.publish_no_rows();
        assert_eq!(plan.revision, 1);
    }

    fn staged_missile(indices: &[u32], sort_key: u8) -> MissileDrawPlan {
        MissileDrawPlan {
            vertices: vec![SmodelVertex {
                position: [0.0; 3],
                normal: [0.0, 0.0, 1.0],
                color: [1.0; 4],
                uv0: [0.0; 2],
            }],
            indices: indices.to_vec(),
            surface_ranges: vec![(0, indices.len() as u32)],
            materials: vec![pass_material(sort_key)],
            packed_vertices: PackedVertexPayload::T6(vec![[0; T6_PACKED_VERTEX]]),
            ..Default::default()
        }
    }

    #[test]
    fn a_rebuild_matching_the_published_plan_moves_nothing() {
        let mut plan = MissileDrawPlan::default();
        plan.publish_rebuild(
            &mut staged_missile(&[0, 0, 0], 1),
            &mut Vec::new(),
            &mut Vec::new(),
        );
        let before = (plan.revision, plan.revisions);

        let mut staged = staged_missile(&[0, 0, 0], 1);
        plan.publish_rebuild(&mut staged, &mut Vec::new(), &mut Vec::new());
        assert_eq!((plan.revision, plan.revisions), before);
        assert!(staged.vertices.is_empty());
    }

    #[test]
    fn a_new_layout_replaces_vertices_and_topology() {
        let mut plan = MissileDrawPlan::default();
        let mut staged = staged_missile(&[0, 0, 0], 1);
        plan.publish_rebuild(
            &mut staged,
            &mut vec![surface_draw(0x400, None)],
            &mut vec![owner(0x400)],
        );
        assert_eq!(plan.vertices().len(), 1);
        assert!(matches!(plan.packed_vertices(), PackedVertexPayload::T6(rows) if rows.len() == 1));
        assert!(payload_is_unavailable(
            &staged.packed_vertices,
            crate::XMODEL_PACKED_UNAVAILABLE
        ));
        assert_eq!(plan.revision, 1);
        assert_eq!(plan.revisions.vertices, 1);
        assert_eq!(plan.revisions.materials, 1);
        assert_eq!(
            plan.revisions.topology,
            topology_fingerprint(&[0, 0, 0], &[(0, 3)], 1)
        );
        assert_eq!(plan.draws().len(), 1);
    }

    #[test]
    fn a_material_change_alone_bumps_materials() {
        let mut plan = MissileDrawPlan::default();
        plan.publish_rebuild(
            &mut staged_missile(&[0, 0, 0], 1),
            &mut Vec::new(),
            &mut Vec::new(),
        );
        let before = plan.revisions;
        plan.publish_rebuild(
            &mut staged_missile(&[0, 0, 0], 2),
            &mut Vec::new(),
            &mut Vec::new(),
        );
        assert_eq!(plan.revisions.materials, before.materials + 1);
        assert_eq!(plan.materials()[0].sort_key, 2);
        assert_eq!(plan.revision, 2);
    }

    #[test]
    fn clearing_a_rebuild_keeps_the_published_rows_out() {
        let mut staged = staged_missile(&[0, 0, 0], 1);
        staged.clear_rebuild();
        assert!(staged.vertices.is_empty() && staged.indices.is_empty());
        assert!(staged.surface_ranges.is_empty() && staged.materials.is_empty());
        assert!(payload_is_unavailable(
            &staged.packed_vertices,
            crate::XMODEL_PACKED_UNAVAILABLE
        ));
    }

    #[test]
    fn seated_lighting_lands_on_the_draw_once() {
        let owner_key = ModelLightingOwner::Item(9);
        let mut plan = ItemDrawPlan::default();
        plan.draws.push(surface_draw(5, Some(owner_key)));
        plan.owners.push(owner(5));
        let mut table = ResolvedModelLightingTable::default();
        table.insert_if_absent(owner_key, seated(11));

        assert!(plan.finalize_lighting(&table).is_empty());
        let draw = plan.draws()[0];
        assert_eq!(
            (
                draw.lighting_handle,
                draw.scene_light_index,
                draw.reflection_probe_index,
                draw.packed_lighting
            ),
            (11, 3, 4, Some([1, 2, 3, 4]))
        );
        assert!(draw.pending_lighting.is_none());
        assert_eq!(plan.revisions.draws, 1);

        plan.draws[0].pending_lighting = Some(ModelLightingRequest {
            owner: owner_key,
            origin: [0.0; 3],
            lookup_fallback: 0,
        });
        plan.finalize_lighting(&table);
        assert_eq!(plan.revisions.draws, 1, "the same seat is not a change");
    }

    #[test]
    fn failed_lighting_drops_the_draw_and_its_owner() {
        let (kept_key, lost_key) = (
            ModelLightingOwner::Missile(1),
            ModelLightingOwner::Missile(2),
        );
        let mut plan = MissileDrawPlan::default();
        plan.draws.extend([
            surface_draw(1, Some(kept_key)),
            surface_draw(2, Some(lost_key)),
        ]);
        plan.owners.extend([owner(1), owner(2)]);
        let mut table = ResolvedModelLightingTable::default();
        table.insert_if_absent(kept_key, seated(6));
        table.insert_if_absent(lost_key, ResolvedModelLighting::Failed);

        assert_eq!(plan.finalize_lighting(&table), [2]);
        assert_eq!(
            plan.draws().iter().map(|d| d.object_id).collect::<Vec<_>>(),
            [1]
        );
        assert_eq!(plan.owners, [owner(1)]);
        assert_eq!(plan.revisions.admission, 1);
        assert_eq!(plan.revisions.draws, 1);
    }

    #[test]
    fn an_unresolved_lookup_counts_as_failed() {
        let mut plan = DynEntDrawPlan::default();
        plan.draws
            .push(surface_draw(7, Some(ModelLightingOwner::Glass(1))));
        plan.owners.push(owner(7));
        assert_eq!(
            plan.finalize_lighting(&ResolvedModelLightingTable::default()),
            [7]
        );
        assert!(plan.draws().is_empty());
    }

    #[test]
    fn script_model_owners_are_dropped_by_object_id() {
        let key = ModelLightingOwner::ScriptModel(Entity::from_raw_u32(3).expect("valid index"));
        let mut plan = ScriptModelDrawPlan::default();
        plan.draws.push(surface_draw(8, Some(key)));
        plan.owners.push(ScriptModelOwnerDraw {
            entity: Entity::from_raw_u32(3).expect("valid index"),
            current_model: MapXModelAssetKey("crate".to_owned()),
            object_id: 8,
        });
        assert_eq!(
            plan.finalize_lighting(&ResolvedModelLightingTable::default()),
            [8]
        );
        assert!(plan.owners.is_empty());
    }

    fn skinned_plan() -> FpvDrawPlan {
        let mut material = pass_material(0);
        material.t6_material = Some(42);
        FpvDrawPlan {
            decoded_n: 3,
            indices: vec![0, 1, 2, 2, 1, 0],
            surface_ranges: vec![(0, 3), (3, 3)],
            materials: vec![material, pass_material(0)],
            draws: vec![
                FpvSurfaceDraw {
                    surface: 1,
                    material: 0,
                    is_scope: false,
                },
                FpvSurfaceDraw {
                    surface: 0,
                    material: 1,
                    is_scope: true,
                },
                FpvSurfaceDraw {
                    surface: 9,
                    material: 0,
                    is_scope: false,
                },
            ],
            packed_vertices: PackedVertexPayload::T6(vec![[0; T6_PACKED_VERTEX]; 3]),
            ..Default::default()
        }
    }

    #[test]
    fn fallback_rows_follow_the_draws_that_have_a_range() {
        let plan = skinned_plan();
        let rows = plan.fallback_rows().expect("skinned");
        assert_eq!(rows.rows.len(), 3);
        assert_eq!(rows.indices.len(), 6);
        assert_eq!(rows.draws, [(3, 3, Some(42)), (0, 3, None)]);
    }

    #[test]
    fn no_fallback_rows_without_packed_rows_or_indices() {
        let mut plan = skinned_plan();
        plan.indices.clear();
        assert!(plan.fallback_rows().is_none());
        let mut plan = skinned_plan();
        plan.packed_vertices = PackedVertexPayload::T6(Vec::new());
        assert!(plan.fallback_rows().is_none());
        plan.packed_vertices = PackedVertexPayload::default();
        assert!(plan.fallback_rows().is_none());
    }

    #[test]
    fn packed_rows_are_writable_only_when_retained() {
        let mut plan = skinned_plan();
        plan.packed_rows_mut().expect("retained")[1] = [9; T6_PACKED_VERTEX];
        assert!(matches!(&plan.packed_vertices, PackedVertexPayload::T6(rows) if rows[1][0] == 9));
        plan.packed_vertices = PackedVertexPayload::default();
        assert!(plan.packed_rows_mut().is_none());
    }

    #[test]
    fn the_viewmodel_shows_only_when_placed_and_skinned() {
        let mut plan = FpvDrawPlan::default();
        plan.settle_visible();
        assert!(!plan.visible);
        plan.geometry_ok = true;
        plan.placement_ok = true;
        plan.settle_visible();
        assert!(plan.visible);
        plan.drawgun = Some(0);
        plan.settle_visible();
        assert!(!plan.visible);
    }

    #[test]
    fn colour_needs_a_visible_lit_viewmodel() {
        let mut plan = FpvDrawPlan {
            visible: true,
            lighting_handle: 2,
            ..Default::default()
        };
        assert!(plan.admits_colour());
        plan.drawgun = Some(0);
        assert!(!plan.admits_colour());
        plan.drawgun = Some(1);
        plan.lighting_handle = 0;
        assert!(!plan.admits_colour());
    }

    #[test]
    fn eye_lighting_shows_a_placed_viewmodel() {
        let mut table = ResolvedModelLightingTable::default();
        table.insert_if_absent(ModelLightingOwner::Eye, seated(5));
        let mut plan = skinned_plan();
        plan.geometry_ok = true;
        plan.placement_ok = true;
        plan.finalize_lighting(&table);
        assert!(plan.visible);
        assert_eq!(
            (
                plan.lighting_handle,
                plan.scene_light_index,
                plan.reflection_probe_index
            ),
            (5, 3, 4)
        );
        assert_eq!((plan.revisions.admission, plan.revisions.draws), (1, 1));

        plan.finalize_lighting(&table);
        assert_eq!((plan.revisions.admission, plan.revisions.draws), (1, 1));
    }

    #[test]
    fn missing_eye_lighting_hides_the_viewmodel() {
        let mut plan = skinned_plan();
        plan.visible = true;
        plan.lighting_handle = 5;
        plan.scene_light_index = 3;
        plan.finalize_lighting(&ResolvedModelLightingTable::default());
        assert!(!plan.visible);
        assert_eq!(
            (
                plan.lighting_handle,
                plan.scene_light_index,
                plan.reflection_probe_index
            ),
            (0, 0, 0)
        );
        assert_eq!((plan.revisions.admission, plan.revisions.draws), (1, 1));
    }

    #[test]
    fn eye_lighting_leaves_visibility_when_nothing_is_drawn() {
        let mut table = ResolvedModelLightingTable::default();
        table.insert_if_absent(ModelLightingOwner::Eye, seated(5));
        let mut plan = FpvDrawPlan {
            visible: true,
            ..Default::default()
        };
        plan.finalize_lighting(&table);
        assert!(plan.visible);
        assert_eq!(plan.revisions.admission, 0);
    }
}
