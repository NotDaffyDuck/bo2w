//! Builders that fold posed model surfaces into the draw plans, and the pass
//! materials those surfaces draw with.

use asset_t6::T6_PACKED_VERTEX;
use asset_world::{MapXModelAssetKey, PackedVertexPayload};
use bevy::prelude::*;
use render_frame::SmodelVertex;
use render_material::RuntimeMaterialCatalog;
use render_scene::{SmodelPassMaterial, WorldModelLightingAtlas};

use crate::anim::fpv_pose::PosedModelSurface;
use crate::anim::fpv_rig::PreparedFpvGeometry;
use crate::draw::{
    BODY_PACKED_UNAVAILABLE, DynEntAssetDraw, DynEntDrawPlan, FPV_PACKED_EMPTY_PLAN,
    FPV_PACKED_UNAVAILABLE, FpvDrawPlan, ItemDrawPlan, MissileDrawPlan, RemoteBodyDrawPlan,
    RemoteBodySurfaceDraw, ScriptModelAssetDraw, ScriptModelDrawPlan, stamp_plan_geometry,
    topology_fingerprint,
};
use crate::geometry::{append_mesh, install_retained_packed, mesh_index_count, mesh_vertices};

pub const XMODEL_PACKED_UNAVAILABLE: &str =
    "xmodel merge GfxPackedVertex missing or count-mismatched; decoded float is not packed VB";
pub const XMODEL_PACKED_EMPTY_PLAN: &str = "xmodel plan has no vertices";

// ---------------------------------------------------------------------------
// Pass materials
// ---------------------------------------------------------------------------

/// A model material that reads the world's lighting atlas, for the material
/// at `ordinal` in the sorted catalog. `None` when the atlas has no usable
/// height.
fn lit_xmodel_pass_material(
    lighting: &WorldModelLightingAtlas,
    ordinal: u32,
) -> Option<SmodelPassMaterial> {
    use lighting_t6::{
        MODEL_LIGHTING_INV_ATLAS_WIDTH, MODEL_LIGHTING_VOLUME_W, model_lighting_inv_image_height,
        model_lighting_lookup_scale,
    };
    let inv_height = model_lighting_inv_image_height(lighting.dims.image_height)?;
    let scale = model_lighting_lookup_scale(inv_height);
    Some(SmodelPassMaterial {
        model_lighting_required: true,
        color: None,
        specular: None,
        probe: None,
        atlas: Some(lighting.image.clone()),
        alpha_mode: AlphaMode::Opaque,
        draw_mode: None,
        cull_mode: None,
        env_map_parms: [0.0; 4],
        lighting_lookup_scale: [scale.u, scale.v, scale.w, scale.q],
        atlas_lookup: [
            MODEL_LIGHTING_INV_ATLAS_WIDTH as f32,
            inv_height,
            MODEL_LIGHTING_VOLUME_W,
            0.0,
        ],
        sort_key: 0,
        material_sorted_index: Some(ordinal),
        t6_material: None,
    })
}

/// The lit material for a material known by name.
pub fn body_lit_pass_material(
    lighting: &WorldModelLightingAtlas,
    catalog: &RuntimeMaterialCatalog,
    material_name: &str,
) -> Option<SmodelPassMaterial> {
    let ordinal = catalog.ordinal_for_material_name(material_name)?.get();
    lit_xmodel_pass_material(lighting, ordinal)
}

/// The lit material for a material already bound in the asset catalog.
pub fn bound_lit_xmodel_pass_material(
    lighting: &WorldModelLightingAtlas,
    catalog: &RuntimeMaterialCatalog,
    material_index: assets::MaterialIndex,
) -> Option<SmodelPassMaterial> {
    let ordinal = catalog.ordinal_for_asset_id(material_index)?.get();
    lit_xmodel_pass_material(lighting, ordinal)
}

/// The lit material for an authored one: the lit defaults, with the maps,
/// blend and cull state and sort key the authored material carries.
pub fn authored_lit_xmodel_pass_material(
    lighting: &WorldModelLightingAtlas,
    catalog: &RuntimeMaterialCatalog,
    image_handles: &[Option<Handle<Image>>],
    authored: assets::MaterialIndex,
) -> Option<SmodelPassMaterial> {
    let derived = catalog.derived(authored)?;
    let ordinal = catalog.ordinal_for_asset_id(authored)?.get();
    let maps = render_scene::runtime_maps(Some(authored), catalog, image_handles);
    Some(SmodelPassMaterial {
        color: maps.color,
        specular: maps.specular,
        alpha_mode: maps.alpha_mode,
        draw_mode: maps.draw_mode,
        cull_mode: maps.cull_mode,
        env_map_parms: maps.env_map_parms,
        sort_key: derived.sort_key,
        ..lit_xmodel_pass_material(lighting, ordinal)?
    })
}

// ---------------------------------------------------------------------------
// Packed rows
// ---------------------------------------------------------------------------

/// The packed rows being collected next to a plan's decoded vertices. They are
/// kept only while every surface supplied exactly one row per vertex.
struct PackedRows {
    intact: bool,
    rows: Vec<[u8; T6_PACKED_VERTEX]>,
}

