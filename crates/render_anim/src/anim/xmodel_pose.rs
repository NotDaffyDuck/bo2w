use bevy::asset::RenderAssetUsages;
use bevy::math::{Mat4, Vec3};
use bevy::prelude::Mesh;
use bevy::render::mesh::{Indices, PrimitiveTopology};

use dobj_t6::{set_hide_part_bit, surface_hidden};

use crate::anim::pose_types::FpvSkel;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum FpvSurfOwner {
    Hands,
    #[default]
    Gun,

    Scope,

    Rocket,
}

#[derive(Debug)]
pub struct PosedModelSurface {
    pub surface_index: usize,
    pub mesh: Mesh,
    pub material: Option<usize>,
    pub packed_vertices: Vec<[u8; asset_t6::T6_PACKED_VERTEX]>,
    pub owner: FpvSurfOwner,
    pub model: u16,
}

pub struct PosedSmodelSurface {
    pub surface_index: usize,
    pub vert_n: usize,
    pub indices: Vec<u32>,
    pub packed_vertices: Vec<[u8; asset_t6::T6_PACKED_VERTEX]>,
}

pub fn skin_model_filtered(
    skel: &FpvSkel,
    bone_to_local: impl Fn(usize) -> Mat4,
    hide_tags: &[String],
    surface_is_visible: impl Fn(usize) -> bool,
    surface_rigid: impl Fn(usize) -> bool,
    lod: u8,
) -> Option<Vec<PosedModelSurface>> {
    let blended = blend_skel_vertices_inner::<true>(
        skel,
        bone_to_local,
        |surface| surface_is_visible(surface),
        surface_rigid,
        lod,
    );
    meshes_from_blended(skel, hide_tags, surface_is_visible, lod, blended)
}

pub struct BlendedSkel {
    pub decoded: Option<DecodedSkel>,
    pub packed: Option<Vec<[u8; asset_t6::T6_PACKED_VERTEX]>>,
}

pub struct DecodedSkel {
    pub positions: Vec<[f32; 3]>,
    pub normals: Vec<[f32; 3]>,
}

pub fn blend_skel_vertices(
    skel: &FpvSkel,
    bone_to_local: impl Fn(usize) -> Mat4,
    surface_rigid: impl Fn(usize) -> bool,
) -> BlendedSkel {
    blend_skel_vertices_inner::<true>(skel, bone_to_local, |_| true, surface_rigid, 0)
}

pub fn blend_skel_packed_vertices(
    skel: &FpvSkel,
    bone_to_local: impl Fn(usize) -> Mat4,
    surface_is_visible: impl Fn(usize) -> bool,
    surface_rigid: impl Fn(usize) -> bool,
    lod: u8,
) -> BlendedSkel {
    blend_skel_vertices_inner::<false>(skel, bone_to_local, surface_is_visible, surface_rigid, lod)
}

