//! bo2zm: Black Ops II models with their skeletons, as the engine's
//! skinned model (`MeshRig`) — viewmodel guns and arms, world guns.
//!
//! Vertices are BO2's `GfxPackedVertex` (32 bytes): position, binormal
//! sign, colour R,G,B,A, uv_semantic half2, normal and tangent packed 10:10:10
//! signed. The model keeps each row as BO2 wrote it. Rigid vertex lists and
//! blend weights use the same bone units (64 bytes per bone) as legacy and
//! Black Ops.

use asset_core::TraversalMaterialSlot;
use asset_t6::T6_PACKED_VERTEX;
use asset_t6::short_float_to_full;
use bevy::math::f32::Quat;
use bevy::math::f32::Vec3;
use xmodel_runtime::PoseDonorModel;

use crate::CornerWeights;
use crate::JointAttachment;
use crate::MeshRig;

const BYTES_PER_BONE: u16 = 1 << 6;
/// A BO2 vertex's extra bone weight is a u16 share of 65536.
const BLEND_WEIGHT_SCALE: f32 = 1.0 / 65536.0;

/// One packed vertex row, read as little-endian words.
#[derive(Clone, Copy)]
struct Row<'a>(&'a [u8]);

impl Row<'_> {
    fn word(self, pos: usize) -> u32 {
        let window: [u8; 4] = self.0[pos..pos + 4].try_into().expect("a four-byte window");
        u32::from_le_bytes(window)
    }

    fn float(self, pos: usize) -> f32 {
        f32::from_bits(self.word(pos))
    }
}

/// A direction of length one; a vector too short to have one points up.
fn unit_or_up(v: Vec3) -> [f32; 3] {
    let len = v.length();
    let direction = if len > 1e-6 { v / len } else { Vec3::Z };
    direction.to_array()
}

/// The two ends of a box as (middle, half-lengths), the convention every
/// model skeleton uses.
fn mid_half(ends: ([f32; 3], [f32; 3])) -> ([f32; 3], [f32; 3]) {
    let low = Vec3::from_array(ends.0);
    let high = Vec3::from_array(ends.1);
    (((low + high) * 0.5).to_array(), ((high - low) * 0.5).to_array())
}

/// A vertex held to one bone at full weight.
fn rigid_skin(bone: u16) -> CornerWeights {
    let mut skin = CornerWeights::default();
    skin.joints[0] = bone;
    skin.weights[0] = 1.0;
    skin
}

/// The bone a byte offset into the bone table names; `None` when the offset
/// is not a whole bone or the model has no such bone.
fn bone_index(offset: u16, bone_total: usize) -> Option<u16> {
    let whole = offset % BYTES_PER_BONE == 0;
    let index = offset / BYTES_PER_BONE;
    (whole && usize::from(index) < bone_total).then_some(index)
}

/// One weighted vertex from its blend words: the first bone alone, then a
/// (bone, share) pair for each further bone. The first bone keeps whatever
/// the others leave of the whole.
fn weighted_skin(words: &[u16], bone_total: usize) -> Option<CornerWeights> {
    let (&first, pairs) = words.split_first()?;
    let mut skin = rigid_skin(bone_index(first, bone_total)?);
    for (slot, pair) in (1..).zip(pairs.chunks_exact(2)) {
        skin.joints[slot] = bone_index(pair[0], bone_total)?;
        skin.pull_u16[slot] = pair[1];
        skin.weights[slot] = f32::from(pair[1]) * BLEND_WEIGHT_SCALE;
    }
    skin.weights[0] = skin.weights[1..]
        .iter()
        .fold(1.0, |rest, share| rest - share);
    Some(skin)
}

