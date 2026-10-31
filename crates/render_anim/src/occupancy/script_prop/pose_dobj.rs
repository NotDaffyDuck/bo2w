//! Posing a DObj and skinning each of its models at the LOD the viewer sees.

use bevy::prelude::*;
use bevy::render::mesh::Indices;

use super::lod::{DetailLevelWindow, submodel_camera_lod};
use crate::motion::figure_pose::{
    SkinnedMeshFacet, lod_surf_slot, skin_chosen_surfaces, rigid_lod_surface,
};

/// `pose_script_dobj_with_controller` with nothing to move by hand.
pub fn pose_script_skeleton_and_materials(
    catalog: Option<&asset_world::LevelModelPlacementRoster>,
    skels: &[&asset_model::MeshRig],
    dobj: &xmodel_runtime::PosedSkeleton,
    ask: &xmodel_runtime::ModelPoseQuery,
    view: Option<DetailLevelWindow>,
    streams: &[dpvs_t6::EntitySkinSlotRecord],
) -> Option<(Vec<SkinnedMeshFacet>, Vec<Option<assets::SurfaceMaterialId>>)> {
    pose_script_dobj_with_controller(catalog, skels, dobj, ask, view, streams, |_, _, _| {})
}

/// Poses `dobj` as `request` asks, lets `controller` move parts of the pose
/// (wheels, rotors, a turret), and skins every model that is within sight.
/// `None` if the pose or any model's skinning fails.
pub fn pose_script_dobj_with_controller(
    catalog: Option<&asset_world::LevelModelPlacementRoster>,
    skels: &[&asset_model::MeshRig],
    dobj: &xmodel_runtime::PosedSkeleton,
    ask: &xmodel_runtime::ModelPoseQuery,
    view: Option<DetailLevelWindow>,
    streams: &[dpvs_t6::EntitySkinSlotRecord],
    controller: impl FnOnce(&xmodel_runtime::PosedSkeleton, &dobj_t6::PartSet, &mut dobj_t6::LocalPose),
) -> Option<(Vec<SkinnedMeshFacet>, Vec<Option<assets::SurfaceMaterialId>>)> {
    let identity = Mat4::IDENTITY;
    let world = xmodel_runtime::pose_skeleton_with_aim(dobj, ask, identity, controller).ok()?;
    skin_script_dobj_with_materials(catalog, skels, dobj, &world, &ask.hidden_mask, view, streams)
}

/// bo2zm: the surfaces of every model of `dobj` that is within sight, skinned by its bones'
/// joint_transforms `world` (model space, as `pose_dobj` gives them) with the parts in
/// `hide_part_bits` left out. `None` if any model's skinning fails.
pub(crate) fn skin_script_dobj_with_materials(
    catalog: Option<&asset_world::LevelModelPlacementRoster>,
    skels: &[&asset_model::MeshRig],
    dobj: &xmodel_runtime::PosedSkeleton,
    world_mats: &[Mat4],
    hide_part_bits: &xmodel_runtime::HiddenParts,
    view: Option<DetailLevelWindow>,
    streams: &[dpvs_t6::EntitySkinSlotRecord],
) -> Option<(Vec<SkinnedMeshFacet>, Vec<Option<assets::SurfaceMaterialId>>)> {
    let sight = Sight {
        dobj,
        joint_transforms: dobj.skin_matrices(world_mats),
        hide: hide_part_bits.words(),
        streams,
        view,
    };

    let mut out: (Vec<SkinnedMeshFacet>, Vec<Option<assets::SurfaceMaterialId>>) = Default::default();
    for (model, bone_rig) in skels.iter().enumerate() {
        let Some(posed) = sight.skin(bone_rig, model)? else {
            continue;
        };
        let key = asset_world::LevelModelAssetId(bone_rig.name.clone());
        let material_of = |surface: &SkinnedMeshFacet| {
            catalog.and_then(|map| map.facet_material(&key, surface.surface_slot_number))
        };
        out.1.extend(posed.iter().map(material_of));
        out.0.extend(posed);
    }
    Some(out)
}

/// What every model of one DObj is skinned against.
struct Sight<'a> {
    dobj: &'a xmodel_runtime::PosedSkeleton,
    joint_transforms: Vec<Mat4>,
    hide: &'a [u32; 5],
    streams: &'a [dpvs_t6::EntitySkinSlotRecord],
    view: Option<DetailLevelWindow>,
}

