//! bo2zm: Black Ops II XModel -> ModelMesh. Not part of the upstream engine.
//!
//! A T6 model vertex (`GfxPackedVertex`, 32 bytes; measured on Nuketown's
//! placed models, t6mat probe) is laid out like the world's: position f32x3
//! @0, binormal sign @12, colour R,G,B,A @16, texcoord half2 @20 (u low),
//! normal @24 and tangent @28 packed 10:10:10 signed. Every vertex lies in
//! its model's bounds (model space). The surface keeps each row as BO2 wrote
//! it, and the decoded position, normal, colour and texcoord go into the
//! surface's mesh.
//!
//! Only the first LOD is built. Surfaces have no collision payload yet.

use bevy::asset::RenderAssetUsages;
use bevy::mesh::{Indices, Mesh, PrimitiveTopology};

use asset_t6::{AssetKey, T6_PACKED_VERTEX, XModelRef, half_to_f32, unpack_unit_vec};

use crate::model_draw::{ModelMesh, ModelSurfaceDraw, XSurfaceCollisionPayload};

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

/// Build a T6 model's first LOD. `material` maps each surface's material to
/// the lane's catalog index. `None` when the model has no drawable surface.
pub fn build_t6_model_mesh(
    model: &XModelRef,
    material: impl Fn(AssetKey) -> Option<usize>,
) -> Option<ModelMesh> {
    let mut surfaces = Vec::with_capacity(model.lod0.len());
    let (mut vertices, mut triangles) = (0usize, 0usize);
    for srf in &model.lod0 {
        let n = usize::from(srf.vert_count);
        if n == 0 || srf.indices.len() < 3 || srf.verts.len() < n * T6_PACKED_VERTEX {
            continue;
        }
        if srf.indices.iter().any(|&i| usize::from(i) >= n) {
            continue;
        }
        let mut positions = Vec::with_capacity(n);
        let mut normals = Vec::with_capacity(n);
        let mut colors = Vec::with_capacity(n);
        let mut uvs = Vec::with_capacity(n);
        let mut packed_vertices = Vec::with_capacity(n);
        for v in srf.verts.chunks_exact(T6_PACKED_VERTEX).take(n) {
            let position = [rd_f32(v, 0), rd_f32(v, 4), rd_f32(v, 8)];
            let [r, g, b, a] = [v[16], v[17], v[18], v[19]];
            let tex = rd_u32(v, 20);
            let uv = [half_to_f32(tex as u16), half_to_f32((tex >> 16) as u16)];
            let normal = normalized(unpack_unit_vec(rd_u32(v, 24)));

            positions.push(position);
            normals.push(normal);
            colors.push([r, g, b, a].map(|c| f32::from(c) / 255.0));
            uvs.push(uv);
            packed_vertices.push(<[u8; T6_PACKED_VERTEX]>::try_from(v).expect("chunks_exact row"));
        }
        let indices: Vec<u32> = srf.indices.iter().map(|&i| u32::from(i)).collect();
        let mut mesh = Mesh::new(
            PrimitiveTopology::TriangleList,
            RenderAssetUsages::MAIN_WORLD | RenderAssetUsages::RENDER_WORLD,
        );
        mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
        mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, normals);
        mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, uvs);
        mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, colors);
        vertices += n;
        triangles += indices.len() / 3;
        mesh.insert_indices(Indices::U32(indices));
        surfaces.push(ModelSurfaceDraw {
            mesh,
            material: srf.material.and_then(&material),
            packed_vertices,
            xsurface_plus_1: None,
            xsurface_base_index: 0,
            xsurface_vert_offset: 0,
            collision: XSurfaceCollisionPayload::Unavailable {
                source_layout: "t6",
            },
        });
    }
    if surfaces.is_empty() {
        return None;
    }
    Some(ModelMesh {
        name: model.name.clone(),
        lod_surfaces: [surfaces, Vec::new(), Vec::new(), Vec::new()],
        vertices,
        triangles,
        lod_smc: None,
        lod: None,
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
    mesh: &ModelMesh,
    colors: &[u32],
    unlit: impl Fn(Option<usize>) -> bool,
    foliage: impl Fn(Option<usize>) -> bool,
) -> Option<ModelMesh> {
    let mut lit = mesh.clone();
    let mut at = 0usize;
    for srf in &mut lit.lod_surfaces[0] {
        let n = srf.packed_vertices.len();
        let block = colors.get(at..at + n)?;
        at += n;
        if unlit(srf.material) {
            continue;
        }
        // bo2zm M3 fix list 2: the light takes the colour's red, green and
        // blue; the alpha stays the model's own, which fades its decals
        // (the garage doors' burn layer is see-through but where painted:
        // drawn solid, it covered the doors in black-and-white speckle).
        let own: Vec<[f32; 4]> = match srf.mesh.attribute(Mesh::ATTRIBUTE_COLOR) {
            Some(bevy::mesh::VertexAttributeValues::Float32x4(c)) => c.clone(),
            _ => vec![[1.0; 4]; n],
        };
        let canopy = foliage(srf.material);
        let baked: Vec<[f32; 4]> = block
            .iter()
            .zip(own.iter().chain(std::iter::repeat(&[1.0; 4])))
            .map(|(c, o)| {
                let bytes = c.to_le_bytes();
                let [r, g, b, _] = bytes.map(|b| f32::from(b) / 255.0);
                if canopy {
                    let own = o.map(|v| (v.clamp(0.0, 1.0) * 255.0).round());
                    let mix = |i: usize| own[i] + f32::from(bytes[i]) / 256.0;
                    [mix(0), mix(1), mix(2), 2.0 + own[3] + 256.0 * f32::from(bytes[3])]
                } else {
                    [r, g, b, o[3]]
                }
            })
            .collect();
        srf.mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, baked);
    }
    (at == colors.len()).then_some(lit)
}

