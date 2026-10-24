//! CPU skinning of a model skeleton: the posed meshes a draw is built from,
//! and the compact packed-vertex layout the first-person rig re-skins each frame.

use bevy::asset::RenderAssetUsages;
use bevy::math::{Mat4, Vec3};
use bevy::prelude::Mesh;
use bevy::render::mesh::{Indices, PrimitiveTopology};

use asset_model::VertSkin;
use dobj_t6::{set_hide_part_bit, surface_hidden};

use crate::anim::pose_types::FpvSkel;

type PackedVertex = [u8; asset_t6::T6_PACKED_VERTEX];

/// Which first-person part a surface belongs to.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum FpvSurfOwner {
    Hands,
    #[default]
    Gun,
    Scope,
    Rocket,
}

/// One skinned surface: its mesh, the material slot it draws with, and the
/// posed packed vertices (empty when the model has none).
#[derive(Debug)]
pub struct PosedModelSurface {
    pub surface_index: usize,
    pub mesh: Mesh,
    pub material: Option<usize>,
    pub packed_vertices: Vec<PackedVertex>,
    pub owner: FpvSurfOwner,
    pub model: u16,
}

// ---------------------------------------------------------------------------
// Vertex streams
// ---------------------------------------------------------------------------

/// Which vertices a pose touches, and which of those follow a single bone.
struct VertexMasks {
    active: Vec<bool>,
    rigid: Vec<bool>,
}

fn vertex_masks(
    skel: &FpvSkel,
    lod: u8,
    surface_is_visible: impl Fn(usize) -> bool,
    surface_rigid: impl Fn(usize) -> bool,
) -> VertexMasks {
    let total = skel.positions.len();
    if skel.surface_vertex_ranges.is_empty() {
        return VertexMasks {
            active: vec![true; total],
            rigid: vec![false; total],
        };
    }
    let mut masks = VertexMasks {
        active: vec![false; total],
        rigid: vec![false; total],
    };
    for surface in skel.surfaces_for_lod(lod) {
        if !surface_is_visible(surface) {
            continue;
        }
        let Some(&(first, count)) = skel.surface_vertex_ranges.get(surface) else {
            continue;
        };
        let span = first..first.saturating_add(count).min(total);
        masks.active[span.clone()].fill(true);
        if surface_rigid(surface) {
            masks.rigid[span].fill(true);
        }
    }
    masks
}

/// The bones that move a vertex: the primary one, then each blended extra.
fn driving_bones(skin: &VertSkin) -> impl Iterator<Item = u16> + '_ {
    std::iter::once(skin.bones[0]).chain(
        (1..4)
            .filter(|&slot| skin.weights[slot] > 0.0)
            .map(|slot| skin.bones[slot]),
    )
}

/// Each bone's matrix, both as a `Mat4` and as the flat array the weighted
/// point skinner reads. Indexed by bone number up to the highest one an
/// active vertex uses.
struct BoneMatrices(Vec<(Mat4, [f32; 16])>);

impl BoneMatrices {
    fn for_active(
        skel: &FpvSkel,
        active: &[bool],
        bone_to_local: impl Fn(usize) -> Mat4,
    ) -> Self {
        let highest = skel
            .vert_skin
            .iter()
            .zip(active)
            .filter(|(_, on)| **on)
            .flat_map(|(skin, _)| driving_bones(skin))
            .max();
        Self(highest.map_or_else(Vec::new, |highest| {
            (0..=usize::from(highest))
                .map(|bone| {
                    let matrix = bone_to_local(bone);
                    (matrix, matrix.to_cols_array())
                })
                .collect()
        }))
    }

    fn get(&self, bone: u16) -> &(Mat4, [f32; 16]) {
        &self.0[usize::from(bone)]
    }
}

/// A vertex after posing. `normal` is `None` when the caller gave no rest
/// normal; `basis` is the matrix its packed tangent frame turns by.
struct PosedVertex {
    position: Vec3,
    normal: Option<Vec3>,
    basis: Mat4,
}