fn blend_skel_vertices_inner<const DECODED: bool>(
    skel: &FpvSkel,
    bone_to_local: impl Fn(usize) -> Mat4,
    surface_is_visible: impl Fn(usize) -> bool,
    surface_rigid: impl Fn(usize) -> bool,
    lod: u8,
) -> BlendedSkel {
    let (active_vert, rigid_vert) =
        vertex_stream_masks(skel, lod, surface_is_visible, surface_rigid);
    let mut positions = DECODED.then(|| skel.positions.clone()).unwrap_or_default();
    let mut normals = DECODED.then(|| skel.normals.clone()).unwrap_or_default();
    let mut posed_packed =
        (skel.packed_vertices.len() == skel.positions.len()).then(|| skel.packed_vertices.clone());
    let max_bone = skel
        .vert_skin
        .iter()
        .zip(&active_vert)
        .filter(|(_, active)| **active)
        .flat_map(|(skin, _)| {
            std::iter::once(skin.bones[0]).chain(
                (1..4)
                    .filter(|&extra| skin.weights[extra] > 0.0)
                    .map(|extra| skin.bones[extra]),
            )
        })
        .max();
    let bone_cache: Vec<_> = max_bone.map_or_else(Vec::new, |max_bone| {
        (0..=usize::from(max_bone))
            .map(|bone| {
                let matrix = bone_to_local(bone);
                (matrix, matrix.to_cols_array())
            })
            .collect()
    });
    for (i, skin) in skel.vert_skin.iter().enumerate() {
        if !active_vert.get(i).copied().unwrap_or(false) {
            continue;
        }
        let p = Vec3::from_array(skel.positions[i]);
        let n = DECODED.then(|| Vec3::from_array(skel.normals[i]));

        let (mut posed_position, mut posed_normal, primary_basis) =
            if rigid_vert.get(i).copied().unwrap_or(false) {
                let matrix = bone_cache[usize::from(skin.bones[0])].0;
                (
                    matrix.transform_point3(p),
                    n.map(|n| matrix.transform_vector3(n)),
                    matrix,
                )
            } else {
                let (primary, primary_esi) = &bone_cache[usize::from(skin.bones[0])];
                let mut extras = [(primary_esi, 0u16); 3];
                let mut extra_n = 0usize;
                for extra in 1..4 {
                    if skin.weights[extra] <= 0.0 {
                        continue;
                    }
                    extras[extra_n] = (
                        &bone_cache[usize::from(skin.bones[extra])].1,
                        skin.weight_u16[extra],
                    );
                    extra_n += 1;
                }
                (
                    Vec3::from_array(dpvs_t6::skin_weighted_point(
                        p.to_array(),
                        primary_esi,
                        &extras[..extra_n],
                    )),
                    n.map(|n| primary.transform_vector3(n)),
                    *primary,
                )
            };
        if posed_position == Vec3::ZERO && skin.weights[0] == 0.0 {
            posed_position = p;
            posed_normal = n;
        }
        if let Some(posed_normal) = posed_normal {
            positions[i] = posed_position.to_array();
            normals[i] = if posed_normal == Vec3::ZERO {
                [0.0, 0.0, 1.0]
            } else {
                posed_normal.normalize_or_zero().to_array()
            };
        }
        if let Some(out) = &mut posed_packed {
            pose_packed_vertex(&mut out[i], posed_position, primary_basis);
        }
    }
    BlendedSkel {
        decoded: DECODED.then_some(DecodedSkel { positions, normals }),
        packed: posed_packed,
    }
}

pub fn lod_local_surface(skel: &FpvSkel, lod: u8, surface_index: usize) -> usize {
    skel.surfaces_for_lod(lod)
        .position(|index| index == surface_index)
        .unwrap_or(usize::MAX)
}

pub fn stream_lod_surface_rigid(
    skel: &FpvSkel,
    lod: u8,
    entries: &[dpvs_t6::SceneEntSkinEntry],
    model: u16,
    surface_index: usize,
) -> bool {
    let lod_local = lod_local_surface(skel, lod, surface_index);
    entries
        .iter()
        .copied()
        .find(|entry| entry.model == model && entry.surface == lod_local as u16)
        .is_some_and(|entry| entry.rigid_placements().is_some())
}

fn vertex_stream_masks(
    skel: &FpvSkel,
    lod: u8,
    surface_is_visible: impl Fn(usize) -> bool,
    surface_rigid: impl Fn(usize) -> bool,
) -> (Vec<bool>, Vec<bool>) {
    if skel.surface_vertex_ranges.is_empty() {
        return (
            vec![true; skel.positions.len()],
            vec![false; skel.positions.len()],
        );
    }
    let mut active = vec![false; skel.positions.len()];
    let mut rigid = vec![false; skel.positions.len()];
    for surface_index in skel.surfaces_for_lod(lod) {
        if !surface_is_visible(surface_index) {
            continue;
        }
        let Some(&(first, count)) = skel.surface_vertex_ranges.get(surface_index) else {
            continue;
        };
        let end = first.saturating_add(count).min(active.len());
        active[first..end].fill(true);
        if surface_rigid(surface_index) {
            rigid[first..end].fill(true);
        }
    }
    (active, rigid)
}

