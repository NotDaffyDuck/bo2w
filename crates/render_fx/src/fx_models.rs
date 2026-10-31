//! Adds one effect model's surfaces to the shared geometry of a `FxModelDrawList`.

use std::sync::Arc;

use asset_t6::T6_PACKED_VERTEX;
use asset_world::PackedVertexPayload;
use render_anim::gun_pose::SkinnedMeshFacet;
use render_anim::geometry::{append_mesh, install_held_compact};
use render_frame::InputChangeCounters;
use render_scene::PropPassMaterial;

use crate::model_draw::{FxModelSurfaces, FxModelDrawList};

pub const FIGURE_COMPACT_UNAVAILABLE: &str =
    "xmodel merge GfxPackedVertex missing or count-mismatched; decoded float is not packed VB";
pub const FIGURE_COMPACT_EMPTY_PLAN: &str = "xmodel plan has no vertices";

/// The packed vertex rows gathered next to the decoded ones. They are kept only while every
/// surface so far supplied exactly one packed row per decoded vertex; the first one that
/// does not drops the packed copy for the whole plan.
struct PackedRun {
    row_list: Vec<[u8; T6_PACKED_VERTEX]>,
    intact: bool,
}

impl PackedRun {
    /// Takes the plan's packed rows out, leaving the "unavailable" marker in their place until
    /// `install` puts the result back.
    fn take_from(payload: &mut PackedVertexPayload, decoded_vertices: usize) -> Self {
        let intact = decoded_vertices == 0 || matches!(payload, PackedVertexPayload::T6(_));
        let marker = PackedVertexPayload::Unavailable {
            origin_layout: FIGURE_COMPACT_UNAVAILABLE,
        };
        let row_list = match std::mem::replace(payload, marker) {
            PackedVertexPayload::T6(rows) => rows,
            PackedVertexPayload::Unavailable { .. } => Vec::new(),
        };
        Self { row_list, intact }
    }

    fn add_surface(&mut self, packed: &[[u8; T6_PACKED_VERTEX]], decoded: usize) {
        if self.intact && packed.len() == decoded {
            self.row_list.extend_from_slice(packed);
        } else {
            self.intact = false;
            self.row_list.clear();
        }
    }

    fn install(self, vertex_count: usize) -> PackedVertexPayload {
        install_held_compact(
            self.intact,
            self.row_list,
            vertex_count,
            FIGURE_COMPACT_EMPTY_PLAN,
            FIGURE_COMPACT_UNAVAILABLE,
        )
    }
}

/// Appends every surface of (`mesh_slot_number`, `lod`) that has a material and records the
/// asset. Returns the (surface range index, material index) pairs it added.
pub fn push_effect_model_asset(
    plan: &mut FxModelDrawList,
    mesh_slot_number: usize,
    lod: u8,
    surfaces: &[SkinnedMeshFacet],
    materials: &[Option<PropPassMaterial>],
) -> Vec<(u32, u32)> {
    let geometry = Arc::make_mut(&mut plan.geometry);
    let mut packed = PackedRun::take_from(&mut geometry.compact_vertices, geometry.vertices.len());

    let mut added = Vec::new();
    for (facet, shader) in surfaces.iter().zip(materials) {
        let Some(material) = shader else {
            continue;
        };
        let first_vertex = geometry.vertices.len();
        let Some(range) = append_mesh(&facet.mesh, &mut geometry.vertices, &mut geometry.indices)
        else {
            continue;
        };
        packed.add_surface(&facet.compact_vertices, geometry.vertices.len() - first_vertex);
        added.push((
            geometry.facet_ranges.len() as u32,
            geometry.materials.len() as u32,
        ));
        geometry.facet_ranges.push(range);
        geometry.materials.push(material.clone());
    }

    geometry.compact_vertices = packed.install(geometry.vertices.len());
    geometry.assets.push(FxModelSurfaces {
        mesh_slot_number,
        lod,
        surf_list: added.clone(),
    });
    refresh_topology(plan);
    added
}

/// Re-derives the topology revision from the geometry as it now stands and counts the packed
/// write.
fn refresh_topology(plan: &mut FxModelDrawList) {
    let shape = &plan.geometry;
    let mut fresh = InputChangeCounters::default();
    fresh.stamp_topology(
        &shape.indices,
        &shape.facet_ranges,
        shape.vertices.len(),
    );
    plan.revisions.topology = fresh.topology;
    plan.revisions.note_packed_data_rewritten(&mut plan.edit_no);
}