fn pose_vertex(
    bones: &BoneMatrices,
    skin: &VertSkin,
    rest: Vec3,
    rest_normal: Option<Vec3>,
    rigid: bool,
) -> PosedVertex {
    let (primary, primary_flat) = bones.get(skin.bones[0]);
    let position = if rigid {
        primary.transform_point3(rest)
    } else {
        let mut extras = [(primary_flat, 0u16); 3];
        let mut used = 0;
        for slot in (1..4).filter(|&slot| skin.weights[slot] > 0.0) {
            extras[used] = (&bones.get(skin.bones[slot]).1, skin.weight_u16[slot]);
            used += 1;
        }
        Vec3::from_array(dpvs_t6::skin_weighted_point(
            rest.to_array(),
            primary_flat,
            &extras[..used],
        ))
    };
    // A vertex that lands on the origin with no primary weight was never
    // skinned: it keeps its rest pose.
    if position == Vec3::ZERO && skin.weights[0] == 0.0 {
        return PosedVertex {
            position: rest,
            normal: rest_normal,
            basis: *primary,
        };
    }
    PosedVertex {
        position,
        normal: rest_normal.map(|n| primary.transform_vector3(n)),
        basis: *primary,
    }
}

/// Writes a posed position and re-orients the two packed unit vectors
/// (normal and tangent, at bytes 24 and 28) by `basis`.
fn write_packed_pose(packed: &mut PackedVertex, position: Vec3, basis: Mat4) {
    packed[0..12].copy_from_slice(bytemuck::bytes_of(&position.to_array()));
    for offset in [24usize, 28] {
        let word = u32::from_le_bytes([
            packed[offset],
            packed[offset + 1],
            packed[offset + 2],
            packed[offset + 3],
        ]);
        let turned = basis.transform_vector3(Vec3::from_array(asset_t6::unpack_unit_vec(word)));
        packed[offset..offset + 4]
            .copy_from_slice(&asset_t6::pack_unit_vec(turned.to_array()).to_le_bytes());
    }
}

/// Position and normal streams, plus packed rows when the model carries one
/// per vertex.
struct PosedStreams {
    positions: Vec<[f32; 3]>,
    normals: Vec<[f32; 3]>,
    packed: Option<Vec<PackedVertex>>,
}

/// Poses every active vertex of `skel`; the rest keep their rest values.
fn pose_streams(
    skel: &FpvSkel,
    masks: &VertexMasks,
    bone_to_local: impl Fn(usize) -> Mat4,
) -> PosedStreams {
    let bones = BoneMatrices::for_active(skel, &masks.active, bone_to_local);
    let mut streams = PosedStreams {
        positions: skel.positions.clone(),
        normals: skel.normals.clone(),
        packed: (skel.packed_vertices.len() == skel.positions.len())
            .then(|| skel.packed_vertices.clone()),
    };
    for (i, skin) in skel.vert_skin.iter().enumerate() {
        if !masks.active.get(i).copied().unwrap_or(false) {
            continue;
        }
        let rest = Vec3::from_array(skel.positions[i]);
        let rest_normal = Vec3::from_array(skel.normals[i]);
        let rigid = masks.rigid.get(i).copied().unwrap_or(false);
        let posed = pose_vertex(&bones, skin, rest, Some(rest_normal), rigid);
        streams.positions[i] = posed.position.to_array();
        streams.normals[i] = match posed.normal.unwrap_or(rest_normal) {
            n if n == Vec3::ZERO => [0.0, 0.0, 1.0],
            n => n.normalize_or_zero().to_array(),
        };
        if let Some(packed) = &mut streams.packed {
            write_packed_pose(&mut packed[i], posed.position, posed.basis);
        }
    }
    streams
}

// ---------------------------------------------------------------------------
// Posed meshes
// ---------------------------------------------------------------------------

