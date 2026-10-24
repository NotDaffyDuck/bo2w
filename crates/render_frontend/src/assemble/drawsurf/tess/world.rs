//! The retained world's draw plan: one decoded/packed vertex set, one index
//! set, per-surface ranges, and the per-surface material facts the renderer's
//! fallback pass reads. Built once when the world finishes loading.

use std::sync::Arc;

use bevy::prelude::*;
use render_material::RuntimeMaterial;

use crate::assemble::drawsurf::{
    RuntimeMaterialCatalog, SurfaceLightmapId, SurfaceReflectionProbeId, SurfaceSamplerInputs,
};
use crate::prepare::scene::spawn::{WorldSpawnJob, WorldSpawnPhase};
use crate::prepare::scene::world::WorldScene;

pub use render_frame::{WorldVertex, WorldVertexRefusal};

/// Texture semantic of the reflection-probe cube array in a world's material
/// images.
const TS_REFLECTION_PROBES: u8 = 0xf3;
/// Texture semantics of a layered material's second and third colour maps.
const TS_LAYER_MAPS: [u8; 2] = [0xf1, 0xf2];

/// What the fallback draw needs to know about one world material: image slots
/// in the uploaded material images.
#[derive(Clone, Debug)]
pub struct WorldPassMaterial {
    pub color_slot: Option<u32>,
    pub t6_draw: Option<asset_core::T6Draw>,
    pub t6_layer_slots: [Option<u32>; 2],
    pub t6_shine_slots: [Option<u32>; 2],
}

impl WorldPassMaterial {
    fn of(runtime: Option<&RuntimeMaterial>) -> Self {
        let slot = |semantic: u8| {
            runtime
                .and_then(|m| m.texture_semantic(semantic))
                .map(|id| id.0)
        };
        let has_draw = runtime.is_some_and(|m| m.t6_draw.is_some());
        let shine = |semantic: u8| if has_draw { slot(semantic) } else { None };
        Self {
            color_slot: slot(asset_material::TS_COLOR_MAP),
            t6_draw: runtime.and_then(|m| m.t6_draw),
            t6_layer_slots: TS_LAYER_MAPS.map(slot),
            t6_shine_slots: [asset_material::TS_NORMAL_MAP, asset_material::TS_SPECULAR_MAP]
                .map(shine),
        }
    }
}

#[derive(Resource, Clone, Debug, Default)]
pub struct WorldDrawGpuPlan {
    pub packed_vertices: asset_world::WorldVertexPayload,
    pub surface_material: Vec<u32>,
    pub surface_sampler_inputs: Vec<SurfaceSamplerInputs>,
    /// Per surface, its primary light index.
    pub surface_primary_light: Vec<u8>,
    pub materials: Vec<WorldPassMaterial>,
    pub vertex_share: Option<Arc<Vec<[u8; asset_t6::T6_WORLD_VERTEX]>>>,
    pub index_share: Option<Arc<Vec<u32>>>,
    pub range_share: Option<Arc<Vec<(u32, u32)>>>,
    pub layer_share: Option<Arc<Vec<u8>>>,
    pub decoded_share: Option<Arc<Vec<WorldVertex>>>,
    /// Per decoded vertex, a layered surface's second and third texcoords
    /// (empty when the world has none).
    pub t6_layer_share: Arc<Vec<[f32; 4]>>,
}

/// The uploaded image slot of the world's reflection-probe cube array.
pub fn t6_probe_slot(catalog: &RuntimeMaterialCatalog) -> Option<u32> {
    catalog
        .materials
        .iter()
        .find_map(|m| m.texture_semantic(TS_REFLECTION_PROBES))
        .map(|id| id.0)
}

/// The uploaded image slot of the sky's cube colour map: the colour map of
/// the first material that blends as a sky.
pub fn t6_sky_slot(catalog: &RuntimeMaterialCatalog) -> Option<u32> {
    catalog
        .materials
        .iter()
        .find(|m| m.t6_draw.is_some_and(|d| d.blend == asset_core::T6Blend::Sky))
        .and_then(|m| m.texture_semantic(asset_material::TS_COLOR_MAP))
        .map(|id| id.0)
}

/// Rows of a shared bank, or nothing when the bank was never published.
fn rows<T>(share: &Option<Arc<Vec<T>>>) -> &[T] {
    super::published_rows(share)
}

impl WorldDrawGpuPlan {
    pub fn indices(&self) -> &[u32] {
        rows(&self.index_share)
    }

    pub fn surface_ranges(&self) -> &[(u32, u32)] {
        rows(&self.range_share)
    }

