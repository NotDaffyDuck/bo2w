//! Lighting query boxes for posed models, from the radii their skeletons carry.

use asset_model::{FirstPersonMeshRoster, MeshRig};
use asset_world::{LevelModelAssetId, LevelModelPlacementAsset, LevelModelPlacementRoster};
use assets::FpvMeshSlot;
use dobj_t6::{DOBJ_NO_ATTACH, RadiusModel, compute_radius};
use lighting_t6::illum_query_half_size;

/// The hands bone a viewmodel's gun sits on.
const GUN_BONE: &str = "tag_weapon";

/// The query box around a DObj of the given radius.
pub fn radius_lighting_box_half(reach: f32) -> [f32; 3] {
    illum_query_half_size(reach)
}

/// The rest-pose translation of every bone of `bone_rig`.
pub fn base_trans(bone_rig: &MeshRig) -> Vec<[f32; 3]> {
    bone_rig.bones.iter().map(|bone| bone.shift_offset).collect()
}

/// The attach byte for a model sitting on `tag` of `bone_rig`: the bone's index,
/// or no attach when the skeleton has no such bone (or too many bones to
/// number it).
pub fn attach_bone(bone_rig: &MeshRig, tag: &str) -> u8 {
    bone_rig.joint_labels
        .iter()
        .position(|name| name == tag)
        .and_then(|bone| u8::try_from(bone).ok())
        .unwrap_or(DOBJ_NO_ATTACH)
}

/// The radius of a DObj whose first entry is model 0. Every other entry
/// carries the tag of model 0 it sits on, if any. `None` when there is no
/// model or one of them has no radius.
fn composed_radius(models: &[(&MeshRig, Option<&str>)]) -> Option<f32> {
    let (model0, _) = models.first()?;
    let radii = models
        .iter()
        .map(|&(bone_rig, tag)| {
            Some(RadiusModel {
                stored_radius: bone_rig.radius?,
                attach: tag.map_or(DOBJ_NO_ATTACH, |tag| attach_bone(model0, tag)),
            })
        })
        .collect::<Option<Vec<_>>>()?;
    Some(compute_radius(&radii, &base_trans(model0)))
}

/// The query box of a script model's DObj. Every model must be a loaded
/// skeleton with a radius; a model counts as attached only when it sits on
/// a tag of model 0.
pub fn script_prop_light_extent(
    catalog: &LevelModelPlacementRoster,
    models: &[xmodel_runtime::RigMeshEntry],
) -> Option<[f32; 3]> {
    let mut placed = Vec::with_capacity(models.len());
    for descriptor in models {
        let asset = catalog.get(&LevelModelAssetId(descriptor.model.clone()))?;
        let LevelModelPlacementAsset::T5(bone_rig) = asset else {
            return None;
        };
        let tag = match descriptor.host_index {
            Some(0) => descriptor.mount_tag.as_deref(),
            _ => None,
        };
        placed.push((&**bone_rig, tag));
    }
    composed_radius(&placed).map(illum_query_half_size)
}

/// Where a DObj's light is read, from its origin in its own axes: model 0's
/// lighting origin offset, or the middle of its box when that is unset. Read
/// at the origin itself, a big model standing on it (Takeoff's shuttle) is lit
/// from inside the floor under it and draws black.
pub fn model_light_offset(model0: &MeshRig) -> [f32; 3] {
    match (model0.light_offset, model0.bounds) {
        (Some(offset), _) if offset != [0.0; 3] => offset,
        (_, Some((mid, _))) => mid,
        _ => [0.0; 3],
    }
}

