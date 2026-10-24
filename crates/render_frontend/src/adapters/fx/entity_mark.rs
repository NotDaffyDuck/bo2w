//! Marks on moving models (players, script models): bullet holes and blood
//! that stay on the model they hit.
//!
//! A hit on a model is queued by [`queue`]. [`generate`] later poses the model
//! it hit, clips the marks against its triangles, and keeps each mark
//! attached to the bone it landed on so it follows the model around.

use std::collections::BTreeMap;

use bevy::prelude::*;
use marks_t6::{FxAllocMarkRequest, FxMarkStagingPoint, FxMarkStagingTri};
use render_fx::{EntityMarkAttachment, EntityMarkRequest, EntityMarkStore, EntityMarks};

use crate::prepare::scene::world::{WorldScene, WorldScriptModelInstance};

/// How far from 1.0 a bone's scale may be and still carry a mark.
const SCALE_TOLERANCE: f32 = 0.0001;

/// Queue the host's latest decal as a mark on the entity it hit. Nothing is
/// queued unless the host marks entities and holds a complete decal.
pub(crate) fn queue(host: &fx::FxSystemHost, marks: &EntityMarks) {
    if !host.mark_receivers.fx_marks_ents {
        return;
    }
    let (
        Some(entity),
        Some(origin),
        Some(axis),
        Some(rotation),
        Some(radius),
        Some(material),
        Some(color),
    ) = (
        host.last_decal_mark_entity,
        host.last_decal_origin,
        host.last_decal_axis,
        host.last_decal_rotation,
        host.last_decal_size0,
        host.last_decal_mat0.as_ref(),
        host.last_decal_color,
    )
    else {
        return;
    };
    marks.lock().pending.push(EntityMarkRequest {
        entity,
        origin,
        axis: fx_t6::runtime::impact_mark_axis(axis, rotation),
        radius,
        material: material.clone(),
        color,
    });
}

/// The model index as it is kept in a mark: 5 bits of the mark's type byte,
/// so models 0 to 31 only.
fn dobj_model_index(model_index: usize) -> Option<u8> {
    u8::try_from(model_index)
        .ok()
        .filter(|&index| index <= marks_t6::MARK_MODEL_DOBJ_MODEL_MASK)
}

/// What a triangle's skinning says about where a mark can go.
#[derive(Debug, PartialEq, Eq)]
enum TriangleBone {
    /// The skin rows are missing; skip it without counting it.
    Missing,
    /// The triangle is not rigidly bound to one bone.
    Unsupported,
    /// Every corner is fully weighted to this bone.
    Rigid(usize),
}

/// The one bone a triangle is fully bound to, judged from its corners.
fn triangle_bone(skins: &[asset_model::VertSkin], corners: [usize; 3]) -> TriangleBone {
    let Some(first) = skins.get(corners[0]) else {
        return TriangleBone::Missing;
    };
    let bone = usize::from(first.bones[0]);
    if bone > usize::from(u8::MAX) {
        return TriangleBone::Unsupported;
    }
    let all_rigid = corners.iter().all(|&corner| {
        skins.get(corner).is_some_and(|skin| {
            usize::from(skin.bones[0]) == bone && skin.weights == [1.0, 0.0, 0.0, 0.0]
        })
    });
    if all_rigid {
        TriangleBone::Rigid(bone)
    } else {
        TriangleBone::Unsupported
    }
}

/// Is a bone's transform free of scaling, so the mark's shape survives it?
fn has_unit_scale(matrix: &Mat4) -> bool {
    matrix
        .to_scale_rotation_translation()
        .0
        .abs_diff_eq(Vec3::ONE, SCALE_TOLERANCE)
}

/// Pose data of a model whose marks are being made.
struct PosedOwner<'a> {
    skels: Vec<&'a asset_model::ModelSkel>,
    /// First bone of each model in the shared skin array.
    bone_bases: Vec<usize>,
    skin: Vec<Mat4>,
}