impl PackedRows {
    /// Takes the plan's rows out to extend them. A plan that already has
    /// vertices but no rows has nothing to extend.
    fn take(
        payload: &mut PackedVertexPayload,
        no_vertices: bool,
        placeholder: &'static str,
    ) -> Self {
        let old = std::mem::replace(
            payload,
            PackedVertexPayload::Unavailable {
                source_layout: placeholder,
            },
        );
        match old {
            PackedVertexPayload::T6(rows) => Self { intact: true, rows },
            PackedVertexPayload::Unavailable { .. } => Self {
                intact: no_vertices,
                rows: Vec::new(),
            },
        }
    }

    /// Adds one surface's rows, which have to match the `decoded` vertices it
    /// added; a surface without them spoils the whole set.
    fn extend(&mut self, rows: &[[u8; T6_PACKED_VERTEX]], decoded: usize) {
        if self.intact && rows.len() == decoded {
            self.rows.extend_from_slice(rows);
        } else {
            self.intact = false;
            self.rows.clear();
        }
    }

    fn spoil(&mut self) {
        self.intact = false;
        self.rows.clear();
    }

    fn into_payload(
        self,
        decoded_count: usize,
        empty: &'static str,
        missing: &'static str,
    ) -> PackedVertexPayload {
        install_retained_packed(self.intact, self.rows, decoded_count, empty, missing)
    }
}

// ---------------------------------------------------------------------------
// Plans that hold decoded vertices
// ---------------------------------------------------------------------------

/// The buffers every vertex-holding plan keeps its geometry in.
struct Geometry<'a> {
    vertices: &'a mut Vec<SmodelVertex>,
    indices: &'a mut Vec<u32>,
    surface_ranges: &'a mut Vec<(u32, u32)>,
    materials: &'a mut Vec<SmodelPassMaterial>,
    packed: &'a mut PackedVertexPayload,
}

trait HoldsGeometry {
    fn geometry(&mut self) -> Geometry<'_>;
}

macro_rules! holds_geometry {
    ($($plan:ty),+) => {$(
        impl HoldsGeometry for $plan {
            fn geometry(&mut self) -> Geometry<'_> {
                Geometry {
                    vertices: &mut self.vertices,
                    indices: &mut self.indices,
                    surface_ranges: &mut self.surface_ranges,
                    materials: &mut self.materials,
                    packed: &mut self.packed_vertices,
                }
            }
        }
    )+};
}

holds_geometry!(
    ScriptModelDrawPlan,
    MissileDrawPlan,
    ItemDrawPlan,
    DynEntDrawPlan
);

/// Appends every surface that has a material and a usable mesh, and returns
/// each one as `(surface range, material)` slots of the plan.
fn append_surfaces(
    plan: &mut impl HoldsGeometry,
    surfaces: &[PosedModelSurface],
    materials: &[Option<SmodelPassMaterial>],
) -> Vec<(u32, u32)> {
    let geometry = plan.geometry();
    let mut packed = PackedRows::take(
        geometry.packed,
        geometry.vertices.is_empty(),
        XMODEL_PACKED_UNAVAILABLE,
    );
    let mut slots = Vec::new();
    for (surface, material) in surfaces.iter().zip(materials) {
        let Some(material) = material else { continue };
        let before = geometry.vertices.len();
        let Some(range) = append_mesh(&surface.mesh, geometry.vertices, geometry.indices) else {
            continue;
        };
        packed.extend(&surface.packed_vertices, geometry.vertices.len() - before);
        slots.push((
            geometry.surface_ranges.len() as u32,
            geometry.materials.len() as u32,
        ));
        geometry.surface_ranges.push(range);
        geometry.materials.push(material.clone());
    }
    *geometry.packed = packed.into_payload(
        geometry.vertices.len(),
        XMODEL_PACKED_EMPTY_PLAN,
        XMODEL_PACKED_UNAVAILABLE,
    );
    slots
}

pub fn append_missile_surfaces(
    plan: &mut MissileDrawPlan,
    surfaces: &[PosedModelSurface],
    materials: &[Option<SmodelPassMaterial>],
) -> Vec<(u32, u32)> {
    let slots = append_surfaces(plan, surfaces, materials);
    plan.revisions.bump_surfaces();
    slots
}

pub fn append_item_surfaces(
    plan: &mut ItemDrawPlan,
    surfaces: &[PosedModelSurface],
    materials: &[Option<SmodelPassMaterial>],
) -> Vec<(u32, u32)> {
    let slots = append_surfaces(plan, surfaces, materials);
    plan.revisions.bump_surfaces();
    slots
}

pub fn append_dynent_surfaces(
    plan: &mut DynEntDrawPlan,
    surfaces: &[PosedModelSurface],
    materials: &[Option<SmodelPassMaterial>],
) -> Vec<(u32, u32)> {
    let slots = append_surfaces(plan, surfaces, materials);
    plan.revisions.bump_surfaces();
    slots
}

pub fn append_dynent_asset(
    plan: &mut DynEntDrawPlan,
    key: MapXModelAssetKey,
    camera_lod: Option<u8>,
    surfaces: &[PosedModelSurface],
    materials: &[Option<SmodelPassMaterial>],
) -> Vec<(u32, u32)> {
    let slots = append_dynent_surfaces(plan, surfaces, materials);
    plan.assets.push(DynEntAssetDraw {
        key,
        camera_lod,
        surfaces: slots.clone(),
    });
    slots
}

// ---------------------------------------------------------------------------
// Script models
// ---------------------------------------------------------------------------

