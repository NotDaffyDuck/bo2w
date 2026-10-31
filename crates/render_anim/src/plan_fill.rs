//! Builders that fold posed model surfaces into the draw plans, and the pass
//! materials those surfaces draw with.

use asset_t6::T6_PACKED_VERTEX;
use asset_world::{LevelModelAssetId, PackedVertexPayload};
use bevy::prelude::*;
use render_frame::PropVertex;
use render_material::LiveMaterialLedger;
use render_scene::{DesignerTextureSet, PropPassMaterial, LevelModelIllumAtlas};

use crate::motion::gun_pose::SkinnedMeshFacet;
use crate::motion::fpv_rig::PreparedFpvGeometry;
use crate::strokes::{BODY_VERTS_DROPPED_NOTE, FPV_PACKED_BLANK_PLAN, FPV_PACKED_MISSING};
use crate::strokes::{PropMeshPaint, DynEntPaintPlan, FirstPersonPaintPlan, PickupPaintPlan, ThrownPaintPlan};
use crate::strokes::{OtherBodyPaintPlan, OtherBodyFacetRun};
use crate::strokes::{ScriptPropMeshPaint, ScriptedModelPaintPlan};
use crate::strokes::{bump_plan_mesh_rev, ix_layout_digest};
use crate::geometry::{append_mesh, install_held_compact, mesh_index_count, mesh_vertices};

/// Reasons recorded when a static-model plan cannot hand out packed rows.
pub const FIGURE_COMPACT_UNAVAILABLE: &str =
    "static-model packed rows absent or fewer than the decoded vertices";
pub const FIGURE_COMPACT_EMPTY_PLAN: &str = "static-model plan holds no vertices";

// ---------------------------------------------------------------------------
// Pass materials
// ---------------------------------------------------------------------------

/// A model material that reads the world's lighting atlas, for the material
/// at `slot` in the sorted catalog, drawn with the given maps and sort key.
/// `None` when the atlas has no usable height.
fn atlas_lit_material(
    lit_atlas: &LevelModelIllumAtlas,
    bay: u32,
    sort_k: u8,
    maps_in: DesignerTextureSet,
) -> Option<PropPassMaterial> {
    lighting_t6::light_atlas_reciprocal_height(lit_atlas.dims.rows_tall).map(|inv_height| {
        let scale = lighting_t6::atlas_lookup_scale(inv_height);
        PropPassMaterial {
            model_illum_required: true,
            material_draw_rank_slot: Some(bay),
            atlas_find: [
                lighting_t6::ATLAS_INV_WIDTH as f32,
                inv_height,
                lighting_t6::MODEL_LIGHT_VOLUME_DEPTH_SPAN,
                0.0,
            ],
            illum_lookup_scale: [scale.u, scale.v, scale.w, scale.q],
            atlas: Some(Handle::clone(&lit_atlas.image)),
            t6_material: None,
            probe: None,
            order_key: sort_k,
            color: maps_in.color,
            specular: maps_in.specular,
            see_through: maps_in.see_through,
            paint_mode: maps_in.paint_mode,
            face_cull: maps_in.cull_mode,
            env_map_params: maps_in.env_map_params,
        }
    })
}

/// A material with no maps of its own: opaque, no culling override.
fn no_maps(catalog: &LiveMaterialLedger) -> DesignerTextureSet {
    render_scene::live_model_tables(None, catalog, &[])
}

/// The lit material for a material known by name.
pub fn body_lighting_material(
    world_atlas: &LevelModelIllumAtlas,
    catalog: &LiveMaterialLedger,
    name: &str,
) -> Option<PropPassMaterial> {
    let ordinal = catalog.rank_for_material_name(name)?.get();
    atlas_lit_material(world_atlas, ordinal, 0, no_maps(catalog))
}

/// The lit material for a material already bound in the asset catalog.
pub fn bound_lit_figure_pass_material(
    world_atlas: &LevelModelIllumAtlas,
    catalog: &LiveMaterialLedger,
    bound: assets::SurfaceMaterialId,
) -> Option<PropPassMaterial> {
    let ordinal = catalog.rank_for_asset_id(bound)?.get();
    atlas_lit_material(world_atlas, ordinal, 0, no_maps(catalog))
}

/// The lit material for an authored one: the lit defaults, with the maps,
/// blend and cull state and sort key the authored material carries.
pub fn designer_lit_mesh_material(
    world_atlas: &LevelModelIllumAtlas,
    catalog: &LiveMaterialLedger,
    handles: &[Option<Handle<Image>>],
    id: assets::SurfaceMaterialId,
) -> Option<PropPassMaterial> {
    let sort_key = catalog.derived(id)?.draw_sort_key;
    let ordinal = catalog.rank_for_asset_id(id)?.get();
    let maps = render_scene::live_model_tables(Some(id), catalog, handles);
    atlas_lit_material(world_atlas, ordinal, sort_key, maps)
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
    fn blank(intact: bool) -> Self {
        let rows = Vec::new();
        Self { intact, rows }
    }

    /// Takes the plan's rows out to extend them. A plan that already has
    /// vertices but no rows has nothing to extend.
    fn hand_over(
        payload: &mut PackedVertexPayload,
        no_vertices: bool,
        placeholder: &'static str,
    ) -> Self {
        let old = std::mem::replace(
            payload,
            PackedVertexPayload::Unavailable {
                origin_layout: placeholder,
            },
        );
        match old {
            PackedVertexPayload::T6(rows) => Self { intact: true, rows },
            PackedVertexPayload::Unavailable { .. } => Self::blank(no_vertices),
        }
    }

    /// Adds one surface's rows, which have to match the `decoded` vertices it
    /// added; a surface without them spoils the whole set.
    fn extend(&mut self, rows: &[[u8; T6_PACKED_VERTEX]], decoded: usize) {
        if !self.intact || rows.len() != decoded {
            self.spoil();
            return;
        }
        self.rows.extend_from_slice(rows);
    }

    fn spoil(&mut self) {
        self.rows = Vec::new();
        self.intact = false;
    }

    fn into_payload(
        self,
        vertex_total: usize,
        if_empty: &'static str,
        if_short: &'static str,
    ) -> PackedVertexPayload {
        let Self { intact, rows } = self;
        install_held_compact(intact, rows, vertex_total, if_empty, if_short)
    }
}

