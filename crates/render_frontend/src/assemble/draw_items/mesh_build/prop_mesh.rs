//! Static-model geometry: every placed prop's surfaces decoded once into
//! shared vertex, index and range rows, the pass materials they draw with,
//! and the placements that point at them.
//!
//! `pack_prop_meshes` turns the loaded meshes into a [`PropGpuPlan`]'s
//! geometry; `build_prop_gpu_plan` adds the materials and placements once
//! the world is spawned. The sky model reuses the packing on its own.

use std::collections::HashMap;
use std::sync::Arc;

use bevy::mesh::VertexAttributeValues;
use bevy::prelude::*;

use crate::prepare::scene::world::{LevelPropMesh, LevelPropFacet};

pub use render_frame::{PackedVertexRefusal, PropVertex};
pub use render_scene::{
    DesignerTextureSet, DetailRampArgs, PropPassMaterial, live_model_tables, prop_camera_detail,
};


/// `(range index, authored material)` of one packed surface.

/// One mesh's surfaces per level of detail, with where each landed in the
/// plan's shared rows.
#[derive(Clone, Debug, Default)]
pub struct PropMeshFacets {
    /// `(range index, authored material)` of every surface that packed.
    pub facets_by_detail: [Vec<(u32, Option<assets::SurfaceMaterialId>)>; 4],
    pub prop_cache_facets_by_detail: [Vec<PropCachedFacetSrc>; 4],
    /// The `xsurface + 1` byte of every authored surface, packed or not.
    pub figure_part_plus_1_by_detail: [Vec<Option<u8>>; 4],

    pub vertex_first: u32,
    pub vertex_tally: u32,
    pub vertex_tally_by_detail: [u32; 4],

    pub detail_prop_cache: Option<[u8; 4]>,
    pub detail_prop_cache_rows: Option<[[u8; 4]; 4]>,
    pub lod: Option<asset_model::ModelDetailSelector>,
}

/// Where one packed surface sits: its range, its packed vertex rows, its
/// decoded vertices and indices, and the source surface it came from.
#[derive(Clone, Debug)]
pub struct PropCachedFacetSrc {
    pub span_idx: u32,
    pub mat: Option<assets::SurfaceMaterialId>,
    pub compact_off: u32,
    pub compact_tally: u32,
    pub base_vertex_at: u32,
    pub first_triangle_slot: u32,
    pub idx_count: u32,
    pub figure_part_base_index: u16,
    pub figure_part_vertex_offset: u16,
}

/// One placed static model.
#[derive(Clone, Debug)]
pub struct PropPlacement {
    pub model_slot: usize,
    pub ent: Entity,
    /// The slot in the world's authored placement list.
    pub illum_slot: Option<usize>,
    pub lit_list: bool,
    pub mode_bits: u8,
    pub env_probe_index: u8,
    pub key_light_index: u8,
    pub compact_illum: Option<[u8; 4]>,

    pub local_to_world: Mat4,
    pub at_pos: [f32; 3],
    pub axes_set: [[f32; 3]; 3],
    pub size_k: f32,

    /// bo2zm: a Black Ops II placement's primary light and its visibility
    /// (see `asset_world::FixedPropPlacement::t6_primary`).
    pub t6_primary: [u8; 2],
    /// bo2mp: see `asset_world::FixedPropPlacement::t6_light`.
    pub t6_light: u32,
    /// bo2zm: beyond this distance the game does not draw the placement
    /// (`GfxStaticModelDrawInst::cullDist`; 0 = always drawn).
    pub t6_cull_dist: f32,
}

#[derive(Resource, Clone, Debug, Default)]
pub struct PropGpuPlan {
    pub compact_vertices: asset_world::PackedVertexPayload,
    pub materials: Vec<PropPassMaterial>,
    pub meshes: Vec<PropMeshFacets>,
    pub placements: Vec<PropPlacement>,
    /// For each authored slot, the placement built for it.
    pub designer_placement_indices: Vec<Option<usize>>,
    pub mat_state_flags: HashMap<assets::SurfaceMaterialId, u8>,
    pub shadow_draw_instances: Vec<dpvs_t6::SmodelShadowInst>,

    /// Pass material index by (authored material, reflection probe).
    pub shaded_material_key: HashMap<(Option<assets::SurfaceMaterialId>, u8), u32>,
    pub flat_material_key: HashMap<Option<assets::SurfaceMaterialId>, u32>,

    pub compact_portion: Option<Arc<Vec<[u8; asset_t6::T6_PACKED_VERTEX]>>>,
    pub index_portion: Option<Arc<Vec<u32>>>,
    pub range_portion: Option<Arc<Vec<(u32, u32)>>>,
    /// Per range, `(first packed row, row count)`.
    pub vert_range_share: Option<Arc<Vec<(u32, u32)>>>,
    pub decoded_share: Option<Arc<Vec<PropVertex>>>,
    /// Baked per-placement vertices. No lane fills it any more; the extract
    /// still reads it, so it stays until that goes.
    pub cached_share: Option<Arc<Vec<[u8; asset_t6::T6_PACKED_VERTEX]>>>,
}