/// Build the owner's skeleton and pose, or `None` if any step is not
/// possible (no models, models not presented, no animation, bad pose).
fn pose_owner<'a>(
    owner: &WorldScriptModelInstance,
    transform: &Transform,
    xanims: Option<&assets::PreparedXAnims>,
    models: &'a asset_world::MapXModelSceneCatalog,
) -> Option<PosedOwner<'a>> {
    if owner.dobj_state.composition.models.is_empty() {
        return None;
    }
    let (specs, skels) =
        crate::adapters::anim::script_model::collect_presented_models(models, &owner.dobj_state)?;
    let dobj = xmodel_runtime::DObj::build(&specs).ok()?;
    let request = owner
        .dobj_state
        .resolve_request(|name| xanims?.0.clip(asset_core::AssetNamespace::T6, name))
        .ok()?;
    let pose = xmodel_runtime::pose_dobj(&dobj, &request, transform.to_matrix()).ok()?;
    Some(PosedOwner {
        skin: dobj.skin_matrices(&pose),
        bone_bases: dobj.models.iter().map(|slot| slot.base).collect(),
        skels,
    })
}

/// Make the entity marks for this frame: stage the new hits onto the models
/// they hit, and add every attached mark that is still on a model to the
/// frame's mark mesh.
pub(crate) fn generate(
    host: &mut fx::FxSystemHost,
    scene: &WorldScene,
    marks: &EntityMarks,
    owners: &Query<(&WorldScriptModelInstance, &Transform, &Visibility)>,
    xanims: Option<&assets::PreparedXAnims>,
    models: &asset_world::MapXModelSceneCatalog,
) {
    let mut state = marks.lock();
    let pending = std::mem::take(&mut state.pending);
    let before_triangles = state.unsupported_triangles;
    let before_receivers = state.unsupported_receivers;
    let mut mesh = host
        .last_mark_mesh
        .take()
        .expect("world mark generation precedes entity marks");

    for (owner, transform, visibility) in owners.iter() {
        let Some(entity) = owner.gentity_number else {
            continue;
        };
        if *visibility == Visibility::Hidden {
            continue;
        }
        let requests: Vec<&EntityMarkRequest> =
            pending.iter().filter(|r| r.entity == entity).collect();
        if requests.is_empty() && !state.attached.iter().any(|a| a.entity == entity) {
            continue;
        }
        let Some(posed) = pose_owner(owner, transform, xanims, models) else {
            state.unsupported_receivers += requests.len() as u64;
            continue;
        };
        let composition = &owner.dobj_state.composition;
        for request in &requests {
            for (model_index, skel) in posed.skels.iter().enumerate() {
                let (Some(&bone_base), Some(desc)) = (
                    posed.bone_bases.get(model_index),
                    composition.models.get(model_index),
                ) else {
                    state.unsupported_receivers += 1;
                    continue;
                };
                let receiver = RigidReceiver {
                    owner,
                    skel,
                    skin: &posed.skin,
                    model_index,
                    bone_base,
                    model: desc.model.as_str(),
                    revision: composition.revision,
                };
                stage(host, scene, &mut state, request, receiver, models);
            }
        }
        if host.mark_receivers.fx_marks && host.mark_receivers.fx_marks_ents {
            append_attached(host, &mut mesh, &state, owner, entity, &posed.skin);
        }
    }

    log_unsupported(&state, before_triangles, before_receivers);
    host.gfx_mark_surf_n = Some(mesh.budget.surf_n);
    host.gfx_mark_vert_n = Some(mesh.budget.vert_n);
    host.gfx_mark_index_n = Some(mesh.budget.index_n);
    host.gfx_mark_packed_n = Some(mesh.packed.len() as u32);
    host.last_mark_mesh = Some(mesh);
}

/// Add the owner's attached marks, each carried by its bone, to the mesh.
/// Stops at the first mark the mesh has no room for.
fn append_attached(
    host: &mut fx::FxSystemHost,
    mesh: &mut fx::GfxMarkMeshCensus,
    state: &EntityMarkStore,
    owner: &WorldScriptModelInstance,
    entity: u16,
    skin: &[Mat4],
) {
    let composition = &owner.dobj_state.composition;
    let current = state.attached.iter().filter(|a| {
        a.entity == entity
            && a.revision == composition.revision
            && composition.models.iter().any(|m| m.model == a.model)
    });
    for attachment in current {
        if owner.dobj_state.hide_part_bits.get(attachment.bone) {
            continue;
        }
        let Some(&matrix) = skin.get(attachment.bone) else {
            continue;
        };
        let Some(&mark) = host.marks.constructed(attachment.handle) else {
            continue;
        };
        let origin = matrix.transform_point3(Vec3::from_array(mark.origin));
        let axis = matrix.transform_vector3(Vec3::from_array(mark.tex_coord_axis));
        let moved = host.marks.append_model_mark(
            mesh,
            attachment.handle,
            origin.to_array(),
            axis.to_array(),
            &|mut point| {
                point.xyz = matrix
                    .transform_point3(Vec3::from_array(point.xyz))
                    .to_array();
                point.normal = matrix
                    .transform_vector3(Vec3::from_array(point.normal))
                    .normalize()
                    .to_array();
                point
            },
        );
        if moved.is_err() {
            break;
        }
    }
}