/// The viewmodel DObj radius from its parts: the hands, plus the gun on the
/// hands' gun bone. With only one part known its own radius stands in; a
/// hands skeleton without a radius leaves the gun's.
fn viewmodel_radius(hands: Option<&MeshRig>, gun_radius: Option<f32>) -> Option<f32> {
    let (hands, gun) = match (hands, gun_radius) {
        (None, gun) => return gun,
        (Some(hands), None) => return hands.radius,
        (Some(hands), Some(gun)) => (hands, gun),
    };
    let Some(hands_radius) = hands.radius else {
        return Some(gun);
    };
    let parts = [
        RadiusModel::unattached(hands_radius),
        RadiusModel {
            stored_radius: gun,
            attach: attach_bone(hands, GUN_BONE),
        },
    ];
    Some(compute_radius(&parts, &base_trans(hands)))
}

fn skel_radius(catalog: &FirstPersonMeshRoster, index: FpvMeshSlot) -> Option<f32> {
    catalog.at_row(index.order())?.bone_rig.radius
}

/// The query box of the viewmodel DObj; both parts must be known and have a
/// radius.
pub fn viewmodel_light_extent(
    catalog: &FirstPersonMeshRoster,
    hands_index: FpvMeshSlot,
    gun_index: FpvMeshSlot,
) -> Option<[f32; 3]> {
    let hands = catalog.at_row(hands_index.order())?;
    let gun = skel_radius(catalog, gun_index)?;
    hands.bone_rig.radius?;
    viewmodel_radius(Some(&hands.bone_rig), Some(gun)).map(illum_query_half_size)
}

/// The viewmodel DObj radius; see [`viewmodel_radius`] for what a missing
/// part does.
pub fn gun_model_reach(
    catalog: &FirstPersonMeshRoster,
    hands_index: FpvMeshSlot,
    gun_index: FpvMeshSlot,
) -> Option<f32> {
    let hands = catalog
        .at_row(hands_index.order())
        .map(|entry| &*entry.bone_rig);
    viewmodel_radius(hands, skel_radius(catalog, gun_index))
}

mod tests {
    #![cfg(test)]
    use super::*;

    #[test]
    fn a_model_is_lit_at_its_offset_else_its_middle() {
        let mut model = MeshRig { bounds: Some(([10.0, 0.0, 1300.0], [600.0, 600.0, 1300.0])), ..MeshRig::default() };
        assert_eq!(model_light_offset(&model), [10.0, 0.0, 1300.0]);
        model.light_offset = Some([0.0; 3]);
        assert_eq!(model_light_offset(&model), [10.0, 0.0, 1300.0], "an unset offset falls back to the middle");
        model.light_offset = Some([0.0, 0.0, 900.0]);
        assert_eq!(model_light_offset(&model), [0.0, 0.0, 900.0]);
        assert_eq!(model_light_offset(&MeshRig::default()), [0.0; 3]);
    }
    use asset_model::JointAttachment;

    fn bone_rig(reach: Option<f32>, bones: &[(&str, [f32; 3])]) -> MeshRig {
        MeshRig {
            name: "test".to_owned(),
            bones: bones
                .iter()
                .map(|&(_, shift_offset)| JointAttachment {
                    spin: [0.0, 0.0, 0.0, 1.0],
                    shift_offset,
                })
                .collect::<_>(),
            joint_hit_volume: Vec::new(),
            joint_labels: bones.iter().map(|&(name, _)| name.to_owned()).collect(),
            eye_joint: None,
            gun_joint: None,
            pose: None,
            positions: Vec::new(),
            surface_directions: Vec::new(),
            colors: Vec::new(),
            texcoord_set: Vec::new(),
            indices: Vec::new(),
            facet_materials: Vec::new(),
            facet_vertex_ranges: Vec::new(),
            facet_index_ranges: Vec::new(),
            face_parts: Vec::new(),
            bent: Vec::new(),
            vert_lists: Vec::new(),
            vert_skin: Vec::new(),
            compact_vertices: Vec::new(),
            radius: reach,
            bounds: None,
            light_offset: None,
            contents: None,
            collision_detail_tier: 0,
            face_bits: 0,
            collision_faces: Vec::new(),
            collmap: None,
            lod: None,
            detail_prop_cache: None,
            detail_face_extent: [(0, 0); 4],
        }
    }

