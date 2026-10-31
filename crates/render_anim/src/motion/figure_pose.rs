//! CPU skinning of a model skeleton: the posed meshes a draw is built from,
//! and the compact packed-vertex layout the first-person rig re-skins each frame.

use std::ops::Range;

use bevy::asset::RenderAssetUsages;
use bevy::math::{Mat4, Vec3};
use bevy::prelude::Mesh;
use bevy::render::mesh::{Indices, PrimitiveTopology};

use asset_model::CornerWeights;
use dobj_t6::{hide_part, any_part_hidden};

use crate::motion::pose_kinds::FpvSkel;

/// A model without a surface table is drawn as one undivided mesh.
fn is_single_mesh(bone_rig: &FpvSkel) -> bool {
    bone_rig.facet_vertex_ranges.is_empty()
}

/// A surface of the gun model itself (model slot 0).
fn gun_surface(
    surface_slot_number: usize,
    mesh: Mesh,
    mat: Option<usize>,
    compact_vertices: Vec<[u8; asset_t6::T6_PACKED_VERTEX]>,
) -> SkinnedMeshFacet {
    SkinnedMeshFacet {
        surface_slot_number,
        mesh,
        shader_ix: mat,
        compact_vertices,
        owner: SurfaceDrawOwner::Gun,
        mdl: 0,
    }
}

/// Which first-person part a surface belongs to.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SurfaceDrawOwner {
    Hands,
    #[default]
    Gun,
    Scope,
    Rocket,
}

/// One skinned surface: its mesh, the material slot it draws with, and the
/// posed packed vertices (empty when the model has none).
#[derive(Debug)]
pub struct SkinnedMeshFacet {
    pub surface_slot_number: usize,
    pub mesh: Mesh,
    pub shader_ix: Option<usize>,
    pub compact_vertices: Vec<[u8; asset_t6::T6_PACKED_VERTEX]>,
    pub owner: SurfaceDrawOwner,
    pub mdl: u16,
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
    bone_rig: &FpvSkel,
    lod: u8,
    surf_is_visible: impl Fn(usize) -> bool,
    surface_rigid_in: impl Fn(usize) -> bool,
) -> VertexMasks {
    let total = bone_rig.positions.len();
    let untouched = vec![false; total];
    if is_single_mesh(bone_rig) {
        return VertexMasks {
            active: vec![true; total],
            rigid: untouched,
        };
    }
    let mut masks = VertexMasks {
        active: untouched.clone(),
        rigid: untouched,
    };
    for surface in bone_rig.facets_at_tier(lod) {
        if !surf_is_visible(surface) {
            continue;
        }
        let Some(&(from, len)) = bone_rig.facet_vertex_ranges.get(surface) else {
            continue;
        };
        let span = from..from.saturating_add(len).min(total);
        let rigid = surface_rigid_in(surface);
        for row in span {
            masks.active[row] = true;
            masks.rigid[row] |= rigid;
        }
    }
    masks
}

/// The bones that move a vertex: the primary one, then each blended extra.
fn driving_bones(skin: &CornerWeights) -> impl Iterator<Item = u16> + '_ {
    let blended = (1..4).filter(|&slot| skin.weights[slot] > 0.0);
    std::iter::once(skin.joints[0]).chain(blended.map(|slot| skin.joints[slot]))
}

/// Each bone's matrix, both as a `Mat4` and as the flat array the weighted
/// point skinner reads. Indexed by bone number up to the highest one an
/// active vertex uses.
struct BoneMatrices(Vec<(Mat4, [f32; 16])>);

impl BoneMatrices {
    fn for_active(bone_rig: &FpvSkel, active: &[bool], bone_to_local: impl Fn(usize) -> Mat4) -> Self {
        let mut top: Option<u16> = None;
        for (skin, _) in bone_rig.vert_skin.iter().zip(active).filter(|(_, on)| **on) {
            top = top.max(driving_bones(skin).max());
        }
        let count = top.map_or(0, |top| usize::from(top) + 1);
        let table = (0..count)
            .map(bone_to_local)
            .map(|matrix| (matrix, matrix.to_cols_array()))
            .collect::<_>();
        Self(table)
    }