// ---------------------------------------------------------------------------
// Plans that hold decoded vertices
// ---------------------------------------------------------------------------

/// The buffers every vertex-holding plan keeps its geometry in.
struct Geometry<'a> {
    verts: &'a mut Vec<PropVertex>,
    indices: &'a mut Vec<u32>,
    facet_ranges: &'a mut Vec<(u32, u32)>,
    mat_list: &'a mut Vec<PropPassMaterial>,
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
                    verts: &mut self.verts,
                    indices: &mut self.indices,
                    facet_ranges: &mut self.facet_ranges,
                    mat_list: &mut self.mat_list,
                    packed: &mut self.compact_vertices,
                }
            }
        }
    )+};
}

holds_geometry!(
    ScriptedModelPaintPlan,
    ThrownPaintPlan,
    PickupPaintPlan,
    DynEntPaintPlan
);

/// Appends every surface that has a material and a usable mesh, and returns
/// each one as `(surface range, material)` slots of the plan.
fn append_surfaces(
    plan: &mut impl HoldsGeometry,
    surfaces: &[SkinnedMeshFacet],
    materials_in: &[Option<PropPassMaterial>],
) -> Vec<(u32, u32)> {
    let geometry_in = plan.geometry();
    let mut packed = PackedRows::hand_over(
        geometry_in.packed,
        geometry_in.verts.is_empty(),
        FIGURE_COMPACT_UNAVAILABLE,
    );
    let mut placed = Vec::new();
    for (surf, mat) in surfaces.iter().zip(materials_in) {
        let Some(mat) = mat else { continue };
        let before = geometry_in.verts.len();
        let Some(range) = append_mesh(&surf.mesh, geometry_in.verts, geometry_in.indices) else {
            continue;
        };
        packed.extend(&surf.compact_vertices, geometry_in.verts.len() - before);
        placed.push((
            geometry_in.facet_ranges.len() as u32,
            geometry_in.mat_list.len() as u32,
        ));
        geometry_in.facet_ranges.push(range);
        geometry_in.mat_list.push(mat.clone());
    }
    let total = geometry_in.verts.len();
    *geometry_in.packed =
        packed.into_payload(total, FIGURE_COMPACT_EMPTY_PLAN, FIGURE_COMPACT_UNAVAILABLE);
    placed
}

/// One `append_*_surfaces` entry point per plan kind: append, then count the
/// new surfaces.
macro_rules! surface_appenders {
    ($($name:ident => $plan:ty),+ $(,)?) => {$(
        pub fn $name(
            plan: &mut $plan,
            surfaces: &[SkinnedMeshFacet],
            mats: &[Option<PropPassMaterial>],
        ) -> Vec<(u32, u32)> {
            let placed = append_surfaces(plan, surfaces, mats);
            plan.revisions.bump_surfaces();
            placed
        }
    )+};
}

surface_appenders!(
    push_projectile_surfaces => ThrownPaintPlan,
    push_pickup_surfaces => PickupPaintPlan,
    push_loose_prop_surfaces => DynEntPaintPlan,
);

pub fn append_debris_asset(
    dynents: &mut DynEntPaintPlan,
    key: LevelModelAssetId,
    lod: Option<u8>,
    surfaces: &[SkinnedMeshFacet],
    materials_v: &[Option<PropPassMaterial>],
) -> Vec<(u32, u32)> {
    let slots = push_loose_prop_surfaces(dynents, surfaces, materials_v);
    dynents.store.push(PropMeshPaint {
        key,
        view_lod: lod,
        surf_pairs: slots.clone(),
    });
    slots
}

// ---------------------------------------------------------------------------
// Script models
// ---------------------------------------------------------------------------

/// Adds a script model's surfaces as a new asset of the plan and returns its
/// index.
pub fn push_script_prop_mesh(
    models: &mut ScriptedModelPaintPlan,
    key: LevelModelAssetId,
    puppet_state: xmodel_runtime::RigPoseRecord,
    detail_tiers: Vec<Option<u8>>,
    surfaces: &[SkinnedMeshFacet],
    materials_here: &[Option<PropPassMaterial>],
) -> usize {
    let slots = append_surfaces(models, surfaces, materials_here);
    models.assets.push(ScriptPropMeshPaint {
        key,
        puppet_state,
        viewer_detail_levels: detail_tiers,
        surfs: slots,
    });
    models.revisions.bump_surfaces();
    restamp_topology(models);
    models.assets.len() - 1
}

fn restamp_topology(plan_v: &mut ScriptedModelPaintPlan) {
    let vertex_total = plan_v.verts.len();
    let fingerprint = ix_layout_digest(&plan_v.indices, &plan_v.facet_ranges, vertex_total);
    plan_v.plan_rev = bump_plan_mesh_rev(&mut plan_v.revisions, plan_v.plan_rev, fingerprint);
}

/// The vertices a surface of the plan reaches: the lowest and highest vertex
/// its indices name, and the indices themselves.
struct SurfaceSpan<'a> {
    run: &'a [u32],
    low: u32,
    high: u32,
}

impl SurfaceSpan<'_> {
    fn vertex_count(&self) -> usize {
        (self.high - self.low) as usize + 1
    }
}