    pub fn decoded_vertices(&self) -> &[WorldVertex] {
        rows(&self.decoded_share)
    }

    pub fn vertex_layer_rows(&self) -> &[u8] {
        rows(&self.layer_share)
    }

    /// Per surface, the material of `surface_material`, or `None` for a
    /// surface with no material.
    fn surface_materials(&self) -> impl Iterator<Item = Option<&WorldPassMaterial>> {
        self.surface_material
            .iter()
            .map(|&m| self.materials.get(m as usize))
    }

    /// Per surface: its colour map's image slot (`u32::MAX` = none), its draw
    /// code (`T6Draw::NONE_CODE` = none) and its lightmap page (`u8::MAX` =
    /// none), for the renderer's fallback pass.
    pub fn fallback_surface_styles(&self) -> (Vec<u32>, Vec<u32>, Vec<u8>) {
        let n = self.surface_material.len();
        let mut colors = Vec::with_capacity(n);
        let mut draws = Vec::with_capacity(n);
        let mut lightmaps = Vec::with_capacity(n);
        for (i, material) in self.surface_materials().enumerate() {
            colors.push(material.and_then(|m| m.color_slot).unwrap_or(u32::MAX));
            draws.push(
                material
                    .and_then(|m| m.t6_draw)
                    .map_or(asset_core::T6Draw::NONE_CODE, asset_core::T6Draw::code),
            );
            let page = self
                .surface_sampler_inputs
                .get(i)
                .and_then(|inputs| inputs.secondary_lightmap);
            lightmaps.push(page.map_or(u8::MAX, |id| id.0));
        }
        (colors, draws, lightmaps)
    }

    /// Per surface: normal and specular map slots (`u32::MAX` = none) and the
    /// reflection probe, for the fallback draw's shine.
    pub fn fallback_surface_shine(&self) -> Vec<[u32; 3]> {
        self.surface_materials()
            .enumerate()
            .map(|(i, material)| {
                let [normal, specular] = material.map_or([None; 2], |m| m.t6_shine_slots);
                let probe = self
                    .surface_sampler_inputs
                    .get(i)
                    .and_then(|inputs| inputs.reflection_probe)
                    .map_or(0, |id| u32::from(id.0));
                [normal.unwrap_or(u32::MAX), specular.unwrap_or(u32::MAX), probe]
            })
            .collect()
    }

    /// Per surface: its second and third colour map slots (`u32::MAX` =
    /// none), for layered materials.
    pub fn fallback_surface_layers(&self) -> Vec<[u32; 2]> {
        self.surface_materials()
            .map(|material| {
                material.map_or([u32::MAX; 2], |m| {
                    m.t6_layer_slots.map(|slot| slot.unwrap_or(u32::MAX))
                })
            })
            .collect()
    }

    /// The packed vertex records, when their layout is the renderer's world
    /// layout and there is one per decoded vertex.
    pub fn exact_packed_vertices(
        &self,
    ) -> Result<&[[u8; asset_t6::T6_WORLD_VERTEX]], WorldVertexRefusal> {
        use render_material::decl_table::{WORLD_VERTEX_TYPE, stream_extent};
        let table_stride = stream_extent(WORLD_VERTEX_TYPE, 0);
        if table_stride != Some(asset_t6::T6_WORLD_VERTEX as u16) {
            return Err(WorldVertexRefusal::StrideMismatch { table_stride });
        }
        let packed = match &self.vertex_share {
            Some(share) => share.as_slice(),
            None => self
                .packed_vertices
                .type2_stream0()
                .map_err(|source_layout| WorldVertexRefusal::ForeignLayout { source_layout })?,
        };
        let decoded = self.decoded_vertices().len();
        if packed.len() != decoded {
            return Err(WorldVertexRefusal::VertexCountMismatch {
                packed: packed.len(),
                decoded,
            });
        }
        Ok(packed)
    }

    /// Moves the packed rows into a shared bank the extract step can hand to
    /// the renderer without copying. A payload that is not rows stays put.
    fn share_packed_rows(&mut self) {
        if self.vertex_share.is_some() {
            return;
        }
        let taken = std::mem::replace(
            &mut self.packed_vertices,
            asset_world::WorldVertexPayload::Unavailable {
                source_layout: "packed payload published for extract",
            },
        );
        match taken {
            asset_world::WorldVertexPayload::T6(packed) => {
                self.vertex_share = Some(Arc::new(packed));
            }
            other => self.packed_vertices = other,
        }
    }