    fn bone_entry(&self, bone: u16) -> &(Mat4, [f32; 16]) {
        &self.0[usize::from(bone)]
    }
}

/// A vertex after posing. `normal` is `None` when the caller gave no rest
/// normal; `basis` is the matrix its packed tangent frame turns by.
struct PosedVertex {
    position: Vec3,
    nrm: Option<Vec3>,
    frame_basis: Mat4,
}

fn pose_vertex(
    bones: &BoneMatrices,
    skin: &CornerWeights,
    rest: Vec3,
    rest_normal: Option<Vec3>,
    rigid: bool,
) -> PosedVertex {
    let (main, primary_flat) = bones.bone_entry(skin.joints[0]);
    let position = if rigid {
        main.transform_point3(rest)
    } else {
        let mut extras = [(primary_flat, 0u16); 3];
        let mut used = 0;
        for slot in (1..4).filter(|&slot| skin.weights[slot] > 0.0) {
            extras[used] = (&bones.bone_entry(skin.joints[slot]).1, skin.pull_u16[slot]);
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
            nrm: rest_normal,
            frame_basis: *main,
        };
    }
    PosedVertex {
        position,
        nrm: rest_normal.map(|n| main.transform_vector3(n)),
        frame_basis: *main,
    }
}

/// Writes a posed position and re-orients the two packed unit vectors
/// (normal and tangent, at bytes 24 and 28) by `basis`.
fn write_packed_pose(packed: &mut [u8; asset_t6::T6_PACKED_VERTEX], pos: Vec3, basis: Mat4) {
    packed[..12].copy_from_slice(bytemuck::bytes_of(&pos.to_array()));
    for at in [24usize, 28] {
        let word_bytes: [u8; 4] = packed[at..at + 4].try_into().expect("four bytes");
        let unit = Vec3::from_array(asset_t6::decode_unit_normal(u32::from_le_bytes(word_bytes)));
        let turned = basis.transform_vector3(unit);
        let word = asset_t6::pack_unit_vec(turned.to_array());
        packed[at..at + 4].copy_from_slice(&word.to_le_bytes());
    }
}

/// Position and normal streams, plus packed rows when the model carries one
/// per vertex.
struct PosedStreams {
    pos_rows: Vec<[f32; 3]>,
    surface_directions: Vec<[f32; 3]>,
    packed: Option<Vec<[u8; asset_t6::T6_PACKED_VERTEX]>>,
}

/// Poses every active vertex of `bone_rig`; the rest keep their rest values.
fn pose_streams(
    bone_rig: &FpvSkel,
    masks: &VertexMasks,
    bone_to_local_in: impl Fn(usize) -> Mat4,
) -> PosedStreams {
    let bones = BoneMatrices::for_active(bone_rig, &masks.active, bone_to_local_in);
    let mut streams = PosedStreams {
        pos_rows: bone_rig.positions.clone(),
        surface_directions: bone_rig.surface_directions.clone(),
        packed: (bone_rig.compact_vertices.len() == bone_rig.positions.len())
            .then(|| bone_rig.compact_vertices.clone()),
    };
    let live = bone_rig
        .vert_skin
        .iter()
        .zip(&masks.active)
        .enumerate()
        .filter(|(_k, (_, on))| **on);
    for (row, (skin, _)) in live {
        let [rest, rest_normal] = [bone_rig.positions[row], bone_rig.surface_directions[row]].map(Vec3::from_array);
        let rigid = masks.rigid.get(row) == Some(&true);
        let PosedVertex {
            position,
            nrm: normal,
            frame_basis: basis,
        } = pose_vertex(&bones, skin, rest, Some(rest_normal), rigid);
        let normal = normal.unwrap_or(rest_normal);
        streams.surface_directions[row] = match normal == Vec3::ZERO {
            true => [0.0, 0.0, 1.0],
            false => normal.normalize_or_zero().into(),
        };
        streams.pos_rows[row] = position.into();
        if let Some(rows) = streams.packed.as_mut() {
            write_packed_pose(&mut rows[row], position, basis);
        }
    }
    streams
}