fn surface_span(plan: &ScriptedModelPaintPlan, surf: u32) -> Option<SurfaceSpan<'_>> {
    let &(first, count) = plan.facet_ranges.get(surf as usize)?;
    let from = first as usize;
    let run = plan
        .indices
        .get(from..from.saturating_add(count as usize))?;
    let low = *run.iter().min()?;
    let high = *run.iter().max()?;
    Some(SurfaceSpan { run, low, high })
}

/// What overwriting one surface of an asset changes, checked before anything
/// is touched.
struct StagedSurface<'a> {
    fresh_material: &'a PropPassMaterial,
    fresh_rows: &'a [[u8; T6_PACKED_VERTEX]],
    fresh_vertices: Vec<PropVertex>,
    first_vertex: usize,
    target_material: usize,
}

/// Pairs each surface with the material it draws with, leaving out the
/// surfaces that have none.
fn surfaces_with_materials<'a>(
    surfaces: &'a [SkinnedMeshFacet],
    mat_list: &'a [Option<PropPassMaterial>],
) -> Vec<(&'a SkinnedMeshFacet, &'a PropPassMaterial)> {
    surfaces
        .iter()
        .zip(mat_list.iter())
        .filter_map(|(surf, mat)| Some((surf, mat.as_ref()?)))
        .collect::<_>()
}

/// The rows a payload holds, if it holds any.
fn held_rows(payload: &PackedVertexPayload) -> Option<&[[u8; T6_PACKED_VERTEX]]> {
    let PackedVertexPayload::T6(rows) = payload else {
        return None;
    };
    Some(&rows[..])
}

/// Checks that every incoming surface still lines up with the slots the asset
/// was added with, and collects what writing it would change. Nothing here
/// touches the plan.
fn stage_overwrite<'a>(
    plan: &ScriptedModelPaintPlan,
    which: usize,
    surfaces: &'a [SkinnedMeshFacet],
    mat_rows: &'a [Option<PropPassMaterial>],
) -> Option<Vec<StagedSurface<'a>>> {
    let asset = plan.assets.get(which)?;
    let incoming = surfaces_with_materials(surfaces, mat_rows);
    if asset.surfs.len() != incoming.len() {
        return None;
    }
    let row_capacity = held_rows(&plan.compact_vertices).map(<[_]>::len);

    incoming
        .into_iter()
        .zip(&asset.surfs)
        .map(|((surface, material), &(surface_slot, surface_material_index))| {
            let span = surface_span(plan, surface_slot)?;
            let count = span.vertex_count();
            let base = span.low as usize;
            let drawn_indices = plan.facet_ranges[surface_slot as usize].1 as usize;
            let fits = surface.compact_vertices.len() == count
                && drawn_indices == mesh_index_count(&surface.mesh).unwrap_or(usize::MAX)
                && base + count <= plan.verts.len()
                && row_capacity.is_none_or(|len| base + count <= len);
            if !fits {
                return None;
            }
            let fresh_vertices = mesh_vertices(&surface.mesh).filter(|rows| rows.len() == count)?;
            Some(StagedSurface {
                fresh_material: material,
                fresh_rows: &surface.compact_vertices,
                fresh_vertices,
                first_vertex: base,
                target_material: surface_material_index as usize,
            })
        })
        .collect::<_>()
}

/// Writes staged surfaces into the plan's vertices, rows and materials.
fn write_staged(plan: &mut ScriptedModelPaintPlan, staged: Vec<StagedSurface<'_>>) {
    for item in staged {
        let reach = item.first_vertex..item.first_vertex + item.fresh_vertices.len();
        plan.verts[reach.clone()].copy_from_slice(&item.fresh_vertices);
        if let PackedVertexPayload::T6(rows) = &mut plan.compact_vertices {
            rows[reach].copy_from_slice(item.fresh_rows);
        }
        if let Some(slot) = plan.mat_list.get_mut(item.target_material) {
            slot.clone_from(item.fresh_material);
        }
    }
}

/// Rewrites the vertices, packed rows and materials of an asset in place, for a
/// pose that kept the asset's layout. Nothing is changed unless every surface
/// still lines up with the slots the asset was added with.
pub fn swap_script_prop_mesh(
    models: &mut ScriptedModelPaintPlan,
    which: usize,
    puppet_state: xmodel_runtime::RigPoseRecord,
    surfaces: &[SkinnedMeshFacet],
    mat_list: &[Option<PropPassMaterial>],
) -> bool {
    let Some(staged) = stage_overwrite(models, which, surfaces, mat_list) else {
        return false;
    };
    write_staged(models, staged);
    models.assets[which].puppet_state = puppet_state;
    models.revisions.bump_surfaces();
    models.plan_rev = models.plan_rev.overflowing_add(1).0;
    true
}

/// The geometry of the assets that survive a retain, packed from zero.
struct Compacted {
    verts: Vec<PropVertex>,
    idx_list: Vec<u32>,
    facet_ranges: Vec<(u32, u32)>,
    mat_list: Vec<PropPassMaterial>,
    assets: Vec<ScriptPropMeshPaint>,
    packed: PackedRows,
}

impl Compacted {
    fn with_room_for(plan: &ScriptedModelPaintPlan, rows_held: bool) -> Self {
        Self {
            verts: Vec::with_capacity(plan.verts.len()),
            idx_list: Vec::with_capacity(plan.indices.len()),
            facet_ranges: Vec::with_capacity(plan.facet_ranges.len()),
            mat_list: Vec::with_capacity(plan.mat_list.len()),
            assets: Vec::with_capacity(plan.assets.len()),
            packed: PackedRows::blank(rows_held),
        }
    }

