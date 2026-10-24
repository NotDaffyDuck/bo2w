//! The skeletons a DObj's models resolve to. All three lookups are all or
//! nothing: one unknown or unposed model, or an attachment that cannot be
//! made, and there is no answer.

use std::sync::Arc;

use asset_model::ModelSkel;
use asset_world::{MapXModelSceneAsset, MapXModelSceneCatalog};
use xmodel_runtime::{Attach, DObjModelDescriptor, DObjSemanticState, ModelPoseSrc};

/// The posable skeleton the catalog holds under `model`.
fn posed_skel<'a>(catalog: &'a MapXModelSceneCatalog, model: &str) -> Option<&'a Arc<ModelSkel>> {
    match catalog.get_name(model)? {
        MapXModelSceneAsset::T5(skel) if skel.pose.is_some() => Some(skel),
        _ => None,
    }
}

/// Shared handles to the skeleton of each model of `state`.
pub(crate) fn presented_skel_arcs(
    catalog: &MapXModelSceneCatalog,
    state: &DObjSemanticState,
) -> Option<Vec<Arc<ModelSkel>>> {
    state
        .composition
        .models
        .iter()
        .map(|descriptor| posed_skel(catalog, &descriptor.model).map(Arc::clone))
        .collect()
}

/// Borrowed skeletons of each model of `state`.
pub(crate) fn presented_skels<'a>(
    catalog: &'a MapXModelSceneCatalog,
    state: &DObjSemanticState,
) -> Option<Vec<&'a ModelSkel>> {
    state
        .composition
        .models
        .iter()
        .map(|descriptor| posed_skel(catalog, &descriptor.model).map(|skel| skel.as_ref()))
        .collect()
}

/// How the model at `index` hangs on the models before it: `Some(None)` for
/// the root, `Some(Some(_))` for a model on a tag of an earlier one, `None`
/// for anything else.
fn attach_for(index: usize, descriptor: &DObjModelDescriptor) -> Option<Option<Attach>> {
    match (descriptor.parent_model, descriptor.attach_tag.as_ref()) {
        (None, None) if index == 0 => Some(None),
        (Some(parent), Some(tag)) if usize::from(parent) < index => Some(Some(Attach {
            parent_model: usize::from(parent),
            tag: tag.clone(),
        })),
        _ => None,
    }
}

/// What a DObj is built from, and the skeletons of its models.
pub fn collect_presented_models<'a>(
    catalog: &'a MapXModelSceneCatalog,
    state: &DObjSemanticState,
) -> Option<(Vec<(&'a ModelPoseSrc, Option<Attach>)>, Vec<&'a ModelSkel>)> {
    let mut specs = Vec::new();
    let mut skels = Vec::new();
    for (index, descriptor) in state.composition.models.iter().enumerate() {
        let attach = attach_for(index, descriptor)?;
        let skel = posed_skel(catalog, &descriptor.model)?;
        specs.push((skel.pose.as_ref()?, attach));
        skels.push(skel.as_ref());
    }
    Some((specs, skels))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn descriptor(parent: Option<u16>, tag: Option<&str>) -> DObjModelDescriptor {
        DObjModelDescriptor {
            model: "m".to_owned(),
            parent_model: parent,
            attach_tag: tag.map(str::to_owned),
            ignore_collision: false,
        }
    }

    #[test]
    fn the_first_model_is_the_root() {
        assert!(matches!(attach_for(0, &descriptor(None, None)), Some(None)));
    }

    #[test]
    fn the_root_cannot_hang_on_anything() {
        assert!(attach_for(0, &descriptor(Some(0), Some("tag"))).is_none());
    }

    #[test]
    fn a_later_model_hangs_on_a_tag_of_an_earlier_one() {
        let Some(Some(attach)) = attach_for(2, &descriptor(Some(1), Some("tag_weapon"))) else {
            panic!("expected an attachment");
        };
        assert_eq!((attach.parent_model, attach.tag.as_str()), (1, "tag_weapon"));
    }

    #[test]
    fn a_later_model_needs_an_earlier_parent_and_a_tag() {
        assert!(attach_for(1, &descriptor(Some(1), Some("tag"))).is_none());
        assert!(attach_for(1, &descriptor(Some(0), None)).is_none());
        assert!(attach_for(1, &descriptor(None, Some("tag"))).is_none());
        assert!(attach_for(1, &descriptor(None, None)).is_none());
    }
}
