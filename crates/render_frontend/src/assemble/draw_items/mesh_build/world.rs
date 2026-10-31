//! The retained world's draw plan: one decoded/packed vertex set, one index
//! set, per-surface ranges, and the per-surface material facts the renderer's
//! fallback pass reads. Built once when the world finishes loading.

use std::sync::Arc;

use bevy::prelude::*;
use render_material::LiveMaterial;

use crate::{
    LiveMaterialLedger, SurfaceBakedLightId, SurfaceCubeProbeId, FacetSamplerInputs,
};
use crate::prepare::scene::spawn::{LevelSpawnJob, LevelSpawnPhase};
use crate::prepare::scene::world::LevelStage;

pub use render_frame::{LevelVertex, WorldVertexRefusal};

/// Texture semantic of the reflection-probe cube array in a world's material
/// images.
const TS_REFLECTION_PROBES: u8 = 0xf3;
/// Texture semantics of a layered material's second and third colour maps.
const TS_LAYER_MAPS: [u8; 2] = [0xf1, 0xf2];
/// bo2mp: texture semantics of a sea's white-water noise and mask maps.
const TS_FOAM_MAPS: [u8; 2] = [0xf4, 0xf5];

/// One compact world vertex as stored.

/// What the fallback draw needs to know about one world material: image slots
/// in the uploaded material images.
#[derive(Clone, Debug)]
pub struct LevelPassMaterial {
    pub color_slot: Option<u32>,
    pub t6_draw: Option<asset_core::T6Draw>,
    pub t6_layer_slots: [Option<u32>; 2],
    pub t6_foam_slots: [Option<u32>; 2],
    pub t6_shine_slots: [Option<u32>; 2],
}

impl LevelPassMaterial {
    fn of(runtime: Option<&LiveMaterial>) -> Self {
        let slot = |semantic: u8| {
            runtime
                .and_then(|m| m.semantic_for_image(semantic))
                .map(|ident| ident.0)
        };
        let has_draw = runtime.is_some_and(|m| m.t6_draw.is_some());
        let shine = |semantic: u8| if has_draw { slot(semantic) } else { None };
        Self {
            color_slot: slot(asset_material::SEMANTIC_COLOR),
            t6_draw: runtime.and_then(|m| m.t6_draw),
            t6_layer_slots: TS_LAYER_MAPS.map(slot),
            t6_foam_slots: TS_FOAM_MAPS.map(slot),
            t6_shine_slots: [asset_material::SEMANTIC_NORMAL, asset_material::SEMANTIC_SPECULAR]
                .map(shine),
        }
    }
}

#[derive(Resource, Clone, Debug, Default)]
pub struct WorldGpuSubmission {
    pub compact_vertices: asset_world::WorldVertexPayload,
    pub facet_material: Vec<u32>,
    pub facet_sampler_inputs: Vec<FacetSamplerInputs>,
    /// Per surface, its primary light index.
    pub surface_primary_light: Vec<u8>,
    pub materials: Vec<LevelPassMaterial>,
    pub vertex_share: Option<Arc<Vec<[u8; asset_t6::T6_WORLD_VERTEX]>>>,
    pub index_portion: Option<Arc<Vec<u32>>>,
    pub range_portion: Option<Arc<Vec<(u32, u32)>>>,
    pub layer_share: Option<Arc<Vec<u8>>>,
    pub decoded_share: Option<Arc<Vec<LevelVertex>>>,
    /// Per decoded vertex, a layered surface's second and third texcoords
    /// (empty when the world has none).
    pub t6_layer_share: Arc<Vec<[f32; 4]>>,
}

/// The uploaded image slot of the world's reflection-probe cube array.
pub fn t6_probe_slot(catalog: &LiveMaterialLedger) -> Option<u32> {
    let materials = &catalog.materials;
    materials
        .iter()
        .find_map(|m| m.semantic_for_image(TS_REFLECTION_PROBES))
        .map(|ident| ident.0)
}

/// The uploaded image slot of the sky's cube colour map: the colour map of
/// the first material that blends as a sky.
pub fn t6_sky_slot(catalog: &LiveMaterialLedger) -> Option<u32> {
    let materials = &catalog.materials;
    materials
        .iter()
        .find(|m| m.t6_draw.is_some_and(|d| d.blend == asset_core::T6Blend::Sky))
        .and_then(|m| m.semantic_for_image(asset_material::SEMANTIC_COLOR))
        .map(|ident| ident.0)
}

/// Rows of a shared bank, or nothing when the bank was never published.
fn rows<T>(share: &Option<Arc<Vec<T>>>) -> &[T] {
    super::published_rows(share)
}

