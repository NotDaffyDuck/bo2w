//! Posing a DObj and skinning each of its models at the LOD the viewer sees.

use bevy::prelude::*;
use bevy::render::mesh::Indices;

use super::lod::{DObjLodView, submodel_camera_lod};
use crate::anim::xmodel_pose::{
    PosedModelSurface, lod_local_surface, skin_model_filtered, stream_lod_surface_rigid,
};

/// What posing yields: the skinned surfaces, and for each the material the
/// map authored for it, if any.
type Posed = (Vec<PosedModelSurface>, Vec<Option<assets::MaterialIndex>>);

/// `pose_script_dobj_with_controller` with nothing to move by hand.
pub fn pose_script_dobj_with_materials(
    catalog: Option<&asset_world::MapXModelSceneCatalog>,
    skels: &[&asset_model::ModelSkel],
    dobj: &xmodel_runtime::DObj,
    request: &xmodel_runtime::DObjPoseRequest,
    lod_view: Option<DObjLodView>,
    skin_entries: &[dpvs_t6::SceneEntSkinEntry],
) -> Option<Posed> {
    pose_script_dobj_with_controller(
        catalog,
        skels,
        dobj,
        request,
        lod_view,
        skin_entries,
        |_, _, _| {},
    )
}

/// Poses `dobj` as `request` asks, lets `controller` move parts of the pose
/// (wheels, rotors, a turret), and skins every model that is within sight.
/// `None` if the pose or any model's skinning fails.
pub fn pose_script_dobj_with_controller(
    catalog: Option<&asset_world::MapXModelSceneCatalog>,
    skels: &[&asset_model::ModelSkel],
    dobj: &xmodel_runtime::DObj,
    request: &xmodel_runtime::DObjPoseRequest,
    lod_view: Option<DObjLodView>,
    skin_entries: &[dpvs_t6::SceneEntSkinEntry],
    controller: impl FnOnce(&xmodel_runtime::DObj, &dobj_t6::PartBits, &mut dobj_t6::LocalPose),
) -> Option<Posed> {
    let world =
        xmodel_runtime::pose_dobj_with_controller(dobj, request, Mat4::IDENTITY, controller).ok()?;
    let skin = dobj.skin_matrices(&world);
    let hide = request.hide_part_bits.words();

    let mut surfaces = Vec::new();
    let mut materials = Vec::new();
    for (model, skel) in skels.iter().enumerate() {
        let Some(lod) = submodel_camera_lod(skel, lod_view) else {
            continue;
        };
        if skel.surfaces_for_lod(lod).is_empty() {
            continue;
        }
        let base = dobj.models.get(model)?.base;

        // Black Ops II hides by bone: a surface is dropped only when all of
        // its bones are hidden (a broken mannequin's head shares its material
        // with her body and arms), and one with some hidden loses their
        // triangles.
        let mut posed = skin_model_filtered(
            skel,
            |bone| skin[base + bone],
            &[],
            |surface| {
                !dobj_t6::surface_hidden_whole(&skel.surface_part_bits[surface], hide, base as u32)
                    && dpvs_t6::SceneEntSkinEntry::stream_draws(
                        skin_entries,
                        model as u16,
                        lod_local_surface(skel, lod, surface) as u16,
                    )
            },
            |surface| stream_lod_surface_rigid(skel, lod, skin_entries, model as u16, surface),
            lod,
        )?;

        let key = asset_world::MapXModelAssetKey(skel.name.clone());
        for surface in &mut posed {
            surface.model = model as u16;
            drop_hidden_triangles(skel, surface, hide, base as u32);
            materials.push(
                catalog.and_then(|catalog| catalog.surface_material(&key, surface.surface_index)),
            );
        }
        surfaces.append(&mut posed);
    }
    Some((surfaces, materials))
}

/// The triangles of `indices` (three to a triangle) with no hidden vertex.
fn keep_visible_triangles(indices: &[u32], hidden: impl Fn(u32) -> bool) -> Vec<u32> {
    indices
        .chunks_exact(3)
        .filter(|triangle| !triangle.iter().any(|&vertex| hidden(vertex)))
        .flatten()
        .copied()
        .collect()
}

/// A rigid surface is skinned a vertex list (one bone) at a time and the
/// lists of hidden bones are skipped, with their triangles. A deformed
/// surface is skinned whole, and a surface with no hidden part is left alone.
fn drop_hidden_triangles(
    skel: &asset_model::ModelSkel,
    surface: &mut PosedModelSurface,
    hide: &[u32; 5],
    base: u32,
) {
    let index = surface.surface_index;
    let rigid = skel.surface_deformed.get(index).copied().flatten() == Some(false);
    if !rigid || !dobj_t6::surface_hidden(&skel.surface_part_bits[index], hide, base) {
        return;
    }
    let Some(&(first, _)) = skel.surface_vertex_ranges.get(index) else {
        return;
    };
    let Some(Indices::U32(indices)) = surface.mesh.indices() else {
        return;
    };
    let kept = keep_visible_triangles(indices, |local| {
        skel.vert_skin.get(first + local as usize).is_some_and(|skin| {
            dobj_t6::hide_part_bit(hide, base as usize + usize::from(skin.bones[0]))
        })
    });
    surface.mesh.insert_indices(Indices::U32(kept));
}

#[cfg(test)]
mod tests {
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
        assert_eq!(keep_visible_triangles(&indices, |_| false), indices.to_vec());
    }

    #[test]
    fn a_trailing_partial_triangle_is_dropped() {
        assert_eq!(keep_visible_triangles(&[0, 1, 2, 3, 4], |_| false), vec![0, 1, 2]);
    }
}