/// Adds a script model's surfaces as a new asset of the plan and returns its
/// index.
pub fn append_script_model_asset(
    plan: &mut ScriptModelDrawPlan,
    key: MapXModelAssetKey,
    dobj_state: xmodel_runtime::DObjSemanticState,
    camera_lods: Vec<Option<u8>>,
    surfaces: &[PosedModelSurface],
    materials: &[Option<SmodelPassMaterial>],
) -> usize {
    let slots = append_surfaces(plan, surfaces, materials);
    plan.assets.push(ScriptModelAssetDraw {
        key,
        dobj_state,
        camera_lods,
        surfaces: slots,
    });
    plan.revisions.bump_surfaces();
    restamp_topology(plan);
    plan.assets.len() - 1
}

fn restamp_topology(plan: &mut ScriptModelDrawPlan) {
    let topology = topology_fingerprint(&plan.indices, &plan.surface_ranges, plan.vertices.len());
    plan.revision = stamp_plan_geometry(&mut plan.revisions, plan.revision, topology);
}

/// The vertices a surface of the plan reaches: the lowest and highest vertex
/// its indices name, and the indices themselves.
struct SurfaceSpan<'a> {
    indices: &'a [u32],
    first_vertex: u32,
    last_vertex: u32,
}

impl SurfaceSpan<'_> {
    fn vertex_count(&self) -> usize {
        (self.last_vertex - self.first_vertex) as usize + 1
    }
}

fn surface_span(plan: &ScriptModelDrawPlan, surface: u32) -> Option<SurfaceSpan<'_>> {
    let &(first, count) = plan.surface_ranges.get(surface as usize)?;
    let first = first as usize;
    let indices = plan
        .indices
        .get(first..first.saturating_add(count as usize))?;
    Some(SurfaceSpan {
        indices,
        first_vertex: *indices.iter().min()?,
        last_vertex: *indices.iter().max()?,
    })
}

/// What overwriting one surface of an asset changes, checked before anything
/// is touched.
struct SurfaceOverwrite<'a> {
    first_vertex: usize,
    vertices: Vec<SmodelVertex>,
    packed: &'a [[u8; T6_PACKED_VERTEX]],
    material_slot: usize,
    material: &'a SmodelPassMaterial,
}

/// Rewrites the vertices, packed rows and materials of an asset in place, for a
/// pose that kept the asset's layout. Nothing is changed unless every surface
/// still lines up with the slots the asset was added with.
pub fn overwrite_script_model_asset(
    plan: &mut ScriptModelDrawPlan,
    asset_index: usize,
    dobj_state: xmodel_runtime::DObjSemanticState,
    surfaces: &[PosedModelSurface],
    materials: &[Option<SmodelPassMaterial>],
) -> bool {
    let Some(asset) = plan.assets.get(asset_index) else {
        return false;
    };
    let incoming: Vec<_> = surfaces
        .iter()
        .zip(materials)
        .filter_map(|(surface, material)| Some((surface, material.as_ref()?)))
        .collect();
    if incoming.len() != asset.surfaces.len() {
        return false;
    }
    let packed_rows = match &plan.packed_vertices {
        PackedVertexPayload::T6(rows) => Some(rows.len()),
        PackedVertexPayload::Unavailable { .. } => None,
    };

    let mut overwrites = Vec::with_capacity(incoming.len());
    for (&(surface, material), &(surface_slot, material_slot)) in
        incoming.iter().zip(&asset.surfaces)
    {
        let Some(span) = surface_span(plan, surface_slot) else {
            return false;
        };
        let count = span.vertex_count();
        let first_vertex = span.first_vertex as usize;
        let vertices = mesh_vertices(&surface.mesh).filter(|rows| rows.len() == count);
        let same_layout = plan.surface_ranges[surface_slot as usize].1 as usize
            == mesh_index_count(&surface.mesh).unwrap_or(usize::MAX);
        let fits = first_vertex + count <= plan.vertices.len()
            && packed_rows.is_none_or(|len| first_vertex + count <= len);
        let (Some(vertices), true, true, true) = (
            vertices,
            same_layout,
            fits,
            surface.packed_vertices.len() == count,
        ) else {
            return false;
        };
        overwrites.push(SurfaceOverwrite {
            first_vertex,
            vertices,
            packed: &surface.packed_vertices,
            material_slot: material_slot as usize,
            material,
        });
    }

    for overwrite in overwrites {
        let range = overwrite.first_vertex..overwrite.first_vertex + overwrite.vertices.len();
        if let PackedVertexPayload::T6(rows) = &mut plan.packed_vertices {
            rows[range.clone()].copy_from_slice(overwrite.packed);
        }
        plan.vertices[range].copy_from_slice(&overwrite.vertices);
        if let Some(slot) = plan.materials.get_mut(overwrite.material_slot) {
            slot.clone_from(overwrite.material);
        }
    }
    plan.assets[asset_index].dobj_state = dobj_state;
    plan.revisions.bump_surfaces();
    plan.revision = plan.revision.wrapping_add(1);
    true
}