/// Skins `skel` at `lod` and cuts it into one [`PosedModelSurface`] per
/// surface of that LOD. Surfaces `surface_is_visible` rejects, or whose bones
/// sit under a hidden tag in `hide_tags`, keep their vertices but draw no
/// triangles. `None` when the skeleton's streams are inconsistent.
pub fn skin_model_filtered(
    skel: &FpvSkel,
    bone_to_local: impl Fn(usize) -> Mat4,
    hide_tags: &[String],
    surface_is_visible: impl Fn(usize) -> bool,
    surface_rigid: impl Fn(usize) -> bool,
    lod: u8,
) -> Option<Vec<PosedModelSurface>> {
    let masks = vertex_masks(skel, lod, &surface_is_visible, surface_rigid);
    let streams = pose_streams(skel, &masks, bone_to_local);
    cut_surfaces(skel, hide_tags, surface_is_visible, lod, streams)
}

fn surface_mesh(
    positions: &[[f32; 3]],
    normals: &[[f32; 3]],
    uvs: &[[f32; 2]],
    colors: &[[f32; 4]],
    indices: Vec<u32>,
) -> Mesh {
    let mut mesh = Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::MAIN_WORLD | RenderAssetUsages::RENDER_WORLD,
    );
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions.to_vec());
    mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, normals.to_vec());
    mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, uvs.to_vec());
    mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, colors.to_vec());
    mesh.insert_indices(Indices::U32(indices));
    mesh
}

/// The bone-part bits of every bone that sits under one of `hide_tags`.
fn hidden_part_bits(skel: &FpvSkel, hide_tags: &[String]) -> [u32; 5] {
    let mut bits = [0u32; 5];
    for bone in 0..skel.bone_names.len() {
        if asset_game::bone_has_hidden_ancestor(
            &skel.bone_names,
            |b| skel.parent_of(b),
            bone,
            hide_tags,
        ) {
            set_hide_part_bit(&mut bits, bone);
        }
    }
    bits
}

/// Rebases a surface's triangle indices to its own first vertex. `None` if
/// one points outside the surface.
fn surface_local_indices(
    global: &[u32],
    first_vertex: usize,
    vertex_count: usize,
) -> Option<Vec<u32>> {
    global
        .iter()
        .map(|&index| {
            (index as usize)
                .checked_sub(first_vertex)
                .filter(|local| *local < vertex_count)
                .and_then(|local| u32::try_from(local).ok())
        })
        .collect()
}

fn cut_surfaces(
    skel: &FpvSkel,
    hide_tags: &[String],
    surface_is_visible: impl Fn(usize) -> bool,
    lod: u8,
    streams: PosedStreams,
) -> Option<Vec<PosedModelSurface>> {
    let PosedStreams {
        positions,
        normals,
        packed,
    } = streams;
    let surface_count = skel.surface_vertex_ranges.len();
    if surface_count == 0 {
        return Some(vec![PosedModelSurface {
            surface_index: 0,
            mesh: surface_mesh(
                &positions,
                &normals,
                &skel.uvs,
                &skel.colors,
                skel.indices.clone(),
            ),
            material: None,
            packed_vertices: packed.unwrap_or_default(),
            owner: FpvSurfOwner::Gun,
            model: 0,
        }]);
    }
    let surfaces = skel.surfaces_for_lod(lod);
    if surfaces.is_empty() {
        return Some(Vec::new());
    }
    if skel.surface_index_ranges.len() != surface_count
        || skel.surface_materials.len() != surface_count
    {
        return None;
    }

    let hidden = (!hide_tags.is_empty() && skel.surface_part_bits.len() == surface_count)
        .then(|| hidden_part_bits(skel, hide_tags));
    let mut out = Vec::with_capacity(surfaces.len());
    for surface in surfaces {
        let (first_vertex, vertex_count) = skel.surface_vertex_ranges[surface];
        let (index_start, index_count) = skel.surface_index_ranges[surface];
        let vertex_end = first_vertex.checked_add(vertex_count)?;
        let index_end = index_start.checked_add(index_count)?;
        let streams_cover = [
            positions.len(),
            normals.len(),
            skel.uvs.len(),
            skel.colors.len(),
        ]
        .into_iter()
        .all(|len| vertex_end <= len);
        if !streams_cover || index_end > skel.indices.len() {
            return None;
        }
        let draws = surface_is_visible(surface)
            && hidden
                .as_ref()
                .is_none_or(|bits| !surface_hidden(&skel.surface_part_bits[surface], bits, 0));
        let indices = if draws {
            surface_local_indices(
                &skel.indices[index_start..index_end],
                first_vertex,
                vertex_count,
            )?
        } else {
            Vec::new()
        };
        let rows = first_vertex..vertex_end;
        out.push(PosedModelSurface {
            surface_index: surface,
            mesh: surface_mesh(
                &positions[rows.clone()],
                &normals[rows.clone()],
                &skel.uvs[rows.clone()],
                &skel.colors[rows.clone()],
                indices,
            ),
            material: skel.surface_materials[surface].map(|slot| slot.get()),
            packed_vertices: packed
                .as_ref()
                .and_then(|vertices| vertices.get(rows))
                .map(<[_]>::to_vec)
                .unwrap_or_default(),
            owner: FpvSurfOwner::Gun,
            model: 0,
        });
    }
    Some(out)
}