/// Say, with `BO2MP_RENDER_LOG=1` only, how many marks could not be placed:
/// the counts change with nearly every hit.
fn log_unsupported(state: &EntityMarkStore, before_triangles: u64, before_receivers: u64) {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    if !*ON.get_or_init(|| std::env::var_os("BO2MP_RENDER_LOG").is_some()) {
        return;
    }
    if state.unsupported_triangles != before_triangles {
        diag::warn!(
            World,
            "entity marks: {} triangles lack a single rigid bone receiver",
            state.unsupported_triangles - before_triangles
        );
    }
    if state.unsupported_receivers != before_receivers {
        diag::warn!(
            World,
            "entity marks: {} requests lack composed rigid pose data",
            state.unsupported_receivers - before_receivers
        );
    }
}

/// One model of an owner that a mark can land on.
struct RigidReceiver<'a> {
    owner: &'a WorldScriptModelInstance,
    skel: &'a asset_model::ModelSkel,
    skin: &'a [Mat4],
    model_index: usize,
    bone_base: usize,
    model: &'a str,
    revision: u32,
}

/// The mark pieces cut for one bone, in that bone's own space.
#[derive(Default)]
struct BonePieces {
    tris: Vec<FxMarkStagingTri>,
    points: Vec<FxMarkStagingPoint>,
}

impl BonePieces {
    /// Add one clipped fragment. The staging arrays are opened to their full
    /// size for the emit and cut back to what it used; a fragment that does
    /// not fit leaves the pieces as they were.
    fn add_fragment(
        &mut self,
        fragment: &[marks_t6::FxWorldMarkPoint],
        normals: [[f32; 3]; 3],
        context: marks_t6::GfxMarkContext,
    ) {
        let used_tris = self.tris.len();
        let used_points = self.points.len();
        self.tris.resize(
            marks_t6::R_MARK_FRAGMENTS_MAX_TRIS as usize,
            FxMarkStagingTri::ZERO,
        );
        self.points.resize(
            marks_t6::R_MARK_FRAGMENTS_MAX_POINTS as usize,
            FxMarkStagingPoint::ZERO,
        );
        let [n0, n1, n2] = normals;
        let emitted = marks_t6::mark_emit_brush_fragment(
            used_tris as u32,
            used_points as u32,
            marks_t6::R_MARK_FRAGMENTS_MAX_TRIS,
            marks_t6::R_MARK_FRAGMENTS_MAX_POINTS,
            fragment,
            [0.0; 2],
            [0.0; 2],
            [0.0; 2],
            n0,
            n1,
            n2,
            context,
            &mut self.tris,
            &mut self.points,
        );
        let (tris, points) = match emitted {
            Ok((tris, points)) => (tris as usize, points as usize),
            Err(_) => (used_tris, used_points),
        };
        self.tris.truncate(tris);
        self.points.truncate(points);
    }
}