/// Drops the assets not marked in `keep` and packs the survivors' geometry
/// together. The drawn rows and owners belong to the old layout and go with it.
pub fn retain_script_model_assets(plan: &mut ScriptModelDrawPlan, keep: &[bool]) {
    debug_assert_eq!(keep.len(), plan.assets.len());
    if keep.iter().all(|&kept| kept) {
        return;
    }
    let old_packed = match &plan.packed_vertices {
        PackedVertexPayload::T6(rows) => Some(rows.as_slice()),
        PackedVertexPayload::Unavailable { .. } => None,
    };
    let mut packed = PackedRows {
        intact: old_packed.is_some(),
        rows: Vec::new(),
    };
    let mut vertices = Vec::with_capacity(plan.vertices.len());
    let mut indices = Vec::with_capacity(plan.indices.len());
    let mut surface_ranges = Vec::with_capacity(plan.surface_ranges.len());
    let mut materials = Vec::with_capacity(plan.materials.len());
    let mut assets = Vec::with_capacity(plan.assets.len());

    let kept_assets = plan.assets.iter().zip(keep).filter(|&(_, &kept)| kept);
    for (asset, _) in kept_assets {
        let mut slots = Vec::new();
        for &(surface_slot, material_slot) in &asset.surfaces {
            let Some(span) = surface_span(plan, surface_slot) else {
                continue;
            };
            let Some(material) = plan.materials.get(material_slot as usize) else {
                continue;
            };
            let reach = span.first_vertex as usize..=span.last_vertex as usize;
            let Some(source) = plan.vertices.get(reach.clone()) else {
                continue;
            };
            let base = vertices.len() as u32;
            vertices.extend_from_slice(source);
            match old_packed.and_then(|rows| rows.get(reach)) {
                Some(rows) if packed.intact => packed.rows.extend_from_slice(rows),
                _ => packed.spoil(),
            }
            slots.push((surface_ranges.len() as u32, materials.len() as u32));
            surface_ranges.push((indices.len() as u32, span.indices.len() as u32));
            indices.extend(
                span.indices
                    .iter()
                    .map(|&index| base + index - span.first_vertex),
            );
            materials.push(material.clone());
        }
        assets.push(ScriptModelAssetDraw {
            surfaces: slots,
            ..asset.clone()
        });
    }

    plan.packed_vertices = packed.into_payload(
        vertices.len(),
        XMODEL_PACKED_EMPTY_PLAN,
        XMODEL_PACKED_UNAVAILABLE,
    );
    plan.vertices = vertices;
    plan.indices = indices;
    plan.surface_ranges = surface_ranges;
    plan.materials = materials;
    plan.assets = assets;
    plan.draws.clear();
    plan.owners.clear();

    // The revisions carry across: a retain reshuffles slots, it does not make
    // this a different plan, and a revision that restarted at zero is one a
    // consumer could mistake for the one it last saw.
    plan.generation = plan.generation.wrapping_add(1);
    plan.revisions.bump_surfaces();
    plan.revisions.bump_admission();
    restamp_topology(plan);
}

// ---------------------------------------------------------------------------
// Remote bodies
// ---------------------------------------------------------------------------

/// The packed rows of a remote body plan while its blobs are appended.
pub struct BodyPackedSession(PackedRows);

impl BodyPackedSession {
    pub fn reserve_packed(&mut self, n: usize) {
        self.0.rows.reserve(n);
    }
}

pub fn take_body_packed_session(plan: &mut RemoteBodyDrawPlan) -> BodyPackedSession {
    let no_vertices = plan.decoded_n == 0 && plan.vertices.is_empty();
    BodyPackedSession(PackedRows::take(
        &mut plan.packed_vertices,
        no_vertices,
        BODY_PACKED_UNAVAILABLE,
    ))
}

pub fn install_body_packed_session(plan: &mut RemoteBodyDrawPlan, session: BodyPackedSession) {
    plan.packed_vertices = session.0.into_payload(
        plan.decoded_n,
        XMODEL_PACKED_EMPTY_PLAN,
        BODY_PACKED_UNAVAILABLE,
    );
}

/// Appends a body mesh that exists only as packed rows and indices, and
/// returns where it landed as `(first vertex, first index)`.
pub fn append_remote_body_cpu_blob(
    plan: &mut RemoteBodyDrawPlan,
    session: &mut BodyPackedSession,
    packed_rows: &[[u8; T6_PACKED_VERTEX]],
    indices: &[u32],
    decoded_n: usize,
) -> Option<(u32, u32)> {
    if decoded_n == 0 || indices.is_empty() {
        return None;
    }
    let first_vertex = plan.decoded_n as u32;
    let first_index = plan.indices.len() as u32;
    plan.decoded_n = plan.decoded_n.saturating_add(decoded_n);
    plan.indices
        .extend(indices.iter().map(|&index| first_vertex + index));
    session.0.extend(packed_rows, decoded_n);
    Some((first_vertex, first_index))
}

/// Queues a draw of the index run `index_start..index_start + index_count`; an
/// unlit or empty one draws nothing.
#[allow(clippy::too_many_arguments)]
pub fn push_remote_body_cpu_draw(
    plan: &mut RemoteBodyDrawPlan,
    index_start: u32,
    index_count: u32,
    world_from_local: Mat4,
    lighting_handle: u32,
    scene_light_index: u8,
    reflection_probe_index: u8,
    material: u32,
    scene_entnum: Option<u32>,
) {
    if lighting_handle == 0 || index_count == 0 {
        return;
    }
    let surface = plan.surface_ranges.len() as u32;
    plan.surface_ranges.push((index_start, index_count));
    plan.draws.push(RemoteBodySurfaceDraw {
        surface,
        material,
        world_from_local,
        lighting_handle,
        scene_light_index,
        reflection_probe_index,
        scene_entnum,
    });
}

