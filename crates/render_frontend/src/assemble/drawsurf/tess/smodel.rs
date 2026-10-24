//! Static-model geometry: every placed prop's surfaces decoded once into
//! shared vertex, index and range rows, the pass materials they draw with,
//! and the placements that point at them.
//!
//! `pack_smodel_meshes` turns the loaded meshes into a [`SmodelGpuPlan`]'s
//! geometry; `build_smodel_gpu_plan` adds the materials and placements once
//! the world is spawned. The sky model reuses the packing on its own.

use std::collections::HashMap;
use std::sync::Arc;

use bevy::mesh::VertexAttributeValues;
use bevy::prelude::*;

use crate::prepare::scene::world::{WorldStaticModelMesh, WorldStaticModelSurface};

pub use render_frame::{PackedVertexRefusal, SmodelVertex};
pub use render_scene::{
    AuthoredMaps, LodRampArgs, SmodelPassMaterial, runtime_maps, smodel_camera_lod,
};

type PackedRow = [u8; asset_t6::T6_PACKED_VERTEX];

/// One mesh's surfaces per level of detail, with where each landed in the
/// plan's shared rows.
#[derive(Clone, Debug, Default)]
pub struct SmodelMeshSurfaces {
    /// `(range index, authored material)` of every surface that packed.
    pub surfaces_by_lod: [Vec<(u32, Option<assets::MaterialIndex>)>; 4],
    pub smc_surfs_by_lod: [Vec<SmodelCachedSurfSrc>; 4],
    /// The `xsurface + 1` byte of every authored surface, packed or not.
    pub xsurface_plus_1_by_lod: [Vec<Option<u8>>; 4],

    pub vert_start: u32,
    pub vert_count: u32,
    pub vert_count_by_lod: [u32; 4],

    pub lod_smc: Option<[u8; 4]>,
    pub lod_smc_rows: Option<[[u8; 4]; 4]>,
    pub lod: Option<asset_model::ModelLodSelector>,
}

/// Where one packed surface sits: its range, its packed vertex rows, its
/// decoded vertices and indices, and the source surface it came from.
#[derive(Clone, Debug)]
pub struct SmodelCachedSurfSrc {
    pub range_idx: u32,
    pub material: Option<assets::MaterialIndex>,
    pub packed_off: u32,
    pub packed_n: u32,
    pub vert_base: u32,
    pub index_start: u32,
    pub index_count: u32,
    pub xsurface_base_index: u16,
    pub xsurface_vert_offset: u16,
}

/// One placed static model.
#[derive(Clone, Debug)]
pub struct SmodelPlacement {
    pub mesh: usize,
    pub entity: Entity,
    /// The slot in the world's authored placement list.
    pub lighting_slot: Option<usize>,
    pub lit: bool,
    pub flags: u8,
    pub reflection_probe_index: u8,
    pub primary_light_index: u8,
    pub packed_lighting: Option<[u8; 4]>,

    pub world_from_local: Mat4,
    pub origin: [f32; 3],
    pub axis: [[f32; 3]; 3],
    pub scale: f32,

    /// bo2zm: a Black Ops II placement's primary light and its visibility
    /// (see `asset_world::StaticModelPlacement::t6_primary`).
    pub t6_primary: [u8; 2],
    /// bo2mp: see `asset_world::StaticModelPlacement::t6_light`.
    pub t6_light: u32,
    /// bo2zm: beyond this distance the game does not draw the placement
    /// (`GfxStaticModelDrawInst::cullDist`; 0 = always drawn).
    pub t6_cull_dist: f32,
}

#[derive(Resource, Clone, Debug, Default)]
pub struct SmodelGpuPlan {
    pub packed_vertices: asset_world::PackedVertexPayload,
    pub materials: Vec<SmodelPassMaterial>,
    pub meshes: Vec<SmodelMeshSurfaces>,
    pub placements: Vec<SmodelPlacement>,
    /// For each authored slot, the placement built for it.
    pub authored_placement_indices: Vec<Option<usize>>,
    pub material_state_flags: HashMap<assets::MaterialIndex, u8>,
    pub shadow_draw_insts: Vec<dpvs_t6::SmodelShadowInst>,