    /// Copies one surface of `plan` over, if it is whole, and returns its new
    /// `(surface range, material)` slots.
    fn take_surface(
        &mut self,
        plan: &ScriptedModelPaintPlan,
        old_rows: Option<&[[u8; T6_PACKED_VERTEX]]>,
        (surface_slot, surface_material_index): (u32, u32),
    ) -> Option<(u32, u32)> {
        let span = surface_span(plan, surface_slot)?;
        let material = plan.mat_list.get(surface_material_index as usize)?;
        let reach = span.low as usize..=span.high as usize;
        let source = plan.verts.get(reach.clone())?;
        let shift = self.verts.len() as u32;
        self.verts.extend_from_slice(source);
        match old_rows.and_then(|rows| rows.get(reach)) {
            Some(rows) if self.packed.intact => self.packed.rows.extend_from_slice(rows),
            _ => self.packed.spoil(),
        }
        let slots = (
            self.facet_ranges.len() as u32,
            self.mat_list.len() as u32,
        );
        self.facet_ranges
            .push((self.idx_list.len() as u32, span.run.len() as u32));
        let rebased = span.run.iter().map(|&at| at - span.low + shift);
        self.idx_list.extend(rebased);
        self.mat_list.push(material.clone());
        Some(slots)
    }
}

fn compact_survivors(plan: &ScriptedModelPaintPlan, keep: &[bool]) -> Compacted {
    let old_rows = held_rows(&plan.compact_vertices);
    let mut out = Compacted::with_room_for(plan, old_rows.is_some());
    for (asset, _) in plan.assets.iter().zip(keep).filter(|&(_, &kept)| kept) {
        let moved = asset
            .surfs
            .iter()
            .filter_map(|&slots| out.take_surface(plan, old_rows, slots))
            .collect::<_>();
        out.assets.push(ScriptPropMeshPaint {
            surfs: moved,
            ..asset.clone()
        });
    }
    out
}

/// Drops the assets not marked in `keep` and packs the survivors' geometry
/// together. The drawn rows and owners belong to the old layout and go with it.
pub fn keep_script_model_assets(models: &mut ScriptedModelPaintPlan, keep_in: &[bool]) {
    debug_assert!(keep_in.len() == models.assets.len(), "one keep flag per asset");
    if keep_in.iter().all(|&kept| kept) {
        return;
    }
    let done = compact_survivors(models, keep_in);
    models.compact_vertices = done.packed.into_payload(
        done.verts.len(),
        FIGURE_COMPACT_EMPTY_PLAN,
        FIGURE_COMPACT_UNAVAILABLE,
    );
    models.verts = done.verts;
    models.indices = done.idx_list;
    models.facet_ranges = done.facet_ranges;
    models.mat_list = done.mat_list;
    models.assets = done.assets;
    models.draws.clear();
    models.owners.clear();

    // The revisions carry across: a retain reshuffles slots, it does not make
    // this a different plan, and a revision that restarted at zero is one a
    // consumer could mistake for the one it last saw.
    models.generation = models.generation.wrapping_add(1);
    models.revisions.bump_surfaces();
    models.revisions.advance_acceptance_stamp();
    restamp_topology(models);
}

// ---------------------------------------------------------------------------
// Remote bodies
// ---------------------------------------------------------------------------

/// The packed rows of a remote body plan while its blobs are appended.
pub struct RemoteBodyVertexStore(PackedRows);

impl RemoteBodyVertexStore {
    pub fn reserve_pack(&mut self, n: usize) {
        self.0.rows.reserve(n);
    }
}

pub fn end_body_packed_session(plan: &mut OtherBodyPaintPlan) -> RemoteBodyVertexStore {
    let no_vertices = plan.decoded_tally == 0 && plan.verts.is_empty();
    let drained = &mut plan.compact_vertices;
    RemoteBodyVertexStore(PackedRows::hand_over(
        drained,
        no_vertices,
        BODY_VERTS_DROPPED_NOTE,
    ))
}

pub fn begin_body_packed_session(plan_val: &mut OtherBodyPaintPlan, session: RemoteBodyVertexStore) {
    let RemoteBodyVertexStore(rows) = session;
    let (total, reason) = (plan_val.decoded_tally, BODY_VERTS_DROPPED_NOTE);
    plan_val.compact_vertices = rows.into_payload(total, FIGURE_COMPACT_EMPTY_PLAN, reason);
}

/// Appends a body mesh that exists only as packed rows and indices, and
/// returns where it landed as `(first vertex, first index)`.
pub fn add_other_body_software_mesh(
    body: &mut OtherBodyPaintPlan,
    packed: &mut RemoteBodyVertexStore,
    packed_rows_in: &[[u8; T6_PACKED_VERTEX]],
    indices_in: &[u32],
    decoded_tally: usize,
) -> Option<(u32, u32)> {
    if indices_in.is_empty() || decoded_tally == 0 {
        return None;
    }
    let landed = (body.decoded_tally as u32, body.indices.len() as u32);
    body.indices
        .extend(indices_in.iter().map(|&index| index + landed.0));
    body.decoded_tally = body.decoded_tally.saturating_add(decoded_tally);
    packed.0.extend(packed_rows_in, decoded_tally);
    Some(landed)
}

/// Queues a draw of the index run `first_triangle_slot..first_triangle_slot + index_count`; an
/// unlit or empty one draws nothing.
#[allow(clippy::too_many_arguments, reason = "kept as written")]
pub fn queue_other_body_cpu_paint(
    body: &mut OtherBodyPaintPlan,
    from_index: u32,
    run_len: u32,
    scene_from_local: Mat4,
    illum_handle: u32,
    light_slot: u8,
    env_probe_index: u8,
    mat_id: u32,
    ent: Option<u32>,
) {
    if run_len == 0 || illum_handle == 0 {
        return;
    }
    body.draws.push(OtherBodyFacetRun {
        surf: body.facet_ranges.len() as u32,
        material: mat_id,
        model_to_world: scene_from_local,
        illum_handle,
        stage_light_index: light_slot,
        env_probe_index,
        stage_actor_num: ent,
    });
    body.facet_ranges.push((from_index, run_len));
}