    fn hands(reach: Option<f32>) -> MeshRig {
        bone_rig(reach, &[("root", [0.0; 3]), (GUN_BONE, [3.0, 4.0, 0.0])])
    }

    #[test]
    fn the_box_is_a_cube_of_the_radius() {
        assert_eq!(illum_query_half_size(12.5), [12.5; 3]);
    }

    #[test]
    fn base_trans_lists_every_bone() {
        let rig = bone_rig(None, &[("a", [1.0, 2.0, 3.0]), ("b", [4.0, 5.0, 6.0])]);
        assert_eq!(base_trans(&rig), [[1.0, 2.0, 3.0], [4.0, 5.0, 6.0]]);
        assert!(base_trans(&bone_rig(None, &[])).is_empty());
    }

    #[test]
    fn attach_bone_finds_a_tag_by_name() {
        let rig = hands(None);
        assert_eq!(attach_bone(&rig, "root"), 0);
        assert_eq!(attach_bone(&rig, GUN_BONE), 1);
        assert_eq!(attach_bone(&rig, "tag_missing"), DOBJ_NO_ATTACH);
    }

    #[test]
    fn a_bone_too_high_to_number_does_not_attach() {
        let names = (0..300).map(|i| format!("bone{i}")).collect::<Vec<_>>();
        let mut rig = bone_rig(None, &[]);
        rig.joint_labels = names;
        assert_eq!(attach_bone(&rig, "bone299"), DOBJ_NO_ATTACH);
        assert_eq!(attach_bone(&rig, "bone12"), 12);
    }

    #[test]
    fn composed_radius_needs_models_with_radii() {
        assert_eq!(composed_radius(&[]), None);
        let blank = bone_rig(None, &[]);
        assert_eq!(composed_radius(&[(&blank, None)]), None);
        let lone = bone_rig(Some(8.0), &[]);
        assert_eq!(composed_radius(&[(&lone, None)]), Some(8.0));
        assert_eq!(composed_radius(&[(&lone, None), (&blank, None)]), None);
    }

    #[test]
    fn an_attached_model_adds_its_bone_offset() {
        let body = hands(Some(10.0));
        let prop = bone_rig(Some(2.0), &[]);
        assert_eq!(
            composed_radius(&[(&body, None), (&prop, Some(GUN_BONE))]),
            Some(10.0)
        );
        let big = bone_rig(Some(9.0), &[]);
        assert_eq!(
            composed_radius(&[(&body, None), (&big, Some(GUN_BONE))]),
            Some(14.0)
        );
        assert_eq!(
            composed_radius(&[(&body, None), (&big, Some("nowhere"))]),
            Some(10.0)
        );
    }

    #[test]
    fn one_known_viewmodel_part_stands_alone() {
        let rig = hands(Some(6.0));
        assert_eq!(viewmodel_radius(None, None), None);
        assert_eq!(viewmodel_radius(None, Some(4.0)), Some(4.0));
        assert_eq!(viewmodel_radius(Some(&rig), None), Some(6.0));
        assert_eq!(viewmodel_radius(Some(&hands(None)), None), None);
    }

    #[test]
    fn the_gun_rides_the_hands_gun_bone() {
        let rig = hands(Some(6.0));
        assert_eq!(viewmodel_radius(Some(&rig), Some(4.0)), Some(9.0));
        assert_eq!(viewmodel_radius(Some(&rig), Some(0.5)), Some(6.0));
    }

    #[test]
    fn a_gun_without_a_gun_bone_counts_alone() {
        let bare = bone_rig(Some(6.0), &[("root", [0.0; 3])]);
        assert_eq!(viewmodel_radius(Some(&bare), Some(8.0)), Some(8.0));
    }

    #[test]
    fn hands_without_a_radius_leave_the_gun() {
        assert_eq!(viewmodel_radius(Some(&hands(None)), Some(4.0)), Some(4.0));
    }
}
