//! bo2zm: Black Ops II XModel -> StaticMeshData. Not part of the upstream engine.
//!
//! A T6 model vertex (`GfxPackedVertex`, 32 bytes; measured on Nuketown's
//! placed models, t6mat probe) is laid out like the world's: position f32x3
//! @0, binormal sign @12, colour R,G,B,A @16, uv_semantic half2 @20 (u low),
//! normal @24 and tangent @28 packed 10:10:10 signed. Every vertex lies in
//! its model's bounds (model space). The surface keeps each row as BO2 wrote
//! it, and the decoded position, normal, colour and uv_semantic go into the
//! surface's mesh.
//!
//! Only the first LOD is built. Surfaces have no collision payload yet.

use bevy::asset::RenderAssetUsages;
use bevy::mesh::{Indices, Mesh, PrimitiveTopology};

use asset_t6::{AssetIdentity, T6_PACKED_VERTEX, XModelRef, short_float_to_full, decode_unit_normal};

use crate::prop_meshes::{StaticMeshData, MeshFaceDrawCall, XSurfaceCollisionPayload};

fn rd_u32(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]])
}

fn rd_f32(b: &[u8], at: usize) -> f32 {
    f32::from_bits(rd_u32(b, at))
}

fn normalized(v: [f32; 3]) -> [f32; 3] {
    let l = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    if l > 1e-6 {
        [v[0] / l, v[1] / l, v[2] / l]
    } else {
        [0.0, 0.0, 1.0]
    }
}

/// How many vertices `srf` has when it can be drawn: it holds all its rows
/// and at least one triangle's worth of indices, none past the rows.
fn drawable_rows(srf: &asset_t6::XSurfaceRef) -> Option<usize> {
    let rows = usize::from(srf.vertex_tally);
    let sound = rows > 0
        && srf.indices.len() >= 3
        && srf.vertex_points.len() >= rows * T6_PACKED_VERTEX
        && srf.indices.iter().all(|&i| usize::from(i) < rows);
    sound.then_some(rows)
}

/// One decoded model vertex.
struct ModelVertex {
    position: [f32; 3],
    normal: [f32; 3],
    color: [f32; 4],
    uv: [f32; 2],
}

fn decode_row(row: &[u8; T6_PACKED_VERTEX]) -> ModelVertex {
    let tex = rd_u32(row, 20);
    ModelVertex {
        position: [0, 4, 8].map(|at| rd_f32(row, at)),
        normal: normalized(decode_unit_normal(rd_u32(row, 24))),
        color: [row[16], row[17], row[18], row[19]].map(|c| f32::from(c) / 255.0),
        uv: [tex as u16, (tex >> 16) as u16].map(short_float_to_full),
    }
}

/// The drawable form of `srf`, whose first `rows` rows are its vertices.
fn surface_draw(srf: &asset_t6::XSurfaceRef, row_count: usize, material: Option<usize>) -> MeshFaceDrawCall {
    let compact_vertices: Vec<[u8; T6_PACKED_VERTEX]> = srf
        .vertex_points
        .chunks_exact(T6_PACKED_VERTEX)
        .take(row_count)
        .map(|row| row.try_into().expect("chunks_exact row"))
        .collect();
    let decoded: Vec<ModelVertex> = compact_vertices.iter().map(decode_row).collect();
    let column = |pick: fn(&ModelVertex) -> [f32; 4]| -> Vec<[f32; 4]> { decoded.iter().map(pick).collect() };
    let mesh = Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::MAIN_WORLD | RenderAssetUsages::RENDER_WORLD,
    )
    .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, decoded.iter().map(|v| v.position).collect::<Vec<_>>())
    .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, decoded.iter().map(|v| v.normal).collect::<Vec<_>>())
    .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, decoded.iter().map(|v| v.uv).collect::<Vec<_>>())
    .with_inserted_attribute(Mesh::ATTRIBUTE_COLOR, column(|v| v.color))
    .with_inserted_indices(Indices::U32(srf.indices.iter().copied().map(u32::from).collect()));
    MeshFaceDrawCall {
        face_mesh: mesh,
        mat_slot: material,
        compact_vertices,
        figure_part_plus_1: Default::default(),
        figure_part_base_index: Default::default(),
        figure_part_vertex_offset: Default::default(),
        collision: XSurfaceCollisionPayload::Unavailable { origin_layout: "t6" },
    }
}