pub(crate) fn meshes_from_blended(
    skel: &FpvSkel,
    hide_tags: &[String],
    surface_is_visible: impl Fn(usize) -> bool,
    lod: u8,
    blended: BlendedSkel,
) -> Option<Vec<PosedModelSurface>> {
    let BlendedSkel {
        decoded,
        packed: posed_packed,
    } = blended;
    let DecodedSkel { positions, normals } = decoded?;
    let surface_count = skel.surface_vertex_ranges.len();
    if surface_count == 0 {
        let mut mesh = Mesh::new(
            PrimitiveTopology::TriangleList,
            RenderAssetUsages::MAIN_WORLD | RenderAssetUsages::RENDER_WORLD,
        );
        mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
        mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, normals);
        mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, skel.uvs.clone());
        mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, skel.colors.clone());
        mesh.insert_indices(Indices::U32(skel.indices.clone()));
        return Some(vec![PosedModelSurface {
            surface_index: 0,
            mesh,
            material: None,
            packed_vertices: posed_packed.unwrap_or_default(),
            owner: FpvSurfOwner::Gun,
            model: 0,
        }]);
    }
    let lod_range = skel.surfaces_for_lod(lod);
    if lod_range.is_empty() {
        return Some(Vec::new());
    }
    if skel.surface_index_ranges.len() != surface_count
        || skel.surface_materials.len() != surface_count
    {
        return None;
    }

    let hide_active = !hide_tags.is_empty() && skel.surface_part_bits.len() == surface_count;
    let mut hide_words = [0u32; 5];
    if hide_active {
        for bone in 0..skel.bone_names.len() {
            if asset_game::bone_has_hidden_ancestor(
                &skel.bone_names,
                |b| skel.parent_of(b),
                bone,
                hide_tags,
            ) {
                set_hide_part_bit(&mut hide_words, bone);
            }
        }
    }
    let mut out = Vec::with_capacity(lod_range.len());
    for surface_index in lod_range {
        let (first_vertex, vertex_count) = skel.surface_vertex_ranges[surface_index];
        let (index_start, index_count) = skel.surface_index_ranges[surface_index];
        let vertex_end = first_vertex.checked_add(vertex_count)?;
        let index_end = index_start.checked_add(index_count)?;
        if vertex_end > positions.len()
            || vertex_end > skel.uvs.len()
            || vertex_end > skel.colors.len()
            || vertex_end > normals.len()
            || index_end > skel.indices.len()
        {
            return None;
        }
        let visible = surface_is_visible(surface_index)
            && (!hide_active
                || !surface_hidden(&skel.surface_part_bits[surface_index], &hide_words, 0));
        let local_indices = if visible {
            skel.indices[index_start..index_end]
                .iter()
                .map(|&global| {
                    let global = global as usize;
                    global
                        .checked_sub(first_vertex)
                        .filter(|local| *local < vertex_count)
                        .map(|local| local as u32)
                })
                .collect::<Option<Vec<_>>>()?
        } else {
            Vec::new()
        };
        let mut mesh = Mesh::new(
            PrimitiveTopology::TriangleList,
            RenderAssetUsages::MAIN_WORLD | RenderAssetUsages::RENDER_WORLD,
        );
        mesh.insert_attribute(
            Mesh::ATTRIBUTE_POSITION,
            positions[first_vertex..vertex_end].to_vec(),
        );
        mesh.insert_attribute(
            Mesh::ATTRIBUTE_NORMAL,
            normals[first_vertex..vertex_end].to_vec(),
        );
        mesh.insert_attribute(
            Mesh::ATTRIBUTE_UV_0,
            skel.uvs[first_vertex..vertex_end].to_vec(),
        );
        mesh.insert_attribute(
            Mesh::ATTRIBUTE_COLOR,
            skel.colors[first_vertex..vertex_end].to_vec(),
        );
        mesh.insert_indices(Indices::U32(local_indices));
        let packed_vertices = posed_packed
            .as_ref()
            .and_then(|vertices| vertices.get(first_vertex..vertex_end))
            .map(<[_]>::to_vec)
            .unwrap_or_default();
        out.push(PosedModelSurface {
            surface_index,
            mesh,
            material: skel.surface_materials[surface_index].map(|i| i.get()),
            packed_vertices,
            owner: FpvSurfOwner::Gun,
            model: 0,
        });
    }
    Some(out)
}

#[derive(Clone, Debug)]
pub struct SkinLayout {
    pub surfaces: Vec<SkinSurfaceLayout>,
    pub dest_vertex_n: usize,
    pub indices: Vec<u32>,
}

#[derive(Clone, Copy, Debug)]
pub struct SkinSurfaceLayout {
    pub surface_index: usize,
    pub src_first_vertex: usize,
    pub dest_first_vertex: usize,
    pub vertex_count: usize,
    pub index_start: u32,
    pub index_count: u32,
    pub visible: bool,
}