/// Per-vertex bone influences of one surface: weighted vertices first (by
/// influence count), then the rigid vertex lists. `None` when a bone
/// offset is not a whole bone or names a bone the model lacks.
fn surface_skins(surf: &asset_t6::XSurfaceRef, bone_total: usize) -> Option<Vec<CornerWeights>> {
    let vert_total = usize::from(surf.vertex_tally);
    let mut skins = Vec::with_capacity(vert_total);
    let mut unread: &[u16] = &surf.blend;
    for (extra_bones, &count) in surf.blend_counts.iter().enumerate() {
        let span = 2 * extra_bones + 1;
        for _ in 0..count.max(0) {
            let (words, rest) = unread.split_at_checked(span)?;
            unread = rest;
            skins.push(weighted_skin(words, bone_total)?);
        }
    }
    if skins.len() > vert_total {
        return None;
    }
    for &(first_byte, run, _, _) in &surf.vert_lists {
        let held = rigid_skin(bone_index(first_byte, bone_total)?);
        let room = vert_total - skins.len();
        skins.extend(std::iter::repeat_n(held, usize::from(run).min(room)));
    }
    skins.resize(vert_total, rigid_skin(0));
    let finished = skins;
    Some(finished)
}

/// A surface whose vertex bytes cover its count and whose indices all name
/// one of its vertices.
fn surface_is_sound(surf: &asset_t6::XSurfaceRef) -> bool {
    let vert_total = usize::from(surf.vertex_tally);
    surf.vertex_points.len() >= vert_total * T6_PACKED_VERTEX
        && surf.indices.iter().all(|&ix| usize::from(ix) < vert_total)
}

/// Adds one surface's vertices and indices after the others already in
/// `bone_rig`; says where they landed, each as (first, count).
fn append_surface(
    bone_rig: &mut MeshRig,
    surf: &asset_t6::XSurfaceRef,
    skins: &[CornerWeights],
) -> ((usize, usize), (usize, usize)) {
    let first_vertex = bone_rig.positions.len();
    let first_index = bone_rig.indices.len();
    for (row, &skin) in surf.vertex_points.chunks_exact(T6_PACKED_VERTEX).zip(skins) {
        let fields = Row(row);
        let uv_semantic = fields.word(20);
        bone_rig.positions.push([0, 4, 8].map(|at| fields.float(at)));
        let squeezed_facing = asset_t6::decode_unit_normal(fields.word(24));
        bone_rig.surface_directions
            .push(unit_or_up(Vec3::from_array(squeezed_facing)));
        bone_rig.colors.push(std::array::from_fn(|chan| {
            f32::from(row[16 + chan]) / 255.0
        }));
        let halves = [uv_semantic & 0xffff, uv_semantic >> 16];
        bone_rig.texcoord_set.push(halves.map(|half| short_float_to_full(half as u16)));
        bone_rig.compact_vertices
            .push(row.try_into().expect("a whole packed vertex"));
        bone_rig.vert_skin.push(skin);
    }
    let shift = first_vertex as u32;
    bone_rig.indices
        .extend(surf.indices.iter().map(|&ix| shift + u32::from(ix)));
    (
        (first_vertex, skins.len()),
        (first_index, surf.indices.len()),
    )
}

/// What a model keeps once per surface.
struct SurfaceMeta {
    vertex_points: (usize, usize),
    indices: (usize, usize),
    part_bits: [u32; 5],
    deformed: bool,
    list_count: u32,
    material: Option<TraversalMaterialSlot>,
}

/// The bind pose and local pose the runtime animates from.
fn pose_source(model: &asset_t6::XModelRef) -> PoseDonorModel {
    PoseDonorModel {
        name: String::from(&*model.name),
        skeleton_joint_total: model.joint_labels.len(),
        top_joint_total: usize::from(model.top_joint_total),
        size_scale: 1.0,
        fixed_size_parts: Default::default(),
        joint_labels: model.joint_labels.clone(),
        ancestor_chain: model.ancestor_chain.clone(),
        rotation_quaternions: model.rotation_quaternions.clone(),
        shift_offset: model.shift_offset.clone(),
        foundation_surface: model
            .foundation_surface
            .iter()
            .map(|&(quat, shift_offset)| (Quat::from_array(quat), Vec3::from_array(shift_offset)))
            .collect(),
    }
}

