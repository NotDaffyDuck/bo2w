//! Decal marks: one shared mesh of mark triangles, split into draw runs by material and by
//! the lighting inputs (lightmap, light, probe) the surface under the mark uses.

use std::sync::Arc;

use asset_t6::T6_WORLD_VERTEX;
use bevy::prelude::*;
use marks_t6::{
    DECAL_MESH_VERTEX_BYTES, SURFACE_BAKED_LIGHT_ABSENT, GfxMarkContext,
    MARK_MODEL_TYPE_STATIC_MODEL,
};
use render_frame::{SurfaceBakedLightId, SurfaceCubeProbeId, FacetSamplerInputs};

use super::fx::FxMaterialSlot;

// Mark vertices are stored as world-surface vertices.
const _: () = assert!(DECAL_MESH_VERTEX_BYTES == T6_WORLD_VERTEX);

/// Everything that makes two marks unable to share a draw run.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DecalSubmeshKey {
    pub baked_light_slot: u8,
    pub packed: bool,
    pub ent_id: Option<u16>,
    pub smodel: Option<u16>,
    pub glass: Option<u16>,
    pub dominant_light: u8,
    pub probe: u8,
}

impl DecalSubmeshKey {
    pub fn from_context(context: &GfxMarkContext) -> Self {
        // A mark on a moving entity is lit by the entity, not by a lightmap; a mark on the
        // player's own viewmodel counts as world.
        let entity_model = (context.is_entity() && !context.is_viewmodel())
            .then_some(context.mesh_slot_number);
        let smodel_model = (context.model_type() == MARK_MODEL_TYPE_STATIC_MODEL)
            .then_some(context.mesh_slot_number);
        Self {
            baked_light_slot: match entity_model {
                Some(_) => SURFACE_BAKED_LIGHT_ABSENT,
                None => context.baked_light_slot_number,
            },
            packed: !context.uses_world_vertex(),
            ent_id: entity_model,
            smodel: smodel_model,
            // TODO(fact): BO2 glass marks are world marks whose modelIndex is the glass id
            // (marks-t6.md); nothing adds marks to glass yet, so none gets glass lighting.
            glass: None,
            dominant_light: context.key_light_index,
            probe: context.env_probe_index,
        }
    }
}

/// Sampler inputs for a mark run: its probe, and its lightmap if it has one.
pub fn decal_mesh_surface_samplers(minor_key: DecalSubmeshKey) -> FacetSamplerInputs {
    let lightmap = match minor_key.baked_light_slot {
        SURFACE_BAKED_LIGHT_ABSENT => None,
        index => Some(SurfaceBakedLightId(index)),
    };
    FacetSamplerInputs {
        reflection_probe: Some(SurfaceCubeProbeId(minor_key.probe)),
        primary_lightmap: lightmap,
        secondary_lightmap: lightmap,
    }
}

/// A run of mark indices drawn with one material and one lighting key.
#[derive(Clone, Copy, Debug)]
pub struct DecalMeshDrawCall {
    pub shader: u32,
    pub first_triangle_slot: u32,
    pub index_count: u32,
    pub minor_key: DecalSubmeshKey,
}

impl DecalMeshDrawCall {
    fn index_end(&self) -> u32 {
        self.first_triangle_slot + self.index_count
    }
}

/// The mark triangles of a frame and the runs they are drawn in.
#[derive(Resource, Clone, Debug, Default)]
pub struct VisualStainMeshPlan {
    pub vertices: Arc<Vec<[u8; T6_WORLD_VERTEX]>>,
    pub indices: Arc<Vec<u16>>,
    pub mat_list: Vec<FxMaterialSlot>,
    pub draws: Vec<DecalMeshDrawCall>,
    pub revision: u64,
    /// Why the plan was left empty this frame, if it was.
    pub bypass_cause: Option<&'static str>,
    pub range_portion: Option<Arc<Vec<(u32, u32)>>>,
}

impl VisualStainMeshPlan {
    pub fn clear(&mut self) {
        super::recycle_rows(&mut self.vertices);
        super::recycle_rows(&mut self.indices);
        self.range_portion = None;
        self.mat_list.clear();
        self.draws.clear();
        self.bypass_cause = None;
    }

    pub fn next_edit(&mut self) {
        self.revision = self.revision.wrapping_add(1);
    }

    pub fn release_shared_copy(&mut self) {
        let runs = self.draws.iter().map(|d| (d.first_triangle_slot, d.index_count));
        self.range_portion = Some(super::collect_index_ranges(runs));
    }

    pub fn bit_rows(&self) -> &[[u8; T6_WORLD_VERTEX]] {
        &self.vertices
    }

    pub fn corner_rows(&self) -> &[u16] {
        &self.indices
    }

    /// Whether the newest run can take triangles starting at index `start`: same material,
    /// same lighting key, and no gap.
    fn newest_run_takes(
        &self,
        order_key: u8,
        material_draw_rank_slot: Option<u32>,
        minor_key: DecalSubmeshKey,
        start: u32,
    ) -> bool {
        let Some(last) = self.draws.last() else {
            return false;
        };
        let material = &self.mat_list[last.shader as usize];
        last.index_end() == start
            && last.minor_key == minor_key
            && material.bin == order_key
            && material.sorted_rank == material_draw_rank_slot
    }

    /// Appends triangles for one mark. They join the previous draw run when material and
    /// lighting key match and the indices are contiguous; otherwise they start a new run.
    pub fn extend_with_run_indices(
        &mut self,
        order_key: u8,
        material_draw_rank_slot: Option<u32>,
        minor_key: DecalSubmeshKey,
        index_list: &[u16],
    ) {
        let start = self.indices.len() as u32;
        let count = index_list.len() as u32;
        Arc::make_mut(&mut self.indices).extend_from_slice(index_list);

        if self.newest_run_takes(order_key, material_draw_rank_slot, minor_key, start) {
            if let Some(last) = self.draws.last_mut() {
                last.index_count += count;
            }
            return;
        }
        let material = self.mat_list.len() as u32;
        self.mat_list.push(FxMaterialSlot {
            texture: None,
            sorted_rank: material_draw_rank_slot,
            bin: order_key,
        });
        self.draws.push(DecalMeshDrawCall {
            shader: material,
            first_triangle_slot: start,
            index_count: count,
            minor_key,
        });
    }
}