// ---------------------------------------------------------------------------
// Posed meshes
// ---------------------------------------------------------------------------

/// Skins `bone_rig` at `lod` and cuts it into one [`SkinnedMeshSurface`] per
/// surface of that LOD. Surfaces `surface_is_visible` rejects, or whose bones
/// sit under a hidden tag in `concealed_bones`, keep their vertices but draw no
/// triangles. `None` when the skeleton's streams are inconsistent.
pub fn skin_chosen_surfaces(
    bone_rig: &FpvSkel,
    bone_ix_to_local: impl Fn(usize) -> Mat4,
    concealed_bones: &[String],
    surface_is_shown: impl Fn(usize) -> bool,
    surf_rigid: impl Fn(usize) -> bool,
    lod: u8,
) -> Option<Vec<SkinnedMeshFacet>> {
    let masks = vertex_masks(bone_rig, lod, &surface_is_shown, surf_rigid);
    let streams = pose_streams(bone_rig, &masks, bone_ix_to_local);
    cut_surfaces(bone_rig, concealed_bones, surface_is_shown, lod, streams)
}

fn surface_mesh(
    pos_list: &[[f32; 3]],
    surface_directions: &[[f32; 3]],
    texcoord_set: &[[f32; 2]],
    colors_in: &[[f32; 4]],
    index_list: Vec<u32>,
) -> Mesh {
    let topology = PrimitiveTopology::TriangleList;
    let mut mesh_data = Mesh::new(
        topology,
        RenderAssetUsages::default(),
    );
    mesh_data.insert_attribute(Mesh::ATTRIBUTE_POSITION, pos_list.to_vec());
    mesh_data.insert_attribute(Mesh::ATTRIBUTE_NORMAL, surface_directions.to_vec());
    mesh_data.insert_attribute(Mesh::ATTRIBUTE_UV_0, texcoord_set.to_vec());
    mesh_data.insert_attribute(Mesh::ATTRIBUTE_COLOR, colors_in.to_vec());
    mesh_data.with_inserted_indices(Indices::U32(index_list))
}

/// The bone-part bits of every bone that sits under one of `concealed_bones`.
fn hidden_part_bits(bone_rig: &FpvSkel, concealed_bones: &[String]) -> [u32; 5] {
    let mut mask = [0u32; 5];
    let under_hidden_tag = |bone: usize| {
        asset_game::joint_under_concealed_parent(
            &bone_rig.joint_labels,
            |b| bone_rig.hanging_from(b),
            bone,
            concealed_bones,
        )
    };
    for bone in (0..bone_rig.joint_labels.len()).filter(|&bone| under_hidden_tag(bone)) {
        hide_part(&mut mask, bone);
    }
    mask
}

/// Rebases a surface's triangle indices to its own first vertex. `None` if
/// one points outside the surface.
fn surface_local_indices(
    global: &[u32],
    first_vertex: usize,
    vert_count: usize,
) -> Option<Vec<u32>> {
    let rebase = |&index: &u32| {
        let local = (index as usize).checked_sub(first_vertex)?;
        if local >= vert_count {
            return None;
        }
        u32::try_from(local).ok()
    };
    global.iter().map(rebase).collect()
}