pub fn finish_remote_body_draw_plan(plan: &mut RemoteBodyDrawPlan) {
    plan.revisions.topology =
        topology_fingerprint(&plan.indices, &plan.surface_ranges, plan.decoded_n);
}

// ---------------------------------------------------------------------------
// First-person viewmodel
// ---------------------------------------------------------------------------

/// Publishes the rows a prepared rig settled: its indices, surface ranges,
/// materials and draws, plus a packed vertex buffer sized to the layout it laid
/// out. This runs when the composition changes; a pose writes into the buffer
/// and touches nothing else here.
pub fn install_prepared_fpv_plan(
    plan: &mut FpvDrawPlan,
    geometry: &PreparedFpvGeometry,
    lighting_handle: u32,
) {
    geometry.write_indices(&mut plan.indices);
    plan.surface_ranges.clone_from(&geometry.surface_ranges);
    plan.materials.clone_from(&geometry.materials);
    plan.draws.clone_from(&geometry.draws);
    plan.decoded_n = geometry.dest_n;
    plan.lighting_handle = lighting_handle;
    plan.geometry_ok = false;
    plan.packed_vertices = install_retained_packed(
        geometry.packed_ok,
        vec![[0; T6_PACKED_VERTEX]; geometry.dest_n],
        geometry.dest_n,
        FPV_PACKED_EMPTY_PLAN,
        FPV_PACKED_UNAVAILABLE,
    );
    plan.hands_plan_n = Some(geometry.hands_plan_n);
    plan.gun_plan_n = Some(geometry.gun_plan_n);
    plan.scope_plan_n = Some(geometry.scope_plan_n);
    plan.scope_house_plan_n = Some(geometry.scope_house_plan_n);
    plan.scope_lens_plan_n = Some(geometry.scope_lens_plan_n);
    plan.plan_draw_n = Some(geometry.plan_draw_n);
    plan.plan_skip_n = Some(geometry.plan_skip_n);
    plan.revisions.bump_surfaces();
    let topology = topology_fingerprint(&plan.indices, &plan.surface_ranges, plan.decoded_n);
    plan.revision = stamp_plan_geometry(&mut plan.revisions, plan.revision, topology);
}