/// bo2zm M3: hit boxes per bone (what bullets hit on a zombie); a bone
/// without a box has none.
fn bone_hit_boxes(model: &asset_t6::XModelRef) -> Vec<Option<xmodel_runtime::BoneHitShape>> {
    let classes = model
        .body_part_category
        .iter()
        .copied()
        .chain(std::iter::repeat(0));
    let infos = model
        .bone_info
        .iter()
        .copied()
        .chain(std::iter::repeat(None));
    infos
        .zip(classes)
        .take(model.joint_labels.len())
        .map(|(info, class)| {
            let (lo, hi, reach_squared) = info?;
            let (mid, half) = mid_half((lo, hi));
            Some(xmodel_runtime::BoneHitShape {
                midpoint: mid,
                half_ext: half,
                reach_squared,
                body_part_category: class,
            })
        })
        .collect()
}

/// bo2mp: the collision surfaces a bullet hits (BO2's DObjGeomTraceline
/// runs XModelTraceLineAnimated on them). Without them a destructible
/// car stopped bullets on rough bone boxes. T6 keeps each surface's
/// mins/maxs where legacy keeps (mid, half).
fn collision_surfaces(model: &asset_t6::XModelRef) -> Vec<xmodel_runtime::SurfaceHitMesh> {
    let as_record = |surf: &asset_t6::XModelCollSurfRef| {
        let (mid, half) = mid_half((surf.lo_corner, surf.hi_corner));
        let contents = surf.contents;
        let face_property_bits = surf.face_property_bits;
        let triangle_list = surf
            .triangle_list
            .iter()
            .map(|&[plane, surface_tangent, surface_bitangent]| xmodel_runtime::HitTriangle { plane, surface_tangent, surface_bitangent });
        let triangle_list: Vec<_> = triangle_list.collect();
        let half_size = half.map(|len| len.max(0.0));
        u16::try_from(surf.joint_position)
            .ok()
            .map(|bone| xmodel_runtime::SurfaceHitMesh {
                bone,
                contents,
                face_property_bits,
                midpoint: mid,
                half_ext: half_size,
                triangle_list,
            })
    };
    model.collision_faces.iter().filter_map(as_record).collect()
}

/// A BO2 model (first LOD) as the engine's skinned model. `material` maps a
/// surface's material to the catalog it was linked into.
pub fn capture_model_skel_t6(
    model: &asset_t6::XModelRef,
    material: impl Fn(asset_t6::AssetIdentity) -> Option<usize>,
) -> Option<MeshRig> {
    let skeleton_joint_total = model.joint_labels.len();
    if skeleton_joint_total == 0 || model.foundation_surface.len() != skeleton_joint_total {
        return None;
    }

    let mut bone_rig = MeshRig::default();
    let mut metas = Vec::with_capacity(model.lod0.len());
    for surf in &model.lod0 {
        if !surface_is_sound(surf) {
            return None;
        }
        let skins = surface_skins(surf, skeleton_joint_total)?;
        let (vertex_points, indices) = append_surface(&mut bone_rig, surf, &skins);
        metas.push(SurfaceMeta {
            vertex_points,
            indices,
            part_bits: surf.piece_mask,
            deformed: surf.flags & 0x80 != 0,
            list_count: surf.vert_lists.len() as u32,
            material: surf
                .material
                .and_then(&material)
                .map(TraversalMaterialSlot::from_walk),
        });
    }
    bone_rig.facet_materials = metas.iter().map(|meta| meta.material).collect();
    bone_rig.facet_vertex_ranges = metas.iter().map(|meta| meta.vertex_points).collect();
    bone_rig.facet_index_ranges = metas.iter().map(|meta| meta.indices).collect();
    bone_rig.face_parts = metas.iter().map(|meta| meta.part_bits).collect();
    bone_rig.bent = metas.iter().map(|meta| Some(meta.deformed)).collect();
    bone_rig.vert_lists = metas.iter().map(|meta| Some(meta.list_count)).collect();
    bone_rig.detail_face_extent[0] = (0, metas.len() as u16);

    let find = |wanted: &str| model.joint_labels.iter().position(|have| have == wanted);
    bone_rig.name = String::from(&*model.name);
    bone_rig.bones = model
        .foundation_surface
        .iter()
        .map(|&(quat, shift_offset)| JointAttachment { spin: quat, shift_offset })
        .collect();
    bone_rig.joint_hit_volume = bone_hit_boxes(model);
    bone_rig.joint_labels = model.joint_labels.clone();
    bone_rig.eye_joint = find("tag_view");
    bone_rig.gun_joint = find("tag_weapon");
    bone_rig.pose = Some(pose_source(model));
    bone_rig.radius = Some(model.radius);
    bone_rig.bounds = Some(mid_half((model.lo_corner, model.hi_corner)));
    bone_rig.light_offset = Some(model.lighting_origin_offset);
    bone_rig.contents = Some(model.contents);
    bone_rig.collision_detail_tier = model.collision_detail_tier;
    bone_rig.collision_faces = collision_surfaces(model);
    bone_rig.collmap = t6_collmap(model);
    bone_rig.lod = Some(crate::ModelDetailSelector::T5 {
        detail_tier_total: 1,
        tier_reach: [0.0; 4],
    });
    Some(bone_rig)
}