/// The vertex rows and the index rows `surface` covers. `None` when a range overflows.
fn surface_spans(bone_rig: &FpvSkel, surf: usize) -> Option<(Range<usize>, Range<usize>)> {
    let (v_from, v_len) = bone_rig.facet_vertex_ranges[surf];
    let (i_from, i_len) = bone_rig.facet_index_ranges[surf];
    Some((
        v_from..v_from.checked_add(v_len)?,
        i_from..i_from.checked_add(i_len)?,
    ))
}

fn cut_surfaces(
    bone_rig: &FpvSkel,
    concealed_bones: &[String],
    surface_is_visible_v: impl Fn(usize) -> bool,
    lod: u8,
    streams: PosedStreams,
) -> Option<Vec<SkinnedMeshFacet>> {
    let PosedStreams {
        pos_rows: vert_pts,
        surface_directions,
        packed,
    } = streams;
    if is_single_mesh(bone_rig) {
        let whole = surface_mesh(
            &vert_pts,
            &surface_directions,
            &bone_rig.texcoord_set,
            &bone_rig.colors,
            bone_rig.indices.clone(),
        );
        let only = gun_surface(0, whole, None, packed.unwrap_or_default());
        return Some(vec![only]);
    }
    let in_lod = bone_rig.facets_at_tier(lod);
    if in_lod.len() < 1 {
        return Some(vec![]);
    }
    let table = bone_rig.facet_vertex_ranges.len();
    let tables = [
        bone_rig.facet_index_ranges.len(),
        bone_rig.facet_materials.len(),
    ];
    if tables.iter().any(|&len| len != table) {
        return None;
    }

    let hidden_bits = if !concealed_bones.is_empty() && bone_rig.face_parts.len() == table {
        Some(hidden_part_bits(bone_rig, concealed_bones))
    } else {
        None
    };
    let stream_lens = [
        bone_rig.colors.len(),
        bone_rig.texcoord_set.len(),
        surface_directions.len(),
        vert_pts.len(),
    ];
    let mut cut = Vec::with_capacity(in_lod.len());
    for surface in in_lod {
        let (rows, index_rows) = surface_spans(bone_rig, surface)?;
        if stream_lens.iter().any(|&len| rows.end > len) || index_rows.end > bone_rig.indices.len() {
            return None;
        }
        let part_hidden = hidden_bits
            .as_ref()
            .is_some_and(|bits| any_part_hidden(&bone_rig.face_parts[surface], bits, 0));
        let triangles = if surface_is_visible_v(surface) && !part_hidden {
            surface_local_indices(&bone_rig.indices[index_rows], rows.start, rows.len())?
        } else {
            vec![]
        };
        let compact_vertices = packed
            .as_ref()
            .and_then(|all| all.get(rows.start..rows.end))
            .map(<[_]>::to_vec).unwrap_or_default();
        let mesh = surface_mesh(
            &vert_pts[rows.clone()],
            &surface_directions[rows.clone()],
            &bone_rig.texcoord_set[rows.clone()],
            &bone_rig.colors[rows],
            triangles,
        );
        let material = bone_rig.facet_materials[surface].map(|slot| slot.get());
        cut.push(gun_surface(surface, mesh, material, compact_vertices));
    }
    Some(cut)
}

/// `surface_slot_number`'s place among the surfaces of `lod`; `usize::MAX` when that
/// LOD does not draw it.
pub fn lod_surf_slot(bone_rig: &FpvSkel, lod: u8, surface_slot_number: usize) -> usize {
    bone_rig.facets_at_tier(lod)
        .position(|candidate| candidate == surface_slot_number)
        .unwrap_or(!0)
}

/// Whether the scene-skin stream placed `surface_slot_number` of `model` as rigid.
pub fn rigid_lod_surface(
    bone_rig: &FpvSkel,
    lod: u8,
    entries: &[dpvs_t6::EntitySkinSlotRecord],
    mdl: u16,
    surface_slot_number: usize,
) -> bool {
    let local = lod_surf_slot(bone_rig, lod, surface_slot_number);
    let placed = entries
        .iter()
        .find(|entry| entry.mdl == mdl && usize::from(entry.surface_no) == local);
    placed.is_some_and(|entry| entry.fixed_mesh_instance_total().is_some())
}