/// Nothing to draw. A plan that was already empty publishes no new revision: a
/// hidden viewmodel is not a reason for the merge downstream to re-copy a
/// vertex bank every frame.
pub fn clear_fpv_draw_plan(plan: &mut FpvDrawPlan, lighting_handle: u32) {
    let was_empty = plan.draws.is_empty() && plan.decoded_n == 0;
    plan.lighting_handle = lighting_handle;
    plan.geometry_ok = false;
    plan.settle_visible();
    // The rows are gone, so the rig that published them has to publish them
    // again before this plan draws anything: a viewmodel hidden without a
    // respawn behind it comes back to the same composition, not to an empty
    // plan.
    plan.rig_generation = 0;
    if was_empty {
        return;
    }
    plan.indices.clear();
    plan.surface_ranges.clear();
    plan.materials.clear();
    plan.draws.clear();
    plan.decoded_n = 0;
    plan.packed_vertices = PackedVertexPayload::Unavailable {
        source_layout: FPV_PACKED_EMPTY_PLAN,
    };
    plan.hands_plan_n = None;
    plan.gun_plan_n = None;
    plan.scope_plan_n = Some(0);
    plan.scope_house_plan_n = Some(0);
    plan.scope_lens_plan_n = Some(0);
    plan.plan_draw_n = Some(0);
    plan.plan_skip_n = Some(0);
    plan.revisions.bump_surfaces();
    let topology = topology_fingerprint(&plan.indices, &plan.surface_ranges, 0);
    plan.revision = stamp_plan_geometry(&mut plan.revisions, plan.revision, topology);
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::asset::RenderAssetUsages;
    use bevy::mesh::{Indices, PrimitiveTopology};

    use crate::anim::xmodel_pose::FpvSurfOwner;

    fn row(tag: u8) -> [u8; T6_PACKED_VERTEX] {
        [tag; T6_PACKED_VERTEX]
    }

    /// A single triangle whose vertices sit at x = `x`, `x + 1`, `x + 2`.
    fn surface(x: f32, packed_tag: Option<u8>) -> PosedModelSurface {
        let mut mesh = Mesh::new(
            PrimitiveTopology::TriangleList,
            RenderAssetUsages::default(),
        );
        mesh.insert_attribute(
            Mesh::ATTRIBUTE_POSITION,
            vec![[x, 0.0, 0.0], [x + 1.0, 0.0, 0.0], [x + 2.0, 0.0, 0.0]],
        );
        mesh.insert_indices(Indices::U32(vec![0, 1, 2]));
        PosedModelSurface {
            surface_index: 0,
            mesh,
            material: None,
            packed_vertices: packed_tag.map_or_else(Vec::new, |tag| vec![row(tag); 3]),
            owner: FpvSurfOwner::Gun,
            model: 0,
        }
    }

    fn material(sort_key: u8) -> Option<SmodelPassMaterial> {
        Some(SmodelPassMaterial {
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
        })
    }

    fn xs(plan: &ScriptModelDrawPlan) -> Vec<f32> {
        plan.vertices()
            .iter()
            .map(|vertex| vertex.position[0])
            .collect()
    }

    fn script_state() -> xmodel_runtime::DObjSemanticState {
        xmodel_runtime::DObjSemanticState::bind_pose("crate".to_owned(), 1, 1)
    }

    fn add_asset(plan: &mut ScriptModelDrawPlan, name: &str, x: f32, packed: Option<u8>) -> usize {
        append_script_model_asset(
            plan,
            MapXModelAssetKey(name.to_owned()),
            script_state(),
            vec![None],
            &[surface(x, packed)],
            &[material(1)],
        )
    }

    // ----- packed rows ---------------------------------------------------

    #[test]
    fn packed_rows_start_intact_only_for_a_plan_that_can_extend_them() {
        let mut empty = PackedVertexPayload::Unavailable { source_layout: "x" };
        assert!(PackedRows::take(&mut empty, true, "gone").intact);
        let mut stale = PackedVertexPayload::Unavailable { source_layout: "x" };
        assert!(!PackedRows::take(&mut stale, false, "gone").intact);
        let mut held = PackedVertexPayload::T6(vec![row(1)]);
        let taken = PackedRows::take(&mut held, false, "gone");
        assert!(taken.intact && taken.rows.len() == 1);
        assert!(matches!(
            held,
            PackedVertexPayload::Unavailable {
                source_layout: "gone"
            }
        ));
    }

    #[test]
    fn a_surface_without_rows_spoils_the_set() {
        let mut rows = PackedRows {
            intact: true,
            rows: vec![row(1)],
        };
        rows.extend(&[row(2), row(2)], 2);
        assert!(rows.intact && rows.rows.len() == 3);
        rows.extend(&[row(3)], 2);
        assert!(!rows.intact && rows.rows.is_empty());
        rows.extend(&[row(4)], 1);
        assert!(rows.rows.is_empty(), "once spoiled, always spoiled");
    }

    // ----- appending surfaces ---------------------------------------------

    #[test]
    fn appended_surfaces_land_in_consecutive_slots() {
        let mut plan = MissileDrawPlan::default();
        let slots = append_missile_surfaces(
            &mut plan,
            &[surface(0.0, Some(1)), surface(10.0, Some(2))],
            &[material(1), material(2)],
        );
        assert_eq!(slots, [(0, 0), (1, 1)]);
        assert_eq!(plan.surface_ranges(), [(0, 3), (3, 3)]);
        assert_eq!(plan.indices(), [0, 1, 2, 3, 4, 5]);
        assert_eq!(plan.vertices().len(), 6);
        assert!(matches!(plan.packed_vertices(), PackedVertexPayload::T6(rows) if rows.len() == 6));
        assert_eq!(plan.revisions.vertices, 1);
        assert_eq!(plan.revisions.materials, 1);
    }

    #[test]
    fn surfaces_without_a_material_or_mesh_are_skipped() {
        let mut plan = ItemDrawPlan::default();
        let mut no_mesh = surface(0.0, Some(1));
        no_mesh.mesh = Mesh::new(
            PrimitiveTopology::TriangleList,
            RenderAssetUsages::default(),
        );
        let slots = append_item_surfaces(
            &mut plan,
            &[surface(0.0, Some(1)), no_mesh, surface(5.0, Some(1))],
            &[None, material(1), None],
        );
        assert!(slots.is_empty());
        assert!(plan.vertices().is_empty());
        assert!(matches!(
            plan.packed_vertices(),
            PackedVertexPayload::Unavailable { source_layout } if *source_layout == XMODEL_PACKED_EMPTY_PLAN
        ));
    }

    #[test]
    fn missing_rows_leave_the_plan_without_packed_vertices() {
        let mut plan = DynEntDrawPlan::default();
        append_dynent_surfaces(
            &mut plan,
            &[surface(0.0, Some(1)), surface(10.0, None)],
            &[material(1), material(1)],
        );
        assert_eq!(plan.vertices().len(), 6);
        assert!(matches!(
            plan.packed_vertices(),
            PackedVertexPayload::Unavailable { source_layout } if *source_layout == XMODEL_PACKED_UNAVAILABLE
        ));
        // Rows that were never complete cannot be completed by a later asset.
        append_dynent_surfaces(&mut plan, &[surface(20.0, Some(1))], &[material(1)]);
        assert!(matches!(
            plan.packed_vertices(),
            PackedVertexPayload::Unavailable { .. }
        ));
    }

    #[test]
    fn a_dynent_asset_remembers_its_slots() {
        let mut plan = DynEntDrawPlan::default();
        let key = MapXModelAssetKey("door".to_owned());
        let slots = append_dynent_asset(
            &mut plan,
            key.clone(),
            Some(1),
            &[surface(0.0, Some(1))],
            &[material(1)],
        );
        assert_eq!(
            plan.asset(&key, Some(1))
                .map(|asset| asset.surfaces.clone()),
            Some(slots)
        );
    }

    // ----- script models ---------------------------------------------------

    #[test]
    fn script_model_assets_are_numbered_in_order() {
        let mut plan = ScriptModelDrawPlan::default();
        assert_eq!(add_asset(&mut plan, "a", 0.0, Some(1)), 0);
        assert_eq!(add_asset(&mut plan, "b", 10.0, Some(2)), 1);
        assert_eq!(plan.assets[1].surfaces, [(1, 1)]);
        assert_eq!(plan.revision, 2, "each new layout is a revision");
        assert_eq!(
            plan.revisions.topology,
            topology_fingerprint(plan.indices(), plan.surface_ranges(), 6)
        );
    }

    #[test]
    fn overwriting_an_asset_rewrites_its_slots_only() {
        let mut plan = ScriptModelDrawPlan::default();
        add_asset(&mut plan, "a", 0.0, Some(1));
        add_asset(&mut plan, "b", 10.0, Some(2));
        let revision = plan.revision;

        assert!(overwrite_script_model_asset(
            &mut plan,
            1,
            script_state(),
            &[surface(50.0, Some(9))],
            &[material(7)],
        ));
        assert_eq!(xs(&plan), [0.0, 1.0, 2.0, 50.0, 51.0, 52.0]);
        assert!(
            matches!(plan.packed_vertices(), PackedVertexPayload::T6(rows) if rows[3] == row(9) && rows[0] == row(1))
        );
        assert_eq!(plan.materials()[1].sort_key, 7);
        assert_eq!(plan.materials()[0].sort_key, 1);
        assert_eq!(plan.revision, revision + 1);
    }

    #[test]
    fn an_overwrite_that_does_not_fit_changes_nothing() {
        let mut plan = ScriptModelDrawPlan::default();
        add_asset(&mut plan, "a", 0.0, Some(1));
        let revision = plan.revision;
        let before = xs(&plan);

        let refusals: [(usize, Vec<PosedModelSurface>); 3] = [
            (5, vec![surface(1.0, Some(1))]),
            (0, vec![surface(1.0, Some(1)), surface(1.0, Some(1))]),
            (0, vec![surface(1.0, None)]),
        ];
        for (index, surfaces) in refusals {
            let materials = vec![material(1); surfaces.len()];
            assert!(!overwrite_script_model_asset(
                &mut plan,
                index,
                script_state(),
                &surfaces,
                &materials
            ));
        }
        assert_eq!(xs(&plan), before);
        assert_eq!(plan.revision, revision);
    }

    #[test]
    fn an_overwrite_with_a_different_index_count_is_refused() {
        let mut plan = ScriptModelDrawPlan::default();
        add_asset(&mut plan, "a", 0.0, Some(1));
        let mut longer = surface(1.0, Some(1));
        longer
            .mesh
            .insert_indices(Indices::U32(vec![0, 1, 2, 2, 1, 0]));
        assert!(!overwrite_script_model_asset(
            &mut plan,
            0,
            script_state(),
            &[longer],
            &[material(1)]
        ));
    }

    #[test]
    fn retaining_every_asset_is_a_no_op() {
        let mut plan = ScriptModelDrawPlan::default();
        add_asset(&mut plan, "a", 0.0, Some(1));
        let before = (plan.revision, plan.revisions, plan.generation);
        retain_script_model_assets(&mut plan, &[true]);
        assert_eq!((plan.revision, plan.revisions, plan.generation), before);
    }

    #[test]
    fn retaining_repacks_the_survivors() {
        let mut plan = ScriptModelDrawPlan::default();
        add_asset(&mut plan, "a", 0.0, Some(1));
        add_asset(&mut plan, "b", 10.0, Some(2));
        add_asset(&mut plan, "c", 20.0, Some(3));
        plan.draws.push(XModelSurfaceDrawStub::draw());
        let revision = plan.revision;

        retain_script_model_assets(&mut plan, &[false, true, true]);

        assert_eq!(xs(&plan), [10.0, 11.0, 12.0, 20.0, 21.0, 22.0]);
        assert_eq!(plan.indices(), [0, 1, 2, 3, 4, 5]);
        assert_eq!(plan.surface_ranges(), [(0, 3), (3, 3)]);
        assert_eq!(plan.assets.len(), 2);
        assert_eq!(plan.assets[0].key, MapXModelAssetKey("b".to_owned()));
        assert_eq!(plan.assets[1].surfaces, [(1, 1)]);
        assert!(matches!(
            plan.packed_vertices(),
            PackedVertexPayload::T6(rows) if rows.len() == 6 && rows[0] == row(2) && rows[5] == row(3)
        ));
        assert!(plan.draws().is_empty() && plan.owners.is_empty());
        assert_eq!(plan.generation, 1);
        assert_eq!(plan.revision, revision + 1);
    }

    #[test]
    fn retaining_nothing_leaves_an_empty_plan() {
        let mut plan = ScriptModelDrawPlan::default();
        add_asset(&mut plan, "a", 0.0, Some(1));
        retain_script_model_assets(&mut plan, &[false]);
        assert!(plan.assets.is_empty() && plan.vertices().is_empty() && plan.indices().is_empty());
        assert!(matches!(
            plan.packed_vertices(),
            PackedVertexPayload::Unavailable { .. }
        ));
    }

    /// A draw row with nothing set, for plans whose rows are only counted.
    struct XModelSurfaceDrawStub;

    impl XModelSurfaceDrawStub {
        fn draw() -> render_scene::XModelSurfaceDraw {
            render_scene::XModelSurfaceDraw {
                surface: 0,
                material: 0,
                world_from_local: Mat4::IDENTITY,
                lighting_handle: 0,
                pending_lighting: None,
                colour_refusal: None,
                object_id: 0,
                scene_light_index: 0,
                reflection_probe_index: 0,
                packed_lighting: None,
                is_scope: false,
                scene_entnum: None,
                caster_bound: None,
            }
        }
    }

    // ----- remote bodies ---------------------------------------------------

    #[test]
    fn body_blobs_rebase_their_indices() {
        let mut plan = RemoteBodyDrawPlan::default();
        let mut session = take_body_packed_session(&mut plan);
        assert_eq!(
            append_remote_body_cpu_blob(&mut plan, &mut session, &[row(1); 3], &[0, 1, 2], 3),
            Some((0, 0))
        );
        assert_eq!(
            append_remote_body_cpu_blob(&mut plan, &mut session, &[row(2); 2], &[1, 0, 1], 2),
            Some((3, 3))
        );
        install_body_packed_session(&mut plan, session);
        assert_eq!(plan.decoded_n(), 5);
        assert_eq!(plan.indices(), [0, 1, 2, 4, 3, 4]);
        assert!(matches!(plan.packed_vertices(), PackedVertexPayload::T6(rows) if rows.len() == 5));
    }

    #[test]
    fn empty_body_blobs_add_nothing() {
        let mut plan = RemoteBodyDrawPlan::default();
        let mut session = take_body_packed_session(&mut plan);
        assert_eq!(
            append_remote_body_cpu_blob(&mut plan, &mut session, &[], &[0], 0),
            None
        );
        assert_eq!(
            append_remote_body_cpu_blob(&mut plan, &mut session, &[row(1)], &[], 1),
            None
        );
        assert_eq!(plan.decoded_n(), 0);
    }

    #[test]
    fn a_body_blob_with_the_wrong_row_count_drops_the_packed_rows() {
        let mut plan = RemoteBodyDrawPlan::default();
        let mut session = take_body_packed_session(&mut plan);
        append_remote_body_cpu_blob(&mut plan, &mut session, &[row(1)], &[0, 0, 0], 3);
        install_body_packed_session(&mut plan, session);
        assert!(matches!(
            plan.packed_vertices(),
            PackedVertexPayload::Unavailable { source_layout } if *source_layout == BODY_PACKED_UNAVAILABLE
        ));
        assert_eq!(plan.decoded_n(), 3, "the blob itself is still counted");
    }

    #[test]
    fn body_draws_need_light_and_indices() {
        let mut plan = RemoteBodyDrawPlan::default();
        push_remote_body_cpu_draw(&mut plan, 0, 3, Mat4::IDENTITY, 0, 0, 0, 0, None);
        push_remote_body_cpu_draw(&mut plan, 0, 0, Mat4::IDENTITY, 5, 0, 0, 0, None);
        assert!(plan.draws().is_empty() && plan.surface_ranges().is_empty());

        push_remote_body_cpu_draw(&mut plan, 3, 6, Mat4::IDENTITY, 5, 1, 2, 4, Some(8));
        push_remote_body_cpu_draw(&mut plan, 9, 6, Mat4::IDENTITY, 5, 1, 2, 4, None);
        assert_eq!(plan.surface_ranges(), [(3, 6), (9, 6)]);
        let drawn = plan.draws()[0];
        assert_eq!(
            (drawn.surface, drawn.material, drawn.lighting_handle),
            (0, 4, 5)
        );
        assert_eq!(
            (drawn.scene_light_index, drawn.reflection_probe_index),
            (1, 2)
        );
        assert_eq!(drawn.scene_entnum, Some(8));
        assert_eq!(plan.draws()[1].surface, 1);
    }

    #[test]
    fn finishing_a_body_plan_stamps_its_topology() {
        let mut plan = RemoteBodyDrawPlan::default();
        let mut session = take_body_packed_session(&mut plan);
        append_remote_body_cpu_blob(&mut plan, &mut session, &[row(1); 3], &[0, 1, 2], 3);
        push_remote_body_cpu_draw(&mut plan, 0, 3, Mat4::IDENTITY, 1, 0, 0, 0, None);
        finish_remote_body_draw_plan(&mut plan);
        assert_eq!(
            plan.revisions.topology,
            topology_fingerprint(&[0, 1, 2], &[(0, 3)], 3)
        );
    }

    // ----- viewmodel ---------------------------------------------------

    #[test]
    fn clearing_an_empty_viewmodel_plan_publishes_nothing() {
        let mut plan = FpvDrawPlan {
            rig_generation: 4,
            ..Default::default()
        };
        clear_fpv_draw_plan(&mut plan, 3);
        assert_eq!(plan.lighting_handle, 3);
        assert_eq!(plan.rig_generation, 0);
        assert_eq!((plan.revision, plan.revisions), (0, Default::default()));
    }

    #[test]
    fn clearing_a_viewmodel_plan_with_rows_publishes_once() {
        let mut plan = FpvDrawPlan {
            decoded_n: 3,
            indices: vec![0, 1, 2],
            surface_ranges: vec![(0, 3)],
            geometry_ok: true,
            placement_ok: true,
            visible: true,
            ..Default::default()
        };
        plan.draws.push(crate::draw::FpvSurfaceDraw {
            surface: 0,
            material: 0,
            is_scope: false,
        });
        clear_fpv_draw_plan(&mut plan, 0);
        assert!(!plan.visible && !plan.geometry_ok);
        assert!(plan.indices().is_empty() && plan.draws().is_empty() && plan.decoded_n() == 0);
        assert_eq!(
            (plan.hands_plan_n, plan.scope_plan_n, plan.plan_draw_n),
            (None, Some(0), Some(0))
        );
        assert_eq!(plan.revision, 1);

        clear_fpv_draw_plan(&mut plan, 0);
        assert_eq!(plan.revision, 1, "an empty plan stays quiet");
    }
}