/// Build a T6 model's first LOD. `material` maps each surface's material to
/// the lane's catalog index. `None` when the model has no drawable surface.
pub fn build_t6_model_mesh(
    model: &XModelRef,
    material: impl Fn(AssetIdentity) -> Option<usize>,
) -> Option<StaticMeshData> {
    let surfaces: Vec<MeshFaceDrawCall> = model
        .lod0
        .iter()
        .filter_map(|srf| {
            let rows = drawable_rows(srf)?;
            Some(surface_draw(srf, rows, srf.material.and_then(&material)))
        })
        .collect();
    let vertices = surfaces.iter().map(|s| s.compact_vertices.len()).sum();
    let triangles = surfaces.iter().map(|s| s.face_mesh.indices().map_or(0, |i| i.len() / 3)).sum();
    (!surfaces.is_empty()).then(|| StaticMeshData {
        name: model.name.to_owned(),
        detail_facets: [surfaces, vec![], vec![], vec![]],
        vertex_total: vertices,
        triangle_total: triangles,
        detail_prop_cache: Default::default(),
        lod: Default::default(),
    })
}

/// A copy of `mesh` whose vertex colours hold an instance's baked vertex
/// light (`lmapVertexColors`, R,G,B,A bytes, one per first-LOD vertex in
/// surface order); the fallback draw lights such an instance with
/// `colour^2 * 32`, as the game's `mlv_*` shaders do. `None` when the counts
/// disagree. Surfaces whose material `unlit` names keep their own colour:
/// an unlit technique reads no baked light, and the map's compiler stored
/// black for them (Nuketown's moving truck: its multiply logo drew black).
/// bo2mp: surfaces whose material `foliage` names (tree canopies) keep the
/// light's alpha too, the sun's visibility `mlv_treecanopy` reads: their
/// alpha is 2 + the model's own alpha byte + 256 * that byte, and each of
/// red, green and blue the model's own byte + the light's byte / 256 (the
/// own colour picks the wind and how far the leaf sways), which the
/// canopy's vertex shader takes apart.
pub fn t6_vertex_lit_mesh(
    mesh: &StaticMeshData,
    colors: &[u32],
    unlit: impl Fn(Option<usize>) -> bool,
    foliage: impl Fn(Option<usize>) -> bool,
) -> Option<StaticMeshData> {
    let mut lit = mesh.clone();
    let mut cursor = usize::default();
    for srf in &mut lit.detail_facets[0] {
        let stop = cursor + srf.compact_vertices.len();
        let block = colors.get(cursor..stop)?;
        cursor = stop;
        if unlit(srf.mat_slot) {
            continue;
        }
        // bo2zm M3 fix list 2: the light takes the colour's red, green and
        // blue; the alpha stays the model's own, which fades its decals
        // (the garage doors' burn layer is see-through but where painted:
        // drawn solid, it covered the doors in black-and-white speckle).
        let canopy = foliage(srf.mat_slot);
        let own = own_vertex_colors(&srf.face_mesh);
        let baked: Vec<[f32; 4]> = block
            .iter()
            .enumerate()
            .map(|(k, &word)| bake_vertex(word, own.get(k).copied().unwrap_or([1.0; 4]), canopy))
            .collect();
        srf.face_mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, baked);
    }
    (cursor == colors.len()).then_some(lit)
}

/// The colours a surface's mesh carries already (none when it has no colour set).
fn own_vertex_colors(mesh: &Mesh) -> Vec<[f32; 4]> {
    match mesh.attribute(Mesh::ATTRIBUTE_COLOR) {
        Some(bevy::mesh::VertexAttributeValues::Float32x4(c)) => c.clone(),
        _ => Vec::new(),
    }
}