/// Cut the request's mark out of every rigid triangle of the receiver and
/// attach what is left to the bones it landed on.
fn stage(
    host: &mut fx::FxSystemHost,
    scene: &WorldScene,
    state: &mut EntityMarkStore,
    request: &EntityMarkRequest,
    receiver: RigidReceiver<'_>,
    models: &asset_world::MapXModelSceneCatalog,
) {
    let RigidReceiver {
        owner,
        skel,
        skin,
        model_index,
        bone_base,
        model,
        revision,
    } = receiver;
    let Some(dobj_model_index) = dobj_model_index(model_index) else {
        state.unsupported_receivers += 1;
        return;
    };
    let model_key = asset_world::MapXModelAssetKey(model.to_string());
    let mark_bits = super::world_mark::runtime_material_by_name(scene, &request.material)
        .and_then(|m| m.surface_type_bits);
    let planes = marks_t6::mark_fragment_clip_planes(request.origin, request.axis, request.radius);
    let context = marks_t6::GfxMarkContext::entity_model(request.entity, dobj_model_index);
    let mut by_bone = BTreeMap::<usize, BonePieces>::new();
    let mut fragment = [marks_t6::FxWorldMarkPoint::ZERO; marks_t6::R_MARK_CHOP_MAX_POINTS];

    for surface in skel.surfaces_for_lod(0) {
        if skel.surface_deformed.get(surface) != Some(&Some(false)) {
            continue;
        }
        let material = models.surface_material(&model_key, surface);
        let receiver_kind = super::world_mark::mark_receiver(scene, material);
        if !marks_t6::mark_include_in_world_clip(marks_t6::mark_allow(receiver_kind, mark_bits)) {
            continue;
        }
        let Some(&(start, count)) = skel.surface_index_ranges.get(surface) else {
            continue;
        };
        let Some(indices) = skel.indices.get(start..start + count) else {
            continue;
        };
        for triangle in indices.chunks_exact(3) {
            let corners = [
                triangle[0] as usize,
                triangle[1] as usize,
                triangle[2] as usize,
            ];
            let bone = match triangle_bone(&skel.vert_skin, corners) {
                TriangleBone::Missing => continue,
                TriangleBone::Unsupported => {
                    state.unsupported_triangles += 1;
                    continue;
                }
                TriangleBone::Rigid(bone) => bone_base + bone,
            };
            if owner.dobj_state.hide_part_bits.get(bone) {
                continue;
            }
            let Some(&matrix) = skin.get(bone) else {
                continue;
            };
            if !has_unit_scale(&matrix) {
                state.unsupported_triangles += 1;
                continue;
            }
            let [Some(p0), Some(p1), Some(p2)] = corners.map(|i| skel.positions.get(i).copied())
            else {
                continue;
            };
            let world =
                [p0, p1, p2].map(|p| matrix.transform_point3(Vec3::from_array(p)).to_array());
            if marks_t6::mark_is_triangle_rejected(request.axis[0], world[0], world[1], world[2]) {
                continue;
            }
            let count = marks_t6::mark_chop_world_triangle_points(
                &planes,
                world[0],
                world[1],
                world[2],
                &mut fragment,
            ) as usize;
            if count < 3 {
                continue;
            }
            let [Some(n0), Some(n1), Some(n2)] = corners.map(|i| skel.normals.get(i).copied())
            else {
                continue;
            };
            let to_bone = matrix.inverse();
            for point in &mut fragment[..count] {
                point.xyz = to_bone
                    .transform_point3(Vec3::from_array(point.xyz))
                    .to_array();
            }
            by_bone.entry(bone).or_default().add_fragment(
                &fragment[..count],
                [n0, n1, n2],
                context,
            );
        }
    }

    for (bone, pieces) in by_bone {
        if pieces.tris.is_empty() {
            continue;
        }
        let to_bone = skin[bone].inverse();
        let alloc = FxAllocMarkRequest {
            any_marks: true,
            tri_count: pieces.tris.len() as u32,
            point_count: pieces.points.len() as u32,
            origin: to_bone
                .transform_point3(Vec3::from_array(request.origin))
                .to_array(),
            radius: request.radius,
            tex_coord_axis: to_bone
                .transform_vector3(Vec3::from_array(request.axis[1]))
                .to_array(),
            native_color: request.color,
            material: 0,
            frame_count: host.frame_stamp,
            first_tri_context: pieces.tris[0].context,
        };
        let Some(handle) =
            host.marks
                .alloc_mark_from_go_callback(alloc, &pieces.tris, &pieces.points)
        else {
            continue;
        };
        host.marks
            .set_material_name(handle, Some(&request.material));
        state.attached.push(EntityMarkAttachment {
            entity: request.entity,
            model: model.to_string(),
            revision,
            bone,
            handle,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use asset_model::VertSkin;

    fn rigid(bone: u16) -> VertSkin {
        VertSkin {
            bones: [bone, 0, 0, 0],
            weights: [1.0, 0.0, 0.0, 0.0],
            weight_u16: [u16::MAX, 0, 0, 0],
        }
    }

    #[test]
    fn model_indices_up_to_the_mask_fit_in_a_mark() {
        let mask = usize::from(marks_t6::MARK_MODEL_DOBJ_MODEL_MASK);
        assert_eq!(dobj_model_index(0), Some(0));
        assert_eq!(dobj_model_index(mask), Some(mask as u8));
        assert_eq!(dobj_model_index(mask + 1), None);
        assert_eq!(dobj_model_index(usize::MAX), None);
    }

    #[test]
    fn a_triangle_on_one_bone_is_rigid() {
        let skins = [rigid(7), rigid(7), rigid(7)];
        assert_eq!(triangle_bone(&skins, [0, 1, 2]), TriangleBone::Rigid(7));
    }

    #[test]
    fn a_triangle_across_bones_is_unsupported() {
        let skins = [rigid(7), rigid(7), rigid(8)];
        assert_eq!(triangle_bone(&skins, [0, 1, 2]), TriangleBone::Unsupported);
    }

    #[test]
    fn a_blended_corner_is_unsupported() {
        let mut blended = rigid(7);
        blended.weights = [0.5, 0.5, 0.0, 0.0];
        let skins = [rigid(7), blended, rigid(7)];
        assert_eq!(triangle_bone(&skins, [0, 1, 2]), TriangleBone::Unsupported);
    }

    #[test]
    fn a_bone_past_255_is_unsupported() {
        let skins = [rigid(256), rigid(256), rigid(256)];
        assert_eq!(triangle_bone(&skins, [0, 1, 2]), TriangleBone::Unsupported);
        let skins = [rigid(255), rigid(255), rigid(255)];
        assert_eq!(triangle_bone(&skins, [0, 1, 2]), TriangleBone::Rigid(255));
    }

    #[test]
    fn a_missing_first_corner_is_skipped_quietly() {
        let skins = [rigid(1)];
        assert_eq!(triangle_bone(&skins, [3, 0, 0]), TriangleBone::Missing);
    }

    #[test]
    fn a_missing_later_corner_is_unsupported() {
        let skins = [rigid(1)];
        assert_eq!(triangle_bone(&skins, [0, 0, 5]), TriangleBone::Unsupported);
    }

    #[test]
    fn only_unscaled_bones_carry_marks() {
        assert!(has_unit_scale(&Mat4::IDENTITY));
        assert!(has_unit_scale(&Mat4::from_translation(Vec3::new(
            5.0, 6.0, 7.0
        ))));
        assert!(has_unit_scale(&Mat4::from_rotation_z(1.0)));
        assert!(!has_unit_scale(&Mat4::from_scale(Vec3::splat(1.01))));
        assert!(!has_unit_scale(&Mat4::from_scale(Vec3::new(1.0, 1.0, 0.5))));
    }

    #[test]
    fn a_fragment_that_fits_is_kept_and_a_second_one_stacks() {
        let mut pieces = BonePieces::default();
        let square = [
            marks_t6::FxWorldMarkPoint::ZERO,
            marks_t6::FxWorldMarkPoint::ZERO,
            marks_t6::FxWorldMarkPoint::ZERO,
        ];
        let context = marks_t6::GfxMarkContext::entity_model(4, 1);
        pieces.add_fragment(&square, [[0.0, 0.0, 1.0]; 3], context);
        let first = (pieces.tris.len(), pieces.points.len());
        assert!(first.0 > 0 && first.1 >= 3);
        pieces.add_fragment(&square, [[0.0, 0.0, 1.0]; 3], context);
        assert!(pieces.tris.len() > first.0);
        assert!(pieces.points.len() > first.1);
    }

    #[test]
    fn an_overfull_bone_keeps_what_it_had() {
        let mut pieces = BonePieces::default();
        let corner = [marks_t6::FxWorldMarkPoint::ZERO; 3];
        let context = marks_t6::GfxMarkContext::entity_model(4, 1);
        for _ in 0..(marks_t6::R_MARK_FRAGMENTS_MAX_TRIS as usize + 8) {
            pieces.add_fragment(&corner, [[0.0, 0.0, 1.0]; 3], context);
        }
        assert!(pieces.tris.len() <= marks_t6::R_MARK_FRAGMENTS_MAX_TRIS as usize);
        assert!(pieces.points.len() <= marks_t6::R_MARK_FRAGMENTS_MAX_POINTS as usize);
        let kept = (pieces.tris.len(), pieces.points.len());
        pieces.add_fragment(&corner, [[0.0, 0.0, 1.0]; 3], context);
        assert_eq!((pieces.tris.len(), pieces.points.len()), kept);
    }
}