    /// Pass material index by (authored material, reflection probe).
    pub lit_material_key: HashMap<(Option<assets::MaterialIndex>, u8), u32>,
    pub unlit_material_key: HashMap<Option<assets::MaterialIndex>, u32>,

    pub packed_share: Option<Arc<Vec<PackedRow>>>,
    pub index_share: Option<Arc<Vec<u32>>>,
    pub range_share: Option<Arc<Vec<(u32, u32)>>>,
    /// Per range, `(first packed row, row count)`.
    pub vert_range_share: Option<Arc<Vec<(u32, u32)>>>,
    pub decoded_share: Option<Arc<Vec<SmodelVertex>>>,
    /// Baked per-placement vertices. No lane fills it any more; the extract
    /// still reads it, so it stays until that goes.
    pub cached_share: Option<Arc<Vec<PackedRow>>>,
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
pub type T6PropInstance = [f32; T6_PROP_ROW];

// Constant hashes of the tree canopy shaders' parameters.
const CANOPY_LIGHTING_PARMS: u32 = 0x3e1b_784f;
const CANOPY_SCATTER_COLOR: u32 = 0xf5e3_2317;
const CANOPY_COLOR_TINT: u32 = 0xb60c_3b3a;
const CANOPY_SWAY_PARMS: u32 = 0x8147_ad2d;

fn material_constant(material: &render_material::RuntimeMaterial, hash: u32) -> Option<[f32; 4]> {
    material
        .constants
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
fn t6_canopy_params(material: &render_material::RuntimeMaterial) -> [u32; 2] {
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
fn t6_canopy_sway(material: &render_material::RuntimeMaterial) -> [f32; 3] {
    if !material.t6_draw.is_some_and(|d| d.sway) {
        return [0.0; 3];
    }
    material_constant(material, CANOPY_SWAY_PARMS).map_or([0.0; 3], |c| [c[0], c[1], c[2]])
}

/// bo2mp: a lit tree canopy material's constants for its own instance row
/// (`t6_canopy_params`, `t6_canopy_sway`); None for any other material.
/// The placed props and the script models (BO2's animated vines) both
/// draw canopies from such a row.
pub fn t6_canopy(material: &render_material::RuntimeMaterial) -> Option<([u32; 2], [f32; 3])> {
    let lit_foliage = material.t6_draw.is_some_and(|d| d.foliage && !d.unlit);
    lit_foliage.then(|| (t6_canopy_params(material), t6_canopy_sway(material)))
}

/// A placement's fallback-pass row; `light` is the placement's packed
/// lighting and `exposure` scales the decoded colour.
fn t6_prop_row(placement: &SmodelPlacement, light: [u8; 4], exposure: f32) -> T6PropInstance {
    let mut row = [0.0f32; T6_PROP_ROW];
    row[..16].copy_from_slice(&placement.world_from_local.to_cols_array());
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
    row[ROW_PROBE] = f32::from(placement.reflection_probe_index) + 1.0;
    row[ROW_OWN_LIGHT] = placement.t6_light as f32;
    row
}

impl SmodelGpuPlan {
    /// bo2zm: the Black Ops II props the fallback pass draws (their
    /// materials have no technique, so no draw item is ever emitted for
    /// them): for each placement with a packed light, its first-LOD
    /// surfaces as `[surface, colour slot, draw code, instance]`, and each
    /// instance's transform and light. The third list is, per draw, its
    /// normal and specular map slots (shine) and, bo2mp, its layer 1 slot
    /// (an emissive flow's remap).
    pub fn t6_fallback_props(
        &self,
        catalog: &crate::assemble::drawsurf::RuntimeMaterialCatalog,
        exposure: f32,
    ) -> (Vec<[u32; 4]>, Vec<T6PropInstance>, Vec<[u32; 3]>) {
        const SHINE_SEMANTICS: [u8; 3] = [
            asset_material::TS_NORMAL_MAP,
            asset_material::TS_SPECULAR_MAP,
            0xf1,
        ];
        let slot_of = |runtime: Option<&render_material::RuntimeMaterial>, semantic: u8| {
            runtime
                .and_then(|m| m.texture_semantic(semantic))
                .map_or(u32::MAX, |id| id.0)
        };
        let mut draws = Vec::new();
        let mut rows: Vec<T6PropInstance> = Vec::new();
        let mut shine = Vec::new();
        for placement in &self.placements {
            let (Some(light), Some(mesh)) =
                (placement.packed_lighting, self.meshes.get(placement.mesh))
            else {
                continue;
            };
            let plain_row = t6_prop_row(placement, light, exposure);
            let plain_at = rows.len() as u32;
            rows.push(plain_row);
            // bo2mp: a tree canopy gets its own copy of the row per set of
            // canopy constants (see `t6_canopy_params`).
            let mut canopy_rows: HashMap<([u32; 2], [u32; 3]), u32> = HashMap::new();
            for &(surface, material) in &mesh.surfaces_by_lod[0] {
                let runtime = material.and_then(|id| catalog.derived(id));
                let canopy = runtime.and_then(t6_canopy);
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
                let code = runtime
                    .and_then(|m| m.t6_draw)
                    .map_or(asset_core::T6Draw::NONE_CODE, asset_core::T6Draw::code);
                draws.push([
                    surface,
                    slot_of(runtime, asset_material::TS_COLOR_MAP),
                    code,
                    at,
                ]);
                // The canopy shaders read the colour map alone: no shine.
                shine.push(if canopy.is_some() {
                    [u32::MAX; 3]
                } else {
                    SHINE_SEMANTICS.map(|semantic| slot_of(runtime, semantic))
                });
            }
        }
        (draws, rows, shine)
    }

    /// The packed rows the extract may take: the published share, else the
    /// payload still held here.
    fn packed_slice(&self) -> Option<&[PackedRow]> {
        match (&self.packed_share, &self.packed_vertices) {
            (Some(share), _) => Some(share.as_slice()),
            (None, asset_world::PackedVertexPayload::T6(rows)) => Some(rows.as_slice()),
            (None, asset_world::PackedVertexPayload::Unavailable { .. }) => None,
        }
    }

    pub fn exact_packed_vertices(&self) -> Result<&[PackedRow], PackedVertexRefusal> {
        let table_stride = render_material::decl_table::stream_extent(
            render_material::decl_table::PACKED_VERTEX_TYPE,
            0,
        );
        if table_stride != Some(asset_t6::T6_PACKED_VERTEX as u16) {
            return Err(PackedVertexRefusal::StrideMismatch { table_stride });
        }
        let packed = match self.packed_slice() {
            Some(rows) => rows,
            None => {
                let asset_world::PackedVertexPayload::Unavailable { source_layout } =
                    &self.packed_vertices
                else {
                    unreachable!("a T6 payload always has rows");
                };
                return Err(PackedVertexRefusal::ForeignLayout {
                    source_layout: *source_layout,
                });
            }
        };
        let decoded = self.decoded_vertices().len();
        if packed.len() != decoded {
            return Err(PackedVertexRefusal::VertexCountMismatch {
                packed: packed.len(),
                decoded,
            });
        }
        Ok(packed)
    }

    pub fn indices(&self) -> &[u32] {
        super::published_rows(&self.index_share)
    }

    pub fn surface_ranges(&self) -> &[(u32, u32)] {
        super::published_rows(&self.range_share)
    }

    pub fn decoded_vertices(&self) -> &[SmodelVertex] {
        super::published_rows(&self.decoded_share)
    }

    /// Moves the packed payload behind an `Arc` so the extract can share it.
    pub fn publish_extract_shares(&mut self) {
        if self.packed_share.is_some() {
            return;
        }
        let payload = std::mem::replace(
            &mut self.packed_vertices,
            asset_world::PackedVertexPayload::Unavailable {
                source_layout: "packed payload published for extract",
            },
        );
        match payload {
            asset_world::PackedVertexPayload::T6(rows) => self.packed_share = Some(Arc::new(rows)),
            unavailable => self.packed_vertices = unavailable,
        }
    }

    pub fn packed_ok(&self) -> bool {
        self.packed_slice().is_some()
    }

    pub fn packed_row_count(&self) -> Option<usize> {
        self.packed_slice().map(<[_]>::len)
    }
}

/// A float column of a mesh attribute, when the mesh has it in that format.
macro_rules! float_column {
    ($name:ident, $variant:ident, $width:literal) => {
        fn $name(values: &VertexAttributeValues) -> Option<&[[f32; $width]]> {
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

/// Decodes one mesh onto the end of the shared rows. Returns the surface's
/// range index, or None (nothing appended) for a mesh with no positions, no
/// indices or an index past its vertices.
fn decode_surface(
    mesh: &Mesh,
    vertices: &mut Vec<SmodelVertex>,
    indices: &mut Vec<u32>,
    ranges: &mut Vec<(u32, u32)>,
) -> Option<u32> {
    let positions = mesh.attribute(Mesh::ATTRIBUTE_POSITION).and_then(f32x3)?;
    let normals = mesh.attribute(Mesh::ATTRIBUTE_NORMAL).and_then(f32x3);
    let uvs = mesh.attribute(Mesh::ATTRIBUTE_UV_0).and_then(f32x2);
    let colors = mesh.attribute(Mesh::ATTRIBUTE_COLOR).and_then(f32x4);
    let source = mesh.indices()?;
    if positions.is_empty() || source.is_empty() || source.iter().any(|i| i >= positions.len()) {
        return None;
    }
    let base = vertices.len() as u32;
    vertices.extend(positions.iter().enumerate().map(|(i, &position)| SmodelVertex {
        position,
        normal: normals.and_then(|n| n.get(i)).copied().unwrap_or([0.0, 0.0, 1.0]),
        color: colors.and_then(|c| c.get(i)).copied().unwrap_or([1.0; 4]),
        uv0: uvs.and_then(|t| t.get(i)).copied().unwrap_or([0.0; 2]),
    }));
    let index_start = indices.len() as u32;
    indices.extend(source.iter().map(|i| base + i as u32));
    ranges.push((index_start, source.len() as u32));
    Some(ranges.len() as u32 - 1)
}

/// Why a surface did not pack.
enum Unpacked {
    NoGeometry,
    RowCount { packed: usize, decoded: usize },
}

impl Unpacked {
    fn describe(&self, model: &str) -> String {
        match self {
            Self::NoGeometry => format!("empty_ib:{model}"),
            Self::RowCount { packed, decoded } => {
                format!("{model} packed={packed} decoded={decoded}")
            }
        }
    }
}

/// The shared rows while surfaces are appended to them.
#[derive(Default)]
struct Staging {
    vertices: Vec<SmodelVertex>,
    indices: Vec<u32>,
    ranges: Vec<(u32, u32)>,
    packed: Vec<PackedRow>,
    packed_ranges: Vec<(u32, u32)>,
}

impl Staging {
    /// Appends a surface's decoded and packed rows together; they must have
    /// the same count or the surface is left out entirely.
    fn append(&mut self, surface: &WorldStaticModelSurface) -> Result<SmodelCachedSurfSrc, Unpacked> {
        let vert_base = self.vertices.len();
        let range_idx = decode_surface(
            &surface.mesh,
            &mut self.vertices,
            &mut self.indices,
            &mut self.ranges,
        )
        .ok_or(Unpacked::NoGeometry)?;
        let (index_start, index_count) = self.ranges[range_idx as usize];
        let decoded = self.vertices.len() - vert_base;
        if surface.packed_vertices.len() != decoded {
            self.vertices.truncate(vert_base);
            self.indices.truncate(index_start as usize);
            self.ranges.pop();
            return Err(Unpacked::RowCount {
                packed: surface.packed_vertices.len(),
                decoded,
            });
        }
        let packed_off = self.packed.len() as u32;
        self.packed.extend_from_slice(&surface.packed_vertices);
        self.packed_ranges.push((packed_off, decoded as u32));
        Ok(SmodelCachedSurfSrc {
            range_idx,
            material: surface.material,
            packed_off,
            packed_n: decoded as u32,
            vert_base: vert_base as u32,
            index_start,
            index_count,
            xsurface_base_index: surface.xsurface_base_index,
            xsurface_vert_offset: surface.xsurface_vert_offset,
        })
    }
}

/// Packs the meshes' geometry into a fresh plan. Also returns, per mesh,
/// the authored material of every surface in every LOD and the first-LOD
/// collision payloads.
pub fn pack_smodel_meshes(
    meshes: &[WorldStaticModelMesh],
) -> (
    SmodelGpuPlan,
    Vec<Vec<Option<assets::MaterialIndex>>>,
    Vec<Vec<asset_world::XSurfaceCollisionPayload>>,
) {
    let mut stage = Staging::default();
    let mut plan = SmodelGpuPlan::default();
    let mut authored = Vec::with_capacity(meshes.len());
    let mut collision = Vec::with_capacity(meshes.len());
    let mut skipped = 0u32;
    let mut first_skip: Option<String> = None;

    for model in meshes {
        let mut row = SmodelMeshSurfaces {
            vert_start: stage.packed.len() as u32,
            lod_smc: model.lod_smc.map(|rows| rows[0]),
            lod_smc_rows: model.lod_smc,
            lod: model.lod,
            ..Default::default()
        };
        let mut authored_row = Vec::new();
        let mut collision_row = Vec::new();
        for (lod, surfaces) in model.lod_surfaces.iter().enumerate() {
            let lod_start = stage.packed.len();
            for surface in surfaces {
                row.xsurface_plus_1_by_lod[lod].push(surface.xsurface_plus_1);
                authored_row.push(surface.material);
                match stage.append(surface) {
                    Ok(src) => {
                        row.surfaces_by_lod[lod].push((src.range_idx, surface.material));
                        row.smc_surfs_by_lod[lod].push(src);
                        if lod == 0 {
                            collision_row.push(surface.collision.clone());
                        }
                    }
                    Err(why) => {
                        skipped += 1;
                        first_skip.get_or_insert_with(|| why.describe(&model.name));
                    }
                }
            }
            row.vert_count_by_lod[lod] = (stage.packed.len() - lod_start) as u32;
        }
        row.vert_count = stage.packed.len() as u32 - row.vert_start;
        plan.meshes.push(row);
        authored.push(authored_row);
        collision.push(collision_row);
    }

    diag::info!(
        World,
        "smodel packed: verts={} surfaces={} skipped={} first_skip={}",
        stage.vertices.len(),
        stage.ranges.len(),
        skipped,
        first_skip.as_deref().unwrap_or("-"),
    );
    plan.packed_vertices = if stage.packed.is_empty() {
        asset_world::PackedVertexPayload::Unavailable {
            source_layout: "smodel plan has no vertices",
        }
    } else {
        asset_world::PackedVertexPayload::T6(stage.packed)
    };
    plan.decoded_share = Some(Arc::new(stage.vertices));
    plan.index_share = Some(Arc::new(stage.indices));
    plan.range_share = Some(Arc::new(stage.ranges));
    plan.vert_range_share = Some(Arc::new(stage.packed_ranges));
    (plan, authored, collision)
}

/// The pass material of a model surface that takes no model lighting.
pub fn unlit_pass_material(
    maps: AuthoredMaps,
    sort_key: u8,
    sorted_index: Option<u32>,
) -> SmodelPassMaterial {
    SmodelPassMaterial {
        model_lighting_required: false,
        color: maps.color,
        specular: None,
        probe: None,
        atlas: None,
        alpha_mode: maps.alpha_mode,
        draw_mode: maps.draw_mode,
        cull_mode: maps.cull_mode,
        env_map_parms: maps.env_map_parms,
        lighting_lookup_scale: [0.0; 4],
        atlas_lookup: [0.0; 4],
        sort_key,
        material_sorted_index: sorted_index,
        t6_material: None,
    }
}

/// What a lit pass material needs from the model lighting atlas.
struct LitAtlas {
    image: Handle<Image>,
    lookup_scale: [f32; 4],
    atlas_lookup: [f32; 4],
}

/// Builds the pass materials the placements draw with.
struct MaterialFactory<'a> {
    catalog: &'a crate::assemble::drawsurf::RuntimeMaterialCatalog,
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
        authored: Option<assets::MaterialIndex>,
        probe_index: Option<u8>,
    ) -> Option<SmodelPassMaterial> {
        let derived = authored.and_then(|id| self.catalog.derived(id));
        if derived.is_some_and(|m| m.shadow_only) {
            return None;
        }
        let maps = runtime_maps(authored, self.catalog, self.exact_images);
        let sort_key = derived.map_or(0, |m| m.sort_key);
        let sorted = authored.and_then(|id| self.sorted_ordinals.get(id.order()).copied().flatten());
        let Some(probe_index) = probe_index else {
            return Some(unlit_pass_material(maps, sort_key, sorted));
        };
        if maps.color.is_none() {
            diag::info!(
                World,
                "static model material gap: {:?} has no decoded color map",
                derived.map_or("<none>", |m| m.name.as_str())
            );
        }
        let atlas = self
            .atlas
            .as_ref()
            .expect("smodel lighting requires the shared atlas");
        let specular = maps.specular.clone();
        Some(SmodelPassMaterial {
            model_lighting_required: true,
            specular,
            probe: self.probes.get(usize::from(probe_index)).cloned().flatten(),
            atlas: Some(atlas.image.clone()),
            lighting_lookup_scale: atlas.lookup_scale,
            atlas_lookup: atlas.atlas_lookup,
            ..unlit_pass_material(maps, sort_key, sorted)
        })
    }
}

/// The CPU copy the marks and collision queries read.
fn mark_cpu(
    plan: &SmodelGpuPlan,
    mesh_collision: &[Vec<asset_world::XSurfaceCollisionPayload>],
) -> crate::prepare::scene::world::SmodelMarkCpu {
    use crate::prepare::scene::world::{SmodelMarkCpu, SmodelMarkSurface};
    let ranges = plan.surface_ranges();
    let decoded = plan.decoded_vertices();
    SmodelMarkCpu {
        positions: decoded.iter().map(|v| v.position).collect(),
        normals: decoded.iter().map(|v| v.normal).collect(),
        indices: plan.indices().to_vec(),
        meshes: plan
            .meshes
            .iter()
            .zip(mesh_collision)
            .map(|(mesh, collisions)| {
                mesh.surfaces_by_lod[0]
                    .iter()
                    .zip(collisions)
                    .filter_map(|(&(range_idx, material), collision)| {
                        let &(index_start, index_count) = ranges.get(range_idx as usize)?;
                        Some(SmodelMarkSurface {
                            index_start,
                            index_count,
                            material,
                            collision: collision.clone(),
                        })
                    })
                    .collect()
            })
            .collect(),
    }
}

pub(crate) fn build_smodel_gpu_plan(
    mut scene: Option<ResMut<crate::prepare::scene::world::WorldScene>>,
    job: Res<crate::prepare::scene::spawn::WorldSpawnJob>,
    existing: Option<Res<SmodelGpuPlan>>,
    mut commands: Commands,
) {
    use crate::prepare::scene::{smodel_lighting, spawn::WorldSpawnPhase};
    if !matches!(job.phase, WorldSpawnPhase::Gpu | WorldSpawnPhase::Done) || existing.is_some() {
        return;
    }
    let Some(scene) = scene.as_mut() else {
        return;
    };
    let scene: &mut crate::prepare::scene::world::WorldScene = &mut **scene;
    if scene.static_model_meshes.is_empty() {
        return;
    }
    let slot_count = scene.static_model_instances.len();
    let slot_entities = match scene.cull.as_ref() {
        Some(cull) if cull.static_model_entities.len() == slot_count => {
            cull.static_model_entities.clone()
        }
        _ => return,
    };

    let meshes = std::mem::take(&mut scene.static_model_meshes);
    let (mut plan, mesh_authored, mesh_collision) = pack_smodel_meshes(&meshes);
    drop(meshes);

    let catalog = &scene.runtime_material_catalog;
    plan.material_state_flags = plan
        .meshes
        .iter()
        .flat_map(|mesh| mesh.smc_surfs_by_lod.iter().flatten())
        .filter_map(|surface| surface.material)
        .filter_map(|id| Some((id, catalog.derived(id)?.state_flags)))
        .collect();

    let lighting = scene.model_lighting_dims.and_then(|dims| {
        smodel_lighting::prepare_smodel_lighting(dims, &scene.smodel_lighting_samples, slot_count)
    });
    let atlas = scene
        .model_lighting_dims
        .zip(scene.model_lighting_image.clone())
        .map(|(dims, image)| {
            let inv_h = lighting_t6::model_lighting_inv_image_height(dims.image_height)
                .expect("mcv=1 atlas height is non-zero");
            let scale = lighting_t6::model_lighting_lookup_scale(inv_h);
            LitAtlas {
                image,
                lookup_scale: scale.as_array(),
                atlas_lookup: [
                    lighting_t6::MODEL_LIGHTING_INV_ATLAS_WIDTH as f32,
                    inv_h,
                    lighting_t6::MODEL_LIGHTING_VOLUME_W,
                    0.0,
                ],
            }
        });
    let (exact_images, _, probes) = job.tess_image_handles();
    let factory = MaterialFactory {
        catalog,
        exact_images,
        probes,
        sorted_ordinals: (0..catalog.materials.len())
            .map(|id| {
                catalog
                    .sorted_materials
                    .ordinal_for_asset_id(id)
                    .map(crate::assemble::drawsurf::SortedMaterialOrdinal::get)
            })
            .collect(),
        atlas,
    };

    plan.authored_placement_indices.resize(slot_count, None);
    for (index, instance) in scene.static_model_instances.iter().enumerate() {
        let Some(instance) = *instance else {
            continue;
        };
        let Some(parent) = slot_entities[index] else {
            continue;
        };
        let Some(authored_row) = mesh_authored.get(instance.mesh).filter(|row| !row.is_empty())
        else {
            continue;
        };
        let wants_atlas =
            smodel_lighting::surfaces_take_model_lighting(authored_row.iter().copied(), catalog);
        let lit = lighting
            .as_ref()
            .is_some_and(|l| smodel_lighting::slot_is_lit_candidate(l, index, wants_atlas));
        let probe_index = instance.reflection_probe_index;
        for &authored in authored_row {
            let known = if lit {
                plan.lit_material_key.contains_key(&(authored, probe_index))
            } else {
                plan.unlit_material_key.contains_key(&authored)
            };
            if known {
                continue;
            }
            let Some(material) = factory.build(authored, lit.then_some(probe_index)) else {
                continue;
            };
            let at = plan.materials.len() as u32;
            plan.materials.push(material);
            if lit {
                plan.lit_material_key.insert((authored, probe_index), at);
            } else {
                plan.unlit_material_key.insert(authored, at);
            }
        }

        plan.authored_placement_indices[index] = Some(plan.placements.len());
        plan.placements.push(SmodelPlacement {
            mesh: instance.mesh,
            entity: parent,
            lighting_slot: Some(index),
            lit,
            flags: instance.flags,
            reflection_probe_index: probe_index,
            primary_light_index: instance.primary_light_index,
            packed_lighting: instance.packed_lighting,
            world_from_local: instance.transform.to_matrix(),
            origin: instance.origin,
            axis: instance.axis,
            scale: instance.scale,
            t6_primary: instance.t6_primary,
            t6_light: instance.t6_light,
            t6_cull_dist: f32::from(instance.cull_dist),
        });
    }

    let cull_dists = scene
        .cull
        .as_ref()
        .map_or(&[][..], |cull| cull.static_model_cull_dists.as_slice());
    let mut shadow_draw_insts = Vec::new();
    super::super::retained::fill_smodel_draw_inst_shadow(
        &mut shadow_draw_insts,
        cull_dists,
        &plan,
    );
    plan.shadow_draw_insts = shadow_draw_insts;

    scene.smodel_mark_cpu = Some(mark_cpu(&plan, &mesh_collision));
    diag::info!(
        World,
        "static models: {} authored slots; retained verts={} indices={} mats={}",
        slot_count,
        plan.decoded_vertices().len(),
        plan.indices().len(),
        plan.materials.len(),
    );
    plan.publish_extract_shares();
    commands.insert_resource(plan);
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

    fn surface(mesh: Mesh, packed_rows: usize) -> WorldStaticModelSurface {
        WorldStaticModelSurface {
            mesh,
            material: None,
            packed_vertices: vec![[0; asset_t6::T6_PACKED_VERTEX]; packed_rows],
            xsurface_plus_1: None,
            xsurface_base_index: 0,
            xsurface_vert_offset: 0,
            collision: asset_world::XSurfaceCollisionPayload::Unavailable { source_layout: "test" },
        }
    }

    fn model(name: &str, lod0: Vec<WorldStaticModelSurface>) -> WorldStaticModelMesh {
        WorldStaticModelMesh {
            name: name.into(),
            lod_surfaces: [lod0, vec![], vec![], vec![]],
            lod_smc: None,
            lod: None,
        }
    }

    #[test]
    fn surfaces_pack_back_to_back_with_their_ranges() {
        let meshes = [
            model("a", vec![surface(triangle(vec![0, 1, 2]), 3)]),
            model("b", vec![surface(triangle(vec![0, 1, 2]), 3)]),
        ];
        let (plan, authored, collision) = pack_smodel_meshes(&meshes);
        assert_eq!(plan.decoded_vertices().len(), 6);
        assert_eq!(plan.indices(), &[0, 1, 2, 3, 4, 5]);
        assert_eq!(plan.surface_ranges(), &[(0, 3), (3, 3)]);
        assert_eq!(plan.vert_range_share.as_deref().unwrap(), &[(0, 3), (3, 3)]);
        assert_eq!(plan.meshes[1].vert_start, 3);
        assert_eq!(plan.meshes[1].vert_count_by_lod[0], 3);
        assert_eq!(plan.meshes[1].smc_surfs_by_lod[0][0].vert_base, 3);
        assert_eq!(authored.len(), 2);
        assert_eq!(collision[0].len(), 1);
        assert_eq!(plan.packed_row_count(), Some(6));
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
        let (plan, authored, collision) = pack_smodel_meshes(&meshes);
        // Only the middle surface packs; the others leave no rows behind.
        assert_eq!(plan.decoded_vertices().len(), 3);
        assert_eq!(plan.indices(), &[0, 1, 2]);
        assert_eq!(plan.surface_ranges(), &[(0, 3)]);
        assert_eq!(plan.meshes[0].surfaces_by_lod[0].len(), 1);
        // The authored row still lists all three, the collision row one.
        assert_eq!(authored[0].len(), 3);
        assert_eq!(collision[0].len(), 1);
    }

    #[test]
    fn nothing_packed_is_unavailable_and_publishing_keeps_it_so() {
        let (mut plan, _, _) = pack_smodel_meshes(&[model("empty", vec![])]);
        assert!(!plan.packed_ok());
        plan.publish_extract_shares();
        assert!(plan.packed_share.is_none());
        assert_eq!(plan.packed_row_count(), None);
    }

    #[test]
    fn publishing_moves_the_payload_behind_the_share() {
        let (mut plan, _, _) =
            pack_smodel_meshes(&[model("a", vec![surface(triangle(vec![0, 1, 2]), 3)])]);
        assert!(plan.packed_ok());
        plan.publish_extract_shares();
        assert_eq!(plan.packed_share.as_ref().map(|rows| rows.len()), Some(3));
        assert!(plan.packed_ok());
        assert_eq!(plan.packed_row_count(), Some(3));
    }
}
