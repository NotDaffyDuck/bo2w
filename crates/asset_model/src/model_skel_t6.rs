//! bo2zm: Black Ops II models with their skeletons, as the engine's
//! skinned model (`ModelSkel`) — viewmodel guns and arms, world guns.
//!
//! Vertices are BO2's `GfxPackedVertex` (32 bytes): position, binormal
//! sign, colour R,G,B,A, texcoord half2, normal and tangent packed 10:10:10
//! signed. The model keeps each row as BO2 wrote it. Rigid vertex lists and
//! blend weights use the same bone units (64 bytes per bone) as legacy and
//! Black Ops.

use bevy::math::{Quat, Vec3};

use asset_t6::{T6_PACKED_VERTEX, half_to_f32, unpack_unit_vec};
use xmodel_runtime::ModelPoseSrc;

use crate::{BoneBind, ModelSkel, VertSkin, WalkLocalMaterialIndex};

const BONE_STRIDE: u16 = 64;
/// A BO2 vertex's extra bone weight is a u16 share of 65536.
const BLEND_WEIGHT_SCALE: f32 = 1.0 / 65536.0;

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

/// Per-vertex bone influences of one surface: weighted vertices first (by
/// influence count), then the rigid vertex lists. `None` when a bone
/// offset is not a whole bone or names a bone the model lacks.
fn surface_skins(srf: &asset_t6::XSurfaceRef, num_bones: usize) -> Option<Vec<VertSkin>> {
    let n = usize::from(srf.vert_count);
    let mut skins = vec![
        VertSkin {
            bones: [0; 4],
            weights: [1.0, 0.0, 0.0, 0.0],
            ..Default::default()
        };
        n
    ];
    let bone_at = |raw: u16| -> Option<u16> {
        if raw % BONE_STRIDE != 0 {
            return None;
        }
        let bone = raw / BONE_STRIDE;
        ((bone as usize) < num_bones).then_some(bone)
    };
    let mut vertex = 0usize;
    let mut cursor = 0usize;
    for (bucket, &count) in srf.blend_counts.iter().enumerate() {
        let influences = bucket + 1;
        for _ in 0..count.max(0) {
            let words = 1 + (influences - 1) * 2;
            let w = srf.blend.get(cursor..cursor + words)?;
            cursor += words;
            let mut skin = VertSkin {
                bones: [bone_at(w[0])?, 0, 0, 0],
                ..Default::default()
            };
            let mut remaining = 1.0f32;
            for extra in 1..influences {
                let b = w[1 + (extra - 1) * 2];
                let raw = w[2 + (extra - 1) * 2];
                let weight = f32::from(raw) * BLEND_WEIGHT_SCALE;
                skin.bones[extra] = bone_at(b)?;
                skin.weights[extra] = weight;
                skin.weight_u16[extra] = raw;
                remaining -= weight;
            }
            skin.weights[0] = remaining;
            *skins.get_mut(vertex)? = skin;
            vertex += 1;
        }
    }
    for &(bone_offset, run, _, _) in &srf.vert_lists {
        let bone = bone_at(bone_offset)?;
        for _ in 0..run {
            if vertex >= n {
                break;
            }
            skins[vertex] = VertSkin {
                bones: [bone, 0, 0, 0],
                weights: [1.0, 0.0, 0.0, 0.0],
                ..Default::default()
            };
            vertex += 1;
        }
    }
    Some(skins)
}