impl WorldGpuSubmission {
    pub fn indices(&self) -> &[u32] {
        rows(&self.index_portion)
    }

    pub fn facet_ranges(&self) -> &[(u32, u32)] {
        rows(&self.range_portion)
    }

    pub fn decoded_vertices(&self) -> &[LevelVertex] {
        rows(&self.decoded_share)
    }

    pub fn vertex_layer_rows(&self) -> &[u8] {
        rows(&self.layer_share)
    }

    /// Per surface, the material of `facet_material`, or `None` for a
    /// surface with no material.
    fn facet_materials(&self) -> impl Iterator<Item = Option<&LevelPassMaterial>> {
        self.facet_material
            .iter()
            .map(|&m| self.materials.get(m as usize))
    }

    /// Per surface: its colour map's image slot (`u32::MAX` = none), its draw
    /// code (`T6Draw::NONE_CODE` = none) and its lightmap page (`u8::MAX` =
    /// none), for the renderer's fallback pass.
    pub fn fallback_surface_styles(&self) -> (Vec<u32>, Vec<u32>, Vec<u8>) {
        let n = self.facet_material.len();
        let mut colors = Vec::with_capacity(n);
        let mut draws = Vec::with_capacity(n);
        let mut lightmaps = Vec::with_capacity(n);
        for (i, material) in self.facet_materials().enumerate() {
            colors.push(material.and_then(|m| m.color_slot).unwrap_or(u32::MAX));
            draws.push(
                material
                    .and_then(|m| m.t6_draw)
                    .map_or(asset_core::T6Draw::NONE_CODE, asset_core::T6Draw::code),
            );
            let page = self
                .facet_sampler_inputs
                .get(i)
                .and_then(|inputs| inputs.secondary_lightmap);
            lightmaps.push(page.map_or(u8::MAX, |ident| ident.0));
        }
        (colors, draws, lightmaps)
    }

    /// Per surface: normal and specular map slots (`u32::MAX` = none) and the
    /// reflection probe, for the fallback draw's shine.
    pub fn fallback_surface_shine(&self) -> Vec<[u32; 3]> {
        self.facet_materials()
            .enumerate()
            .map(|(i, facet_material)| {
                let [normal, specular] = facet_material.map_or([None; 2], |m| m.t6_shine_slots);
                let probe = self
                    .facet_sampler_inputs
                    .get(i)
                    .and_then(|inputs| inputs.reflection_probe)
                    .map_or(0, |id| u32::from(id.0));
                [normal.unwrap_or(u32::MAX), specular.unwrap_or(u32::MAX), probe]
            })
            .collect()
    }

    /// Per surface: its second and third colour map slots (`u32::MAX` =
    /// none), for layered materials, then a sea's white-water noise and
    /// mask slots.
    pub fn fallback_surface_layers(&self) -> Vec<[u32; 4]> {
        self.facet_materials()
            .map(|entry| {
                entry.map_or([u32::MAX; 4], |m| {
                    let [l1, l2] = m.t6_layer_slots.map(|slot| slot.unwrap_or(u32::MAX));
                    let [f1, f2] = m.t6_foam_slots.map(|slot| slot.unwrap_or(u32::MAX));
                    [l1, l2, f1, f2]
                })
            })
            .collect()
    }