// A prop instance row of the fallback pass: its 4x4 world-from-local, then
// the slots below. `T6_PROP_ROW` floats in all.
const T6_PROP_ROW: usize = 28;
const ROW_LIGHT: usize = 16;
const ROW_PRIMARY: usize = 20;
const ROW_VISIBILITY: usize = 21;
const ROW_CULL: usize = 22;
const ROW_PROBE: usize = 23;
const ROW_SWAY: usize = 24;
const ROW_OWN_LIGHT: usize = 27;

/// bo2zm: one Black Ops II prop instance for the renderer's fallback pass:
/// its world-from-local columns, then its light (rgb, linear and already
/// times the exposure, w = 0), or for baked vertex light w = the exposure;
/// then its primary light index, that light's visibility (0..1) and its
/// cull distance (0 = always drawn). bo2mp: a tree canopy's row holds its
/// colour tint in the primary light's place and, negative, its canopy
/// constants in the probe's (see `t6_canopy_params`), then a swaying
/// canopy's treeCanopySwayParms xyz (zero for any other row), and the
/// placement's own light (`asset_world::t6_model_lights` index plus one).

// Constant hashes of the tree canopy shaders' parameters.
const CANOPY_LIGHTING_PARMS: u32 = 0x3e1b_784f;
const CANOPY_SCATTER_COLOR: u32 = 0xf5e3_2317;
const CANOPY_COLOR_TINT: u32 = 0xb60c_3b3a;
const CANOPY_SWAY_PARMS: u32 = 0x8147_ad2d;

fn material_constant(material: &render_material::LiveMaterial, hash: u32) -> Option<[f32; 4]> {
    let constants = &material.constants;
    constants
        .iter()
        .find(|(h, _)| *h == hash)
        .map(|(_, bits)| bits.map(f32::from_bits))
}

/// bo2mp: a tree canopy material's constants as BO2's `treecanopy` shaders
/// read them (fxc disassembly): `[bend | scatter r << 6 | g << 12 | b << 18,
/// tint r | g << 6 | b << 12]`, each six bits for 0..1. Bend is
/// treeCanopyLightingParms.x (how far the normal turns out from the model's
/// origin), scatter treeCanopyScatterColor (the sun through the leaves),
/// tint colorTint (times the colour map). A missing one is 0, white, white.
fn t6_canopy_params(material: &render_material::LiveMaterial) -> [u32; 2] {
    let six_bits = |x: f32| (x.clamp(0.0, 1.0) * 63.0).round() as u32;
    let pack_rgb = |c: [f32; 4]| six_bits(c[0]) | six_bits(c[1]) << 6 | six_bits(c[2]) << 12;
    let bend = material_constant(material, CANOPY_LIGHTING_PARMS).map_or(0.0, |c| c[0]);
    let scatter = material_constant(material, CANOPY_SCATTER_COLOR).unwrap_or([1.0; 4]);
    let tint = material_constant(material, CANOPY_COLOR_TINT).unwrap_or([1.0; 4]);
    [six_bits(bend) | pack_rgb(scatter) << 6, pack_rgb(tint)]
}

/// bo2mp: a swaying canopy's treeCanopySwayParms xyz (the `_sm` canopy
/// vertex shader, fxc disassembly): how much of the vertex's turn always
/// shows (x), how much its own green adds (y), and the turn's scale on the
/// wind's rustle (z). Zero for a canopy that does not sway.
fn t6_canopy_sway(material: &render_material::LiveMaterial) -> [f32; 3] {
    if !material.t6_draw.is_some_and(|d| d.sway) {
        return [0.0; 3];
    }
    material_constant(material, CANOPY_SWAY_PARMS).map_or([0.0; 3], |c| [c[0], c[1], c[2]])
}

/// bo2mp: a lit tree canopy material's constants for its own instance row
/// (`t6_canopy_params`, `t6_canopy_sway`); None for any other material.
/// The placed props and the script models (BO2's animated vines) both
/// draw canopies from such a row.
pub fn t6_canopy(material: &render_material::LiveMaterial) -> Option<([u32; 2], [f32; 3])> {
    let lit_foliage = material.t6_draw.is_some_and(|d| d.foliage && !d.unlit);
    lit_foliage.then(|| (t6_canopy_params(material), t6_canopy_sway(material)))
}

