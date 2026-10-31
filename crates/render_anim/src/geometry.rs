//! Moves posed mesh data into the flat buffers that a draw plan keeps.

use asset_t6::T6_PACKED_VERTEX;
use asset_world::PackedVertexPayload;
use bevy::mesh::{Indices, VertexAttributeValues};
use bevy::prelude::*;
use render_frame::PropVertex;

/// What a vertex gets for each attribute its mesh does not carry.
const WHITE: [f32; 4] = [1.0; 4];
const UP: [f32; 3] = [0.0, 0.0, 1.0];
const ORIGIN_UV: [f32; 2] = [0.0, 0.0];

/// Chooses the packed-vertex payload that a plan holds on to once rows were
/// gathered for it. Rows are kept only if they stayed valid, exist, and match
/// the `decoded_total` vertices one for one. Otherwise the plan names the
/// reason it has none: `empty` when it has no vertices at all, `missing` when
/// it has vertices but nothing usable to go with them.
pub fn install_held_compact(
    compact_ok: bool,
    packed_in: Vec<[u8; T6_PACKED_VERTEX]>,
    decoded_total: usize,
    blank: &'static str,
    absent_label: &'static str,
) -> PackedVertexPayload {
    let matches_one_for_one = packed_in.len() == decoded_total && decoded_total > 0;
    if compact_ok && matches_one_for_one {
        return PackedVertexPayload::T6(packed_in);
    }
    let origin_layout = if decoded_total == 0 { blank } else { absent_label };
    PackedVertexPayload::Unavailable { origin_layout }
}

macro_rules! float_attribute {
    ($getter:ident, $variant:ident, $width:literal) => {
        pub(crate) fn $getter(values: &VertexAttributeValues) -> Option<&[[f32; $width]]> {
            match values {
                VertexAttributeValues::$variant(rows) => Some(rows),
                _ => None,
            }
        }
    };
}

float_attribute!(f32x2, Float32x2, 2);
float_attribute!(f32x3, Float32x3, 3);
float_attribute!(f32x4, Float32x4, 4);

/// An index list with every index widened to 32 bits.
fn widened(list: &Indices) -> Vec<u32> {
    match list {
        Indices::U32(items) => items.clone(),
        Indices::U16(items) => items.iter().copied().map(u32::from).collect(),
    }
}

/// Row `at` of an optional attribute list, or `fallback` when the mesh has no
/// such list or it is too short.
fn row_or<T: Copy>(list: Option<&[T]>, at: usize, fallback: T) -> T {
    list.and_then(|rows| rows.get(at))
        .copied()
        .unwrap_or(fallback)
}

/// The vertices of `mesh`: its positions, plus an up normal, the origin uv and
/// opaque white for each attribute that the mesh lacks.
pub(crate) fn mesh_vertices(mesh: &Mesh) -> Option<Vec<PropVertex>> {
    let pos_list = mesh.attribute(Mesh::ATTRIBUTE_POSITION).and_then(f32x3)?;
    let colours = mesh.attribute(Mesh::ATTRIBUTE_COLOR).and_then(f32x4);
    let texcoord_set = mesh.attribute(Mesh::ATTRIBUTE_UV_0).and_then(f32x2);
    let surface_directions = mesh.attribute(Mesh::ATTRIBUTE_NORMAL).and_then(f32x3);
    let mut vertices = Vec::with_capacity(pos_list.len());
    for (at, &position) in pos_list.iter().enumerate() {
        vertices.push(PropVertex {
            first_texcoord: row_or(texcoord_set, at, ORIGIN_UV),
            color: row_or(colours, at, WHITE),
            normal: row_or(surface_directions, at, UP),
            position,
        });
    }
    Some(vertices)
}

/// How many indices `mesh` lists, if it is indexed.
pub(crate) fn mesh_index_count(mesh: &Mesh) -> Option<usize> {
    Some(mesh.indices()?.len())
}

/// Adds `mesh` to the flat vertex and index buffers and returns where its
/// indices ended up, as `(first index, index count)`.
///
/// A mesh needs positions and an index list to count. One with no vertices, no
/// indices, or an index beyond its last vertex adds nothing.
pub fn append_mesh(
    mesh_in: &Mesh,
    verts: &mut Vec<PropVertex>,
    index_list: &mut Vec<u32>,
) -> Option<(u32, u32)> {
    let incoming = mesh_vertices(mesh_in)?;
    let listed = widened(mesh_in.indices()?);
    let last_vertex = incoming.len().checked_sub(1)?;
    if listed.is_empty() || listed.iter().any(|&index| index as usize > last_vertex) {
        return None;
    }

    let head = index_list.len() as u32;
    let base = verts.len() as u32;
    verts.extend(incoming);
    index_list.extend(listed.iter().map(|&index| index + base));
    Some((head, listed.len() as u32))
}

mod tests {
    #![cfg(test)]
    use super::*;
    use bevy::asset::RenderAssetUsages;
    use bevy::mesh::PrimitiveTopology;