    /// The packed vertex records, when their layout is the renderer's world
    /// layout and there is one per decoded vertex.
    pub fn precise_compact_vertices(
        &self,
    ) -> Result<&'_ [[u8; asset_t6::T6_WORLD_VERTEX]], WorldVertexRefusal> {
        use render_material::decl_table::{WORLD_VERTEX_LAYOUT_ID, stream_extent};
        let table_stride = stream_extent(WORLD_VERTEX_LAYOUT_ID, 0);
        if table_stride != Some(asset_t6::T6_WORLD_VERTEX as u16) {
            return Err(WorldVertexRefusal::StrideMismatch { table_stride });
        }
        let packed = match &self.vertex_share {
            Some(share) => share.as_slice(),
            None => {
                let stream = &self.compact_vertices;
                let refusal = || WorldVertexRefusal::ForeignLayout { origin_layout: stream.dump_variety_name() };
                stream.type2_stream0().ok_or_else(refusal)?
            }
        };
        let decoded = self.decoded_vertices().len();
        if packed.len() != decoded {
            return Err(WorldVertexRefusal::CountsDiffer {
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
            &mut self.compact_vertices,
            asset_world::WorldVertexPayload::Unavailable {
                origin_layout: "packed payload published for extract",
            },
        );
        match taken {
            asset_world::WorldVertexPayload::T6(packed) => {
                self.vertex_share = Some(Arc::new(packed));
            }
            other => self.compact_vertices = other,
        }
    }

    /// Takes the world's geometry out of `scene` and builds the plan.
    fn take_from(scene: &mut LevelStage) -> Self {
        let compact_vertices = std::mem::replace(
            &mut scene.retained_packed_vertices,
            asset_world::WorldVertexPayload::Unavailable {
                origin_layout: "packed payload consumed by WorldDrawGpuPlan",
            },
        );
        let decoded = decode_vertices(scene);
        // Only the decoded rows keep these; the scene's copies are dead weight.
        scene.held_tangents.clear();
        scene.held_colors.clear();
        scene.held_texture_uvs.clear();

        let ledger = &scene.live_material_ledger;
        let materials: Vec<_> = scene
            .batches
            .iter()
            .map(|b| LevelPassMaterial::of(b.material.and_then(|id| ledger.derived(id))))
            .collect();

        let mut plan = Self {
            compact_vertices,
            materials,
            layer_share: Some(Arc::new(scene.held_vertex_layer.clone())),
            t6_layer_share: Arc::new(scene.retained_t6_layer_uvs.clone()),
            ..Self::default()
        };
        plan.decoded_share = Some(Arc::new(decoded));

        let (index_list, ranges) = match scene.culling.as_ref() {
            Some(sight) => {
                plan.surface_primary_light = sight.facet_key_lights.clone();
                plan.facet_material = sight
                    .facet_batch_ranges
                    .iter()
                    .zip(&sight.facet_index_ranges)
                    .map(|(&(slot, _, _), &(_, indices))| match indices {
                        0 => u32::MAX,
                        _ if slot >= plan.materials.len() => u32::MAX,
                        _ => slot as u32,
                    })
                    .collect();
                plan.facet_sampler_inputs = sight
                    .facet_batch_ranges
                    .iter()
                    .map(|&(slot, _, _)| {
                        let Some(b) = scene.batches.get(slot) else {
                            return FacetSamplerInputs::default();
                        };
                        let baked = b.lmapped.then_some(SurfaceBakedLightId(b.lightbake_index));
                        FacetSamplerInputs {
                            reflection_probe: Some(SurfaceCubeProbeId(b.env_probe_index)),
                            primary_lightmap: baked,
                            secondary_lightmap: baked,
                        }
                    })
                    .collect();
                (sight.compact_indices.clone(), sight.facet_index_ranges.clone())
            }
            None => (Vec::new(), Vec::new()),
        };
        plan.index_portion = Some(Arc::new(index_list));
        plan.range_portion = Some(Arc::new(ranges));
        plan.share_packed_rows();
        plan
    }
}

/// One `LevelVertex` per retained position; attributes the scene lacks get
/// the neutral value.
fn decode_vertices(scene: &LevelStage) -> Vec<LevelVertex> {
    (0..scene.held_positions.len())
        .map(|i| LevelVertex {
            position: scene.held_positions[i],
            normal: scene.held_normals.get(i).copied().unwrap_or([0., 0., 1.]),
            tangent: scene.held_tangents.get(i).copied().unwrap_or([1., 0., 0., 1.]),
            color: scene.held_colors.get(i).copied().unwrap_or([1.; 4]),
            first_texcoord: scene.held_texture_uvs.get(i).copied().unwrap_or([0.; 2]),
            lightmap_uv: scene.held_lightbake_uvs.get(i).copied().unwrap_or([0.; 2]),
        })
        .collect()
}

pub(crate) fn build_level_draw_gpu_plan(
    mut scene: Option<ResMut<LevelStage>>,
    job: Res<LevelSpawnJob>,
    already: Option<Res<WorldGpuSubmission>>,
    mut commands: Commands,
) {
    if job.stage_now != LevelSpawnPhase::WorldTess || already.is_some() {
        return;
    }
    let Some(stage) = scene.as_mut() else {
        return;
    };
    let plan = WorldGpuSubmission::take_from(stage);
    let (batch_total, material_total) = (stage.batches.len(), plan.materials.len());
    let refusal = plan.precise_compact_vertices().err();
    let with = |pick: fn(&FacetSamplerInputs) -> bool| {
        plan.facet_sampler_inputs
            .iter()
            .filter(|inputs| pick(inputs))
            .count()
    };
    diag::info!(
        World,
        "world drawsurf tess plan: batches={} verts={} indices={} materials={} packed_refusal={:?} surface_inputs={}/{} probe/lightmap",
        batch_total,
        plan.decoded_vertices().len(),
        plan.indices().len(),
        material_total,
        refusal,
        with(|i| i.reflection_probe.is_some()),
        with(|i| i.secondary_lightmap.is_some()),
    );
    commands.insert_resource::<WorldGpuSubmission>(plan);
}