/// A placement's fallback-pass row; `light` is the placement's packed
/// lighting and `exposure` scales the decoded colour.
fn t6_prop_row(placement: &PropPlacement, light: [u8; 4], exposure: f32) -> [f32; T6_PROP_ROW] {
    let mut row = [0.0f32; T6_PROP_ROW];
    row[..16].copy_from_slice(&placement.local_to_world.to_cols_array());
    // 255 in the last byte marks baked vertex light: the row carries only
    // the exposure.
    row[ROW_LIGHT..ROW_LIGHT + 4].copy_from_slice(&if light[3] == 255 {
        [0.0, 0.0, 0.0, exposure]
    } else {
        let decode = |byte: u8| {
            let v = f32::from(byte) / 255.0;
            v * v * 32.0 * exposure
        };
        [decode(light[0]), decode(light[1]), decode(light[2]), 0.0]
    });
    row[ROW_PRIMARY] = f32::from(placement.t6_primary[0]);
    row[ROW_VISIBILITY] = f32::from(placement.t6_primary[1]) / 255.0;
    row[ROW_CULL] = placement.t6_cull_dist;
    // The placement's reflection probe, plus one (0 = none).
    row[ROW_PROBE] = f32::from(placement.env_probe_index) + 1.0;
    row[ROW_OWN_LIGHT] = placement.t6_light as f32;
    row
}

impl PropGpuPlan {
    /// bo2zm: the Black Ops II props the fallback pass draws (their
    /// materials have no technique, so no draw item is ever emitted for
    /// them): for each placement with a packed light, its first-LOD
    /// surfaces as `[surface, colour slot, draw code, instance]`, and each
    /// instance's transform and light. The third list is, per draw, its
    /// normal and specular map slots (shine) and, bo2mp, its layer 1 slot
    /// (an emissive flow's remap).
    pub fn t6_fallback_props(
        &self,
        catalog: &crate::assemble::draw_items::LiveMaterialLedger,
        exposure: f32,
    ) -> (Vec<[u32; 4]>, Vec<[f32; T6_PROP_ROW]>, Vec<[u32; 3]>) {
        const SHINE_SEMANTICS: [u8; 3] = [
            asset_material::SEMANTIC_NORMAL,
            asset_material::SEMANTIC_SPECULAR,
            0xf1,
        ];
        let slot_of = |runtime: Option<&render_material::LiveMaterial>, semantic: u8| {
            runtime
                .and_then(|m| m.semantic_for_image(semantic))
                .map_or(u32::MAX, |id| id.0)
        };
        let mut draws = vec![];
        let mut rows: Vec<[f32; T6_PROP_ROW]> = Vec::new();
        let mut shine = Vec::new();
        for placement in &self.placements {
            let (Some(light), Some(mesh)) =
                (placement.compact_illum, self.meshes.get(placement.model_slot))
            else {
                continue;
            };
            let plain_row = t6_prop_row(placement, light, exposure);
            let plain_at = rows.len() as u32;
            rows.push(plain_row);
            // bo2mp: a tree canopy gets its own copy of the row per set of
            // canopy constants (see `t6_canopy_params`).
            let mut canopy_rows: HashMap<([u32; 2], [u32; 3]), u32> = HashMap::default();
            for &(facet, material) in &mesh.facets_by_detail[0] {
                let live = material.and_then(|id| catalog.derived(id));
                let canopy = live.and_then(t6_canopy);
                let at = match canopy {
                    None => plain_at,
                    Some((params, sway)) => {
                        *canopy_rows
                            .entry((params, sway.map(f32::to_bits)))
                            .or_insert_with(|| {
                                let mut copy = plain_row;
                                copy[ROW_PRIMARY] = params[1] as f32;
                                copy[ROW_PROBE] = -1.0 - params[0] as f32;
                                copy[ROW_SWAY..ROW_SWAY + 3].copy_from_slice(&sway);
                                rows.push(copy);
                                rows.len() as u32 - 1
                            })
                    }
                };
                let code = live
                    .and_then(|m| m.t6_draw)
                    .map_or(asset_core::T6Draw::NONE_CODE, asset_core::T6Draw::code);
                // bo2zm 27c: a cloud sheet takes no light: its own row holds
                // its constants in the light slots (1 / FadeFactor,
                // ColorSpeed.xy, ColorScale.x) and hdrAmount squared in the
                // primary light's (the cull distance kept).
                let at = match live.filter(|m| m.t6_draw.is_some_and(|d| d.sheet)) {
                    Some(m) => {
                        let sheet = m.sheet();
                        let mut sheet_row = rows[at as usize];
                        sheet_row[ROW_LIGHT..ROW_LIGHT + 4].copy_from_slice(&sheet[..4]);
                        sheet_row[ROW_PRIMARY] = sheet[4];
                        rows.push(sheet_row);
                        rows.len() as u32 - 1
                    }
                    None => at,
                };
                draws.push([
                    facet,
                    slot_of(live, asset_material::SEMANTIC_COLOR),
                    code,
                    at,
                ]);
                // The canopy shaders read the colour map alone: no shine.
                shine.push(if canopy.is_some() {
                    [u32::MAX; 3]
                } else {
                    SHINE_SEMANTICS.map(|semantic| slot_of(live, semantic))
                });
            }
        }
        (draws, rows, shine)
    }