/// bo2mp: the model's physics collmap as boxes. A brush piece is its bounds
/// (the collision_* script models are all side-less boxes; a brush with
/// extra sides, like collision_clip_ramp, keeps only its bounds); any other
/// piece is a box of its own axes, middle and half-lengths (a stand-in for
/// a cylinder or capsule).
fn t6_collmap(model: &asset_t6::XModelRef) -> Option<xmodel_runtime::ModelCollmap> {
    const UNIT_AXES: [[f32; 3]; 3] = [Vec3::X.to_array(), Vec3::Y.to_array(), Vec3::Z.to_array()];
    if model.collmap_contents == 0 && model.collmap.is_empty() {
        return None;
    }
    let boxes = model
        .collmap
        .iter()
        .map(|piece| match piece.brush.filter(|_| piece.ty == 2) {
            Some((lo, hi, _)) => {
                let (mid, half) = mid_half((lo, hi));
                (mid, UNIT_AXES, half)
            }
            None => (piece.offset, piece.orientation, piece.half_lengths),
        });
    Some(xmodel_runtime::ModelCollmap {
        contents: model.collmap_contents | model.contents,
        boxes: boxes
            .map(|(center, axis, half)| xmodel_runtime::CollmapBox { center, axis, half })
            .collect(),
        brush_verts: model.collmap_brush_verts.clone(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_vectors_point_up_and_others_have_length_one() {
        assert_eq!(unit_or_up(Vec3::ZERO), [0.0, 0.0, 1.0]);
        assert_eq!(unit_or_up(Vec3::new(0.0, 3.0, 4.0)), [0.0, 0.6, 0.8]);
    }

    #[test]
    fn bone_offsets_must_be_whole_bones_the_model_has() {
        assert_eq!(bone_index(128, 3), Some(2));
        assert_eq!(bone_index(130, 3), None);
        assert_eq!(bone_index(192, 3), None);
    }

    #[test]
    fn the_first_bone_keeps_what_the_others_leave() {
        let skin = weighted_skin(&[0, 64, 16384, 128, 16384], 4).expect("a good vertex");
        assert_eq!(skin.joints, [0, 1, 2, 0]);
        assert_eq!(skin.weights, [0.5, 0.25, 0.25, 0.0]);
        assert_eq!(skin.pull_u16, [0, 16384, 16384, 0]);
        assert!(weighted_skin(&[0, 640, 100], 4).is_none());
    }

    #[test]
    fn rigid_lists_fill_what_the_weighted_vertices_left() {
        let surf = asset_t6::XSurfaceRef {
            vertex_tally: 4,
            blend_counts: [1, 0, 0, 0],
            blend: vec![64],
            vert_lists: vec![(128, 9, 0, 0)],
            ..Default::default()
        };
        let skins = surface_skins(&surf, 3).expect("skins");
        let bones: Vec<u16> = skins.iter().map(|skin| skin.joints[0]).collect();
        assert_eq!(bones, [1, 2, 2, 2]);
    }

    #[test]
    fn boxes_are_kept_as_middle_and_half_lengths() {
        assert_eq!(
            mid_half(([-1.0, 0.0, 2.0], [3.0, 4.0, 2.0])),
            ([1.0, 2.0, 2.0], [2.0, 2.0, 0.0])
        );
    }
}