// ---------------------------------------------------------------------------
// Packed re-skinning
// ---------------------------------------------------------------------------

/// Where each visible surface's vertices land in one compact destination
/// buffer, and the triangle list over that buffer.
#[derive(Clone, Debug)]
pub struct VertexSkinPlan {
    pub surfs: Vec<SkinSurfSpan>,
    pub output_vertex_total: usize,
    pub idx_list: Vec<u32>,
}

#[derive(Clone, Copy, Debug)]
pub struct SkinSurfSpan {
    pub surface_slot_number: usize,
    pub in_vertex_start: usize,
    pub out_vertex_start: usize,
    pub vertex_total: usize,
    pub first_triangle_slot: u32,
    pub idx_count: u32,
    pub shown: bool,
}

/// Lays out the surfaces of `lod`; `surface_is_visible` is asked by position
/// within the LOD. Empty or hidden surfaces take no room. `None` when the
/// skeleton's ranges or indices are inconsistent.
pub fn plan_vertex_skinning(
    bone_rig: &FpvSkel,
    lod: u8,
    surface_is_visible_in: impl Fn(usize) -> bool,
) -> Option<VertexSkinPlan> {
    if is_single_mesh(bone_rig) {
        let (tri_idx, output_vertex_total) = (bone_rig.indices.clone(), bone_rig.compact_vertices.len());
        return Some(VertexSkinPlan { idx_list: tri_idx, output_vertex_total, surfs: Vec::new() });
    }
    if bone_rig.facet_index_ranges.len() != bone_rig.facet_vertex_ranges.len() {
        return None;
    }
    let mut layout = VertexSkinPlan { idx_list: Vec::new(), output_vertex_total: 0, surfs: Vec::new() };
    for (position, surface_slot_number) in bone_rig.facets_at_tier(lod).enumerate() {
        let (rows, index_rows) = surface_spans(bone_rig, surface_slot_number)?;
        if rows.end > bone_rig.compact_vertices.len() || index_rows.end > bone_rig.indices.len() {
            return None;
        }
        let (vertex_count, index_count) = (rows.len(), index_rows.len());
        let drawable = surface_is_visible_in(position) && vertex_count > 0 && index_count > 0;
        if !drawable {
            layout.surfs.push(SkinSurfSpan {
                surface_slot_number,
                in_vertex_start: rows.start,
                out_vertex_start: 0,
                vertex_total: vertex_count,
                first_triangle_slot: 0,
                idx_count: 0, shown: false,
            });
            continue;
        }
        let first_triangle_slot = layout.idx_list.len() as u32;
        let out_vertex_start = layout.output_vertex_total;
        let rebase = |global: u32| {
            let local = usize::try_from(global).ok()?.checked_sub(rows.start)?;
            (local < vertex_count).then(|| (out_vertex_start + local) as u32)
        };
        for &global in &bone_rig.indices[index_rows] {
            layout.idx_list.push(rebase(global)?);
        }
        layout.output_vertex_total += vertex_count;
        layout.surfs.push(SkinSurfSpan {
            surface_slot_number,
            in_vertex_start: rows.start,
            out_vertex_start,
            vertex_total: vertex_count,
            first_triangle_slot,
            idx_count: index_count as u32, shown: true,
        });
    }
    Some(layout)
}