/// A BO2 model (first LOD) as the engine's skinned model. `material` maps a
/// surface's material to the catalog it was linked into.
pub fn capture_model_skel_t6(
    model: &asset_t6::XModelRef,
    material: impl Fn(asset_t6::AssetKey) -> Option<usize>,
) -> Option<ModelSkel> {
    let num_bones = model.bone_names.len();
    if num_bones == 0 || model.base_mat.len() != num_bones {
        return None;
    }
    let bones: Vec<BoneBind> = model
        .base_mat
        .iter()
        .map(|(q, t)| BoneBind { quat: *q, trans: *t })
        .collect();
    let find = |name: &str| model.bone_names.iter().position(|b| b == name);
    let num_root_bones = usize::from(model.num_root_bones);
    let pose = ModelPoseSrc {
        name: model.name.clone(),
        num_bones,
        num_root_bones,
        scale: 1.0,
        no_scale_part_bits: [0; 5],
        bone_names: model.bone_names.clone(),
        parent_list: model.parent_list.clone(),
        quats: model.quats.clone(),
        trans: model.trans.clone(),
        base_mat: bones
            .iter()
            .map(|b| {
                (
                    Quat::from_xyzw(b.quat[0], b.quat[1], b.quat[2], b.quat[3]),
                    Vec3::from_array(b.trans),
                )
            })
            .collect(),
    };

    let mut positions = Vec::new();
    let mut normals = Vec::new();
    let mut colors = Vec::new();
    let mut uvs = Vec::new();
    let mut indices = Vec::new();
    let mut surface_materials = Vec::with_capacity(model.lod0.len());
    let mut surface_vertex_ranges = Vec::with_capacity(model.lod0.len());
    let mut surface_index_ranges = Vec::with_capacity(model.lod0.len());
    let mut surface_part_bits = Vec::with_capacity(model.lod0.len());
    let mut surface_deformed = Vec::with_capacity(model.lod0.len());
    let mut surface_vert_list_count = Vec::with_capacity(model.lod0.len());
    let mut vert_skin = Vec::new();
    let mut packed_vertices = Vec::new();
    for srf in &model.lod0 {
        let n = usize::from(srf.vert_count);
        if srf.verts.len() < n * T6_PACKED_VERTEX || srf.indices.iter().any(|&i| usize::from(i) >= n) {
            return None;
        }
        let skins = surface_skins(srf, num_bones)?;
        let base = positions.len();
        let index_start = indices.len();
        for (v, skin) in srf.verts.chunks_exact(T6_PACKED_VERTEX).take(n).zip(&skins) {
            positions.push([rd_f32(v, 0), rd_f32(v, 4), rd_f32(v, 8)]);
            normals.push(normalized(unpack_unit_vec(rd_u32(v, 24))));
            colors.push([v[16], v[17], v[18], v[19]].map(|c| f32::from(c) / 255.0));
            let tex = rd_u32(v, 20);
            uvs.push([half_to_f32(tex as u16), half_to_f32((tex >> 16) as u16)]);
            packed_vertices.push(<[u8; T6_PACKED_VERTEX]>::try_from(v).expect("chunks_exact row"));
            vert_skin.push(*skin);
        }
        indices.extend(srf.indices.iter().map(|&i| base as u32 + u32::from(i)));
        surface_vertex_ranges.push((base, n));
        surface_index_ranges.push((index_start, indices.len() - index_start));
        surface_part_bits.push(srf.part_bits);
        surface_deformed.push(Some(srf.flags & 0x80 != 0));
        surface_vert_list_count.push(Some(srf.vert_lists.len() as u32));
        surface_materials.push(
            srf.material
                .and_then(&material)
                .map(WalkLocalMaterialIndex::from_walk),
        );
    }
    let surfaces = surface_vertex_ranges.len();
    Some(ModelSkel {
        name: model.name.clone(),
        bones,
        // bo2zm M3: hit boxes per bone (what bullets hit on a zombie).
        bone_collision: (0..num_bones)
            .map(|b| {
                let (mins, maxs, radius_sq) = (*model.bone_info.get(b)?)?;
                Some(xmodel_runtime::BoneCollision {
                    midpoint: std::array::from_fn(|k| (mins[k] + maxs[k]) * 0.5),
                    half_size: std::array::from_fn(|k| (maxs[k] - mins[k]) * 0.5),
                    radius_sq,
                    part_classification: model.part_classification.get(b).copied().unwrap_or(0),
                })
            })
            .collect(),
        bone_names: model.bone_names.clone(),
        tag_view: find("tag_view"),
        tag_weapon: find("tag_weapon"),
        pose: Some(pose),
        positions,
        normals,
        colors,
        uvs,
        indices,
        surface_materials,
        surface_vertex_ranges,
        surface_index_ranges,
        surface_part_bits,
        surface_deformed,
        surface_vert_list_count,
        vert_skin,
        packed_vertices,
        radius: Some(model.radius),
        // (mid, half), the convention every model skeleton uses.
        bounds: Some((
            std::array::from_fn(|k| (model.mins[k] + model.maxs[k]) * 0.5),
            std::array::from_fn(|k| (model.maxs[k] - model.mins[k]) * 0.5),
        )),
        contents: Some(model.contents),
        // bo2mp: the collision surfaces a bullet hits (BO2's DObjGeomTraceline
        // runs XModelTraceLineAnimated on them). Without them a destructible
        // car stopped bullets on rough bone boxes. T6 keeps each surface's
        // mins/maxs where legacy keeps (mid, half).
        coll_lod: model.coll_lod,
        coll_surfs: model
            .coll_surfs
            .iter()
            .filter_map(|s| {
                Some(xmodel_runtime::CollSurfCollision {
                    bone: u16::try_from(s.bone_idx).ok()?,
                    contents: s.contents,
                    surf_flags: s.surf_flags,
                    midpoint: std::array::from_fn(|k| (s.mins[k] + s.maxs[k]) * 0.5),
                    half_size: std::array::from_fn(|k| ((s.maxs[k] - s.mins[k]) * 0.5).max(0.0)),
                    tris: s
                        .tris
                        .iter()
                        .map(|t| xmodel_runtime::CollTri {
                            plane: t[0],
                            svec: t[1],
                            tvec: t[2],
                        })
                        .collect(),
                })
            })
            .collect(),
        collmap: t6_collmap(model),
        lod: Some(crate::ModelLodSelector::T5 {
            num_lods: 1,
            lod_dist: [0.0; 4],
        }),
        lod_smc: None,
        lod_surf_span: [(0, surfaces as u16), (0, 0), (0, 0), (0, 0)],
    })
}

/// bo2mp: the model's physics collmap as boxes. A brush piece is its bounds
/// (the collision_* script models are all side-less boxes; a brush with
/// extra sides, like collision_clip_ramp, keeps only its bounds); any other
/// piece is a box of its own axes, middle and half-lengths (a stand-in for
/// a cylinder or capsule).
fn t6_collmap(model: &asset_t6::XModelRef) -> Option<xmodel_runtime::ModelCollmap> {
    if model.collmap.is_empty() && model.collmap_contents == 0 {
        return None;
    }
    let boxes = model
        .collmap
        .iter()
        .map(|g| match g.brush {
            Some((mins, maxs, _)) if g.ty == 2 => xmodel_runtime::CollmapBox {
                center: std::array::from_fn(|k| (mins[k] + maxs[k]) * 0.5),
                axis: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
                half: std::array::from_fn(|k| (maxs[k] - mins[k]) * 0.5),
            },
            _ => xmodel_runtime::CollmapBox {
                center: g.offset,
                axis: g.orientation,
                half: g.half_lengths,
            },
        })
        .collect();
    Some(xmodel_runtime::ModelCollmap {
        contents: model.contents | model.collmap_contents,
        boxes,
        brush_verts: model.collmap_brush_verts.clone(),
    })
}