impl Sight<'_> {
    /// One model's posed surfaces, or `Ok(None)` (as `Some(None)`) when it
    /// is too far away to show anything; `None` when skinning failed.
    fn skin(
        &self,
        bone_rig: &asset_model::MeshRig,
        xmodel: usize,
    ) -> Option<Option<Vec<SkinnedMeshFacet>>> {
        let Some(lod) = submodel_camera_lod(bone_rig, self.view) else {
            return Some(None);
        };
        if bone_rig.facets_at_tier(lod).is_empty() {
            return Some(None);
        }
        let base = self.dobj.models.get(xmodel)?.first_joint;
        let model_id = xmodel as u16;
        let streams = self.streams;
        let hide = self.hide;

        // Black Ops II hides by bone: a surface is dropped only when all of
        // its bones are hidden (a broken mannequin's head shares its material
        // with her body and arms), and one with some hidden loses their
        // triangles.
        let shown = |surface: usize| {
            let local = lod_surf_slot(bone_rig, lod, surface) as u16;
            let whole =
                dobj_t6::all_parts_hidden(&bone_rig.face_parts[surface], hide, base as u32);
            dpvs_t6::EntitySkinSlotRecord::draws_streamed(streams, model_id, local) && !whole
        };
        let rigid =
            |surface: usize| rigid_lod_surface(bone_rig, lod, streams, model_id, surface);
        let bone_matrix = |bone: usize| self.joint_transforms[base + bone];
        let mut posed = skin_chosen_surfaces(bone_rig, bone_matrix, &[], shown, rigid, lod)?;
        for surface in posed.iter_mut() {
            surface.mdl = model_id;
            drop_hidden_triangles(bone_rig, surface, hide, base as u32);
        }
        Some(Some(posed))
    }
}

/// The triangles of `indices` (three to a triangle) with no hidden vertex.
fn keep_visible_triangles(indices: &[u32], hidden: impl Fn(u32) -> bool) -> Vec<u32> {
    let mut kept = Vec::with_capacity(indices.len());
    for triangle_in in indices.chunks_exact(3) {
        if triangle_in.iter().all(|&vertex| !hidden(vertex)) {
            kept.extend_from_slice(triangle_in);
        }
    }
    kept
}

/// A rigid surface is skinned a vertex list (one bone) at a time and the
/// lists of hidden bones are skipped, with their triangles. A deformed
/// surface is skinned whole, and a surface with no hidden part is left alone.
fn drop_hidden_triangles(
    bone_rig: &asset_model::MeshRig,
    surf: &mut SkinnedMeshFacet,
    hidden_bits: &[u32; 5],
    base: u32,
) {
    let index = surf.surface_slot_number;
    let is_rigid = matches!(bone_rig.bent.get(index), Some(Some(false)));
    if !is_rigid || !dobj_t6::any_part_hidden(&bone_rig.face_parts[index], hidden_bits, base) {
        return;
    }
    let Some((first, _)) = bone_rig.facet_vertex_ranges.get(index).copied() else {
        return;
    };
    let Some(Indices::U32(list)) = surf.mesh.indices() else {
        return;
    };
    let bone_hidden = |local: u32| {
        let Some(skin) = bone_rig.vert_skin.get(first + local as usize) else {
            return false;
        };
        dobj_t6::part_is_hidden(hidden_bits, base as usize + usize::from(skin.joints[0]))
    };
    let kept = keep_visible_triangles(list, bone_hidden);
    surf.mesh.insert_indices(Indices::U32(kept));
}

mod tests {
    #![cfg(test)]
    use super::*;

    #[test]
    fn a_triangle_with_one_hidden_vertex_goes() {
        let indices = [0, 1, 2, 2, 3, 4, 5, 6, 7];
        let kept = keep_visible_triangles(&indices, |vertex| vertex == 3);
        assert_eq!(kept, vec![0, 1, 2, 5, 6, 7]);
    }

    #[test]
    fn nothing_hidden_keeps_every_triangle() {
        let indices = [0, 1, 2, 3, 4, 5];
        assert_eq!(
            keep_visible_triangles(&indices, |_| false),
            indices.to_vec()
        );
    }

    #[test]
    fn a_trailing_partial_triangle_is_dropped() {
        assert_eq!(
            keep_visible_triangles(&[0, 1, 2, 3, 4], |_| false),
            vec![0, 1, 2]
        );
    }
}