/// One vertex's colour from its baked-light word and its own colour: plain
/// surfaces take the light's RGB with their own alpha; a canopy packs its own
/// bytes plus the light's into all four channels.
fn bake_vertex(word: u32, own: [f32; 4], canopy: bool) -> [f32; 4] {
    let bytes = word.to_le_bytes();
    if !canopy {
        let [r, g, b, _] = bytes.map(|v| f32::from(v) / 255.0);
        return [r, g, b, own[3]];
    }
    let own = own.map(|v| (v.clamp(0.0, 1.0) * 255.0).round());
    let channel = |i: usize| own[i] + f32::from(bytes[i]) / 256.0;
    [channel(0), channel(1), channel(2), 2.0 + own[3] + 256.0 * f32::from(bytes[3])]
}

#[cfg(test)]
mod bake_tests {
    use super::*;

    #[test]
    fn plain_vertex_takes_light_rgb_and_own_alpha() {
        let v = bake_vertex(0x8040_2010, [0.2, 0.3, 0.4, 0.5], false);
        assert_eq!(v, [16.0 / 255.0, 32.0 / 255.0, 64.0 / 255.0, 0.5]);
    }

    #[test]
    fn canopy_vertex_packs_own_bytes_and_light() {
        let v = bake_vertex(0x8040_2010, [0.2, 0.4, 0.6, 0.5], true);
        assert_eq!(v, [51.0 + 16.0 / 256.0, 102.0 + 32.0 / 256.0, 153.0 + 64.0 / 256.0, 2.0 + 128.0 + 256.0 * 128.0]);
        assert_eq!(bake_vertex(0, [2.0, -1.0, 0.0, 9.0], true)[..3], [255.0, 0.0, 0.0]);
    }
}

#[cfg(test)]
mod build_tests {
    use super::*;
    use bevy::mesh::VertexAttributeValues;

    fn row(at: [f32; 3], rgba: [u8; 4]) -> [u8; T6_PACKED_VERTEX] {
        let mut cells = [0u8; T6_PACKED_VERTEX];
        for (k, c) in at.iter().enumerate() {
            cells[k * 4..k * 4 + 4].copy_from_slice(&c.to_le_bytes());
        }
        cells[16..20].copy_from_slice(&rgba);
        cells[20..24].copy_from_slice(&0x3800_3c00u32.to_le_bytes());
        cells[24..28].copy_from_slice(&(0x1ffu32 << 20).to_le_bytes());
        cells
    }

    fn surface(vertex_points: usize, indices: &[u16]) -> asset_t6::XSurfaceRef {
        let rows: Vec<u8> = (0..vertex_points).flat_map(|k| row([k as f32, 2.0, 3.0], [255, 0, 51, 255])).collect();
        asset_t6::XSurfaceRef {
            vertex_points: rows,
            vertex_tally: vertex_points as u16,
            indices: indices.to_vec(),
            ..Default::default()
        }
    }

    fn model(lod0: Vec<asset_t6::XSurfaceRef>) -> XModelRef {
        XModelRef {
            name: "crate".into(),
            numsurfs: lod0.len() as u8,
            materials: vec![],
            skeleton_joint_total: 0,
            detail_tier_total: 1,
            lo_corner: [0.0; 3],
            hi_corner: [0.0; 3],
            radius: Default::default(),
            lighting_origin_offset: [0.0; 3],
            lighting_origin_range: 0.0,
            lod0,
            joint_labels: vec![],
            top_joint_total: 0,
            ancestor_chain: vec![],
            rotation_quaternions: vec![],
            shift_offset: vec![],
            foundation_surface: vec![],
            body_part_category: vec![],
            bone_info: vec![],
            detail_tiers: vec![],
            contents: Default::default(),
            collision_detail_tier: -1,
            collision_faces: vec![],
            collmap_brushes: vec![],
            physics_template: Default::default(),
            collmap: vec![],
            collmap_contents: 0,
            collmap_brush_verts: vec![],
        }
    }