    /// The packed rows the extract may take: the published share, else the
    /// payload still held here.
    fn packed_slice(&self) -> Option<&[[u8; asset_t6::T6_PACKED_VERTEX]]> {
        match (&self.compact_portion, &self.compact_vertices) {
            (Some(share), _) => Some(share.as_slice()),
            (None, asset_world::PackedVertexPayload::T6(rows)) => Some(rows.as_slice()),
            (None, asset_world::PackedVertexPayload::Unavailable { .. }) => None,
        }
    }

    pub fn precise_compact_vertices(&self) -> Result<&[[u8; asset_t6::T6_PACKED_VERTEX]], PackedVertexRefusal> {
        let table_stride = render_material::decl_table::stream_extent(
            render_material::decl_table::COMPRESSED_VERTEX_KIND,
            0,
        );
        if table_stride != Some(asset_t6::T6_PACKED_VERTEX as u16) {
            return Err(PackedVertexRefusal::StrideMismatch { table_stride });
        }
        let packed = match self.packed_slice() {
            Some(rows) => rows,
            None => {
                let asset_world::PackedVertexPayload::Unavailable { origin_layout } =
                    &self.compact_vertices
                else {
                    unreachable!("a T6 payload always has rows");
                };
                return Err(PackedVertexRefusal::ForeignLayout {
                    origin_layout: *origin_layout,
                });
            }
        };
        let decoded = self.decoded_vertices().len();
        if packed.len() != decoded {
            return Err(PackedVertexRefusal::CountsDiffer {
                packed: packed.len(),
                decoded,
            });
        }
        Ok(packed)
    }

    pub fn indices(&self) -> &[u32] {
        super::published_rows(&self.index_portion)
    }

    pub fn facet_ranges(&self) -> &[(u32, u32)] {
        super::published_rows(&self.range_portion)
    }

    pub fn decoded_vertices(&self) -> &[PropVertex] {
        super::published_rows(&self.decoded_share)
    }

    /// Moves the packed payload behind an `Arc` so the extract can share it.
    pub fn publish_extract_shares(&mut self) {
        if self.compact_portion.is_some() {
            return;
        }
        let payload = std::mem::replace(
            &mut self.compact_vertices,
            asset_world::PackedVertexPayload::Unavailable {
                origin_layout: "packed payload published for extract",
            },
        );
        if let asset_world::PackedVertexPayload::T6(rows) = payload {
            self.compact_portion = Some(Arc::new(rows));
        } else {
            self.compact_vertices = payload;
        }
    }

    /// Gives `source` (lit against `lit_probe`, or unlit) a pass material
    /// unless it already has one. A shadow-only material never gets one.
    fn intern_material(
        &mut self,
        factory: &MaterialFactory,
        source: Option<assets::SurfaceMaterialId>,
        lit_probe: Option<u8>,
    ) {
        let seen = match lit_probe {
            Some(probe) => self.shaded_material_key.contains_key(&(source, probe)),
            None => self.flat_material_key.contains_key(&source),
        };
        if seen {
            return;
        }
        let Some(material) = factory.build(source, lit_probe) else {
            return;
        };
        let slot = self.materials.len() as u32;
        self.materials.push(material);
        match lit_probe {
            Some(probe) => self.shaded_material_key.insert((source, probe), slot),
            None => self.flat_material_key.insert(source, slot),
        };
    }

    pub fn compact_ok(&self) -> bool {
        self.packed_slice().is_some()
    }

    pub fn compact_row_tally(&self) -> Option<usize> {
        self.packed_slice().map(<[_]>::len)
    }
}

/// A float column of a mesh attribute, when the mesh has it in that format.
macro_rules! float_column {
    ($fn_name:ident, $variant:ident, $width:literal) => {
        fn $fn_name(values: &VertexAttributeValues) -> Option<&[[f32; $width]]> {
            match values {
                VertexAttributeValues::$variant(rows) => Some(rows.as_slice()),
                _ => None,
            }
        }
    };
}
float_column!(f32x2, Float32x2, 2);
float_column!(f32x3, Float32x3, 3);
float_column!(f32x4, Float32x4, 4);

/// Why a surface did not pack.
enum Unpacked {
    MissingGeometry,
    RowCount { fitted: usize, unpacked: usize },
}

impl Unpacked {
    fn describe(&self, model: &str) -> String {
        match self {
            Self::MissingGeometry => format!("empty_ib:{model}"),
            Self::RowCount { fitted, unpacked: decoded } => {
                format!("{model} packed={fitted} decoded={decoded}")
            }
        }
    }
}

/// The shared rows while surfaces are appended to them.
#[derive(Default)]
struct Staging {
    vertices: Vec<PropVertex>,
    index_list: Vec<u32>,
    index_spans: Vec<(u32, u32)>,
    t6_rows: Vec<[u8; asset_t6::T6_PACKED_VERTEX]>,
    packed_ranges: Vec<(u32, u32)>,
}