    const ROW: [u8; T6_PACKED_VERTEX] = [7; T6_PACKED_VERTEX];

    fn triangle() -> Mesh {
        let topology = PrimitiveTopology::TriangleList;
        let mut mesh_data = Mesh::new(
            topology,
            RenderAssetUsages::default(),
        );
        let pos_attr = Mesh::ATTRIBUTE_POSITION;
        mesh_data.insert_attribute(
            pos_attr,
            vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
        );
        mesh_data.insert_indices(Indices::U32(vec![0, 1, 2]));
        mesh_data
    }

    fn payload_layout(payload_in: &PackedVertexPayload) -> Option<&'static str> {
        match payload_in {
            PackedVertexPayload::Unavailable { origin_layout } => Some(origin_layout),
            PackedVertexPayload::T6(_) => None,
        }
    }

    #[test]
    fn matching_rows_are_kept() {
        let kept = install_held_compact(true, vec![ROW; 3], 3, "blank", "absent");
        assert!(matches!(kept, PackedVertexPayload::T6(rows) if rows.len() == 3));
    }

    #[test]
    fn rows_are_dropped_when_invalidated_or_miscounted() {
        let invalid = install_held_compact(false, vec![ROW; 3], 3, "blank", "absent");
        assert_eq!(payload_layout(&invalid), Some("absent"));
        let short = install_held_compact(true, vec![ROW; 2], 3, "blank", "absent");
        assert_eq!(payload_layout(&short), Some("absent"));
        let none = install_held_compact(true, vec![], 3, "blank", "absent");
        assert_eq!(payload_layout(&none), Some("absent"));
    }

    #[test]
    fn a_plan_without_vertices_reports_empty() {
        let none = install_held_compact(true, vec![], 0, "blank", "absent");
        assert_eq!(payload_layout(&none), Some("blank"));
        let stray = install_held_compact(true, vec![ROW; 2], 0, "blank", "absent");
        assert_eq!(payload_layout(&stray), Some("blank"));
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
        let (mut vertices, mut indices) = (vec![], vec![]);
        let at = append_mesh(&triangle(), &mut vertices, &mut indices);
        assert_eq!(at, Some((0, 3)));
        assert_eq!(indices, [0, 1, 2]);
        assert_eq!(vertices.len(), 3);
        let second = &vertices[1];
        assert_eq!(second.position, [1.0, 0.0, 0.0]);
        assert_eq!(
            (second.normal, second.first_texcoord, second.color),
            (UP, ORIGIN_UV, WHITE)
        );
    }

    #[test]
    fn authored_attributes_are_copied() {
        let mut mesh_val = triangle();
        mesh_val.insert_attribute(Mesh::ATTRIBUTE_NORMAL, vec![[1.0, 0.0, 0.0]; 3]);
        mesh_val.insert_attribute(Mesh::ATTRIBUTE_UV_0, vec![[0.25, 0.75]; 3]);
        mesh_val.insert_attribute(Mesh::ATTRIBUTE_COLOR, vec![[0.1, 0.2, 0.3, 0.4]; 3]);
        let (mut vertices, mut indices) = (vec![], vec![]);
        append_mesh(&mesh_val, &mut vertices, &mut indices).expect("appended");
        let last = &vertices[2];
        assert_eq!(last.normal, [1.0, 0.0, 0.0]);
        assert_eq!(last.first_texcoord, [0.25, 0.75]);
        assert_eq!(last.color, [0.1, 0.2, 0.3, 0.4]);
    }

    #[test]
    fn short_attribute_lists_fall_back_per_vertex() {
        let mut mesh_v = triangle();
        mesh_v.insert_attribute(Mesh::ATTRIBUTE_UV_0, vec![[0.5, 0.5]]);
        let (mut vertices, mut indices) = (vec![], vec![]);
        append_mesh(&mesh_v, &mut vertices, &mut indices);
        assert_eq!(vertices[0].first_texcoord, [0.5, 0.5]);
        assert_eq!(vertices[1].first_texcoord, ORIGIN_UV);
    }

    #[test]
    fn a_second_mesh_lands_after_the_first() {
        let (mut vertices, mut indices) = (vec![], vec![]);
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
        let (mut vertices, mut indices) = (vec![], vec![]);

        let mut out_of_range = triangle();
        out_of_range.insert_indices(Indices::U32(vec![0, 1, 3]));
        assert_eq!(
            append_mesh(&out_of_range, &mut vertices, &mut indices),
            None
        );

        let mut no_indices = triangle();
        no_indices.insert_indices(Indices::U32(Vec::new()));
        assert_eq!(append_mesh(&no_indices, &mut vertices, &mut indices), None);

        let topology = PrimitiveTopology::TriangleList;
        let unindexed = Mesh::new(
            topology,
            RenderAssetUsages::default(),
        );
        assert_eq!(append_mesh(&unindexed, &mut vertices, &mut indices), None);

        assert!(vertices.is_empty() && indices.is_empty());
    }
}