/// `surface_index`'s place among the surfaces of `lod`; `usize::MAX` when that
/// LOD does not draw it.
pub fn lod_local_surface(skel: &FpvSkel, lod: u8, surface_index: usize) -> usize {
    skel.surfaces_for_lod(lod)
        .position(|index| index == surface_index)
        .unwrap_or(usize::MAX)
}

/// Whether the scene-skin stream placed `surface_index` of `model` as rigid.
pub fn stream_lod_surface_rigid(
    skel: &FpvSkel,
    lod: u8,
    entries: &[dpvs_t6::SceneEntSkinEntry],
    model: u16,
    surface_index: usize,
) -> bool {
    let local = lod_local_surface(skel, lod, surface_index);
    entries
        .iter()
        .find(|entry| entry.model == model && usize::from(entry.surface) == local)
        .is_some_and(|entry| entry.rigid_placements().is_some())
}

// ---------------------------------------------------------------------------
// Packed re-skinning
// ---------------------------------------------------------------------------

/// Where each visible surface's vertices land in one compact destination
/// buffer, and the triangle list over that buffer.
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

/// Lays out the surfaces of `lod`; `surface_is_visible` is asked by position
/// within the LOD. Empty or hidden surfaces take no room. `None` when the
/// skeleton's ranges or indices are inconsistent.
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
    let mut layout = SkinLayout {
        surfaces: Vec::new(),
        dest_vertex_n: 0,
        indices: Vec::new(),
    };
    for (lod_local, surface_index) in skel.surfaces_for_lod(lod).enumerate() {
        let (first_vertex, vertex_count) = skel.surface_vertex_ranges[surface_index];
        let (index_start, index_count) = skel.surface_index_ranges[surface_index];
        let vertex_end = first_vertex.checked_add(vertex_count)?;
        let index_end = index_start.checked_add(index_count)?;
        if vertex_end > skel.packed_vertices.len() || index_end > skel.indices.len() {
            return None;
        }
        if !surface_is_visible(lod_local) || vertex_count == 0 || index_count == 0 {
            layout.surfaces.push(SkinSurfaceLayout {
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
        let dest_first = layout.dest_vertex_n;
        let index_start_in_dest = layout.indices.len() as u32;
        for &global in &skel.indices[index_start..index_end] {
            let local = (global as usize).checked_sub(first_vertex)?;
            if local >= vertex_count {
                return None;
            }
            layout.indices.push((dest_first + local) as u32);
        }
        layout.dest_vertex_n += vertex_count;
        layout.surfaces.push(SkinSurfaceLayout {
            surface_index,
            src_first_vertex: first_vertex,
            dest_first_vertex: dest_first,
            vertex_count,
            index_start: index_start_in_dest,
            index_count: index_count as u32,
            visible: true,
        });
    }
    Some(layout)
}

/// Re-skins the packed rows of every visible surface of `layout` into `dest`
/// (which must hold `layout.dest_vertex_n` rows).
pub fn skin_packed_into(
    skel: &FpvSkel,
    bone_to_local: impl Fn(usize) -> Mat4,
    surface_rigid: impl Fn(usize) -> bool,
    lod: u8,
    layout: &SkinLayout,
    dest: &mut [PackedVertex],
) {
    debug_assert_eq!(dest.len(), layout.dest_vertex_n);
    if dest.is_empty() {
        return;
    }
    let masks = vertex_masks(skel, lod, |_| true, surface_rigid);
    let bones = BoneMatrices::for_active(skel, &masks.active, bone_to_local);
    for surface in layout.surfaces.iter().filter(|surface| surface.visible) {
        for offset in 0..surface.vertex_count {
            let src = surface.src_first_vertex + offset;
            let row = &mut dest[surface.dest_first_vertex + offset];
            *row = skel.packed_vertices.get(src).copied().unwrap_or([0; 32]);
            if !masks.active.get(src).copied().unwrap_or(false) {
                continue;
            }
            let Some(skin) = skel.vert_skin.get(src) else {
                continue;
            };
            let rest = Vec3::from_array(skel.positions[src]);
            let rigid = masks.rigid.get(src).copied().unwrap_or(false);
            let posed = pose_vertex(&bones, skin, rest, None, rigid);
            write_packed_pose(row, posed.position, posed.basis);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::anim::test_skel::blank_skel;
    use bevy::render::mesh::VertexAttributeValues;

    /// `surfaces` one-triangle surfaces, every vertex fully weighted to bone 0.
    fn triangles(surfaces: usize) -> FpvSkel {
        let mut skel = blank_skel("tri");
        for s in 0..surfaces {
            let first = s * 3;
            skel.positions
                .extend([[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]]);
            skel.normals.extend([[0.0, 0.0, 1.0]; 3]);
            skel.uvs.extend([[0.0, 0.0]; 3]);
            skel.colors.extend([[1.0; 4]; 3]);
            skel.packed_vertices.extend([[0u8; 32]; 3]);
            skel.indices.extend((first..first + 3).map(|i| i as u32));
            skel.surface_vertex_ranges.push((first, 3));
            skel.surface_index_ranges.push((first, 3));
            skel.surface_materials.push(None);
            skel.surface_part_bits.push([0; 5]);
        }
        skel.vert_skin = vec![
            VertSkin {
                weights: [1.0, 0.0, 0.0, 0.0],
                ..VertSkin::default()
            };
            surfaces * 3
        ];
        skel
    }

    fn positions_of(mesh: &Mesh) -> Vec<[f32; 3]> {
        match mesh.attribute(Mesh::ATTRIBUTE_POSITION) {
            Some(VertexAttributeValues::Float32x3(values)) => values.clone(),
            other => panic!("no position stream: {other:?}"),
        }
    }

    fn index_count(mesh: &Mesh) -> usize {
        mesh.indices().map_or(0, |indices| indices.len())
    }

    fn lift(bone: usize) -> Mat4 {
        assert_eq!(bone, 0);
        Mat4::from_translation(Vec3::new(1.0, 2.0, 3.0))
    }

    #[test]
    fn a_surface_sits_at_its_lod_position() {
        let skel = triangles(3);
        assert_eq!(lod_local_surface(&skel, 0, 2), 2);
        assert_eq!(lod_local_surface(&skel, 0, 9), usize::MAX);
        assert_eq!(lod_local_surface(&skel, 1, 0), usize::MAX);
    }

    #[test]
    fn rigid_surfaces_move_with_their_bone() {
        let skel = triangles(1);
        let posed =
            skin_model_filtered(&skel, lift, &[], |_| true, |_| true, 0).expect("consistent");
        assert_eq!(posed.len(), 1);
        assert_eq!(
            positions_of(&posed[0].mesh),
            [[1.0, 2.0, 3.0], [2.0, 2.0, 3.0], [1.0, 3.0, 3.0]]
        );
        assert_eq!(index_count(&posed[0].mesh), 3);
        assert_eq!(posed[0].surface_index, 0);
    }

    #[test]
    fn a_hidden_surface_keeps_its_vertices_but_draws_nothing() {
        let skel = triangles(2);
        let posed = skin_model_filtered(&skel, lift, &[], |s| s == 1, |_| true, 0).unwrap();
        assert_eq!(index_count(&posed[0].mesh), 0);
        assert_eq!(index_count(&posed[1].mesh), 3);
        assert_eq!(positions_of(&posed[0].mesh).len(), 3);
    }

    #[test]
    fn a_vertex_with_no_primary_weight_that_lands_on_the_origin_stays_at_rest() {
        let mut skel = triangles(1);
        skel.vert_skin[1].weights[0] = 0.0;
        skel.positions[1] = [5.0, 0.0, 0.0];
        let posed =
            skin_model_filtered(&skel, |_| Mat4::ZERO, &[], |_| true, |_| true, 0).unwrap();
        assert_eq!(positions_of(&posed[0].mesh)[1], [5.0, 0.0, 0.0]);
    }

    #[test]
    fn a_zeroed_normal_falls_back_to_up() {
        let skel = triangles(1);
        let posed =
            skin_model_filtered(&skel, |_| Mat4::ZERO, &[], |_| true, |_| true, 0).unwrap();
        match posed[0].mesh.attribute(Mesh::ATTRIBUTE_NORMAL) {
            Some(VertexAttributeValues::Float32x3(normals)) => {
                assert!(normals.iter().all(|n| *n == [0.0, 0.0, 1.0]));
            }
            other => panic!("no normal stream: {other:?}"),
        }
    }

    #[test]
    fn a_skeleton_with_mismatched_ranges_is_refused() {
        let mut skel = triangles(2);
        skel.surface_index_ranges.pop();
        assert!(skin_model_filtered(&skel, lift, &[], |_| true, |_| true, 0).is_none());
        assert!(build_skin_layout(&skel, 0, |_| true).is_none());
    }

    #[test]
    fn the_layout_packs_only_the_visible_surfaces() {
        let skel = triangles(2);
        let layout = build_skin_layout(&skel, 0, |local| local == 1).unwrap();
        assert_eq!(layout.dest_vertex_n, 3);
        assert_eq!(layout.indices, [0, 1, 2]);
        assert!(!layout.surfaces[0].visible);
        let kept = layout.surfaces[1];
        assert!(kept.visible);
        assert_eq!((kept.src_first_vertex, kept.dest_first_vertex), (3, 0));
        assert_eq!((kept.index_start, kept.index_count), (0, 3));
    }

    #[test]
    fn an_index_that_leaves_its_surface_spoils_the_layout() {
        let mut skel = triangles(1);
        skel.indices[2] = 7;
        assert!(build_skin_layout(&skel, 0, |_| true).is_none());
    }

    #[test]
    fn re_skinning_packed_rows_moves_their_positions() {
        let skel = triangles(1);
        let layout = build_skin_layout(&skel, 0, |_| true).unwrap();
        let mut dest = vec![[0u8; 32]; layout.dest_vertex_n];
        skin_packed_into(&skel, lift, |_| true, 0, &layout, &mut dest);
        assert_eq!(dest[1][0..12], *bytemuck::bytes_of(&[2.0f32, 2.0, 3.0]));
    }

    #[test]
    fn a_stream_entry_marks_a_surface_rigid_by_its_lod_position() {
        let skel = triangles(2);
        let entry = |surface: u16, tag: i32| dpvs_t6::SceneEntSkinEntry {
            model: 4,
            surface,
            tag,
            bone_base: 0,
            bone_count: 0,
            bytes: 0,
        };
        let rigid = [entry(1, dpvs_t6::SKIN_SURFACE_SKIP_TAG - 1)];
        assert!(stream_lod_surface_rigid(&skel, 0, &rigid, 4, 1));
        assert!(!stream_lod_surface_rigid(&skel, 0, &rigid, 4, 0));
        assert!(!stream_lod_surface_rigid(&skel, 0, &rigid, 5, 1));
    }
}
