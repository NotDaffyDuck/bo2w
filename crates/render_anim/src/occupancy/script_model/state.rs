//! What the script-model systems keep from one frame to the next.

use std::collections::HashMap;
use std::sync::Arc;

use bevy::prelude::*;
use xmodel_runtime::{DObj, DObjReuseKey, DObjSemanticState};

use super::skels::collect_presented_models;
use super::{HeliRotors, TurretGun, VehicleWheels};
use crate::anim::model_materials::PreparedModelMaterials;

/// The DObj built for one owner, and the key it was built under.
pub(super) struct PersistentScriptDobj {
    pub(super) dobj: Arc<DObj>,
    pub(super) reuse_key: DObjReuseKey,
}

/// Per-owner state, keyed by the owner's wire id (`by_id` and the vehicle
/// drivers) or by its map-entity ordinal (`mover_angles`).
#[derive(Resource, Default)]
pub(super) struct ScriptModelDobjs {
    pub(super) by_id: HashMap<u32, PersistentScriptDobj>,
    /// Angles each mover was placed at this frame; a mover absent here had
    /// no centity to be placed by.
    pub(super) mover_angles: HashMap<u32, [f32; 3]>,
    pub(super) wheels: HashMap<u32, VehicleWheels>,
    pub(super) rotors: HashMap<u32, HeliRotors>,
    pub(super) turrets: HashMap<u32, TurretGun>,
}

/// Two states build the same DObj when they stack the same models on the
/// same tags.
pub(super) fn reuse_key_for(state: &DObjSemanticState) -> DObjReuseKey {
    let parts: Vec<&str> = state
        .composition
        .models
        .iter()
        .flat_map(|model| [model.model.as_str(), model.attach_tag.as_deref().unwrap_or("")])
        .collect();
    DObjReuseKey {
        e_type: i32::from(entity_t6::et::SCRIPTMOVER),
        model: xmodel_runtime::model_token(&parts),
    }
}

impl ScriptModelDobjs {
    /// The DObj for owner `id` under `state`: the one already held if it was
    /// built for the same models, else a new one (a lone model's comes
    /// prebuilt from `prepared`). `None` when the models are not all known.
    pub(super) fn dobj_for(
        &mut self,
        id: u32,
        catalog: &asset_world::MapXModelSceneCatalog,
        state: &DObjSemanticState,
        prepared: &PreparedModelMaterials,
    ) -> Option<Arc<DObj>> {
        let wanted = reuse_key_for(state);
        if let Some(held) = self
            .by_id
            .get(&id)
            .filter(|held| xmodel_runtime::reuse_matches(held.reuse_key, wanted))
        {
            return Some(Arc::clone(&held.dobj));
        }
        let (specs, _) = collect_presented_models(catalog, state)?;
        let prebuilt = match state.composition.models.as_slice() {
            [only] => prepared.scene_dobj(&only.model).cloned(),
            _ => None,
        };
        let dobj = match prebuilt {
            Some(dobj) => dobj,
            None => Arc::new(DObj::build(&specs).ok()?),
        };
        self.by_id.insert(
            id,
            PersistentScriptDobj {
                dobj: Arc::clone(&dobj),
                reuse_key: wanted,
            },
        );
        Some(dobj)
    }

    /// Forgets everything about owners that were not posed this frame.
    pub(super) fn keep_only(&mut self, live: &std::collections::HashSet<u32>) {
        self.by_id.retain(|id, _| live.contains(id));
        let Self {
            by_id,
            wheels,
            rotors,
            turrets,
            ..
        } = self;
        wheels.retain(|id, _| by_id.contains_key(id));
        rotors.retain(|id, _| by_id.contains_key(id));
        turrets.retain(|id, _| by_id.contains_key(id));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use xmodel_runtime::{DObjModelDescriptor, DObjSemanticState};

    fn stacked(models: &[(&str, Option<&str>)]) -> DObjSemanticState {
        let mut state = DObjSemanticState::bind_pose(models[0].0.to_owned(), 1, 1);
        state.composition.models = models
            .iter()
            .enumerate()
            .map(|(index, (model, tag))| DObjModelDescriptor {
                model: (*model).to_owned(),
                parent_model: tag.map(|_| 0).filter(|_| index > 0),
                attach_tag: tag.map(str::to_owned),
                ignore_collision: false,
            })
            .collect();
        state
    }

    fn same(a: &DObjSemanticState, b: &DObjSemanticState) -> bool {
        xmodel_runtime::reuse_matches(reuse_key_for(a), reuse_key_for(b))
    }

    #[test]
    fn the_same_models_on_the_same_tags_share_a_dobj() {
        let a = stacked(&[("body", None), ("gun", Some("tag_weapon"))]);
        let mut b = a.clone();
        b.pose_revision += 7;
        assert!(same(&a, &b));
    }

    #[test]
    fn another_model_or_another_tag_needs_its_own_dobj() {
        let base = stacked(&[("body", None), ("gun", Some("tag_weapon"))]);
        assert!(!same(&base, &stacked(&[("body", None), ("rifle", Some("tag_weapon"))])));
        assert!(!same(&base, &stacked(&[("body", None), ("gun", Some("tag_flash"))])));
        assert!(!same(&base, &stacked(&[("body", None)])));
    }

    #[test]
    fn keeping_only_the_live_owners_drops_their_drivers_too() {
        let mut dobjs = ScriptModelDobjs::default();
        for id in [1, 2] {
            dobjs.by_id.insert(
                id,
                PersistentScriptDobj {
                    dobj: Arc::new(DObj::build(&[]).expect("empty dobj")),
                    reuse_key: reuse_key_for(&stacked(&[("body", None)])),
                },
            );
            dobjs.turrets.insert(id, TurretGun::new());
        }
        dobjs.turrets.insert(9, TurretGun::new());
        dobjs.keep_only(&[2].into_iter().collect());
        assert_eq!(dobjs.by_id.keys().copied().collect::<Vec<_>>(), vec![2]);
        assert_eq!(dobjs.turrets.keys().copied().collect::<Vec<_>>(), vec![2]);
    }
}