pub fn build_skin_layout(
    skel: &FpvSkel,
    lod: u8,
    surface_is_visible: impl Fn(usize) -> bool,
) -> Option<SkinLayout> {
    let surface_count = skel.surface_vertex_ranges.len();
    if surface_count == 0 {
        return Some(SkinLayout {
            surfaces: Vec::new(),
            dest_vertex_n: skel.packed_vertices.len(),
            indices: skel.indices.clone(),
        });
    }
    if skel.surface_index_ranges.len() != surface_count {
        return None;
    }
    let lod_range = skel.surfaces_for_lod(lod);
    let mut surfaces = Vec::with_capacity(lod_range.len());
    let mut indices = Vec::new();
    let mut dest_vertex_n = 0usize;
    for (lod_local, surface_index) in lod_range.enumerate() {
        let (first_vertex, vertex_count) = skel.surface_vertex_ranges[surface_index];
        let (index_start, index_count) = skel.surface_index_ranges[surface_index];
        let vertex_end = first_vertex.checked_add(vertex_count)?;
        let index_end = index_start.checked_add(index_count)?;
        if vertex_end > skel.packed_vertices.len() || index_end > skel.indices.len() {
            return None;
        }
        let visible = surface_is_visible(lod_local);
        if !visible || vertex_count == 0 || index_count == 0 {
            surfaces.push(SkinSurfaceLayout {
                surface_index,
                src_first_vertex: first_vertex,
                dest_first_vertex: 0,
                vertex_count,
                index_start: 0,
                index_count: 0,
                visible: false,
            });
            continue;
        }
        let dest_first = dest_vertex_n;
        let new_start = indices.len() as u32;
        for &global in &skel.indices[index_start..index_end] {
            let local = (global as usize).checked_sub(first_vertex)?;
            if local >= vertex_count {
                return None;
            }
            indices.push(dest_first as u32 + local as u32);
        }
        dest_vertex_n = dest_vertex_n.saturating_add(vertex_count);
        surfaces.push(SkinSurfaceLayout {
            surface_index,
            src_first_vertex: first_vertex,
            dest_first_vertex: dest_first,
            vertex_count,
            index_start: new_start,
            index_count: index_count as u32,
            visible: true,
        });
    }
    Some(SkinLayout {
        surfaces,
        dest_vertex_n,
        indices,
    })
}

pub fn skin_packed_into(
    skel: &FpvSkel,
    bone_to_local: impl Fn(usize) -> Mat4,
    surface_rigid: impl Fn(usize) -> bool,
    lod: u8,
    layout: &SkinLayout,
    dest: &mut [[u8; asset_t6::T6_PACKED_VERTEX]],
) {
    debug_assert_eq!(dest.len(), layout.dest_vertex_n);
    if dest.is_empty() {
        return;
    }
    let (active_vert, rigid_vert) = vertex_stream_masks(skel, lod, |_| true, surface_rigid);
    let max_bone = skel
        .vert_skin
        .iter()
        .zip(&active_vert)
        .filter(|(_, active)| **active)
        .flat_map(|(skin, _)| {
            std::iter::once(skin.bones[0]).chain(
                (1..4)
                    .filter(|&extra| skin.weights[extra] > 0.0)
                    .map(|extra| skin.bones[extra]),
            )
        })
        .max();
    let bone_cache: Vec<_> = max_bone.map_or_else(Vec::new, |max_bone| {
        (0..=usize::from(max_bone))
            .map(|bone| {
                let matrix = bone_to_local(bone);
                (matrix, matrix.to_cols_array())
            })
            .collect()
    });
    for surf in &layout.surfaces {
        if !surf.visible {
            continue;
        }
        for local in 0..surf.vertex_count {
            let src = surf.src_first_vertex + local;
            let dst = surf.dest_first_vertex + local;
            dest[dst] = skel.packed_vertices.get(src).copied().unwrap_or([0; 32]);
            if !active_vert.get(src).copied().unwrap_or(false) {
                continue;
            }
            let Some(skin) = skel.vert_skin.get(src) else {
                continue;
            };
            let p = Vec3::from_array(skel.positions[src]);
            let (posed_position, primary_basis) = if rigid_vert.get(src).copied().unwrap_or(false) {
                let matrix = bone_cache[usize::from(skin.bones[0])].0;
                (matrix.transform_point3(p), matrix)
            } else {
                let (primary, primary_esi) = &bone_cache[usize::from(skin.bones[0])];
                let mut extras = [(primary_esi, 0u16); 3];
                let mut extra_n = 0usize;
                for extra in 1..4 {
                    if skin.weights[extra] <= 0.0 {
                        continue;
                    }
                    extras[extra_n] = (
                        &bone_cache[usize::from(skin.bones[extra])].1,
                        skin.weight_u16[extra],
                    );
                    extra_n += 1;
                }
                (
                    Vec3::from_array(dpvs_t6::skin_weighted_point(
                        p.to_array(),
                        primary_esi,
                        &extras[..extra_n],
                    )),
                    *primary,
                )
            };
            let posed_position = if posed_position == Vec3::ZERO && skin.weights[0] == 0.0 {
                p
            } else {
                posed_position
            };
            pose_packed_vertex(&mut dest[dst], posed_position, primary_basis);
        }
    }
}

