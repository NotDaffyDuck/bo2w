//! Helpers that move posed mesh data into the flat buffers a draw plan keeps.

use asset_t6::T6_PACKED_VERTEX;
use asset_world::PackedVertexPayload;
use bevy::mesh::{Indices, VertexAttributeValues};
use bevy::prelude::*;
use render_frame::SmodelVertex;

const DEFAULT_NORMAL: [f32; 3] = [0.0, 0.0, 1.0];
const DEFAULT_UV: [f32; 2] = [0.0, 0.0];
const DEFAULT_COLOUR: [f32; 4] = [1.0; 4];

/// Decides what packed-vertex payload a plan keeps after rows were collected
/// for it. The rows count only when they were never invalidated, are not empty
/// and line up one-to-one with the `decoded_count` vertices; otherwise the plan
/// says why it has none: `empty` for a plan without vertices, `missing` for a
/// plan that has vertices but no usable rows.
pub fn install_retained_packed(
    packed_ok: bool,
    packed: Vec<[u8; T6_PACKED_VERTEX]>,
    decoded_count: usize,
    empty: &'static str,
    missing: &'static str,
) -> PackedVertexPayload {
    let usable = packed_ok && !packed.is_empty() && packed.len() == decoded_count;
    if usable {
        PackedVertexPayload::T6(packed)
    } else {
        PackedVertexPayload::Unavailable {
            source_layout: if decoded_count == 0 { empty } else { missing },
        }
    }
}

macro_rules! float_attribute {
    ($name:ident, $variant:ident, $width:literal) => {
        pub(crate) fn $name(values: &VertexAttributeValues) -> Option<&[[f32; $width]]> {
            if let VertexAttributeValues::$variant(rows) = values {
                Some(rows)
            } else {
                None
            }
        }
    };
}

float_attribute!(f32x2, Float32x2, 2);
float_attribute!(f32x3, Float32x3, 3);
float_attribute!(f32x4, Float32x4, 4);

fn index_count(indices: &Indices) -> usize {
    match indices {
        Indices::U16(rows) => rows.len(),
        Indices::U32(rows) => rows.len(),
    }
}

fn index_values(indices: &Indices) -> Box<dyn Iterator<Item = u32> + '_> {
    match indices {
        Indices::U16(rows) => Box::new(rows.iter().map(|&i| u32::from(i))),
        Indices::U32(rows) => Box::new(rows.iter().copied()),
    }
}

/// The vertices of `mesh`: its positions, with an up normal, the origin uv and
/// opaque white standing in for any attribute the mesh lacks.
pub(crate) fn mesh_vertices(mesh: &Mesh) -> Option<Vec<SmodelVertex>> {
    let positions = mesh.attribute(Mesh::ATTRIBUTE_POSITION).and_then(f32x3)?;
    let normals = mesh.attribute(Mesh::ATTRIBUTE_NORMAL).and_then(f32x3);
    let uvs = mesh.attribute(Mesh::ATTRIBUTE_UV_0).and_then(f32x2);
    let colours = mesh.attribute(Mesh::ATTRIBUTE_COLOR).and_then(f32x4);
    Some(
        positions
            .iter()
            .enumerate()
            .map(|(i, &position)| SmodelVertex {
                position,
                normal: normals
                    .and_then(|a| a.get(i))
                    .copied()
                    .unwrap_or(DEFAULT_NORMAL),
                color: colours
                    .and_then(|a| a.get(i))
                    .copied()
                    .unwrap_or(DEFAULT_COLOUR),
                uv0: uvs.and_then(|a| a.get(i)).copied().unwrap_or(DEFAULT_UV),
            })
            .collect(),
    )
}

/// How many indices `mesh` lists, if it is indexed.
pub(crate) fn mesh_index_count(mesh: &Mesh) -> Option<usize> {
    mesh.indices().map(index_count)
}