impl Staging {
    /// Decodes one mesh onto the end of the shared rows. Returns the surface's
    /// range index, or None (nothing appended) for a mesh with no positions, no
    /// indices or an index past its vertices.
    fn decode(&mut self, mesh: &Mesh) -> Option<u32> {
        let points = mesh.attribute(Mesh::ATTRIBUTE_POSITION).and_then(f32x3)?;
        let surface_directions = mesh.attribute(Mesh::ATTRIBUTE_NORMAL).and_then(f32x3);
        let texcoord_set = mesh.attribute(Mesh::ATTRIBUTE_UV_0).and_then(f32x2);
        let tints = mesh.attribute(Mesh::ATTRIBUTE_COLOR).and_then(f32x4);
        let corners = mesh.indices()?;
        let in_bounds = !corners.is_empty() && corners.iter().all(|i| i < points.len());
        if points.is_empty() || !in_bounds {
            return None;
        }
        let first_vertex = self.vertices.len() as u32;
        self.vertices.extend(points.iter().enumerate().map(|(i, &position)| PropVertex {
            position,
            normal: surface_directions.and_then(|n| n.get(i)).copied().unwrap_or([0.0, 0.0, 1.0]),
            color: tints.and_then(|c| c.get(i)).copied().unwrap_or([1.0; 4]),
            first_texcoord: texcoord_set.and_then(|t| t.get(i)).copied().unwrap_or([0.0; 2]),
        }));
        let first_index = self.index_list.len() as u32;
        self.index_list.extend(corners.iter().map(|i| first_vertex + i as u32));
        self.index_spans.push((first_index, corners.len() as u32));
        Some(self.index_spans.len() as u32 - 1)
    }

    /// Appends a surface's decoded and packed rows together; they must have
    /// the same count or the surface is left out entirely.
    fn append(&mut self, facet: &LevelPropFacet) -> Result<PropCachedFacetSrc, Unpacked> {
        let base_vertex_at = self.vertices.len();
        let range_idx = self.decode(&facet.mesh_h).ok_or(Unpacked::MissingGeometry)?;
        let (first_triangle_slot, index_count) = self.index_spans[range_idx as usize];
        let decoded = self.vertices.len() - base_vertex_at;
        if facet.compact_vertices.len() != decoded {
            self.vertices.truncate(base_vertex_at);
            self.index_list.truncate(first_triangle_slot as usize);
            self.index_spans.pop();
            return Err(Unpacked::RowCount {
                fitted: facet.compact_vertices.len(),
                unpacked: decoded,
            });
        }
        let compact_off = self.t6_rows.len() as u32;
        self.t6_rows.extend_from_slice(&facet.compact_vertices);
        self.packed_ranges.push((compact_off, decoded as u32));
        Ok(PropCachedFacetSrc {
            span_idx: range_idx,
            mat: facet.material,
            compact_off,
            compact_tally: decoded as u32,
            base_vertex_at: base_vertex_at as u32,
            first_triangle_slot,
            idx_count: index_count,
            figure_part_base_index: facet.figure_part_base_index,
            figure_part_vertex_offset: facet.figure_part_vertex_offset,
        })
    }
}

/// Per mesh, the authored material of every surface in every LOD.

/// Packs the meshes' geometry into a fresh plan. Also returns, per mesh,
/// the authored material of every surface in every LOD and the first-LOD
/// collision payloads.
pub fn pack_prop_meshes(
    meshes: &[LevelPropMesh],
) -> (
    PropGpuPlan,
    Vec<Vec<Option<assets::SurfaceMaterialId>>>,
    Vec<Vec<asset_world::XSurfaceCollisionPayload>>,
) {
    let mut stage = Staging::default();
    let mut plan = PropGpuPlan::default();
    let mut authored = Vec::with_capacity(meshes.len());
    let mut clip_hit = Vec::with_capacity(meshes.len());
    let mut passed_over = 0u32;
    let mut first_skip: Option<String> = None;

    for source in meshes {
        let mut row = PropMeshFacets {
            vertex_first: stage.t6_rows.len() as u32,
            detail_prop_cache: source.detail_prop_cache.map(|rows| rows[0]),
            detail_prop_cache_rows: source.detail_prop_cache,
            lod: source.lod,
            ..Default::default()
        };
        let mut designer_row = Vec::new();
        let mut collide_row = Vec::new();
        for (detail, surf_list) in source.detail_facets.iter().enumerate() {
            let lod_start = stage.t6_rows.len();
            for surface in surf_list {
                row.figure_part_plus_1_by_detail[detail].push(surface.figure_part_plus_1);
                let authored = surface.material;
                designer_row.push(authored);
                match stage.append(surface) {
                    Ok(src) => {
                        row.facets_by_detail[detail].push((src.span_idx, authored));
                        row.prop_cache_facets_by_detail[detail].push(src);
                        if detail == 0 {
                            collide_row.push(surface.collision.clone());
                        }
                    }
                    Err(why) => {
                        passed_over += 1;
                        first_skip.get_or_insert_with(|| why.describe(&source.name));
                    }
                }
            }
            row.vertex_tally_by_detail[detail] = (stage.t6_rows.len() - lod_start) as u32;
        }
        row.vertex_tally = stage.t6_rows.len() as u32 - row.vertex_first;
        plan.meshes.push(row);
        authored.push(designer_row);
        clip_hit.push(collide_row);
    }

    diag::info!(
        World,
        "smodel packed: verts={} surfaces={} skipped={} first_skip={}",
        stage.vertices.len(),
        stage.index_spans.len(),
        passed_over,
        first_skip.as_deref().unwrap_or("-"),
    );
    plan.compact_vertices = if stage.t6_rows.is_empty() {
        asset_world::PackedVertexPayload::Unavailable {
            origin_layout: "smodel plan has no vertices",
        }
    } else {
        asset_world::PackedVertexPayload::T6(stage.t6_rows)
    };
    plan.decoded_share = Some(Arc::new(stage.vertices));
    plan.index_portion = Some(Arc::new(stage.index_list));
    plan.range_portion = Some(Arc::new(stage.index_spans));
    plan.vert_range_share = Some(Arc::new(stage.packed_ranges));
    (plan, authored, clip_hit)
}