pub fn smodel_surfaces_from_blended(
    skel: &FpvSkel,
    blended: BlendedSkel,
    lod: u8,
    surface_is_visible: impl Fn(usize) -> bool,
) -> Option<Vec<PosedSmodelSurface>> {
    let BlendedSkel {
        decoded,
        packed: posed_packed,
    } = blended;
    let vertex_n = posed_packed.as_ref().map_or_else(
        || {
            decoded.as_ref().map_or(0, |decoded| {
                decoded.positions.len().min(decoded.normals.len())
            })
        },
        Vec::len,
    );
    let surface_count = skel.surface_vertex_ranges.len();
    if surface_count == 0 {
        return Some(vec![PosedSmodelSurface {
            surface_index: 0,
            vert_n: vertex_n,
            indices: skel.indices.clone(),
            packed_vertices: posed_packed.unwrap_or_default(),
        }]);
    }
    let lod_range = skel.surfaces_for_lod(lod);
    if lod_range.is_empty() {
        return Some(Vec::new());
    }
    if skel.surface_index_ranges.len() != surface_count
        || skel.surface_materials.len() != surface_count
    {
        return None;
    }

    let mut out = Vec::with_capacity(lod_range.len());
    for (lod_local, surface_index) in lod_range.enumerate() {
        let (first_vertex, vertex_count) = skel.surface_vertex_ranges[surface_index];
        let (index_start, index_count) = skel.surface_index_ranges[surface_index];
        let vertex_end = first_vertex.checked_add(vertex_count)?;
        let index_end = index_start.checked_add(index_count)?;
        if vertex_end > vertex_n
            || vertex_end > skel.uvs.len()
            || vertex_end > skel.colors.len()
            || index_end > skel.indices.len()
        {
            return None;
        }
        let visible = surface_is_visible(lod_local);
        let indices = if visible {
            skel.indices[index_start..index_end]
                .iter()
                .map(|&global| {
                    let global = global as usize;
                    global
                        .checked_sub(first_vertex)
                        .filter(|local| *local < vertex_count)
                        .map(|local| local as u32)
                })
                .collect::<Option<Vec<_>>>()?
        } else {
            Vec::new()
        };
        let packed_vertices = posed_packed
            .as_ref()
            .and_then(|vertices| vertices.get(first_vertex..vertex_end))
            .map(<[_]>::to_vec)
            .unwrap_or_default();
        out.push(PosedSmodelSurface {
            surface_index,
            vert_n: vertex_count,
            indices,
            packed_vertices,
        });
    }
    Some(out)
}

fn pose_packed_vertex(packed: &mut [u8; asset_t6::T6_PACKED_VERTEX], position: Vec3, basis: Mat4) {
    packed[0..12].copy_from_slice(bytemuck::bytes_of(&position.to_array()));
    for offset in [24usize, 28] {
        let source = u32::from_le_bytes([
            packed[offset],
            packed[offset + 1],
            packed[offset + 2],
            packed[offset + 3],
        ]);
        let direction = basis.transform_vector3(Vec3::from_array(asset_t6::unpack_unit_vec(source)));
        packed[offset..offset + 4].copy_from_slice(&asset_t6::pack_unit_vec(direction.to_array()).to_le_bytes());
    }
}