    #[test]
    fn only_drawable_surfaces_are_built() {
        let drawn = surface(3, &[0, 1, 2, 2, 1, 0]);
        let past_the_end = surface(3, &[0, 1, 3]);
        let mut short = surface(3, &[0, 1, 2]);
        short.vertex_points.truncate(64);
        let none = surface(0, &[0, 1, 2]);
        let built = build_t6_model_mesh(&model(vec![none, past_the_end, drawn, short]), |_| Some(1)).unwrap();
        assert_eq!((built.name.as_str(), built.vertex_total, built.triangle_total), ("crate", 3, 2));
        assert_eq!(built.facet_tally(), 1);
        let only = &built.surfaces()[0];
        assert_eq!(only.mat_slot, None);
        assert_eq!(only.compact_vertices.len(), 3);
        assert_eq!(only.compact_vertices[1], row([1.0, 2.0, 3.0], [255, 0, 51, 255]));
        assert_eq!((only.figure_part_plus_1, only.figure_part_base_index, only.figure_part_vertex_offset), (None, 0, 0));
        assert!(built.detail_prop_cache.is_none() && built.lod.is_none());
        assert!(built.detail_facets[1..].iter().all(|l| l.iter().count() == 0));
        match only.face_mesh.attribute(Mesh::ATTRIBUTE_POSITION) {
            Some(VertexAttributeValues::Float32x3(p)) => assert_eq!(p, &vec![[0.0, 2.0, 3.0], [1.0, 2.0, 3.0], [2.0, 2.0, 3.0]]),
            other => panic!("{other:?}"),
        }
        match only.face_mesh.attribute(Mesh::ATTRIBUTE_NORMAL) {
            Some(VertexAttributeValues::Float32x3(n)) => assert_eq!(n[0], [0.0, 0.0, 1.0]),
            other => panic!("{other:?}"),
        }
        match only.face_mesh.attribute(Mesh::ATTRIBUTE_COLOR) {
            Some(VertexAttributeValues::Float32x4(c)) => assert_eq!(c[2], [1.0, 0.0, 0.2, 1.0]),
            other => panic!("{other:?}"),
        }
        match only.face_mesh.attribute(Mesh::ATTRIBUTE_UV_0) {
            Some(VertexAttributeValues::Float32x2(uv)) => assert_eq!(uv[0], [1.0, 0.5]),
            other => panic!("{other:?}"),
        }
        match only.face_mesh.indices() {
            Some(Indices::U32(i)) => assert_eq!(i, &vec![0, 1, 2, 2, 1, 0]),
            other => panic!("{other:?}"),
        }
        assert!(build_t6_model_mesh(&model(vec![surface(3, &[0, 1])]), |_| None).is_none());
    }

    #[test]
    fn baked_light_is_shared_out_in_surface_order() {
        let two = build_t6_model_mesh(&model(vec![surface(2, &[0, 1, 0]), surface(1, &[0, 0, 0])]), |_| None).unwrap();
        assert!(t6_vertex_lit_mesh(&two, &[0; 2], |_| false, |_| false).is_none());
        assert!(t6_vertex_lit_mesh(&two, &[0; 4], |_| false, |_| false).is_none());
        let word = 0x8040_2010u32;
        let lit = t6_vertex_lit_mesh(&two, &[word; 3], |_| false, |_| false).unwrap();
        let colors = |m: &StaticMeshData, k: usize| match m.detail_facets[0][k].face_mesh.attribute(Mesh::ATTRIBUTE_COLOR) {
            Some(VertexAttributeValues::Float32x4(c)) => c.clone(),
            other => panic!("{other:?}"),
        };
        assert_eq!(colors(&lit, 1), vec![[16.0 / 255.0, 32.0 / 255.0, 64.0 / 255.0, 1.0]]);
        // An unlit material keeps its own colours.
        let kept = t6_vertex_lit_mesh(&two, &[word; 3], |_| true, |_| false).unwrap();
        assert_eq!(colors(&kept, 0), colors(&two, 0));
        assert_eq!(colors(&kept, 1), colors(&two, 1));
    }
}
