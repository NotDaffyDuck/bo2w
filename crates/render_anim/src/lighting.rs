//! Lighting query boxes for posed models, from the radii their skeletons carry.

use asset_model::{FpvMeshCatalog, ModelSkel};
use asset_world::{MapXModelAssetKey, MapXModelSceneAsset, MapXModelSceneCatalog};
use assets::FpvMeshIndex;
use dobj_t6::{DOBJ_NO_ATTACH, RadiusModel, compute_radius};
use lighting_t6::lighting_query_box_half;

/// The hands bone a viewmodel's gun sits on.
const GUN_BONE: &str = "tag_weapon";

/// The query box around a DObj of the given radius.
pub fn radius_lighting_box_half(radius: f32) -> [f32; 3] {
    lighting_query_box_half(radius)
}

/// The rest-pose translation of every bone of `skel`.
pub fn base_trans(skel: &ModelSkel) -> Vec<[f32; 3]> {
    skel.bones.iter().map(|bone| bone.trans).collect()
}

/// The attach byte for a model sitting on `tag` of `skel`: the bone's index,
/// or no attach when the skeleton has no such bone (or too many bones to
/// number it).
pub fn attach_bone(skel: &ModelSkel, tag: &str) -> u8 {
    skel.bone_names
        .iter()
        .position(|name| name == tag)
        .and_then(|bone| u8::try_from(bone).ok())
        .unwrap_or(DOBJ_NO_ATTACH)
}

/// The radius of a DObj whose first entry is model 0. Every other entry
/// carries the tag of model 0 it sits on, if any. `None` when there is no
/// model or one of them has no radius.
fn composed_radius(models: &[(&ModelSkel, Option<&str>)]) -> Option<f32> {
    let (model0, _) = models.first()?;
    let radii = models
        .iter()
        .map(|&(skel, tag)| {
            Some(RadiusModel {
                radius: skel.radius?,
                attach: tag.map_or(DOBJ_NO_ATTACH, |tag| attach_bone(model0, tag)),
            })
        })
        .collect::<Option<Vec<_>>>()?;
    Some(compute_radius(&radii, &base_trans(model0)))
}

/// The query box of a script model's DObj. Every model must be a loaded
/// skeleton with a radius; a model counts as attached only when it sits on
/// a tag of model 0.
pub fn script_model_lighting_box_half(
    catalog: &MapXModelSceneCatalog,
    models: &[xmodel_runtime::DObjModelDescriptor],
) -> Option<[f32; 3]> {
    let mut placed = Vec::with_capacity(models.len());
    for descriptor in models {
        let asset = catalog.get(&MapXModelAssetKey(descriptor.model.clone()))?;
        let MapXModelSceneAsset::T5(skel) = asset else {
            return None;
        };
        let tag = match descriptor.parent_model {
            Some(0) => descriptor.attach_tag.as_deref(),
            _ => None,
        };
        placed.push((&**skel, tag));
    }
    composed_radius(&placed).map(lighting_query_box_half)
}

/// The viewmodel DObj radius from its parts: the hands, plus the gun on the
/// hands' gun bone. With only one part known its own radius stands in; a
/// hands skeleton without a radius leaves the gun's.
fn viewmodel_radius(hands: Option<&ModelSkel>, gun_radius: Option<f32>) -> Option<f32> {
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
            radius: gun,
            attach: attach_bone(hands, GUN_BONE),
        },
    ];
    Some(compute_radius(&parts, &base_trans(hands)))
}

fn skel_radius(catalog: &FpvMeshCatalog, index: FpvMeshIndex) -> Option<f32> {
    catalog.get_at(index.order())?.skel.radius
}

/// The query box of the viewmodel DObj; both parts must be known and have a
/// radius.
pub fn fpv_dobj_lighting_box_half(
    catalog: &FpvMeshCatalog,
    hands_index: FpvMeshIndex,
    gun_index: FpvMeshIndex,
) -> Option<[f32; 3]> {
    let hands = catalog.get_at(hands_index.order())?;
    let gun = skel_radius(catalog, gun_index)?;
    hands.skel.radius?;
    viewmodel_radius(Some(&hands.skel), Some(gun)).map(lighting_query_box_half)
}