/// Re-skins the packed rows of every visible surface of `layout` into `write_target`
/// (which must hold `layout.output_vertex_total` rows).
pub fn write_skinned_verts(
    bone_rig: &FpvSkel,
    bone_to_own_client: impl Fn(usize) -> Mat4,
    surface_rigid_v: impl Fn(usize) -> bool,
    lod: u8,
    layout: &VertexSkinPlan,
    write_target: &mut [[u8; asset_t6::T6_PACKED_VERTEX]],
) {
    debug_assert_eq!(write_target.len(), layout.output_vertex_total);
    if write_target.is_empty() {
        return;
    }
    let masks = vertex_masks(bone_rig, lod, |_| true, surface_rigid_v);
    let bones = BoneMatrices::for_active(bone_rig, &masks.active, bone_to_own_client);
    for placed in &layout.surfs {
        if !placed.shown {
            continue;
        }
        let rows_in = &mut write_target[placed.out_vertex_start..][..placed.vertex_total];
        for (step, row) in rows_in.iter_mut().enumerate() {
            let src = placed.in_vertex_start + step;
            *row = bone_rig.compact_vertices.get(src).copied().unwrap_or([0; 32]);
            let live = masks.active.get(src).copied().unwrap_or(false);
            let (true, Some(skin)) = (live, bone_rig.vert_skin.get(src)) else {
                continue;
            };
            let rigid = masks.rigid.get(src).copied().unwrap_or(false);
            let rest = Vec3::from_array(bone_rig.positions[src]);
            let posed = pose_vertex(&bones, skin, rest, None, rigid);
            write_packed_pose(row, posed.position, posed.frame_basis);
        }
    }
}

mod tests {
    #![cfg(test)]
    use super::*;
    use crate::motion::test_skel::blank_skel;
    use bevy::render::mesh::VertexAttributeValues;

    /// `surfaces` one-triangle surfaces, every vertex fully weighted to bone 0.
    fn triangles(surfaces: usize) -> FpvSkel {
        let mut bone_rig = blank_skel("tri");
        for s in 0..surfaces {
            let first = s * 3;
            bone_rig.positions
                .extend([[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]]);
            bone_rig.surface_directions.extend([[0.0, 0.0, 1.0]; 3]);
            bone_rig.texcoord_set.extend([[0.0, 0.0]; 3]);
            bone_rig.colors.extend([[1.0; 4]; 3]);
            bone_rig.compact_vertices.extend([[0u8; 32]; 3]);
            bone_rig.indices.extend((first..first + 3).map(|i| i as u32));
            bone_rig.facet_vertex_ranges.push((first, 3));
            bone_rig.facet_index_ranges.push((first, 3));
            bone_rig.facet_materials.push(None);
            bone_rig.face_parts.push([0; 5]);
        }
        bone_rig.vert_skin = vec![
            CornerWeights {
                weights: [1.0, 0.0, 0.0, 0.0],
                ..CornerWeights::default()
            };
            surfaces * 3
        ];
        bone_rig
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
        let bone_rig = triangles(3);
        assert_eq!(lod_surf_slot(&bone_rig, 0, 2), 2);
        assert_eq!(lod_surf_slot(&bone_rig, 0, 9), usize::MAX);
        assert_eq!(lod_surf_slot(&bone_rig, 1, 0), usize::MAX);
    }

    #[test]
    fn rigid_surfaces_move_with_their_bone() {
        let bone_rig = triangles(1);
        let posed =
            skin_chosen_surfaces(&bone_rig, lift, &[], |_| true, |_| true, 0).expect("consistent");
        assert_eq!(posed.len(), 1);
        assert_eq!(
            positions_of(&posed[0].mesh),
            [[1.0, 2.0, 3.0], [2.0, 2.0, 3.0], [1.0, 3.0, 3.0]]
        );
        assert_eq!(index_count(&posed[0].mesh), 3);
        assert_eq!(posed[0].surface_slot_number, 0);
    }

    #[test]
    fn a_hidden_surface_keeps_its_vertices_but_draws_nothing() {
        let bone_rig = triangles(2);
        let posed = skin_chosen_surfaces(&bone_rig, lift, &[], |s| s == 1, |_| true, 0).unwrap();
        assert_eq!(index_count(&posed[0].mesh), 0);
        assert_eq!(index_count(&posed[1].mesh), 3);
        assert_eq!(positions_of(&posed[0].mesh).len(), 3);
    }