/// The pass material of a model surface that takes no model lighting.
pub fn unlit_pass_material(
    map_set: DesignerTextureSet,
    order_key: u8,
    sorted_index: Option<u32>,
) -> PropPassMaterial {
    let (specular, atlas) = (None, None);
    PropPassMaterial {
        model_illum_required: false,
        color: map_set.color,
        specular,
        probe: None,
        atlas,
        see_through: map_set.see_through,
        paint_mode: map_set.paint_mode,
        face_cull: map_set.cull_mode,
        env_map_params: map_set.env_map_params,
        illum_lookup_scale: [0.0; 4],
        atlas_find: [0.0; 4],
        order_key,
        material_draw_rank_slot: sorted_index,
        t6_material: None,
    }
}

/// What a lit pass material needs from the model lighting atlas.
struct LitAtlas {
    image: Handle<Image>,
    lookup_scale: [f32; 4],
    atlas_find: [f32; 4],
}

impl LitAtlas {
    /// The scene's model lighting atlas, when it has both its size and image.
    fn of(scene: &crate::prepare::scene::world::LevelStage) -> Option<Self> {
        let dims = scene.model_illum_dims?;
        let image = scene.model_illum_image.clone()?;
        let inv_h = lighting_t6::light_atlas_reciprocal_height(dims.rows_tall)
            .expect("a model lighting atlas always has rows");
        Some(Self {
            image,
            lookup_scale: lighting_t6::atlas_lookup_scale(inv_h).as_array(),
            atlas_find: [
                lighting_t6::ATLAS_INV_WIDTH as f32,
                inv_h,
                lighting_t6::MODEL_LIGHT_VOLUME_DEPTH_SPAN,
                0.0,
            ],
        })
    }
}

/// Builds the pass materials the placements draw with.
struct MaterialFactory<'a> {
    fx_catalog: &'a crate::assemble::draw_items::LiveMaterialLedger,
    exact_images: &'a [Option<Handle<Image>>],
    probes: &'a [Option<Handle<Image>>],
    sorted_ordinals: Vec<Option<u32>>,
    atlas: Option<LitAtlas>,
}

impl MaterialFactory<'_> {
    /// The material for an authored one, lit against `probe_index` or not;
    /// None for a shadow-only material, which never draws in colour.
    fn build(
        &self,
        source: Option<assets::SurfaceMaterialId>,
        probe_index: Option<u8>,
    ) -> Option<PropPassMaterial> {
        let derived = source.and_then(|id| self.fx_catalog.derived(id));
        if derived.is_some_and(|m| m.occluder_exclusive) {
            return None;
        }
        let textures = live_model_tables(source, self.fx_catalog, self.exact_images);
        let sort_key = derived.map_or(0, |m| m.draw_sort_key);
        let sorted = source.and_then(|id| self.sorted_ordinals.get(id.order()).copied().flatten());
        let Some(probe_index) = probe_index else {
            return Some(unlit_pass_material(textures, sort_key, sorted));
        };
        if textures.color.is_none() {
            diag::info!(
                World,
                "static model {:?}: material lacks a decoded colour map",
                derived.map_or("<none>", |m| m.name.as_str())
            );
        }
        let shared = self
            .atlas
            .as_ref()
            .expect("the shared atlas must exist for smodel lighting");
        let specular = textures.specular.clone();
        Some(PropPassMaterial {
            model_illum_required: true,
            specular,
            probe: self.probes.get(usize::from(probe_index)).cloned().flatten(),
            atlas: Some(shared.image.clone()),
            illum_lookup_scale: shared.lookup_scale,
            atlas_find: shared.atlas_find,
            ..unlit_pass_material(textures, sort_key, sorted)
        })
    }
}

