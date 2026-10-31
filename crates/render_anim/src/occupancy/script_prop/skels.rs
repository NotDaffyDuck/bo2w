//! The skeletons a DObj's models resolve to. All three lookups are all or
//! nothing: one unknown or unposed model, or an attachment that cannot be
//! made, and there is no answer.

use std::sync::Arc;

use asset_model::MeshRig;
use asset_world::{LevelModelPlacementAsset, LevelModelPlacementRoster};
use xmodel_runtime::{MountedModel, RigMeshEntry, RigPoseRecord, PoseDonorModel};

/// The posable skeleton the catalog holds under `model`.
fn posed_skel<'a>(catalog: &'a LevelModelPlacementRoster, mdl: &str) -> Option<&'a Arc<MeshRig>> {
    let LevelModelPlacementAsset::T5(bone_rig) = catalog.look_up_label(mdl)? else {
        return None;
    };
    bone_rig.pose.is_some().then_some(bone_rig)
}

/// Shared handles to the skeleton of each model of `state`.
pub(crate) fn shown_skeleton_handles(
    catalog: &LevelModelPlacementRoster,
    state: &RigPoseRecord,
) -> Option<Vec<Arc<MeshRig>>> {
    let models = &state.composition.models;
    models
        .iter()
        .map(|of| posed_skel(catalog, &of.model).cloned())
        .collect::<_>()
}

/// Borrowed skeletons of each model of `state`.
pub(crate) fn shown_skeletons<'a>(
    catalog: &'a LevelModelPlacementRoster,
    state: &RigPoseRecord,
) -> Option<Vec<&'a MeshRig>> {
    let models = &state.composition.models;
    models
        .iter()
        .map(|of| Some(posed_skel(catalog, &of.model)?.as_ref()))
        .collect::<_>()
}

/// How the model at `index` hangs on the models before it: `Some(None)` for
/// the root, `Some(Some(_))` for a model on a tag of an earlier one, `None`
/// for anything else.
fn attach_for(index: usize, descriptor: &RigMeshEntry) -> Option<Option<MountedModel>> {
    let RigMeshEntry {
        host_index: parent_model,
        mount_tag: attach_tag,
        ..
    } = descriptor;
    match (parent_model, attach_tag) {
        (None, None) => (index == 0).then_some(None),
        (Some(owner), Some(tag_name)) => {
            let parent_model = usize::from(*owner);
            let hangs_on_earlier = parent_model < index;
            hangs_on_earlier.then(|| {
                Some(MountedModel {
                    mount_tag: tag_name.clone(),
                    host_model: parent_model,
                })
            })
        }
        _ => None,
    }
}

/// What a DObj is built from, and the skeletons of its models.
pub fn gather_shown_models<'a>(
    catalog: &'a LevelModelPlacementRoster,
    state: &RigPoseRecord,
) -> Option<(Vec<(&'a PoseDonorModel, Option<MountedModel>)>, Vec<&'a MeshRig>)> {
    let models = &state.composition.models;
    let mut parts = Vec::with_capacity(models.len());
    for (index, descriptor) in models.iter().enumerate() {
        let mut attach = attach_for(index, descriptor)?;
        // bo2zm: a weapon part (magazine, scope) goes on its gun's bone of that name in any
        // case, as first person mounts it (`fpv_catalog`: a def's `TAG_CLIP` is the model's
        // `tag_clip`); on a tag its gun lacks it hangs on the gun's root bone rather than
        // losing the whole gun.
        if let (Some(attach), Some(_)) = (attach.as_mut(), descriptor.tag_offset) {
            let parent: Option<&(_, &MeshRig)> = parts.get(attach.host_model);
            if let Some(bones) = parent.and_then(|(_, bone_rig)| bone_rig.pose.as_ref()).map(|pose| &pose.joint_labels) {
                let found = bones.iter().find(|bone| bone.eq_ignore_ascii_case(&attach.mount_tag));
                attach.mount_tag = found.or_else(|| bones.first()).unwrap_or(&attach.mount_tag).clone();
            }
        }
        let bone_rig = posed_skel(catalog, &descriptor.model)?;
        let pose = bone_rig.pose.as_ref()?;
        parts.push(((pose, attach), bone_rig.as_ref()));
    }
    Some(parts.into_iter().unzip())
}

mod tests {
    #![cfg(test)]
    use super::*;

    fn hung(owner: Option<u16>, tag: Option<&str>) -> RigMeshEntry {
        RigMeshEntry {
            no_hits: false,
            mount_tag: tag.map(String::from),
            host_index: owner,
            model: String::from("m"),
            tag_offset: None,
        }
    }

    #[test]
    fn the_first_model_is_the_root() {
        let root = attach_for(0, &hung(None, None));
        assert!(root.is_some_and(|attach| attach.is_none()));
    }

    #[test]
    fn the_root_cannot_hang_on_anything() {
        assert!(attach_for(0, &hung(Some(0), Some("tag"))).is_none());
    }

    #[test]
    fn a_later_model_hangs_on_a_tag_of_an_earlier_one() {
        let attach = attach_for(2, &hung(Some(1), Some("tag_weapon")))
            .flatten()
            .expect("an attachment");
        assert_eq!(attach.mount_tag, "tag_weapon");
        assert_eq!(attach.host_model, 1);
    }

    #[test]
    fn a_later_model_needs_an_earlier_parent_and_a_tag() {
        let refused = [
            hung(Some(1), Some("tag")),
            hung(Some(0), None),
            hung(None, Some("tag")),
            hung(None, None),
        ];
        for descriptor in &refused {
            assert!(attach_for(1, descriptor).is_none());
        }
    }
}