pub fn seal_other_body_batch(plan: &mut OtherBodyPaintPlan) {
    plan.revisions.topology =
        ix_layout_digest(&plan.indices, &plan.facet_ranges, plan.decoded_tally);
}

// ---------------------------------------------------------------------------
// First-person viewmodel
// ---------------------------------------------------------------------------

/// Publishes the rows a prepared rig settled: its indices, surface ranges,
/// materials and draws, plus a packed vertex buffer sized to the layout it laid
/// out. This runs when the composition changes; a pose writes into the buffer
/// and touches nothing else here.
pub fn install_prepared_fpv_plan(
    viewmodel: &mut FirstPersonPaintPlan,
    geometry: &PreparedFpvGeometry,
    hnd: u32,
) {
    let plan = viewmodel;
    let [if_empty, if_short] = [FPV_PACKED_BLANK_PLAN, FPV_PACKED_MISSING];
    geometry.write_indices(&mut plan.indices);
    plan.facet_ranges.clone_from(&geometry.facet_ranges);
    plan.mat_list.clone_from(&geometry.mat_list);
    plan.draws.clone_from(&geometry.draws);
    plan.geometry_ok = false;
    plan.illum_handle = hnd;
    plan.decoded_tally = geometry.dest_n;
    plan.compact_vertices = install_held_compact(
        geometry.compact_ok,
        vec![[0; T6_PACKED_VERTEX]; geometry.dest_n],
        geometry.dest_n,
        if_empty,
        if_short,
    );
    note_plan_counts(
        plan,
        [
            Some(geometry.hand_batches_planned),
            Some(geometry.gun_batches_planned),
            Some(geometry.scope_batches),
            Some(geometry.scope_body_batches),
            Some(geometry.scope_glass_batches),
            Some(geometry.batches_drawn),
            Some(geometry.batches_skipped),
        ],
    );
    restamp_fpv(plan);
}

/// Records how many surfaces each part of the viewmodel contributed, in the
/// order hands, gun, scope, scope housing, scope lens, draws, skipped.
fn note_plan_counts(
    fpv: &mut FirstPersonPaintPlan,
    [hands, gun, scope, house, lens, draws, skipped]: [Option<u32>; 7],
) {
    fpv.hand_batches_planned = hands;
    fpv.gun_batches_planned = gun;
    fpv.scope_batches = scope;
    fpv.scope_body_batches = house;
    fpv.scope_glass_batches = lens;
    fpv.batches_drawn = draws;
    fpv.batches_skipped = skipped;
}

/// Counts the surface change and follows the plan's topology.
fn restamp_fpv(plan_in: &mut FirstPersonPaintPlan) {
    plan_in.revisions.bump_surfaces();
    let fingerprint = ix_layout_digest(&plan_in.indices, &plan_in.facet_ranges, plan_in.decoded_tally);
    plan_in.plan_rev = bump_plan_mesh_rev(&mut plan_in.revisions, plan_in.plan_rev, fingerprint);
}

/// Nothing to draw. A plan that was already empty publishes no new revision: a
/// hidden viewmodel is not a reason for the merge downstream to re-copy a
/// vertex bank every frame.
pub fn clear_fpv_draw_plan(plan: &mut FirstPersonPaintPlan, illum_handle: u32) {
    let was_empty = plan.draws.is_empty() && plan.decoded_tally == 0;
    (plan.illum_handle, plan.geometry_ok) = (illum_handle, false);
    plan.settle_visible();
    // The rows are gone, so the rig that published them has to publish them
    // again before this plan draws anything: a viewmodel hidden without a
    // respawn behind it comes back to the same composition, not to an empty
    // plan.
    plan.rig_generation = 0;
    if was_empty {
        return;
    }
    let FirstPersonPaintPlan {
        indices: index_list,
        facet_ranges: range_list,
        mat_list: material_list,
        draws: draw_list,
        decoded_tally: vertex_total,
        ..
    } = plan;
    index_list.clear();
    range_list.clear();
    material_list.clear();
    draw_list.clear();
    *vertex_total = 0;
    plan.compact_vertices = PackedVertexPayload::Unavailable {
        origin_layout: FPV_PACKED_BLANK_PLAN,
    };
    note_plan_counts(
        plan,
        [None, None, Some(0), Some(0), Some(0), Some(0), Some(0)],
    );
    restamp_fpv(plan);
}

mod tests {
    #![cfg(test)]
    use super::*;
    use bevy::asset::RenderAssetUsages;
    use bevy::mesh::{Indices, PrimitiveTopology};

    use crate::motion::figure_pose::SurfaceDrawOwner;

    fn row(tag: u8) -> [u8; T6_PACKED_VERTEX] {
        [tag; T6_PACKED_VERTEX]
    }

    /// A single triangle whose vertices sit at x = `x`, `x + 1`, `x + 2`.
    fn surf(x: f32, packed_tag: Option<u8>) -> SkinnedMeshFacet {
        let topology = PrimitiveTopology::TriangleList;
        let mut geo = Mesh::new(
            topology,
            RenderAssetUsages::default(),
        );
        let corners = (0..3)
            .map(|step| [x + step as f32, 0.0, 0.0])
            .collect::<Vec<_>>();
        geo.insert_attribute(Mesh::ATTRIBUTE_POSITION, corners);
        geo.insert_indices(Indices::U32(vec![0, 1, 2]));
        let (surface_slot_number, model, owner) = (0, 0, SurfaceDrawOwner::Gun);
        let material = None;
        SkinnedMeshFacet {
            mesh: geo,
            compact_vertices: packed_tag.map_or_else(Vec::new, |tag| vec![row(tag); 3]),
            surface_slot_number,
            shader_ix: material,
            owner,
            mdl: model,
        }
    }

    fn material(sort_key: u8) -> Option<PropPassMaterial> {
        Some(crate::strokes::plain_test_material(sort_key))
    }