/// The CPU copy the marks and collision queries read.
fn mark_cpu(
    gpu: &PropGpuPlan,
    mesh_collision: &[Vec<asset_world::XSurfaceCollisionPayload>],
) -> crate::prepare::scene::world::PropStainCpu {
    use crate::prepare::scene::world::{PropStainCpu, PropStainFacet};
    let ranges = gpu.facet_ranges();
    let decoded = gpu.decoded_vertices();
    PropStainCpu {
        pos_list: decoded.iter().map(|v| v.position).collect(),
        surface_directions: decoded.iter().map(|v| v.normal).collect(),
        tri_list: gpu.indices().to_vec(),
        meshes: gpu
            .meshes
            .iter()
            .zip(mesh_collision)
            .map(|(mesh, collisions)| {
                mesh.facets_by_detail[0]
                    .iter()
                    .zip(collisions)
                    .filter_map(|(&(span_idx, material), payload)| {
                        let &(first_triangle_slot, idx_count) = ranges.get(span_idx as usize)?;
                        Some(PropStainFacet {
                            first_triangle_slot,
                            index_total: idx_count,
                            material,
                            collision: payload.clone(),
                        })
                    })
                    .collect()
            })
            .collect(),
    }
}

pub(crate) fn build_prop_gpu_plan(
    mut scene: Option<ResMut<crate::prepare::scene::world::LevelStage>>,
    job: Res<crate::prepare::scene::spawn::LevelSpawnJob>,
    existing: Option<Res<PropGpuPlan>>,
    mut commands: Commands,
) {
    use crate::prepare::scene::prop_light_slots;
    use crate::prepare::scene::spawn::LevelSpawnPhase;
    if !matches!(job.stage_now, LevelSpawnPhase::Gpu | LevelSpawnPhase::Done) || existing.is_some() {
        return;
    }
    let Some(scene) = scene.as_deref_mut() else {
        return;
    };
    if scene.prop_meshes.is_empty() {
        return;
    }
    let slot_total = scene.prop_instances.len();
    let slot_entities = match scene.culling.as_ref() {
        Some(cull) if cull.prop_entities.len() == slot_total => {
            cull.prop_entities.clone()
        }
        _ => return,
    };

    // The loaded meshes are only needed for packing; they go when it is done.
    let (mut draw_plan, mesh_from_author, mesh_collision) =
        pack_prop_meshes(&std::mem::take(&mut scene.prop_meshes));

    let catalog = &scene.live_material_ledger;
    draw_plan.mat_state_flags = draw_plan
        .meshes
        .iter()
        .flat_map(|mesh| mesh.prop_cache_facets_by_detail.iter().flatten())
        .filter_map(|facet| facet.mat)
        .filter_map(|id| Some((id, catalog.derived(id)?.condition_bits)))
        .collect();

    let lighting = scene.model_illum_dims.and_then(|dims| {
        prop_light_slots::prepare_prop_illum(dims, &scene.prop_illum_samples, slot_total)
    });
    let atlas = LitAtlas::of(scene);
    let (exact_images, _, probes) = job.stitch_image_handles();
    let factory = MaterialFactory {
        fx_catalog: catalog,
        exact_images,
        probes,
        sorted_ordinals: (0..catalog.materials.len())
            .map(|id| {
                catalog
                    .ranked_materials
                    .rank_for_asset_id(id)
                    .map(crate::assemble::draw_items::SortedMaterialRank::get)
            })
            .collect(),
        atlas,
    };

    draw_plan.designer_placement_indices.resize(slot_total, None);
    for (index, placed) in scene.prop_instances.iter().enumerate() {
        let Some(placed) = *placed else {
            continue;
        };
        let Some(owner) = slot_entities[index] else {
            continue;
        };
        let Some(designer_row) = mesh_from_author.get(placed.mesh).filter(|row| !row.is_empty())
        else {
            continue;
        };
        let wants_atlas =
            prop_light_slots::facets_take_model_illum(designer_row.iter().copied(), catalog);
        let lit = lighting
            .as_ref()
            .is_some_and(|l| prop_light_slots::seat_is_lit_candidate(l, index, wants_atlas));
        let probe = placed.env_probe_index;
        let lit_probe = lit.then_some(probe);
        for &authored in designer_row {
            draw_plan.intern_material(&factory, authored, lit_probe);
        }

        draw_plan.designer_placement_indices[index] = Some(draw_plan.placements.len());
        draw_plan.placements.push(PropPlacement {
            model_slot: placed.mesh,
            ent: owner,
            illum_slot: Some(index),
            lit_list: lit,
            mode_bits: placed.flags,
            env_probe_index: probe,
            key_light_index: placed.key_light_index,
            compact_illum: placed.compact_illum,
            local_to_world: placed.transform.to_matrix(),
            at_pos: placed.origin,
            axes_set: placed.axis,
            size_k: placed.scale,
            t6_primary: placed.t6_primary,
            t6_light: placed.t6_light,
            t6_cull_dist: f32::from(placed.hide_dist),
        });
    }

    let culling = scene.culling.as_ref();
    let hide_dists = culling.map_or(&[][..], |cull| cull.prop_hide_dists.as_slice());
    let mut shadow_casters = Vec::new();
    super::super::kept_lanes::fill_prop_draw_inst_shadow(&mut shadow_casters, hide_dists, &draw_plan);
    draw_plan.shadow_draw_instances = shadow_casters;

    scene.smodel_mark_cpu = Some(mark_cpu(&draw_plan, &mesh_collision));
    diag::info!(
        World,
        "static models: authored slots={} kept verts={} indices={} mats={}",
        slot_total,
        draw_plan.decoded_vertices().len(),
        draw_plan.indices().len(),
        draw_plan.materials.len(),
    );
    draw_plan.publish_extract_shares();
    commands.insert_resource(draw_plan);
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::asset::RenderAssetUsages;
    use bevy::mesh::PrimitiveTopology;

    fn triangle(indices: Vec<u32>) -> Mesh {
        Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::default())
            .with_inserted_attribute(
                Mesh::ATTRIBUTE_POSITION,
                vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
            )
            .with_inserted_indices(bevy::mesh::Indices::U32(indices))
    }

    fn surface(mesh: Mesh, packed_rows: usize) -> LevelPropFacet {
        let material = None;
        LevelPropFacet {
            mesh_h: mesh,
            material,
            compact_vertices: vec![[0; asset_t6::T6_PACKED_VERTEX]; packed_rows],
            figure_part_plus_1: None,
            figure_part_base_index: 0,
            figure_part_vertex_offset: 0,
            collision: asset_world::XSurfaceCollisionPayload::Unavailable { origin_layout: "test" },
        }
    }

    fn model(name: &str, lod0: Vec<LevelPropFacet>) -> LevelPropMesh {
        LevelPropMesh {
            name: name.into(),
            detail_facets: [lod0, vec![], vec![], vec![]],
            detail_prop_cache: None,
            lod: None,
        }
    }

    #[test]
    fn surfaces_pack_back_to_back_with_their_ranges() {
        let meshes = [
            model("a", vec![surface(triangle(vec![0, 1, 2]), 3)]),
            model("b", vec![surface(triangle(vec![0, 1, 2]), 3)]),
        ];
        let (plan, authored, collision) = pack_prop_meshes(&meshes);
        assert_eq!(plan.decoded_vertices().len(), 6);
        assert_eq!(plan.indices(), &[0, 1, 2, 3, 4, 5]);
        assert_eq!(plan.facet_ranges(), &[(0, 3), (3, 3)]);
        assert_eq!(plan.vert_range_share.as_deref().unwrap(), &[(0, 3), (3, 3)]);
        assert_eq!(plan.meshes[1].vertex_first, 3);
        assert_eq!(plan.meshes[1].vertex_tally_by_detail[0], 3);
        assert_eq!(plan.meshes[1].prop_cache_facets_by_detail[0][0].base_vertex_at, 3);
        assert_eq!(authored.len(), 2);
        assert_eq!(collision[0].len(), 1);
        assert_eq!(plan.compact_row_tally(), Some(6));
    }

    #[test]
    fn a_surface_whose_rows_disagree_is_dropped_whole() {
        let meshes = [model(
            "odd",
            vec![
                surface(triangle(vec![0, 1, 2]), 2),
                surface(triangle(vec![0, 1, 2]), 3),
                surface(triangle(vec![0, 1, 7]), 3),
            ],
        )];
        let (plan, authored, collision) = pack_prop_meshes(&meshes);
        // Only the middle surface packs; the others leave no rows behind.
        assert_eq!(plan.decoded_vertices().len(), 3);
        assert_eq!(plan.indices(), &[0, 1, 2]);
        assert_eq!(plan.facet_ranges(), &[(0, 3)]);
        assert_eq!(plan.meshes[0].facets_by_detail[0].len(), 1);
        // The authored row still lists all three, the collision row one.
        assert_eq!(authored[0].len(), 3);
        assert_eq!(collision[0].len(), 1);
    }

    #[test]
    fn nothing_packed_is_unavailable_and_publishing_keeps_it_so() {
        let (mut draw_plan, _, _) = pack_prop_meshes(&[model("empty", vec![])]);
        assert!(!draw_plan.compact_ok());
        draw_plan.publish_extract_shares();
        assert!(draw_plan.compact_portion.is_none());
        assert_eq!(draw_plan.compact_row_tally(), None);
    }

    #[test]
    fn publishing_moves_the_payload_behind_the_share() {
        let (mut draw_plan, _, _) =
            pack_prop_meshes(&[model("a", vec![surface(triangle(vec![0, 1, 2]), 3)])]);
        assert!(draw_plan.compact_ok());
        draw_plan.publish_extract_shares();
        assert_eq!(draw_plan.compact_portion.as_ref().map(|rows| rows.len()), Some(3));
        assert!(draw_plan.compact_ok());
        assert_eq!(draw_plan.compact_row_tally(), Some(3));
    }
}