    /// Takes the world's geometry out of `scene` and builds the plan.
    fn take_from(scene: &mut WorldScene) -> Self {
        let packed_vertices = std::mem::replace(
            &mut scene.retained_packed_vertices,
            asset_world::WorldVertexPayload::Unavailable {
                source_layout: "packed payload consumed by WorldDrawGpuPlan",
            },
        );
        let decoded = decode_vertices(scene);
        // Only the decoded rows keep these; the scene's copies are dead weight.
        scene.retained_tangents.clear();
        scene.retained_colors.clear();
        scene.retained_texture_uvs.clear();

        let mut materials = Vec::with_capacity(scene.batches.len());
        for batch in &scene.batches {
            let runtime = batch
                .material
                .and_then(|id| scene.runtime_material_catalog.derived(id));
            materials.push(WorldPassMaterial::of(runtime));
        }

        let mut plan = Self {
            packed_vertices,
            materials,
            layer_share: Some(Arc::new(scene.retained_vertex_layer.clone())),
            t6_layer_share: Arc::new(scene.retained_t6_layer_uvs.clone()),
            ..Self::default()
        };
        plan.decoded_share = Some(Arc::new(decoded));

        let (indices, ranges) = match scene.cull.as_ref() {
            Some(cull) => {
                plan.surface_primary_light = cull.surface_primary_lights.clone();
                plan.surface_material = cull
                    .surface_batch_ranges
                    .iter()
                    .zip(&cull.surface_index_ranges)
                    .map(|(&(batch, _, _), &(_, count))| {
                        if count == 0 || batch >= plan.materials.len() {
                            u32::MAX
                        } else {
                            batch as u32
                        }
                    })
                    .collect();
                plan.surface_sampler_inputs = cull
                    .surface_batch_ranges
                    .iter()
                    .map(|&(batch, _, _)| {
                        scene
                            .batches
                            .get(batch)
                            .map(|b| SurfaceSamplerInputs {
                                reflection_probe: Some(SurfaceReflectionProbeId(
                                    b.reflection_probe_index,
                                )),
                                primary_lightmap: b
                                    .lightmapped
                                    .then_some(SurfaceLightmapId(b.lightmap_index)),
                                secondary_lightmap: b
                                    .lightmapped
                                    .then_some(SurfaceLightmapId(b.lightmap_index)),
                            })
                            .unwrap_or_default()
                    })
                    .collect();
                (cull.packed_indices.clone(), cull.surface_index_ranges.clone())
            }
            None => (Vec::new(), Vec::new()),
        };
        plan.index_share = Some(Arc::new(indices));
        plan.range_share = Some(Arc::new(ranges));
        plan.share_packed_rows();
        plan
    }
}

/// One `WorldVertex` per retained position; attributes the scene lacks get
/// the neutral value.
fn decode_vertices(scene: &WorldScene) -> Vec<WorldVertex> {
    (0..scene.retained_positions.len())
        .map(|i| WorldVertex {
            position: scene.retained_positions[i],
            normal: scene.retained_normals.get(i).copied().unwrap_or([0., 0., 1.]),
            tangent: scene
                .retained_tangents
                .get(i)
                .copied()
                .unwrap_or([1., 0., 0., 1.]),
            color: scene.retained_colors.get(i).copied().unwrap_or([1.; 4]),
            uv0: scene.retained_texture_uvs.get(i).copied().unwrap_or([0.; 2]),
            uv1: scene.retained_lightmap_uvs.get(i).copied().unwrap_or([0.; 2]),
        })
        .collect()
}

pub(crate) fn build_world_draw_gpu_plan(
    mut scene: Option<ResMut<WorldScene>>,
    job: Res<WorldSpawnJob>,
    existing: Option<Res<WorldDrawGpuPlan>>,
    mut commands: Commands,
) {
    if job.phase != WorldSpawnPhase::WorldTess || existing.is_some() {
        return;
    }
    let Some(scene) = scene.as_mut() else {
        return;
    };
    let plan = WorldDrawGpuPlan::take_from(scene);
    let refusal = plan.exact_packed_vertices().err();
    let with = |pick: fn(&SurfaceSamplerInputs) -> bool| {
        plan.surface_sampler_inputs
            .iter()
            .filter(|inputs| pick(inputs))
            .count()
    };
    diag::info!(
        World,
        "world drawsurf tess plan: batches={} verts={} indices={} materials={} packed_refusal={:?} surface_inputs={}/{} probe/lightmap",
        scene.batches.len(),
        plan.decoded_vertices().len(),
        plan.indices().len(),
        plan.materials.len(),
        refusal,
        with(|i| i.reflection_probe.is_some()),
        with(|i| i.secondary_lightmap.is_some()),
    );
    commands.insert_resource(plan);
}