    #[test]
    fn a_vertex_with_no_primary_weight_that_lands_on_the_origin_stays_at_rest() {
        let mut bone_rig = triangles(1);
        bone_rig.vert_skin[1].weights[0] = 0.0;
        bone_rig.positions[1] = [5.0, 0.0, 0.0];
        let posed = skin_chosen_surfaces(&bone_rig, |_| Mat4::ZERO, &[], |_| true, |_| true, 0).unwrap();
        assert_eq!(positions_of(&posed[0].mesh)[1], [5.0, 0.0, 0.0]);
    }

    #[test]
    fn a_zeroed_normal_falls_back_to_up() {
        let bone_rig = triangles(1);
        let posed = skin_chosen_surfaces(&bone_rig, |_| Mat4::ZERO, &[], |_| true, |_| true, 0).unwrap();
        match posed[0].mesh.attribute(Mesh::ATTRIBUTE_NORMAL) {
            Some(VertexAttributeValues::Float32x3(surface_directions)) => {
                assert!(surface_directions.iter().all(|n| *n == [0.0, 0.0, 1.0]));
            }
            other => panic!("no normal stream: {other:?}"),
        }
    }

    #[test]
    fn a_skeleton_with_mismatched_ranges_is_refused() {
        let mut bone_rig = triangles(2);
        bone_rig.facet_index_ranges.pop();
        assert!(skin_chosen_surfaces(&bone_rig, lift, &[], |_| true, |_| true, 0).is_none());
        assert!(plan_vertex_skinning(&bone_rig, 0, |_| true).is_none());
    }

    #[test]
    fn the_layout_packs_only_the_visible_surfaces() {
        let bone_rig = triangles(2);
        let layout = plan_vertex_skinning(&bone_rig, 0, |local| local == 1).unwrap();
        assert_eq!(layout.output_vertex_total, 3);
        assert_eq!(layout.idx_list, [0, 1, 2]);
        assert!(!layout.surfs[0].shown);
        let kept = layout.surfs[1];
        assert!(kept.shown);
        assert_eq!((kept.in_vertex_start, kept.out_vertex_start), (3, 0));
        assert_eq!((kept.first_triangle_slot, kept.idx_count), (0, 3));
    }

    #[test]
    fn an_index_that_leaves_its_surface_spoils_the_layout() {
        let mut bone_rig = triangles(1);
        bone_rig.indices[2] = 7;
        assert!(plan_vertex_skinning(&bone_rig, 0, |_| true).is_none());
    }

    #[test]
    fn re_skinning_packed_rows_moves_their_positions() {
        let bone_rig = triangles(1);
        let layout = plan_vertex_skinning(&bone_rig, 0, |_| true).unwrap();
        let mut write_target = vec![[0u8; 32]; layout.output_vertex_total];
        write_skinned_verts(&bone_rig, lift, |_| true, 0, &layout, &mut write_target);
        assert_eq!(write_target[1][0..12], *bytemuck::bytes_of(&[2.0f32, 2.0, 3.0]));
    }

    #[test]
    fn a_stream_entry_marks_a_surface_rigid_by_its_lod_position() {
        let bone_rig = triangles(2);
        let entry = |surface: u16, tag: i32| dpvs_t6::EntitySkinSlotRecord {
            mdl: 4,
            surface_no: surface,
            bone_tag: tag,
            joint_start_offset: 0,
            joint_total: 0,
            bytes: 0,
        };
        let rigid = [entry(1, dpvs_t6::SKIN_SURFACE_SKIP_TAG - 1)];
        assert!(rigid_lod_surface(&bone_rig, 0, &rigid, 4, 1));
        assert!(!rigid_lod_surface(&bone_rig, 0, &rigid, 4, 0));
        assert!(!rigid_lod_surface(&bone_rig, 0, &rigid, 5, 1));
    }
}