/// The viewmodel DObj radius; see [`viewmodel_radius`] for what a missing
/// part does.
pub fn fpv_dobj_radius(
    catalog: &FpvMeshCatalog,
    hands_index: FpvMeshIndex,
    gun_index: FpvMeshIndex,
) -> Option<f32> {
    let hands = catalog
        .get_at(hands_index.order())
        .map(|entry| &*entry.skel);
    viewmodel_radius(hands, skel_radius(catalog, gun_index))
}

#[cfg(test)]
mod tests {
    use super::*;
    use asset_model::BoneBind;

    fn skel(radius: Option<f32>, bones: &[(&str, [f32; 3])]) -> ModelSkel {
        ModelSkel {
            name: "test".to_owned(),
            bones: bones
                .iter()
                .map(|&(_, trans)| BoneBind {
                    quat: [0.0, 0.0, 0.0, 1.0],
                    trans,
                })
                .collect(),
            bone_collision: Vec::new(),
            bone_names: bones.iter().map(|&(name, _)| name.to_owned()).collect(),
            tag_view: None,
            tag_weapon: None,
            pose: None,
            positions: Vec::new(),
            normals: Vec::new(),
            colors: Vec::new(),
            uvs: Vec::new(),
            indices: Vec::new(),
            surface_materials: Vec::new(),
            surface_vertex_ranges: Vec::new(),
            surface_index_ranges: Vec::new(),
            surface_part_bits: Vec::new(),
            surface_deformed: Vec::new(),
            surface_vert_list_count: Vec::new(),
            vert_skin: Vec::new(),
            packed_vertices: Vec::new(),
            radius,
            bounds: None,
            contents: None,
            coll_lod: 0,
            coll_surfs: Vec::new(),
            collmap: None,
            lod: None,
            lod_smc: None,
            lod_surf_span: [(0, 0); 4],
        }
    }

    fn hands(radius: Option<f32>) -> ModelSkel {
        skel(radius, &[("root", [0.0; 3]), (GUN_BONE, [3.0, 4.0, 0.0])])
    }

    #[test]
    fn the_box_is_a_cube_of_the_radius() {
        assert_eq!(radius_lighting_box_half(12.5), [12.5; 3]);
    }

    #[test]
    fn base_trans_lists_every_bone() {
        let rig = skel(None, &[("a", [1.0, 2.0, 3.0]), ("b", [4.0, 5.0, 6.0])]);
        assert_eq!(base_trans(&rig), [[1.0, 2.0, 3.0], [4.0, 5.0, 6.0]]);
        assert!(base_trans(&skel(None, &[])).is_empty());
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
        let mut rig = skel(None, &[]);
        rig.bone_names = names;
        assert_eq!(attach_bone(&rig, "bone299"), DOBJ_NO_ATTACH);
        assert_eq!(attach_bone(&rig, "bone12"), 12);
    }

    #[test]
    fn composed_radius_needs_models_with_radii() {
        assert_eq!(composed_radius(&[]), None);
        let blank = skel(None, &[]);
        assert_eq!(composed_radius(&[(&blank, None)]), None);
        let lone = skel(Some(8.0), &[]);
        assert_eq!(composed_radius(&[(&lone, None)]), Some(8.0));
        assert_eq!(composed_radius(&[(&lone, None), (&blank, None)]), None);
    }

    #[test]
    fn an_attached_model_adds_its_bone_offset() {
        let body = hands(Some(10.0));
        let prop = skel(Some(2.0), &[]);
        assert_eq!(
            composed_radius(&[(&body, None), (&prop, Some(GUN_BONE))]),
            Some(10.0)
        );
        let big = skel(Some(9.0), &[]);
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
        let bare = skel(Some(6.0), &[("root", [0.0; 3])]);
        assert_eq!(viewmodel_radius(Some(&bare), Some(8.0)), Some(8.0));
    }

    #[test]
    fn hands_without_a_radius_leave_the_gun() {
        assert_eq!(viewmodel_radius(Some(&hands(None)), Some(4.0)), Some(4.0));
    }
}