    fn xs(plan: &ScriptedModelPaintPlan) -> Vec<f32> {
        plan.vertices()
            .iter()
            .map(|vertex| vertex.position[0])
            .collect::<_>()
    }

    fn script_state() -> xmodel_runtime::RigPoseRecord {
        xmodel_runtime::RigPoseRecord::neutral_pose("crate".to_owned(), 1, 1)
    }

    fn add_asset(plan: &mut ScriptedModelPaintPlan, name: &str, x: f32, packed: Option<u8>) -> usize {
        push_script_prop_mesh(
            plan,
            LevelModelAssetId(name.to_owned()),
            script_state(),
            vec![None],
            &[surf(x, packed)],
            &[material(1)],
        )
    }

    // ----- packed rows ---------------------------------------------------

    #[test]
    fn packed_rows_start_intact_only_for_a_plan_that_can_extend_them() {
        let mut empty = PackedVertexPayload::Unavailable { origin_layout: "x" };
        assert!(PackedRows::hand_over(&mut empty, true, "gone").intact);
        let mut stale = PackedVertexPayload::Unavailable { origin_layout: "x" };
        assert!(!PackedRows::hand_over(&mut stale, false, "gone").intact);
        let mut held = PackedVertexPayload::T6(vec![row(1)]);
        let taken = PackedRows::hand_over(&mut held, false, "gone");
        assert!(taken.intact && taken.rows.len() == 1);
        assert!(matches!(
            held,
            PackedVertexPayload::Unavailable {
                origin_layout: "gone"
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

    #[test]
    fn empty_rows_are_intact_only_when_asked() {
        assert!(PackedRows::blank(true).intact && PackedRows::blank(true).rows.is_empty());
        assert!(!PackedRows::blank(false).intact);
    }

    #[test]
    fn pairing_drops_surfaces_without_a_material() {
        let surfaces = [surf(0.0, None), surf(1.0, None), surf(2.0, None)];
        let materials_val = [material(4), None, material(6)];
        let pairs = surfaces_with_materials(&surfaces, &materials_val);
        let keys: Vec<u8> = pairs.iter().map(|(_, m)| m.order_key).collect();
        assert_eq!(keys, [4, 6]);
        assert!(std::ptr::eq(pairs[1].0, &surfaces[2]));
    }

    #[test]
    fn held_rows_only_for_t6_payloads() {
        let held = PackedVertexPayload::T6(vec![row(1), row(2)]);
        assert_eq!(held_rows(&held).map(<[_]>::len), Some(2));
        assert!(held_rows(&PackedVertexPayload::default()).is_none());
    }

    // ----- appending surfaces ---------------------------------------------

    #[test]
    fn appended_surfaces_land_in_consecutive_slots() {
        let mut missiles = ThrownPaintPlan::default();
        let slots = push_projectile_surfaces(
            &mut missiles,
            &[surf(0.0, Some(1)), surf(10.0, Some(2))],
            &[material(1), material(2)],
        );
        assert_eq!(slots, [(0, 0), (1, 1)]);
        assert_eq!(missiles.facet_ranges(), [(0, 3), (3, 3)]);
        assert_eq!(missiles.indices(), [0, 1, 2, 3, 4, 5]);
        assert_eq!(missiles.vertices().len(), 6);
        assert!(
            matches!(missiles.compact_vertices(), PackedVertexPayload::T6(rows) if rows.len() == 6)
        );
        assert_eq!(missiles.revisions.vertices, 1);
        assert_eq!(missiles.revisions.materials, 1);
    }

    #[test]
    fn surfaces_without_a_material_or_mesh_are_skipped() {
        let mut plan = PickupPaintPlan::default();
        let mut no_mesh = surf(0.0, Some(1));
        let topology = PrimitiveTopology::TriangleList;
        no_mesh.mesh = Mesh::new(
            topology,
            RenderAssetUsages::default(),
        );
        let slots = push_pickup_surfaces(
            &mut plan,
            &[surf(0.0, Some(1)), no_mesh, surf(5.0, Some(1))],
            &[None, material(1), None],
        );
        assert!(slots.is_empty());
        assert!(plan.vertices().is_empty());
        assert!(matches!(
            plan.compact_vertices(),
            PackedVertexPayload::Unavailable { origin_layout } if *origin_layout == FIGURE_COMPACT_EMPTY_PLAN
        ));
    }

    #[test]
    fn missing_rows_leave_the_plan_without_packed_vertices() {
        let mut plan = DynEntPaintPlan::default();
        push_loose_prop_surfaces(
            &mut plan,
            &[surf(0.0, Some(1)), surf(10.0, None)],
            &[material(1), material(1)],
        );
        assert_eq!(plan.vertices().len(), 6);
        assert!(matches!(
            plan.compact_vertices(),
            PackedVertexPayload::Unavailable { origin_layout } if *origin_layout == FIGURE_COMPACT_UNAVAILABLE
        ));
        // Rows that were never complete cannot be completed by a later asset.
        push_loose_prop_surfaces(&mut plan, &[surf(20.0, Some(1))], &[material(1)]);
        assert!(matches!(
            plan.compact_vertices(),
            PackedVertexPayload::Unavailable { .. }
        ));
    }

    #[test]
    fn a_dynent_asset_remembers_its_slots() {
        let mut plan = DynEntPaintPlan::default();
        let door = || LevelModelAssetId("door".to_owned());
        let slots = append_debris_asset(
            &mut plan,
            door(),
            Some(1),
            &[surf(0.0, Some(1))],
            &[material(1)],
        );
        let remembered = plan
            .asset_for(&door(), Some(1))
            .map(|asset| asset.surf_pairs.clone());
        assert_eq!(remembered, Some(slots));
    }

    // ----- script models ---------------------------------------------------

    #[test]
    fn script_model_assets_are_numbered_in_order() {
        let mut plan = ScriptedModelPaintPlan::default();
        assert_eq!(add_asset(&mut plan, "a", 0.0, Some(1)), 0);
        assert_eq!(add_asset(&mut plan, "b", 10.0, Some(2)), 1);
        assert_eq!(plan.assets[1].surfs, [(1, 1)]);
        assert_eq!(plan.plan_rev, 2, "each new layout is a revision");
        assert_eq!(
            plan.revisions.topology,
            ix_layout_digest(plan.indices(), plan.facet_ranges(), 6)
        );
    }

    #[test]
    fn overwriting_an_asset_rewrites_its_slots_only() {
        let mut plan = ScriptedModelPaintPlan::default();
        add_asset(&mut plan, "a", 0.0, Some(1));
        add_asset(&mut plan, "b", 10.0, Some(2));
        let revision = plan.plan_rev;

        assert!(swap_script_prop_mesh(
            &mut plan,
            1,
            script_state(),
            &[surf(50.0, Some(9))],
            &[material(7)],
        ));
        assert_eq!(xs(&plan), [0.0, 1.0, 2.0, 50.0, 51.0, 52.0]);
        assert!(
            matches!(plan.compact_vertices(), PackedVertexPayload::T6(rows) if rows[3] == row(9) && rows[0] == row(1))
        );
        assert_eq!(plan.materials()[1].order_key, 7);
        assert_eq!(plan.materials()[0].order_key, 1);
        assert_eq!(plan.plan_rev, revision + 1);
    }

    #[test]
    fn an_overwrite_that_does_not_fit_changes_nothing() {
        let mut plan = ScriptedModelPaintPlan::default();
        add_asset(&mut plan, "a", 0.0, Some(1));
        let revision = plan.plan_rev;
        let before = xs(&plan);

        let refusals: [(usize, Vec<SkinnedMeshFacet>); 3] = [
            (5, vec![surf(1.0, Some(1))]),
            (0, vec![surf(1.0, Some(1)), surf(1.0, Some(1))]),
            (0, vec![surf(1.0, None)]),
        ];
        for (index, surfaces) in refusals {
            let mats = vec![material(1); surfaces.len()];
            assert!(!swap_script_prop_mesh(
                &mut plan,
                index,
                script_state(),
                &surfaces,
                &mats
            ));
        }
        assert_eq!(xs(&plan), before);
        assert_eq!(plan.plan_rev, revision);
    }

    #[test]
    fn an_overwrite_with_a_different_index_count_is_refused() {
        let mut plan = ScriptedModelPaintPlan::default();
        add_asset(&mut plan, "a", 0.0, Some(1));
        let mut longer = surf(1.0, Some(1));
        longer
            .mesh
            .insert_indices(Indices::U32(vec![0, 1, 2, 2, 1, 0]));
        assert!(!swap_script_prop_mesh(
            &mut plan,
            0,
            script_state(),
            &[longer],
            &[material(1)]
        ));
    }

    #[test]
    fn retaining_every_asset_is_a_no_op() {
        let mut plan_here = ScriptedModelPaintPlan::default();
        add_asset(&mut plan_here, "a", 0.0, Some(1));
        let before = (plan_here.plan_rev, plan_here.revisions, plan_here.generation);
        keep_script_model_assets(&mut plan_here, &[true]);
        assert_eq!((plan_here.plan_rev, plan_here.revisions, plan_here.generation), before);
    }

    #[test]
    fn retaining_repacks_the_survivors() {
        let mut batch = ScriptedModelPaintPlan::default();
        add_asset(&mut batch, "a", 0.0, Some(1));
        add_asset(&mut batch, "b", 10.0, Some(2));
        add_asset(&mut batch, "c", 20.0, Some(3));
        batch.draws.push(XModelSurfaceDrawStub::draw());
        let revision = batch.plan_rev;

        keep_script_model_assets(&mut batch, &[false, true, true]);

        assert_eq!(xs(&batch), [10.0, 11.0, 12.0, 20.0, 21.0, 22.0]);
        assert_eq!(batch.indices(), [0, 1, 2, 3, 4, 5]);
        assert_eq!(batch.facet_ranges(), [(0, 3), (3, 3)]);
        assert_eq!(batch.assets.len(), 2);
        assert_eq!(batch.assets[0].key, LevelModelAssetId("b".to_owned()));
        assert_eq!(batch.assets[1].surfs, [(1, 1)]);
        assert!(matches!(
            batch.compact_vertices(),
            PackedVertexPayload::T6(rows) if rows.len() == 6 && rows[0] == row(2) && rows[5] == row(3)
        ));
        assert!(batch.draws().is_empty() && batch.owners.is_empty());
        assert_eq!(batch.generation, 1);
        assert_eq!(batch.plan_rev, revision + 1);
    }

    #[test]
    fn retaining_nothing_leaves_an_empty_plan() {
        let mut script_plan = ScriptedModelPaintPlan::default();
        add_asset(&mut script_plan, "a", 0.0, Some(1));
        keep_script_model_assets(&mut script_plan, &[false]);
        assert!(script_plan.assets.is_empty() && script_plan.vertices().is_empty() && script_plan.indices().is_empty());
        assert!(matches!(
            script_plan.compact_vertices(),
            PackedVertexPayload::Unavailable { .. }
        ));
    }

    /// A draw row with nothing set, for plans whose rows are only counted.
    struct XModelSurfaceDrawStub;

    impl XModelSurfaceDrawStub {
        fn draw() -> render_scene::FigureFacetDraw {
            let ident = Mat4::IDENTITY;
            let material = 0;
            let no_ent = None;
            render_scene::FigureFacetDraw {
                map_from_self: ident,
                material,
                stage_actor_num: no_ent,
                surface: 0,
                illum_handle: 0,
                queued_illum: None,
                colour_refusal: None,
                object_tag: 0,
                stage_light_index: 0,
                env_probe_index: 0,
                compact_illum: None,
                is_scoped_view: false,
                caster_bound: None,
            }
        }
    }

    // ----- remote bodies ---------------------------------------------------

    #[test]
    fn body_blobs_rebase_their_indices() {
        let mut body_plan = OtherBodyPaintPlan::default();
        let mut session_here = end_body_packed_session(&mut body_plan);
        assert_eq!(
            add_other_body_software_mesh(&mut body_plan, &mut session_here, &[row(1); 3], &[0, 1, 2], 3),
            Some((0, 0))
        );
        assert_eq!(
            add_other_body_software_mesh(&mut body_plan, &mut session_here, &[row(2); 2], &[1, 0, 1], 2),
            Some((3, 3))
        );
        begin_body_packed_session(&mut body_plan, session_here);
        assert_eq!(body_plan.decoded_tally(), 5);
        assert_eq!(body_plan.indices(), [0, 1, 2, 4, 3, 4]);
        assert!(matches!(body_plan.compact_vertices(), PackedVertexPayload::T6(rows) if rows.len() == 5));
    }

    #[test]
    fn empty_body_blobs_add_nothing() {
        let mut plan = OtherBodyPaintPlan::default();
        let mut session_in = end_body_packed_session(&mut plan);
        assert_eq!(
            add_other_body_software_mesh(&mut plan, &mut session_in, &[], &[0], 0),
            None
        );
        assert_eq!(
            add_other_body_software_mesh(&mut plan, &mut session_in, &[row(1)], &[], 1),
            None
        );
        assert_eq!(plan.decoded_tally(), 0);
    }

    #[test]
    fn a_body_blob_with_the_wrong_row_count_drops_the_packed_rows() {
        let mut body_plan = OtherBodyPaintPlan::default();
        let mut session_val = end_body_packed_session(&mut body_plan);
        add_other_body_software_mesh(&mut body_plan, &mut session_val, &[row(1)], &[0, 0, 0], 3);
        begin_body_packed_session(&mut body_plan, session_val);
        assert!(matches!(
            body_plan.compact_vertices(),
            PackedVertexPayload::Unavailable { origin_layout } if *origin_layout == BODY_VERTS_DROPPED_NOTE
        ));
        assert_eq!(body_plan.decoded_tally(), 3, "the blob itself is still counted");
    }

    #[test]
    fn body_draws_need_light_and_indices() {
        let mut plan = OtherBodyPaintPlan::default();
        let ident = Mat4::IDENTITY;
        queue_other_body_cpu_paint(&mut plan, 0, 3, ident, 0, 0, 0, 0, None);
        queue_other_body_cpu_paint(&mut plan, 0, 0, ident, 5, 0, 0, 0, None);
        assert!(plan.draws().is_empty() && plan.facet_ranges().is_empty());

        queue_other_body_cpu_paint(&mut plan, 3, 6, ident, 5, 1, 2, 4, Some(8));
        queue_other_body_cpu_paint(&mut plan, 9, 6, ident, 5, 1, 2, 4, None);
        assert_eq!(plan.facet_ranges(), [(3, 6), (9, 6)]);
        let drawn = plan.draws()[0];
        assert_eq!(
            (drawn.surf, drawn.material, drawn.illum_handle),
            (0, 4, 5)
        );
        assert_eq!(
            (drawn.stage_light_index, drawn.env_probe_index),
            (1, 2)
        );
        assert_eq!(drawn.stage_actor_num, Some(8));
        assert_eq!(plan.draws()[1].surf, 1);
    }

    #[test]
    fn finishing_a_body_plan_stamps_its_topology() {
        let mut plan = OtherBodyPaintPlan::default();
        let mut session_v = end_body_packed_session(&mut plan);
        add_other_body_software_mesh(&mut plan, &mut session_v, &[row(1); 3], &[0, 1, 2], 3);
        let ident = Mat4::IDENTITY;
        queue_other_body_cpu_paint(&mut plan, 0, 3, ident, 1, 0, 0, 0, None);
        let finished = &mut plan;
        seal_other_body_batch(finished);
        assert_eq!(
            plan.revisions.topology,
            ix_layout_digest(&[0, 1, 2], &[(0, 3)], 3)
        );
    }

    // ----- viewmodel ---------------------------------------------------

    #[test]
    fn clearing_an_empty_viewmodel_plan_publishes_nothing() {
        let mut plan = FirstPersonPaintPlan {
            rig_generation: 4,
            ..Default::default()
        };
        clear_fpv_draw_plan(&mut plan, 3);
        assert_eq!(plan.illum_handle, 3);
        assert_eq!(plan.rig_generation, 0);
        assert_eq!((plan.plan_rev, plan.revisions), (0, Default::default()));
    }

    #[test]
    fn clearing_a_viewmodel_plan_with_rows_publishes_once() {
        let mut plan = FirstPersonPaintPlan {
            decoded_tally: 3,
            indices: Vec::from([0, 1, 2]),
            facet_ranges: vec![(0, 3)],
            geometry_ok: true,
            placement_ok: true,
            ..Default::default()
        };
        plan.visible = plan.geometry_ok;
        let (surface, material) = (0, 0);
        plan.draws.push(crate::strokes::GunSurfaceBatch {
            is_scoped_view: false,
            surf: surface,
            material,
        });
        clear_fpv_draw_plan(&mut plan, 0);
        assert!(!plan.visible && !plan.geometry_ok);
        assert!(plan.indices().is_empty() && plan.draws().is_empty() && plan.decoded_tally() == 0);
        assert_eq!(
            (plan.hand_batches_planned, plan.scope_batches, plan.batches_drawn),
            (None, Some(0), Some(0))
        );
        assert_eq!(plan.plan_rev, 1);

        clear_fpv_draw_plan(&mut plan, 0);
        assert_eq!(plan.plan_rev, 1, "an empty plan stays quiet");
    }
}