/// Appends `mesh` to the flat vertex and index buffers and returns where its
/// indices landed as `(first index, index count)`.
///
/// The mesh needs positions and an index list. A mesh with no vertices, no
/// indices or an index past its last vertex adds nothing.
pub fn append_mesh(
    mesh: &Mesh,
    vertices: &mut Vec<SmodelVertex>,
    indices: &mut Vec<u32>,
) -> Option<(u32, u32)> {
    let rows = mesh_vertices(mesh)?;
    let source = mesh.indices()?;
    let count = index_count(source);
    if rows.is_empty() || count == 0 {
        return None;
    }
    if index_values(source).any(|i| i as usize >= rows.len()) {
        return None;
    }

    let base = vertices.len() as u32;
    vertices.extend(rows);
    let first = indices.len() as u32;
    indices.extend(index_values(source).map(|i| base + i));
    Some((first, count as u32))
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::asset::RenderAssetUsages;
    use bevy::mesh::PrimitiveTopology;

    const ROW: [u8; T6_PACKED_VERTEX] = [7; T6_PACKED_VERTEX];

    fn triangle() -> Mesh {
        let mut mesh = Mesh::new(
            PrimitiveTopology::TriangleList,
            RenderAssetUsages::default(),
        );
        mesh.insert_attribute(
            Mesh::ATTRIBUTE_POSITION,
            vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
        );
        mesh.insert_indices(Indices::U32(vec![0, 1, 2]));
        mesh
    }

    fn payload_layout(payload: &PackedVertexPayload) -> Option<&'static str> {
        match payload {
            PackedVertexPayload::Unavailable { source_layout } => Some(source_layout),
            PackedVertexPayload::T6(_) => None,
        }
    }

    #[test]
    fn matching_rows_are_kept() {
        let kept = install_retained_packed(true, vec![ROW; 3], 3, "empty", "missing");
        assert!(matches!(kept, PackedVertexPayload::T6(rows) if rows.len() == 3));
    }

    #[test]
    fn rows_are_dropped_when_invalidated_or_miscounted() {
        let invalid = install_retained_packed(false, vec![ROW; 3], 3, "empty", "missing");
        assert_eq!(payload_layout(&invalid), Some("missing"));
        let short = install_retained_packed(true, vec![ROW; 2], 3, "empty", "missing");
        assert_eq!(payload_layout(&short), Some("missing"));
        let none = install_retained_packed(true, Vec::new(), 3, "empty", "missing");
        assert_eq!(payload_layout(&none), Some("missing"));
    }

    #[test]
    fn a_plan_without_vertices_reports_empty() {
        let none = install_retained_packed(true, Vec::new(), 0, "empty", "missing");
        assert_eq!(payload_layout(&none), Some("empty"));
        let stray = install_retained_packed(true, vec![ROW; 2], 0, "empty", "missing");
        assert_eq!(payload_layout(&stray), Some("empty"));
    }

    #[test]
    fn attribute_views_only_answer_their_own_width() {
        let wide = VertexAttributeValues::Float32x3(vec![[1.0, 2.0, 3.0]]);
        assert_eq!(f32x3(&wide), Some(&[[1.0, 2.0, 3.0]][..]));
        assert!(f32x2(&wide).is_none());
        assert!(f32x4(&wide).is_none());
        let flat = VertexAttributeValues::Float32x2(vec![[4.0, 5.0]]);
        assert_eq!(f32x2(&flat), Some(&[[4.0, 5.0]][..]));
        let colour = VertexAttributeValues::Float32x4(vec![[0.5; 4]]);
        assert_eq!(f32x4(&colour), Some(&[[0.5; 4]][..]));
    }

    #[test]
    fn a_bare_mesh_gets_default_shading_attributes() {
        let (mut vertices, mut indices) = (Vec::new(), Vec::new());
        let at = append_mesh(&triangle(), &mut vertices, &mut indices);
        assert_eq!(at, Some((0, 3)));
        assert_eq!(indices, [0, 1, 2]);
        assert_eq!(vertices.len(), 3);
        assert_eq!(vertices[1].position, [1.0, 0.0, 0.0]);
        assert_eq!(vertices[1].normal, DEFAULT_NORMAL);
        assert_eq!(vertices[1].uv0, DEFAULT_UV);
        assert_eq!(vertices[1].color, DEFAULT_COLOUR);
    }

    #[test]
    fn authored_attributes_are_copied() {
        let mut mesh = triangle();
        mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, vec![[1.0, 0.0, 0.0]; 3]);
        mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, vec![[0.25, 0.75]; 3]);
        mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, vec![[0.1, 0.2, 0.3, 0.4]; 3]);
        let (mut vertices, mut indices) = (Vec::new(), Vec::new());
        append_mesh(&mesh, &mut vertices, &mut indices).expect("appended");
        assert_eq!(vertices[2].normal, [1.0, 0.0, 0.0]);
        assert_eq!(vertices[2].uv0, [0.25, 0.75]);
        assert_eq!(vertices[2].color, [0.1, 0.2, 0.3, 0.4]);
    }

    #[test]
    fn short_attribute_lists_fall_back_per_vertex() {
        let mut mesh = triangle();
        mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, vec![[0.5, 0.5]]);
        let (mut vertices, mut indices) = (Vec::new(), Vec::new());
        append_mesh(&mesh, &mut vertices, &mut indices);
        assert_eq!(vertices[0].uv0, [0.5, 0.5]);
        assert_eq!(vertices[1].uv0, DEFAULT_UV);
    }

    #[test]
    fn a_second_mesh_lands_after_the_first() {
        let (mut vertices, mut indices) = (Vec::new(), Vec::new());
        append_mesh(&triangle(), &mut vertices, &mut indices);
        let mut small = triangle();
        small.insert_indices(Indices::U16(vec![2, 1]));
        let at = append_mesh(&small, &mut vertices, &mut indices);
        assert_eq!(at, Some((3, 2)));
        assert_eq!(vertices.len(), 6);
        assert_eq!(indices, [0, 1, 2, 5, 4]);
    }

    #[test]
    fn unusable_meshes_add_nothing() {
        let (mut vertices, mut indices) = (Vec::new(), Vec::new());

        let mut out_of_range = triangle();
        out_of_range.insert_indices(Indices::U32(vec![0, 1, 3]));
        assert_eq!(
            append_mesh(&out_of_range, &mut vertices, &mut indices),
            None
        );

        let mut no_indices = triangle();
        no_indices.insert_indices(Indices::U32(Vec::new()));
        assert_eq!(append_mesh(&no_indices, &mut vertices, &mut indices), None);

        let unindexed = Mesh::new(
            PrimitiveTopology::TriangleList,
            RenderAssetUsages::default(),
        );
        assert_eq!(append_mesh(&unindexed, &mut vertices, &mut indices), None);

        assert!(vertices.is_empty() && indices.is_empty());
    }
}
